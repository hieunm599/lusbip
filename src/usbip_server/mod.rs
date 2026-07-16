//! A library for running a USB/IP server
#![allow(clippy::all)]

use log::*;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use nusb::transfer::{Buffer, Bulk, Direction, In};
use nusb::{DeviceInfo, Interface, Speed};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinSet;
use usbip_protocol::UsbIpCommand;

pub mod cdc;
mod consts;
mod device;
mod endpoint;
pub mod hid;
mod host;
mod interface;
mod setup;
pub mod usbip_protocol;
mod util;
pub use consts::*;
pub use device::*;
pub use endpoint::*;
pub use host::*;
pub use interface::*;
pub use setup::*;
pub use util::*;

use crate::usbip_server::usbip_protocol::{
    USBIP_RET_SUBMIT, USBIP_RET_UNLINK, UsbIpHeaderBasic, UsbIpResponse,
};

pub type OccupancyMap = Arc<RwLock<HashMap<String, SocketAddr>>>;

#[derive(Default)]
struct PendingBulkIn {
    seqnums: VecDeque<u32>,
}

impl PendingBulkIn {
    fn push(&mut self, seqnum: u32) {
        self.seqnums.push_back(seqnum);
    }

    #[cfg(test)]
    fn complete_next(&mut self) -> Option<u32> {
        self.seqnums.pop_front()
    }

    fn next(&self) -> Option<u32> {
        self.seqnums.front().copied()
    }

    fn is_empty(&self) -> bool {
        self.seqnums.is_empty()
    }

    fn unlink(&mut self, seqnum: u32) -> bool {
        let Some(index) = self.seqnums.iter().position(|pending| *pending == seqnum) else {
            return false;
        };
        self.seqnums.remove(index);
        true
    }
}

struct BulkInRequest {
    header: UsbIpHeaderBasic,
    transfer_buffer_length: u32,
}

enum BulkInCommand {
    Submit(BulkInRequest),
    Unlink(u32),
}

struct BulkInEvent {
    seqnum: u32,
    response: UsbIpResponse,
}

#[derive(Default)]
struct BulkInState {
    read_pending: bool,
}

impl BulkInState {
    fn start_if_idle(&mut self) -> bool {
        if self.read_pending {
            return false;
        }

        self.read_pending = true;
        true
    }

    fn complete(&mut self) {
        self.read_pending = false;
    }
}

fn bulk_in_response_header(mut header: UsbIpHeaderBasic) -> UsbIpHeaderBasic {
    header.command = USBIP_RET_SUBMIT.into();
    header.devid = 0;
    header.direction = 0;
    header.ep = 0;
    header
}

fn should_write_bulk_in_response(cancelled: &mut HashSet<u32>, seqnum: u32) -> bool {
    !cancelled.remove(&seqnum)
}

async fn run_bulk_in_worker(
    interface: Interface,
    endpoint: UsbEndpoint,
    mut commands: mpsc::Receiver<BulkInCommand>,
    events: mpsc::UnboundedSender<BulkInEvent>,
) {
    let mut endpoint_in = match interface.endpoint::<Bulk, In>(endpoint.address) {
        Ok(endpoint_in) => endpoint_in,
        Err(err) => {
            warn!(
                "Unable to open persistent bulk IN endpoint 0x{:02x}: {err}",
                endpoint.address
            );
            return;
        }
    };
    let max_packet_size = endpoint_in.max_packet_size();
    let mut pending = PendingBulkIn::default();
    let mut requests = HashMap::<u32, BulkInRequest>::new();
    let mut state = BulkInState::default();
    let mut active_seqnum = None;

    loop {
        if !state.read_pending && !pending.is_empty() {
            let seqnum = pending.next().expect("non-empty pending queue");
            let Some(request) = requests.get(&seqnum) else {
                pending.unlink(seqnum);
                continue;
            };
            let requested_len = request.transfer_buffer_length.max(1) as usize;
            let transfer_len = requested_len.div_ceil(max_packet_size) * max_packet_size;
            endpoint_in.submit(Buffer::new(transfer_len));
            active_seqnum = Some(seqnum);
            state.start_if_idle();
        }

        if state.read_pending {
            tokio::select! {
                completion = endpoint_in.next_complete() => {
                    state.complete();
                    let Some(seqnum) = active_seqnum.take() else {
                        continue;
                    };
                    let still_pending = pending.unlink(seqnum);
                    let request = requests.remove(&seqnum);
                    if !still_pending {
                        continue;
                    }
                    let Some(request) = request else {
                        continue;
                    };
                    let header = bulk_in_response_header(request.header);
                    let response = match completion.into_result() {
                        Ok(buffer) => UsbIpResponse::usbip_ret_submit_success(
                            &header,
                            0,
                            buffer.len() as u32,
                            buffer.into_vec(),
                            vec![],
                        ),
                        Err(err) => {
                            warn!("Bulk IN transfer on endpoint 0x{:02x} failed: {err}", endpoint.address);
                            UsbIpResponse::usbip_ret_submit_fail(&header, 0)
                        }
                    };
                    if events.send(BulkInEvent { seqnum, response }).is_err() {
                        return;
                    }
                }
                command = commands.recv() => {
                    let Some(command) = command else {
                        endpoint_in.cancel_all();
                        return;
                    };
                    match command {
                        BulkInCommand::Submit(request) => {
                            let seqnum = request.header.seqnum;
                            requests.insert(seqnum, request);
                            pending.push(seqnum);
                        }
                        BulkInCommand::Unlink(seqnum) => {
                            let was_active = active_seqnum == Some(seqnum);
                            pending.unlink(seqnum);
                            requests.remove(&seqnum);
                            if was_active {
                                endpoint_in.cancel_all();
                            }
                        }
                    }
                }
            }
        } else {
            let Some(command) = commands.recv().await else {
                return;
            };
            match command {
                BulkInCommand::Submit(request) => {
                    let seqnum = request.header.seqnum;
                    requests.insert(seqnum, request);
                    pending.push(seqnum);
                }
                BulkInCommand::Unlink(_) => {}
            }
        }
    }
}

