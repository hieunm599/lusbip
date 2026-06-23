//! A library for running a USB/IP server
#![allow(clippy::all)]

use log::*;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use nusb::transfer::Direction;
use nusb::{DeviceInfo, Speed};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
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

/// Main struct of a USB/IP server
#[derive(Default, Debug)]
pub struct UsbIpServer {
    available_devices: RwLock<Vec<UsbDevice>>,
    used_devices: RwLock<Vec<UsbDevice>>,
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
        let mut device = match ad.iter().position(|d| d.bus_id == bus_id) {
            Some(i) => ad.remove(i),
            None => return Err(std::io::Error::other(format!("No available device"))),
        };
        drop(ad);

        if let Some(reopened_device) = reset_and_reopen_host_device(&device).await {
            device = reopened_device;
        }

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
        let devices = self.available_devices.read().await;

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
            devices.push(occupied);
        }

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
    ) -> Result<UsbIpResponse> {
        trace!("Got USBIP_CMD_UNLINK for {unlink_seqnum:10x?}");

        header.command = USBIP_RET_UNLINK.into();
        // Reply header from server should have devid/direction/ep all 0.
        header.devid = 0;
        header.direction = 0;
        header.ep = 0;

        let res = UsbIpResponse::usbip_ret_unlink_success(&header);
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

async fn reset_and_reopen_host_device(device: &UsbDevice) -> Option<UsbDevice> {
    let bus_id = device.bus_id.clone();
    if let Some(handle) = device.device_handler.clone() {
        if let Err(err) = handle.reset().await {
            warn!("USB device {bus_id} reset returned an error, attempting reopen anyway: {err}");
        }
        tokio::time::sleep(Duration::from_millis(3000)).await;
    }

    let serial = device
        .string_pool
        .get(&device.string_serial)
        .map(String::as_str);
    let device_info = match nusb::list_devices().await.ok()?.find(|info| {
        host_export_bus_id(info) == bus_id
            || (info.vendor_id() == device.vendor_id
                && info.product_id() == device.product_id
                && serial.is_none_or(|serial| info.serial_number() == Some(serial)))
    }) {
        Some(info) => info,
        None => {
            warn!("Unable to find USB device {bus_id} after reset");
            return None;
        }
    };

    UsbIpServer::with_nusb_devices(vec![device_info])
        .await
        .into_iter()
        .next()
}

pub async fn handler<T: AsyncReadExt + AsyncWriteExt + Unpin>(
    socket: &mut T,
    server: Arc<UsbIpServer>,
    imported_device: &mut Option<UsbDevice>,
    occupancy: Option<(SocketAddr, OccupancyMap)>,
) -> Result<()> {
    loop {
        let command = match UsbIpCommand::read_from_socket(socket).await {
            Ok(c) => c,
            Err(err) => {
                if let Some(dev) = imported_device.take() {
                    clear_occupancy(&occupancy, &dev.bus_id).await;
                    server.release(dev).await;
                }
                if err.kind() == ErrorKind::UnexpectedEof {
                    info!("Remote closed the connection");
                    return Ok(());
                } else {
                    return Err(err);
                }
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
                        r.write_to_socket(socket).await?;
                    }
                    Err(e) => error!("UsbipCommand OpReqDevlist handling error: {e:?}"),
                }
            }
            UsbIpCommand::OpReqImport { busid, .. } => {
                let previous_bus_id = imported_device.as_ref().map(|dev| dev.bus_id.clone());
                match server.handle_op_req_import(busid, imported_device).await {
                    Ok(r) => {
                        r.write_to_socket(socket).await?;
                        if let Some(bus_id) = previous_bus_id {
                            clear_occupancy(&occupancy, &bus_id).await;
                        }
                        if let Some(dev) = imported_device.as_ref() {
                            set_occupancy(&occupancy, dev.bus_id.clone()).await;
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
                match server.handle_usbip_cmd_submit(
                    header,
                    transfer_buffer_length,
                    setup,
                    data,
                    device,
                ) {
                    Ok(r) => {
                        r.write_to_socket(socket).await?;
                    }
                    Err(e) => error!("UsbipCmdSubmit handling error: {e:?}"),
                }
            }
            UsbIpCommand::UsbIpCmdUnlink {
                header,
                unlink_seqnum,
            } => match server.handle_usbip_cmd_unlink(header, unlink_seqnum) {
                Ok(r) => {
                    r.write_to_socket(socket).await?;
                }
                Err(e) => error!("UsbipCmdUnlink handling error: {e:?}"),
            },
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
