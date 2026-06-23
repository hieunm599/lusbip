use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::signal;
use tokio::sync::RwLock;

use crate::usbip_server::{OccupancyMap, UsbIpServer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDeviceView {
    pub bus_id: String,
    pub vid_pid: String,
    pub product: String,
    pub client: Option<String>,
}

pub fn format_shared_device_row(device: &SharedDeviceView) -> String {
    let status = device
        .client
        .as_deref()
        .map(|client| format!("occupied by {client}"))
        .unwrap_or_else(|| "available".to_string());

    format!(
        "{} | {} | {} | {}",
        device.bus_id, device.vid_pid, device.product, status
    )
}

fn shared_device_views(
    devices: &[nusb::DeviceInfo],
    filter: &ServerDeviceFilter,
) -> Vec<SharedDeviceView> {
    devices
        .iter()
        .filter(|device| filter.matches(device))
        .map(|device| SharedDeviceView {
            bus_id: export_bus_id(device),
            vid_pid: format!("{:04x}:{:04x}", device.vendor_id(), device.product_id()),
            product: device.product_string().unwrap_or("Unknown").to_string(),
            client: None,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerDeviceFilter {
    vid: Option<u16>,
    pid: Option<u16>,
    bus_id: Option<String>,
}

impl ServerDeviceFilter {
    fn matches(&self, device: &nusb::DeviceInfo) -> bool {
        if is_usb_hub(device) {
            return false;
        }
        if let Some(expected) = self.vid
            && device.vendor_id() != expected
        {
            return false;
        }
        if let Some(expected) = self.pid
            && device.product_id() != expected
        {
            return false;
        }
        if let Some(expected) = self.bus_id.as_deref()
            && export_bus_id(device) != expected
            && device.bus_id() != expected
        {
            return false;
        }
        true
    }
}

fn is_usb_hub(device: &nusb::DeviceInfo) -> bool {
    device.class() == 0x09
        || device
            .interfaces()
            .any(|interface| interface.class() == 0x09)
}

fn export_bus_id(device: &nusb::DeviceInfo) -> String {
    #[cfg(target_os = "linux")]
    {
        device
            .sysfs_path()
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-{}-{}", device.busnum(), device.device_address(), 0))
    }

    #[cfg(not(target_os = "linux"))]
    {
        device.bus_id().to_string()
    }
}

async fn render_server_status(
    addr: SocketAddr,
    filter: ServerDeviceFilter,
    occupancy: OccupancyMap,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    let mut last_rendered = String::new();

    loop {
        interval.tick().await;
        let rendered = build_server_status(addr, &filter, &occupancy).await;
        if rendered != last_rendered {
            print!("\x1b[2J\x1b[H{rendered}");
            let _ = io::stdout().flush();
            last_rendered = rendered;
        }
    }
}

fn client_ip_string(ip: IpAddr) -> String {
    ip.to_string()
}

async fn build_server_status(
    addr: SocketAddr,
    filter: &ServerDeviceFilter,
    occupancy: &OccupancyMap,
) -> String {
    let occupancy_snapshot = occupancy.read().await.clone();
    let mut rendered = String::new();
    rendered.push_str(&format!("LUSBIP USB/IP server listening on {addr}\n"));
    rendered.push_str("Press Ctrl+C to stop and release devices.\n\n");
    rendered.push_str("Bus ID | VID:PID | Product | Status\n");
    rendered.push_str("------------------------------------\n");

    let views = match nusb::list_devices().await {
        Ok(devices) => shared_device_views(&devices.collect::<Vec<_>>(), filter),
        Err(err) => {
            rendered.push_str(&format!("Unable to refresh USB list: {err}\n"));
            Vec::new()
        }
    };

    if views.is_empty() {
        rendered.push_str("(no matching USB devices currently plugged in)\n");
        return rendered;
    }

    for view in &views {
        let mut row = view.clone();
        row.client = occupancy_snapshot
            .get(&row.bus_id)
            .map(|addr| client_ip_string(addr.ip()));
        rendered.push_str(&format!("{}\n", format_shared_device_row(&row)));
    }

    rendered
}

async fn sync_server_devices(
    server: Arc<UsbIpServer>,
    filter: ServerDeviceFilter,
    occupancy: OccupancyMap,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));

    loop {
        interval.tick().await;
        let sync_result = server
            .sync_from_host_with_filter(|device| filter.matches(device))
            .await;
        match sync_result {
            Ok(current_bus_ids) => {
                occupancy
                    .write()
                    .await
                    .retain(|bus_id, _| current_bus_ids.contains(bus_id));
            }
            Err(err) => {
                eprintln!("Unable to sync USB devices: {err}");
            }
        }
    }
}

pub async fn run_server(
    host: &str,
    port: u16,
    vid: Option<u16>,
    pid: Option<u16>,
    bus_id: Option<&str>,
) -> Result<(), String> {
    let filter = ServerDeviceFilter {
        vid,
        pid,
        bus_id: bus_id.map(str::to_string),
    };

    let server_filter = filter.clone();
    let server =
        UsbIpServer::new_from_host_with_filter(move |device| server_filter.matches(device)).await;
    let server = Arc::new(server);
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|err| format!("Invalid listen address {host}:{port}: {err}"))?;

    let occupancy: OccupancyMap = Arc::new(RwLock::new(HashMap::new()));
    let display_occupancy = occupancy.clone();
    let display_task = tokio::spawn(render_server_status(
        addr,
        filter.clone(),
        display_occupancy,
    ));

    let sync_server = server.clone();
    let sync_occupancy = occupancy.clone();
    let sync_filter = filter.clone();
    let sync_task = tokio::spawn(sync_server_devices(
        sync_server,
        sync_filter,
        sync_occupancy,
    ));

    let task_server = server.clone();
    let server_occupancy = occupancy.clone();
    let server_task = tokio::spawn(async move {
        crate::usbip_server::server_with_occupancy(addr, task_server, server_occupancy).await;
    });

    signal::ctrl_c()
        .await
        .map_err(|err| format!("Failed to wait for Ctrl+C: {err}"))?;

    println!("Stopping LUSBIP server and releasing devices...");
    display_task.abort();
    sync_task.abort();
    server_task.abort();
    match tokio::time::timeout(Duration::from_secs(5), server.cleanup()).await {
        Ok(()) => {}
        Err(_) => {
            eprintln!("Timed out while releasing USB devices; exiting server shutdown.");
        }
    }
    Ok(())
}