fn start_bulk_in_workers(
    device: &UsbDevice,
    events: mpsc::UnboundedSender<BulkInEvent>,
    tasks: &mut JoinSet<()>,
) -> HashMap<u8, mpsc::Sender<BulkInCommand>> {
    let mut workers = HashMap::new();

    for interface in &device.interfaces {
        for endpoint in &interface.endpoints {
            if endpoint.attributes != EndpointAttributes::Bulk as u8
                || endpoint.direction() != Direction::In
            {
                continue;
            }

            let (commands, receiver) = mpsc::channel(32);
            tasks.spawn(run_bulk_in_worker(
                interface.handler.clone(),
                *endpoint,
                receiver,
                events.clone(),
            ));
            workers.insert(endpoint.address, commands);
        }
    }

    workers
}

async fn stop_bulk_in_workers(tasks: &mut JoinSet<()>) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

/// Main struct of a USB/IP server
#[derive(Default, Debug)]
pub struct UsbIpServer {
    available_devices: RwLock<Vec<UsbDevice>>,
    used_devices: RwLock<Vec<UsbDevice>>,
}

fn prepare_device_for_import(device: UsbDevice) -> UsbDevice {
    device
}

fn extract_configuration_descriptors(descriptors: &[u8]) -> Vec<Vec<u8>> {
    let mut configurations = Vec::new();
    let mut offset = 0;

    while offset + 2 <= descriptors.len() {
        let descriptor_length = descriptors[offset] as usize;
        if descriptor_length < 2 || offset + descriptor_length > descriptors.len() {
            break;
        }

        if descriptors[offset + 1] == DescriptorType::Configuration as u8 && descriptor_length >= 9
        {
            let total_length =
                u16::from_le_bytes([descriptors[offset + 2], descriptors[offset + 3]]) as usize;
            if total_length >= descriptor_length && offset + total_length <= descriptors.len() {
                configurations.push(descriptors[offset..offset + total_length].to_vec());
                offset += total_length;
                continue;
            }
        }

        offset += descriptor_length;
    }

    configurations
}

impl UsbIpServer {
    /// Create a [UsbIpServer] with simulated devices
    pub fn new_simulated(devices: Vec<UsbDevice>) -> Self {
        Self {
            available_devices: RwLock::new(devices),
            used_devices: RwLock::new(Vec::new()),
        }
    }

