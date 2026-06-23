use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{RwLock, mpsc};

use crate::usbip_server::{OccupancyMap, UsbIpServer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerControl {
    Background,
    Stop,
}

fn server_pid_path(port: u16) -> PathBuf {
    std::env::temp_dir().join(format!("lusbip-server-{port}.pid"))
}

fn server_status_path(port: u16) -> PathBuf {
    std::env::temp_dir().join(format!("lusbip-server-{port}.status"))
}

fn write_server_pid(port: u16) {
    let _ = fs::write(server_pid_path(port), std::process::id().to_string());
}

fn remove_server_pid(port: u16) {
    let path = server_pid_path(port);
    if fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(std::process::id())
    {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(server_status_path(port));
    }
}

fn active_server_pid(port: u16) -> Option<u32> {
    let pid = fs::read_to_string(server_pid_path(port))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    PathBuf::from("/proc")
        .join(pid.to_string())
        .exists()
        .then_some(pid)
}

fn listen_error_message(addr: SocketAddr, port: u16, err: io::Error) -> String {
    if err.kind() == io::ErrorKind::AddrInUse {
        if let Some(pid) = active_server_pid(port) {
            return format!(
                "LUSBIP server is already running on {addr} (pid {pid}). Use `sudo kill {pid}` to stop it, or keep using the background server."
            );
        }
        return format!(
            "Port {addr} is already in use. Another process is listening on this port; stop it or choose `--port <PORT>`."
        );
    }
    format!("Cannot listen on {addr}: {err}")
}

#[cfg(test)]
mod tests {
    use super::listen_error_message;
    use std::io;
    use std::net::SocketAddr;

    #[test]
    fn listen_error_message_explains_address_in_use() {
        let addr: SocketAddr = "0.0.0.0:3240".parse().unwrap();
        let message = listen_error_message(
            addr,
            3240,
            io::Error::new(io::ErrorKind::AddrInUse, "Address in use"),
        );

        assert!(message.contains("already in use") || message.contains("already running"));
    }
}

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

async fn write_server_status_snapshots(
    addr: SocketAddr,
    filter: ServerDeviceFilter,
    occupancy: OccupancyMap,
    port: u16,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    let path = server_status_path(port);

    loop {
        interval.tick().await;
        let rendered = build_server_status(addr, &filter, &occupancy).await;
        let _ = fs::write(&path, rendered);
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
    rendered.push_str(&format!("LUSBIP USB/IP server listening on {addr}\r\n"));
    rendered.push_str("Esc: background | Ctrl+C: stop and release devices.\r\n\r\n");
    rendered.push_str("Bus ID | VID:PID | Product | Status\r\n");
    rendered.push_str("------------------------------------\r\n");

    let views = match nusb::list_devices().await {
        Ok(devices) => shared_device_views(&devices.collect::<Vec<_>>(), filter),
        Err(err) => {
            rendered.push_str(&format!("Unable to refresh USB list: {err}\r\n"));
            Vec::new()
        }
    };

    if views.is_empty() {
        rendered.push_str("(no matching USB devices currently plugged in)\r\n");
        return rendered;
    }

    for view in &views {
        let mut row = view.clone();
        row.client = occupancy_snapshot
            .get(&row.bus_id)
            .map(|addr| client_ip_string(addr.ip()));
        rendered.push_str(&format!("{}\r\n", format_shared_device_row(&row)));
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

async fn serve_occupancy_status(addr: SocketAddr, occupancy: OccupancyMap) {
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("Unable to listen on occupancy status {addr}: {err}");
            return;
        }
    };

    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let occupancy_snapshot = occupancy.read().await.clone();
        tokio::spawn(async move {
            let mut body = String::new();
            for (bus_id, client) in occupancy_snapshot {
                body.push_str(&format!("{bus_id}\t{}\n", client.ip()));
            }
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

struct ServerTerminalGuard;

impl ServerTerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|err| format!("Failed to enable server key mode: {err}"))?;
        Ok(Self)
    }
}

impl Drop for ServerTerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn spawn_server_control_listener()
-> Result<(ServerTerminalGuard, mpsc::UnboundedReceiver<ServerControl>), String> {
    let terminal = ServerTerminalGuard::enter()?;
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) if key.code == KeyCode::Esc => {
                        let _ = tx.send(ServerControl::Background);
                    }
                    Ok(Event::Key(key))
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        let _ = tx.send(ServerControl::Stop);
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    });

    Ok((terminal, rx))
}

fn spawn_background_server(
    host: &str,
    port: u16,
    vid: Option<u16>,
    pid: Option<u16>,
    bus_id: Option<&str>,
) -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|err| format!("Cannot locate current binary: {err}"))?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("server")
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(vid) = vid {
        command.arg("--vid").arg(format!("{vid:04x}"));
    }
    if let Some(pid) = pid {
        command.arg("--pid").arg(format!("{pid:04x}"));
    }
    if let Some(bus_id) = bus_id {
        command.arg("--bus-id").arg(bus_id);
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Failed to start background server: {err}"))
}