    /// Create a [UsbIpServer] with Vec<[nusb::DeviceInfo]> for sharing host devices
    pub async fn with_nusb_devices(nusb_device_infos: Vec<nusb::DeviceInfo>) -> Vec<UsbDevice> {
        let mut devices = vec![];
        for device_info in nusb_device_infos {
            let dev = match device_info.open().await {
                Ok(dev) => dev,
                Err(err) => {
                    warn!("Impossible to open device {device_info:?}: {err}, ignoring device",);
                    continue;
                }
            };

            #[cfg(target_os = "linux")]
            let path = device_info.sysfs_path().to_path_buf();
            #[cfg(not(target_os = "linux"))]
            let path = device_info.bus_id().to_string();
            #[cfg(target_os = "linux")]
            let bus_id = match path.file_name() {
                Some(s) => s.to_os_string().into_string().unwrap_or(format!(
                    "{}-{}-{}",
                    device_info.busnum(),
                    device_info.device_address(),
                    0,
                )),
                None => format!(
                    "{}-{}-{}",
                    device_info.busnum(),
                    device_info.device_address(),
                    0,
                ),
            };
            #[cfg(not(target_os = "linux"))]
            let bus_id = device_info.bus_id().to_string();

            #[cfg(target_os = "linux")]
            let bus_num = device_info.busnum() as u32;
            #[cfg(not(target_os = "linux"))]
            let bus_num = 0u32;
            let cfg = match dev.active_configuration() {
                Ok(cfg) => cfg,
                Err(err) => {
                    warn!(
                        "Impossible to get active configuration {device_info:?}: {err}, ignoring device",
                    );
                    continue;
                }
            };
            let attributes = cfg.attributes();
            let max_power = cfg.max_power();
            #[cfg(target_os = "linux")]
            let raw_configuration_descriptors = match std::fs::read(path.join("descriptors")) {
                Ok(descriptors) => extract_configuration_descriptors(&descriptors),
                Err(err) => {
                    warn!("Unable to read host descriptors for {device_info:?}: {err}");
                    Vec::new()
                }
            };
            #[cfg(not(target_os = "linux"))]
            let raw_configuration_descriptors = Vec::new();
            let mut interfaces = vec![];
            for intf in cfg.interfaces() {
                // ignore alternate settings
                let intf_num = intf.interface_number();

                #[cfg(target_os = "linux")]
                let _ = dev.detach_kernel_driver(intf_num);

                let intf = match dev.claim_interface(intf_num).await {
                    Ok(intf) => intf,
                    Err(err) => {
                        warn!(
                            "Impossible to claim interface {intf_num} for device {device_info:?}: {err}, ignoring interface",
                        );
                        continue;
                    }
                };
                let intf_desc = match intf.descriptor() {
                    Some(desc) => desc,
                    None => {
                        warn!(
                            "Impossible to get descriptor for interface {intf_num} on device {device_info:?}, ignoring interface",
                        );
                        continue;
                    }
                };

                let mut endpoints = vec![];

                for ep_desc in intf_desc.endpoints() {
                    endpoints.push(UsbEndpoint {
                        address: ep_desc.address(),
                        attributes: ep_desc.transfer_type() as u8,
                        max_packet_size: ep_desc.max_packet_size() as u16,
                        interval: ep_desc.interval(),
                    });
                }

                let handler = intf.clone();

                interfaces.push(UsbInterface {
                    interface_class: intf_desc.class(),
                    interface_subclass: intf_desc.subclass(),
                    interface_protocol: intf_desc.protocol(),
                    endpoints,
                    string_interface: match intf_desc.string_index() {
                        Some(i) => i.into(),
                        None => 0,
                    },
                    class_specific_descriptor: Vec::new(),
                    handler,
                });
            }

            if interfaces.is_empty() {
                warn!("No claimable interfaces found for device {device_info:?}, ignoring device",);
                continue;
            }

            let speed = match device_info.speed() {
                Some(s) => match s {
                    Speed::Low => 1u32,
                    Speed::Full => 2,
                    Speed::High => 3,
                    Speed::Super => 5,
                    Speed::SuperPlus => 6,
                    _ => s as u32 + 1,
                },
                None => 0u32,
            };
            let mut device = UsbDevice {
                path,
                bus_id,
                bus_num,
                dev_num: device_info.device_address() as u32,
                speed,
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
                device_class: device_info.class(),
                device_subclass: device_info.subclass(),
                device_protocol: device_info.protocol(),
                device_bcd: device_info.device_version().into(),
                configuration_value: cfg.configuration_value(),
                num_configurations: dev.configurations().count() as u8,
                ep0_in: UsbEndpoint {
                    address: 0x80,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: EP0_MAX_PACKET_SIZE,
                    interval: 0,
                },
                ep0_out: UsbEndpoint {
                    address: 0x00,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: EP0_MAX_PACKET_SIZE,
                    interval: 0,
                },
                interfaces,
                raw_configuration_descriptors,
                device_handler: Some(dev),
                usb_version: device_info.usb_version().into(),
                attributes,
                max_power,
                ..UsbDevice::default()
            };

            // set strings
            if let Some(s) = device_info.manufacturer_string() {
                device.string_manufacturer = device.new_string(s)
            }
            if let Some(s) = device_info.product_string() {
                device.string_product = device.new_string(s)
            }
            if let Some(s) = device_info.serial_number() {
                device.string_serial = device.new_string(s)
            }
            devices.push(device);
        }
        devices
    }

    /// Create a [UsbIpServer] exposing devices in the host, and redirect all USB transfers to them using libusb
    pub async fn new_from_host() -> Self {
        Self::new_from_host_with_filter(|_| true).await
    }

    /// Create a [UsbIpServer] exposing filtered devices in the host, and redirect all USB transfers to them using libusb
    pub async fn new_from_host_with_filter<F>(filter: F) -> Self
    where
        F: FnMut(&DeviceInfo) -> bool,
    {
        match nusb::list_devices().await {
            Ok(list) => {
                let devs: Vec<DeviceInfo> = list
                    .filter(|device| !is_usb_hub_info(device))
                    .filter(filter)
                    .collect();
                // info!("devices: {devs:?}");
                Self {
                    available_devices: RwLock::new(Self::with_nusb_devices(devs).await),
                    ..Default::default()
                }
            }
            Err(_) => Default::default(),
        }
    }

    pub async fn add_device(&self, device: UsbDevice) {
        self.available_devices.write().await.push(device);
    }

    pub async fn sync_from_host_with_filter<F>(&self, mut filter: F) -> Result<HashSet<String>>
    where
        F: FnMut(&DeviceInfo) -> bool,
    {
        let current_infos = nusb::list_devices()
            .await?
            .filter(|device| !is_usb_hub_info(device))
            .filter(|device| filter(device))
            .collect::<Vec<_>>();
        let current_bus_ids = current_infos
            .iter()
            .map(host_export_bus_id)
            .collect::<HashSet<_>>();

        let known_bus_ids = {
            let mut available_devices = self.available_devices.write().await;
            let mut used_devices = self.used_devices.write().await;

            available_devices.retain(|device| current_bus_ids.contains(&device.bus_id));
            used_devices.retain(|device| current_bus_ids.contains(&device.bus_id));
            dedup_devices_by_bus_id(&mut available_devices);
            dedup_devices_by_bus_id(&mut used_devices);

            available_devices
                .iter()
                .chain(used_devices.iter())
                .map(|device| device.bus_id.clone())
                .collect::<HashSet<_>>()
        };

        let new_infos = current_infos
            .into_iter()
            .filter(|device| !known_bus_ids.contains(&host_export_bus_id(device)))
            .collect::<Vec<_>>();
        let mut new_devices = Self::with_nusb_devices(new_infos).await;
        if !new_devices.is_empty() {
            self.available_devices
                .write()
                .await
                .append(&mut new_devices);
        }

        Ok(current_bus_ids)
    }