async fn attach_existing_server_view(addr: SocketAddr, port: u16, pid: u32) -> Result<(), String> {
    let _terminal = ServerTerminalGuard::enter()?;
    let status_path = server_status_path(port);
    let mut last_rendered = String::new();

    loop {
        if event::poll(Duration::from_millis(100))
            .map_err(|err| format!("Failed to poll server viewer event: {err}"))?
        {
            match event::read()
                .map_err(|err| format!("Failed to read server viewer event: {err}"))?
            {
                Event::Key(key) if key.code == KeyCode::Esc => {
                    print!("\r\nDetached from LUSBIP server view. Server keeps running.\r\n");
                    let _ = io::stdout().flush();
                    return Ok(());
                }
                Event::Key(key)
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    print!("\r\nStopping background LUSBIP server pid {pid}...\r\n");
                    let _ = io::stdout().flush();
                    let status = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .status()
                        .map_err(|err| format!("Failed to stop server pid {pid}: {err}"))?;
                    if status.success() {
                        remove_server_pid(port);
                        return Ok(());
                    }
                    return Err(format!("Failed to stop server pid {pid}"));
                }
                _ => {}
            }
        }

        let rendered = fs::read_to_string(&status_path).unwrap_or_else(|_| {
            format!(
                "LUSBIP USB/IP server already running on {addr} (pid {pid})\r\n\
                 Waiting for status snapshot...\r\n\r\n\
                 Esc: detach view | Ctrl+C: stop background server\r\n"
            )
        });
        let rendered = rendered.replace(
            "Esc: background | Ctrl+C: stop and release devices.",
            "Esc: detach view | Ctrl+C: stop background server.",
        );

        if rendered != last_rendered {
            print!("\x1b[2J\x1b[H{rendered}");
            let _ = io::stdout().flush();
            last_rendered = rendered;
        }
    }
}

pub async fn run_server(
    host: &str,
    port: u16,
    vid: Option<u16>,
    pid: Option<u16>,
    bus_id: Option<&str>,
    background: bool,
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
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            if let Some(pid) = active_server_pid(port) {
                return attach_existing_server_view(addr, port, pid).await;
            }
            return Err(listen_error_message(addr, port, err));
        }
        Err(err) => return Err(listen_error_message(addr, port, err)),
    };
    write_server_pid(port);

    let occupancy: OccupancyMap = Arc::new(RwLock::new(HashMap::new()));
    let display_task = if background {
        None
    } else {
        let display_occupancy = occupancy.clone();
        Some(tokio::spawn(render_server_status(
            addr,
            filter.clone(),
            display_occupancy,
        )))
    };

    let sync_server = server.clone();
    let sync_occupancy = occupancy.clone();
    let sync_filter = filter.clone();
    let sync_task = tokio::spawn(sync_server_devices(
        sync_server,
        sync_filter,
        sync_occupancy,
    ));
    let status_task = tokio::spawn(write_server_status_snapshots(
        addr,
        filter.clone(),
        occupancy.clone(),
        port,
    ));
    let occupancy_status_addr = SocketAddr::new(addr.ip(), port.saturating_add(1));
    let occupancy_status_task = tokio::spawn(serve_occupancy_status(
        occupancy_status_addr,
        occupancy.clone(),
    ));

    let task_server = server.clone();
    let server_occupancy = occupancy.clone();
    let server_task = tokio::spawn(async move {
        crate::usbip_server::server_with_occupancy_listener(
            listener,
            task_server,
            server_occupancy,
        )
        .await;
    });

    let should_background = if background {
        signal::ctrl_c()
            .await
            .map_err(|err| format!("Failed to wait for Ctrl+C: {err}"))?;
        false
    } else {
        let (_terminal, mut control_rx) = spawn_server_control_listener()?;
        loop {
            match control_rx.recv().await {
                Some(ServerControl::Background) => {
                    if occupancy.read().await.is_empty() {
                        break true;
                    }
                    print!(
                        "\r\nCannot move to background while clients are attached; keep this server running or detach clients first.\r\n"
                    );
                    let _ = io::stdout().flush();
                }
                Some(ServerControl::Stop) | None => break false,
            }
        }
    };

    if should_background {
        print!("\r\nMoving LUSBIP server to background...\r\n");
    } else if !background {
        print!("\r\nStopping LUSBIP server and releasing devices...\r\n");
    }
    if let Some(display_task) = display_task {
        display_task.abort();
    }
    status_task.abort();
    occupancy_status_task.abort();
    sync_task.abort();
    server_task.abort();
    match tokio::time::timeout(Duration::from_secs(5), server.cleanup()).await {
        Ok(()) => {}
        Err(_) => {
            eprintln!("Timed out while releasing USB devices; exiting server shutdown.");
        }
    }
    remove_server_pid(port);
    if should_background {
        spawn_background_server(host, port, vid, pid, bus_id)?;
        print!("LUSBIP server is running in background on {host}:{port}.\r\n");
    }
    Ok(())
}