    pub async fn remove_device(&self, bus_id: &str) -> Result<()> {
        let mut available_devices = self.available_devices.write().await;

        if let Some(i) = available_devices.iter().position(|d| d.bus_id == bus_id) {
            #[cfg(target_os = "linux")]
            if let Some(dev) = available_devices[i].device_handler.clone() {
                release_claim(dev);
            }
            available_devices.remove(i);
            Ok(())
        } else if self
            .used_devices
            .read()
            .await
            .iter()
            .any(|d| d.bus_id == bus_id)
        {
            Err(std::io::Error::other(format!(
                "Device {} is in use",
                bus_id
            )))
        } else {
            Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("Device {bus_id} not found"),
            ))
        }
    }

    pub async fn occupy(&self, bus_id: &str) -> Result<UsbDevice> {
        let mut ad = self.available_devices.write().await;
        let device = match ad.iter().position(|d| d.bus_id == bus_id) {
            Some(i) => ad.remove(i),
            None => return Err(std::io::Error::other(format!("No available device"))),
        };
        drop(ad);
        let device = prepare_device_for_import(device);

        let mut ud = self.used_devices.write().await;
        if !ud.iter().any(|d| d.bus_id == device.bus_id) {
            ud.push(device.clone());
        }
        Ok(device)
    }

    pub async fn release(&self, device: UsbDevice) {
        let mut ud = self.used_devices.write().await;
        let mut ad = self.available_devices.write().await;
        let new_vec = ud.clone();
        let new_ud: Vec<UsbDevice> = new_vec
            .into_iter()
            .filter(|d| d.bus_id != device.bus_id)
            .collect();
        if !ad.iter().any(|d| d.bus_id == device.bus_id) {
            ad.push(device);
        }
        *ud = new_ud;
    }

    /// Reclaim the detached os driver.
    pub async fn cleanup(&self) {
        let mut ud = self.used_devices.write().await;
        let mut ad = self.available_devices.write().await;
        for d in ud.clone() {
            if !ad.iter().any(|dev| d.bus_id == dev.bus_id) {
                ad.push(d);
            }
        }
        *ud = Vec::new();
        #[cfg(target_os = "linux")]
        {
            for d in ad.iter() {
                if let Some(dh) = d.device_handler.clone() {
                    release_claim(dh);
                }
            }
            *ad = Vec::new();
        }
    }

    pub async fn handle_op_req_devlist(&self) -> Result<UsbIpResponse> {
        trace!("Got OP_REQ_DEVLIST");
        let devices = unique_devices_by_bus_id(self.available_devices.read().await.clone());

        // OP_REP_DEVLIST
        let usbip_resp = UsbIpResponse::op_rep_devlist(&devices);
        trace!("Sent OP_REP_DEVLIST");
        Ok(usbip_resp)
    }

    pub async fn handle_op_req_devlist_with_occupancy(
        &self,
        occupancy: &OccupancyMap,
    ) -> Result<UsbIpResponse> {
        trace!("Got OP_REQ_DEVLIST");
        let mut devices = self.available_devices.read().await.clone();
        let occupancy_snapshot = occupancy.read().await.clone();
        let used_devices = self.used_devices.read().await;

        for device in used_devices.iter() {
            let mut occupied = device.clone();
            if let Some(peer) = occupancy_snapshot.get(&occupied.bus_id) {
                let product = occupied
                    .string_pool
                    .get(&occupied.string_product)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                occupied.string_product =
                    occupied.new_string(&format!("{product} [occupied by {}]", peer.ip()));
            }
            devices.retain(|device| device.bus_id != occupied.bus_id);
            devices.push(occupied);
        }
        let devices = unique_devices_by_bus_id(devices);

        let usbip_resp = UsbIpResponse::op_rep_devlist(&devices);
        trace!("Sent OP_REP_DEVLIST");
        Ok(usbip_resp)
    }

    pub async fn handle_op_req_import(
        &self,
        busid: [u8; 32],
        imported_device: &mut Option<UsbDevice>,
    ) -> Result<UsbIpResponse> {
        trace!("Got OP_REQ_IMPORT");

        let trimmed_busid = &busid[..busid.iter().position(|&x| x == 0).unwrap_or(busid.len())];
        let bus_id = match str::from_utf8(trimmed_busid) {
            Ok(s) => s,
            Err(_e) => return Err(std::io::Error::other(format!("Invalid bus id: {busid:?}"))),
        };

        match imported_device.take() {
            Some(dev) => self.release(dev).await,
            None => (),
        }

        let usbip_resp = match self.occupy(bus_id).await {
            Ok(dev) => {
                let res = UsbIpResponse::op_rep_import_success(&dev);
                *imported_device = Some(dev);
                res
            }
            Err(_) => UsbIpResponse::op_rep_import_fail(),
        };

        trace!("Sent OP_REP_IMPORT");
        Ok(usbip_resp)
    }

    pub fn handle_usbip_cmd_submit(
        &self,
        mut header: UsbIpHeaderBasic,
        transfer_buffer_length: u32,
        setup: [u8; 8],
        data: Vec<u8>,
        device: &UsbDevice,
    ) -> Result<UsbIpResponse> {
        let out = header.direction == 0;
        let real_ep = if out { header.ep } else { header.ep | 0x80 };

        header.command = USBIP_RET_SUBMIT.into();

        // Reply header from server should have devid/direction/ep all 0.
        header.devid = 0;
        header.direction = 0;
        header.ep = 0;

        let usbip_resp = match device.find_ep(real_ep as u8) {
            None => {
                warn!("Endpoint {real_ep:02x?} not found");
                UsbIpResponse::usbip_ret_submit_fail(&header, 0)
            }
            Some((ep, intf)) => {
                match device.handle_urb(
                    ep,
                    intf,
                    transfer_buffer_length,
                    SetupPacket::parse(&setup),
                    &data,
                ) {
                    Ok(resp) => {
                        if out {
                            trace!("<-Wrote {}", data.len());
                        } else {
                            trace!("<-Resp {resp:02x?}");
                        }
                        let actual_length = match ep.direction() {
                            Direction::In => resp.len() as u32,
                            Direction::Out => transfer_buffer_length,
                        };
                        UsbIpResponse::usbip_ret_submit_success(
                            &header,
                            0,
                            actual_length,
                            resp,
                            vec![],
                        )
                    }
                    Err(err) => {
                        warn!(
                            "Error handling URB: {err}; real_ep=0x{real_ep:02x} attr={} dir={} transfer_len={} setup={:?} data_len={}",
                            ep.attributes,
                            if out { "out" } else { "in" },
                            transfer_buffer_length,
                            SetupPacket::parse(&setup),
                            data.len()
                        );
                        let actual_length = match ep.direction() {
                            Direction::In => 0,
                            Direction::Out => transfer_buffer_length,
                        };
                        UsbIpResponse::usbip_ret_submit_fail(&header, actual_length)
                    }
                }
            }
        };
        trace!("Sent USBIP_RET_SUBMIT");
        Ok(usbip_resp)
    }

    pub fn handle_usbip_cmd_unlink(
        &self,
        mut header: UsbIpHeaderBasic,
        unlink_seqnum: u32,
        was_pending: bool,
    ) -> Result<UsbIpResponse> {
        trace!("Got USBIP_CMD_UNLINK for {unlink_seqnum:10x?}");

        header.command = USBIP_RET_UNLINK.into();
        // Reply header from server should have devid/direction/ep all 0.
        header.devid = 0;
        header.direction = 0;
        header.ep = 0;

        let res = if was_pending {
            UsbIpResponse::usbip_ret_unlink_success(&header)
        } else {
            UsbIpResponse::usbip_ret_unlink_fail(&header)
        };
        trace!("Sent USBIP_RET_UNLINK");
        Ok(res)
    }
}

fn host_export_bus_id(device_info: &DeviceInfo) -> String {
    #[cfg(target_os = "linux")]
    {
        device_info
            .sysfs_path()
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{}-{}-{}",
                    device_info.busnum(),
                    device_info.device_address(),
                    0,
                )
            })
    }

    #[cfg(not(target_os = "linux"))]
    {
        device_info.bus_id().to_string()
    }
}

fn is_usb_hub_info(device_info: &DeviceInfo) -> bool {
    device_info.class() == 0x09
        || device_info
            .interfaces()
            .any(|interface| interface.class() == 0x09)
}

fn unique_devices_by_bus_id(devices: Vec<UsbDevice>) -> Vec<UsbDevice> {
    let mut seen = HashSet::<String>::new();
    devices
        .into_iter()
        .filter(|device| seen.insert(device.bus_id.clone()))
        .collect()
}

fn dedup_devices_by_bus_id(devices: &mut Vec<UsbDevice>) {
    let mut seen = HashSet::<String>::new();
    devices.retain(|device| seen.insert(device.bus_id.clone()));
}

pub async fn handler<T: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut T,
    server: Arc<UsbIpServer>,
    imported_device: &mut Option<UsbDevice>,
    occupancy: Option<(SocketAddr, OccupancyMap)>,
) -> Result<()> {
    let (mut reader, mut writer) = tokio::io::split(socket);
    let (bulk_in_events, mut bulk_in_events_rx) = mpsc::unbounded_channel::<BulkInEvent>();
    let mut bulk_in_workers = HashMap::<u8, mpsc::Sender<BulkInCommand>>::new();
    let mut bulk_in_seqnums = HashMap::<u32, u8>::new();
    let mut cancelled_bulk_in_seqnums = HashSet::<u32>::new();
    let mut bulk_in_tasks = JoinSet::new();

    loop {
        tokio::select! {
            command = UsbIpCommand::read_from_socket(&mut reader) => {
                let command = match command {
                    Ok(command) => command,
                    Err(err) => {
                        stop_bulk_in_workers(&mut bulk_in_tasks).await;
                        if let Some(dev) = imported_device.take() {
                            clear_occupancy(&occupancy, &dev.bus_id).await;
                            server.release(dev).await;
                        }
                        if err.kind() == ErrorKind::UnexpectedEof {
                            info!("Remote closed the connection");
                            return Ok(());
                        }
                        return Err(err);
                    }
                };

                match command {
            UsbIpCommand::OpReqDevlist { .. } => {
                let response = match &occupancy {
                    Some((_, occupancy)) => {
                        server.handle_op_req_devlist_with_occupancy(occupancy).await
                    }
                    None => server.handle_op_req_devlist().await,
                };
                match response {
                    Ok(r) => {
                        r.write_to_socket(&mut writer).await?;
                    }
                    Err(e) => error!("UsbipCommand OpReqDevlist handling error: {e:?}"),
                }
            }
            UsbIpCommand::OpReqImport { busid, .. } => {
                stop_bulk_in_workers(&mut bulk_in_tasks).await;
                bulk_in_workers.clear();
                bulk_in_seqnums.clear();
                cancelled_bulk_in_seqnums.clear();
                let previous_bus_id = imported_device.as_ref().map(|dev| dev.bus_id.clone());
                match server.handle_op_req_import(busid, imported_device).await {
                    Ok(r) => {
                        r.write_to_socket(&mut writer).await?;
                        if let Some(bus_id) = previous_bus_id {
                            clear_occupancy(&occupancy, &bus_id).await;
                        }
                        if let Some(dev) = imported_device.as_ref() {
                            set_occupancy(&occupancy, dev.bus_id.clone()).await;
                            bulk_in_workers = start_bulk_in_workers(
                                dev,
                                bulk_in_events.clone(),
                                &mut bulk_in_tasks,
                            );
                        }
                    }
                    Err(e) => {
                        error!("UsbipCommand OpReqImport handling error: {e:?}");
                        if let Some(dev) = imported_device.take() {
                            clear_occupancy(&occupancy, &dev.bus_id).await;
                            server.release(dev).await;
                        }
                    }
                }
                info!("Imported device: {imported_device:?}");
            }
            UsbIpCommand::UsbIpCmdSubmit {
                header,
                transfer_buffer_length,
                setup,
                data,
                ..
            } => {
                let device = match imported_device.as_ref() {
                    Some(d) => d,
                    None => {
                        error!("No device currently imported");
                        continue;
                    }
                };
                let real_ep = if header.direction == 0 {
                    header.ep as u8
                } else {
                    (header.ep | 0x80) as u8
                };
                if header.direction == 1 {
                    if let Some(worker) = bulk_in_workers.get(&real_ep) {
                        let seqnum = header.seqnum;
                        let request = BulkInRequest {
                            header: header.clone(),
                            transfer_buffer_length,
                        };
                        if worker.send(BulkInCommand::Submit(request)).await.is_ok() {
                            bulk_in_seqnums.insert(seqnum, real_ep);
                            continue;
                        }
                        warn!("Persistent bulk IN worker for endpoint 0x{real_ep:02x} stopped");
                    }
                }
                match server.handle_usbip_cmd_submit(
                    header,
                    transfer_buffer_length,
                    setup,
                    data,
                    device,
                ) {
                    Ok(r) => r.write_to_socket(&mut writer).await?,
                    Err(e) => error!("UsbipCmdSubmit handling error: {e:?}"),
                }
            }
            UsbIpCommand::UsbIpCmdUnlink {
                header,
                unlink_seqnum,
            } => {
                let was_pending = if let Some(endpoint) = bulk_in_seqnums.remove(&unlink_seqnum) {
                    cancelled_bulk_in_seqnums.insert(unlink_seqnum);
                    if let Some(worker) = bulk_in_workers.get(&endpoint) {
                        let _ = worker.send(BulkInCommand::Unlink(unlink_seqnum)).await;
                    }
                    true
                } else {
                    false
                };
                match server.handle_usbip_cmd_unlink(header, unlink_seqnum, was_pending) {
                    Ok(r) => r.write_to_socket(&mut writer).await?,
                    Err(e) => error!("UsbipCmdUnlink handling error: {e:?}"),
                }
            }
                }
            }
            Some(event) = bulk_in_events_rx.recv() => {
                bulk_in_seqnums.remove(&event.seqnum);
                if should_write_bulk_in_response(&mut cancelled_bulk_in_seqnums, event.seqnum) {
                    event.response.write_to_socket(&mut writer).await?;
                }
            }
        }
    }
}

async fn set_occupancy(occupancy: &Option<(SocketAddr, OccupancyMap)>, bus_id: String) {
    if let Some((peer_addr, occupancy)) = occupancy {
        occupancy.write().await.insert(bus_id, *peer_addr);
    }
}

async fn clear_occupancy(occupancy: &Option<(SocketAddr, OccupancyMap)>, bus_id: &str) {
    if let Some((_, occupancy)) = occupancy {
        occupancy.write().await.remove(bus_id);
    }
}

/// Spawn a USB/IP server at `addr` using [TcpListener]
pub async fn server(addr: SocketAddr, server: Arc<UsbIpServer>) {
    let listener = TcpListener::bind(addr).await.expect("bind to addr");

    while let Ok((mut socket, _addr)) = listener.accept().await {
        if let Err(err) = socket.set_nodelay(true) {
            warn!("Unable to enable TCP_NODELAY for USB/IP client: {err}");
        }
        info!("Got connection from {:?}", socket.peer_addr());
        let new_server = server.clone();
        tokio::spawn(async move {
            let mut imported_device: Box<Option<UsbDevice>> = Box::new(None);
            let res = handler(&mut socket, new_server.clone(), &mut imported_device, None).await;
            info!("Handler ended with {res:?}");
            if let Some(dev) = imported_device.take() {
                new_server.release(dev).await;
            }
        });
    }
}

/// Spawn a USB/IP server and track which peer occupies each bus id.
pub async fn server_with_occupancy(
    addr: SocketAddr,
    server: Arc<UsbIpServer>,
    occupancy: OccupancyMap,
) {
    let listener = TcpListener::bind(addr).await.expect("bind to addr");
    server_with_occupancy_listener(listener, server, occupancy).await;
}

/// Run a USB/IP server from an already-bound listener and track which peer occupies each bus id.
pub async fn server_with_occupancy_listener(
    listener: TcpListener,
    server: Arc<UsbIpServer>,
    occupancy: OccupancyMap,
) {
    while let Ok((mut socket, _addr)) = listener.accept().await {
        if let Err(err) = socket.set_nodelay(true) {
            warn!("Unable to enable TCP_NODELAY for USB/IP client: {err}");
        }
        let peer_addr = match socket.peer_addr() {
            Ok(addr) => addr,
            Err(err) => {
                warn!("Unable to read peer address: {err}");
                continue;
            }
        };
        info!("Got connection from {peer_addr:?}");
        let new_server = server.clone();
        let new_occupancy = occupancy.clone();
        tokio::spawn(async move {
            let mut imported_device: Box<Option<UsbDevice>> = Box::new(None);
            let tracked_peer = Some((peer_addr, new_occupancy.clone()));
            let res = handler(
                &mut socket,
                new_server.clone(),
                &mut imported_device,
                tracked_peer.clone(),
            )
            .await;
            info!("Handler ended with {res:?}");
            if let Some(dev) = imported_device.take() {
                clear_occupancy(&tracked_peer, &dev.bus_id).await;
                new_server.release(dev).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BulkInState, PendingBulkIn, UsbDevice, extract_configuration_descriptors,
        prepare_device_for_import, should_write_bulk_in_response,
    };
    use std::collections::HashSet;

    #[test]
    fn pending_bulk_in_completes_in_submission_order() {
        let mut pending = PendingBulkIn::default();
        pending.push(10);
        pending.push(20);

        assert_eq!(pending.complete_next(), Some(10));
        assert_eq!(pending.complete_next(), Some(20));
    }

    #[test]
    fn pending_bulk_in_unlink_removes_only_target_request() {
        let mut pending = PendingBulkIn::default();
        pending.push(10);
        pending.push(20);

        assert!(pending.unlink(10));
        assert_eq!(pending.complete_next(), Some(20));
    }

    #[test]
    fn bulk_in_worker_submits_once_while_a_read_is_pending() {
        let mut state = BulkInState::default();

        assert!(state.start_if_idle());
        assert!(!state.start_if_idle());
        state.complete();
        assert!(state.start_if_idle());
    }

    #[test]
    fn cancelled_bulk_in_response_is_not_written() {
        let mut cancelled = HashSet::from([20]);

        assert!(!should_write_bulk_in_response(&mut cancelled, 20));
        assert!(should_write_bulk_in_response(&mut cancelled, 21));
    }

    #[test]
    fn import_preserves_the_existing_claimed_device() {
        let device = UsbDevice {
            bus_id: "7-1".into(),
            ..UsbDevice::default()
        };

        let prepared = prepare_device_for_import(device.clone());

        assert_eq!(prepared.bus_id, device.bus_id);
    }

    #[test]
    fn extracts_each_complete_configuration_descriptor_from_sysfs_bytes() {
        let descriptors = vec![
            18, 1, 0, 2, 0, 0, 0, 64, 0x3a, 0x30, 1, 0x10, 2, 1, 1, 2, 3, 1, 9, 2, 18, 0, 1, 1, 0,
            0x80, 50, 9, 4, 0, 0, 0, 2, 2, 0, 0, 9, 2, 9, 0, 0, 2, 0, 0x80, 100,
        ];

        assert_eq!(
            extract_configuration_descriptors(&descriptors),
            vec![
                vec![9, 2, 18, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 0, 2, 2, 0, 0],
                vec![9, 2, 9, 0, 0, 2, 0, 0x80, 100],
            ]
        );
    }
}
