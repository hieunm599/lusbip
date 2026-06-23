use crate::process::{CommandRunner, StdCommandRunner};
use crate::tui::{TuiItem, run_action_list, select_one};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

const VHCI_STATUS_PATH: &str = "/sys/devices/platform/vhci_hcd.0/status";
const VHCI_DETACH_PATH: &str = "/sys/devices/platform/vhci_hcd.0/detach";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUsbDevice {
    pub bus_id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedUsbPort {
    pub port: String,
    pub remote_host: Option<String>,
    pub remote_bus_id: Option<String>,
    pub vid_pid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachTarget {
    pub remote_host: String,
    pub bus_id: Option<String>,
    pub vid_pid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUsbDeviceState {
    pub device: RemoteUsbDevice,
    pub attached_port: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub usbip_available: bool,
    pub sudo_cached: bool,
    pub usbip_port_readable: bool,
    pub remote_export_readable: Option<bool>,
}

impl DoctorReport {
    pub fn is_ok(&self) -> bool {
        self.usbip_available
            && self.sudo_cached
            && self.usbip_port_readable
            && self.remote_export_readable.unwrap_or(true)
    }
}

pub fn run_attach(remote: &str, tcp_port: u16, bus_id: Option<&str>) -> Result<(), String> {
    let _session_lock = ClientSessionLock::acquire(remote, tcp_port)?;
    let runner = StdCommandRunner;
    let devices = query_remote_devices(&runner, remote, tcp_port)?;
    let selected = match bus_id {
        Some(bus_id) => devices
            .iter()
            .find(|device| device.bus_id == bus_id)
            .cloned()
            .unwrap_or_else(|| RemoteUsbDevice {
                bus_id: bus_id.to_string(),
                description: "selected by command line".into(),
            }),
        None => select_remote_device(remote, &devices)?,
    };

    let target = AttachTarget {
        remote_host: remote.to_string(),
        bus_id: Some(selected.bus_id.clone()),
        vid_pid: extract_vid_pid(&selected.description),
    };

    auto_detach_matching_ports(&runner, &target)?;
    attach_remote_device(&runner, remote, tcp_port, &selected.bus_id)?;
    println!(
        "Attached remote USB device {} from {}. Check `lsusb` or `usbip port`.",
        selected.bus_id, remote
    );
    Ok(())
}

pub fn run_detach(port: &str) -> Result<(), String> {
    let runner = StdCommandRunner;
    detach_port(&runner, port)
}

pub fn run_detach_interactive() -> Result<(), String> {
    let runner = StdCommandRunner;
    let ports = query_attached_ports(&runner)?;
    let selected = select_attached_port(&ports)?;
    detach_port(&runner, &selected.port)?;
    println!("Detached USB/IP port {}", selected.port);
    Ok(())
}

pub fn run_remote_control_tui(remote: &str, tcp_port: u16) -> Result<(), String> {
    let _session_lock = ClientSessionLock::acquire(remote, tcp_port)?;
    let title = format!("LUSBIP - Remote USB ports ({remote}:{tcp_port})");
    let load_remote = remote.to_string();
    let action_remote = remote.to_string();
    let exit_remote = remote.to_string();
    run_action_list(
        &title,
        move || {
            let runner = StdCommandRunner;
            let states = load_remote_device_states(&runner, &load_remote, tcp_port)?;
            Ok(states
                .iter()
                .map(|state| TuiItem {
                    id: state.device.bus_id.clone(),
                    label: format_remote_device_state(state),
                })
                .collect())
        },
        move |index| {
            let runner = StdCommandRunner;
            let states = load_remote_device_states(&runner, &action_remote, tcp_port)?;
            let selected = states
                .get(index)
                .ok_or_else(|| "Selected USB device is no longer available".to_string())?
                .clone();
            toggle_remote_device(&runner, &action_remote, tcp_port, &selected)
        },
        move || {
            let runner = StdCommandRunner;
            detach_attached_remote_devices_on_exit(&runner, &exit_remote, tcp_port)
        },
    )
}

pub fn run_status(remote: Option<&str>, tcp_port: u16) -> Result<(), String> {
    let runner = StdCommandRunner;
    let ports = query_attached_ports_resilient(&runner);

    println!("Attached USB/IP ports:");
    if ports.is_empty() {
        println!("  (none)");
    } else {
        for port in ports {
            println!(
                "  Port {} | host: {} | bus: {} | vid:pid: {}",
                port.port,
                port.remote_host.as_deref().unwrap_or("unknown"),
                port.remote_bus_id.as_deref().unwrap_or("unknown"),
                port.vid_pid.as_deref().unwrap_or("unknown")
            );
        }
    }

    if let Some(remote) = remote {
        println!();
        println!("Exportable devices on {remote}:{tcp_port}:");
        let devices = query_remote_devices(&runner, remote, tcp_port)?;
        if devices.is_empty() {
            println!("  (none)");
        } else {
            for device in devices {
                println!("  {} | {}", device.bus_id, device.description);
            }
        }
    }

    Ok(())
}

pub fn run_doctor(remote: Option<&str>, tcp_port: u16, fix: bool) -> Result<(), String> {
    let runner = StdCommandRunner;

    if fix {
        apply_doctor_fixes(&runner)?;
    }

    let mut report = DoctorReport {
        usbip_available: false,
        sudo_cached: false,
        usbip_port_readable: false,
        remote_export_readable: None,
    };

    report.usbip_available = runner
        .run("usbip", &["version"])
        .map(|output| output.status.success())
        .unwrap_or(false);
    print_check(
        "usbip command",
        report.usbip_available,
        "install usbip userspace tools if this fails",
    );

    report.sudo_cached = runner
        .run("sudo", &["-n", "true"])
        .map(|output| output.status.success())
        .unwrap_or(false);
    print_check(
        "sudo cached",
        report.sudo_cached,
        "run `sudo -v` before detach/attach if this fails",
    );

    match query_attached_ports(&runner) {
        Ok(ports) => {
            report.usbip_port_readable = true;
            print_check("usbip port", true, "attached port list readable");
            if ports.is_empty() {
                println!("  attached: none");
            } else {
                for port in ports {
                    println!(
                        "  attached: port {} | host {} | bus {} | vid:pid {}",
                        port.port,
                        port.remote_host.as_deref().unwrap_or("unknown"),
                        port.remote_bus_id.as_deref().unwrap_or("unknown"),
                        port.vid_pid.as_deref().unwrap_or("unknown")
                    );
                }
            }
        }
        Err(err) => {
            report.usbip_port_readable = false;
            print_check("usbip port", false, &err);
        }
    }

    if let Some(remote) = remote {
        match query_remote_devices(&runner, remote, tcp_port) {
            Ok(devices) => {
                report.remote_export_readable = Some(true);
                print_check("remote export", true, "remote server responded");
                if devices.is_empty() {
                    println!("  remote: no exportable devices");
                } else {
                    for device in devices {
                        println!("  remote: {} | {}", device.bus_id, device.description);
                    }
                }
            }
            Err(err) => {
                report.remote_export_readable = Some(false);
                print_check("remote export", false, &err);
            }
        }
    }

    if report.is_ok() {
        Ok(())
    } else {
        Err("doctor checks failed".into())
    }
}

fn print_check(name: &str, ok: bool, detail: &str) {
    let status = if ok { "ok" } else { "fail" };
    println!("[{status}] {name}: {detail}");
}

struct ClientSessionLock {
    path: PathBuf,
}

impl ClientSessionLock {
    fn acquire(remote: &str, tcp_port: u16) -> Result<Self, String> {
        let path = client_session_lock_path(remote, tcp_port);
        match fs::create_dir(&path) {
            Ok(()) => {
                let pid_path = path.join("pid");
                if let Err(err) = fs::write(&pid_path, std::process::id().to_string()) {
                    let _ = fs::remove_dir(&path);
                    return Err(format!(
                        "Không ghi được lock file {}: {err}",
                        pid_path.display()
                    ));
                }
                Ok(Self { path })
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let pid_path = path.join("pid");
                let pid = fs::read_to_string(&pid_path).ok().and_then(|value| {
                    value
                        .trim()
                        .parse::<u32>()
                        .ok()
                        .filter(|pid| process_exists(*pid))
                });

                if let Some(pid) = pid {
                    return Err(format!(
                        "Đang có một phiên lusbip client khác cho {remote}:{tcp_port} (pid {pid}). Hãy đóng phiên đó trước khi mở phiên mới."
                    ));
                }

                fs::remove_dir_all(&path).map_err(|remove_err| {
                    format!(
                        "Lock cũ {} không còn tiến trình sống nhưng không xóa được: {remove_err}. Hãy xóa lock này rồi chạy lại.",
                        path.display()
                    )
                })?;
                Self::acquire(remote, tcp_port)
            }
            Err(err) => Err(format!("Không tạo được lock {}: {err}", path.display())),
        }
    }
}

impl Drop for ClientSessionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.join("pid"));
        let _ = fs::remove_dir(&self.path);
    }
}

fn client_session_lock_path(remote: &str, tcp_port: u16) -> PathBuf {
    let remote = remote
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    std::env::temp_dir().join(format!("lusbip-client-{remote}-{tcp_port}.lock"))
}

fn process_exists(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn apply_doctor_fixes(runner: &impl CommandRunner) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("doctor --fix currently supports Linux only".into());
    }

    println!("Applying Linux USB/IP client fixes...");
    if command_success(runner, "apt-get", &["--version"]) {
        let kernel = command_stdout(runner, "uname", &["-r"])?;
        let packages = ubuntu_client_packages(kernel.trim());
        if !command_success(runner, "usbip", &["version"])
            || !command_success(runner, "modinfo", &["vhci-hcd"])
        {
            run_fix_command(runner, "sudo", &["apt-get", "update"])?;
            let mut args = vec!["apt-get", "install", "-y"];
            args.extend(packages.iter().map(String::as_str));
            run_fix_command(runner, "sudo", &args)?;
        }
    } else {
        println!(
            "No apt-get found. Please install usbip tools and vhci-hcd kernel module for this distro."
        );
    }

    run_fix_command(runner, "sudo", &["modprobe", "vhci-hcd"])?;
    detach_stale_vhci_ports(runner)?;
    Ok(())
}

pub fn ubuntu_client_packages(kernel: &str) -> Vec<String> {
    vec![
        "usbip".to_string(),
        "linux-tools-generic".to_string(),
        format!("linux-tools-{kernel}"),
        format!("linux-modules-extra-{kernel}"),
    ]
}

fn command_success(runner: &impl CommandRunner, program: &str, args: &[&str]) -> bool {
    runner
        .run(program, args)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_stdout(
    runner: &impl CommandRunner,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    let output = runner
        .run(program, args)
        .map_err(|err| format!("Failed to execute {program}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_fix_command(
    runner: &impl CommandRunner,
    program: &str,
    args: &[&str],
) -> Result<(), String> {
    let command = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    println!("Running: {command}");
    let status = runner
        .run_foreground(program, args)
        .map_err(|err| format!("Failed to execute {command}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command} failed"))
    }
}

fn detach_stale_vhci_ports(runner: &impl CommandRunner) -> Result<(), String> {
    let ports = stale_vhci_ports();

    if ports.is_empty() {
        return Ok(());
    }

    println!(
        "Found stale VHCI ports: {}. Detaching before reconnecting.",
        ports.join(", ")
    );
    for port in ports {
        if let Err(err) = run_fix_command(runner, "sudo", &["usbip", "detach", "-p", &port]) {
            println!("{err}; falling back to direct VHCI detach");
            run_direct_vhci_detach(runner, &port)?;
        }
    }
    Ok(())
}

fn detach_stale_vhci_ports_quiet(runner: &impl CommandRunner) -> Result<(), String> {
    for port in stale_vhci_ports() {
        if detach_port(runner, &port).is_err() {
            direct_vhci_detach_quiet(runner, &port)?;
        }
    }
    Ok(())
}

fn run_direct_vhci_detach(runner: &impl CommandRunner, port: &str) -> Result<(), String> {
    let port = normalized_vhci_port(port)?;
    let script = format!("printf '{port}' > {VHCI_DETACH_PATH}");
    run_fix_command(runner, "sudo", &["sh", "-c", &script])
}

fn direct_vhci_detach_quiet(runner: &impl CommandRunner, port: &str) -> Result<(), String> {
    let port = normalized_vhci_port(port)?;
    let script = format!("printf '{port}' > {VHCI_DETACH_PATH}");
    let status = runner
        .run_interactive("sudo", &["-n", "sh", "-c", &script])
        .map_err(|err| format!("Failed to detach VHCI port {port}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to detach VHCI port {port}"))
    }
}

fn normalized_vhci_port(port: &str) -> Result<String, String> {
    port.parse::<u16>()
        .map(|port| port.to_string())
        .map_err(|_| format!("Invalid VHCI port {port}"))
}

fn stale_vhci_ports() -> Vec<String> {
    std::fs::read_to_string(VHCI_STATUS_PATH)
        .map(|status| parse_vhci_status_ports(&status))
        .unwrap_or_default()
}

pub fn parse_vhci_status_ports(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 6 || columns[0] == "hub" {
                return None;
            }
            let port = columns[1];
            let state = columns[2];
            let local_busid = columns[5];
            if state == "004" || local_busid == "0-0" {
                return None;
            }
            port.parse::<u16>().ok().map(|port| format!("{port:02}"))
        })
        .collect()
}

pub fn query_remote_devices(
    runner: &impl CommandRunner,
    remote: &str,
    tcp_port: u16,
) -> Result<Vec<RemoteUsbDevice>, String> {
    let tcp_port = tcp_port.to_string();
    let output = runner
        .run("usbip", &["--tcp-port", &tcp_port, "list", "-r", remote])
        .map_err(|err| format!("Failed to execute usbip list: {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "usbip list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(parse_usbip_list_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

pub fn query_attached_ports(runner: &impl CommandRunner) -> Result<Vec<AttachedUsbPort>, String> {
    let output = runner
        .run("usbip", &["port"])
        .map_err(|err| format!("Failed to execute usbip port: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "usbip port failed: {stderr}\nRun `lusbip doctor --fix` to reload vhci-hcd and detach stale USB/IP ports."
        ));
    }

    Ok(parse_usbip_port_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

pub fn auto_detach_matching_ports(
    runner: &impl CommandRunner,
    target: &AttachTarget,
) -> Result<(), String> {
    let ports = query_attached_ports_resilient(runner);
    for port in ports_to_detach(&ports, target) {
        detach_port(runner, &port)?;
    }
    Ok(())
}

fn query_attached_ports_resilient(runner: &impl CommandRunner) -> Vec<AttachedUsbPort> {
    match query_attached_ports(runner) {
        Ok(ports) => ports,
        Err(_) => {
            let _ = detach_stale_vhci_ports_quiet(runner);
            query_attached_ports(runner).unwrap_or_default()
        }
    }
}

pub fn detach_port(runner: &impl CommandRunner, port: &str) -> Result<(), String> {
    let output = runner
        .run("sudo", &["-n", "usbip", "detach", "-p", port])
        .map_err(|err| format!("Failed to execute usbip detach for port {port}: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to detach USB/IP port {port}: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ))
    }
}

pub fn attach_remote_device(
    runner: &impl CommandRunner,
    remote: &str,
    tcp_port: u16,
    bus_id: &str,
) -> Result<(), String> {
    let tcp_port = tcp_port.to_string();
    let output = runner
        .run(
            "sudo",
            &[
                "-n",
                "usbip",
                "--tcp-port",
                &tcp_port,
                "attach",
                "-r",
                remote,
                "-b",
                bus_id,
            ],
        )
        .map_err(|err| format!("Failed to execute usbip attach: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "usbip attach failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ))
    }
}

fn select_remote_device(
    remote: &str,
    devices: &[RemoteUsbDevice],
) -> Result<RemoteUsbDevice, String> {
    if devices.is_empty() {
        return Err(format!("No exportable USB devices found on {remote}"));
    }

    let items = devices
        .iter()
        .map(|device| TuiItem {
            id: device.bus_id.clone(),
            label: format!("{} | {}", device.bus_id, device.description),
        })
        .collect::<Vec<_>>();
    let index = select_one(
        &format!("LUSBIP - Attach remote USB device ({remote})"),
        &items,
        "Enter select | Esc cancel | j/k or arrows navigate",
    )?;

    Ok(devices[index].clone())
}

fn select_attached_port(ports: &[AttachedUsbPort]) -> Result<AttachedUsbPort, String> {
    if ports.is_empty() {
        return Err("No attached USB/IP ports to detach".into());
    }

    let items = ports
        .iter()
        .map(|port| TuiItem {
            id: port.port.clone(),
            label: format_attached_port(port),
        })
        .collect::<Vec<_>>();
    let index = select_one(
        "LUSBIP - Detach USB/IP port",
        &items,
        "Enter detach | Esc cancel | j/k or arrows navigate",
    )?;

    Ok(ports[index].clone())
}

pub fn format_attached_port(port: &AttachedUsbPort) -> String {
    format!(
        "Port {} | host: {} | bus: {} | vid:pid: {}",
        port.port,
        port.remote_host.as_deref().unwrap_or("unknown"),
        port.remote_bus_id.as_deref().unwrap_or("unknown"),
        port.vid_pid.as_deref().unwrap_or("unknown")
    )
}

fn load_remote_device_states(
    runner: &impl CommandRunner,
    remote: &str,
    tcp_port: u16,
) -> Result<Vec<RemoteUsbDeviceState>, String> {
    let devices = query_remote_devices(runner, remote, tcp_port)?;
    let ports = query_attached_ports_resilient(runner);
    let stale_ports = ports_to_detach_for_missing_remote_devices(remote, &devices, &ports);

    for port in &stale_ports {
        let _ = detach_port(runner, port);
    }

    let current_ports = ports
        .into_iter()
        .filter(|port| !stale_ports.contains(&port.port))
        .collect::<Vec<_>>();
    Ok(remote_device_states(remote, &devices, &current_ports))
}

fn toggle_remote_device(
    runner: &impl CommandRunner,
    remote: &str,
    tcp_port: u16,
    selected: &RemoteUsbDeviceState,
) -> Result<String, String> {
    if let Some(port) = selected.attached_port.as_deref() {
        detach_port(runner, port)?;
        return Ok(format!("Detached USB/IP port {port}"));
    }

    let target = AttachTarget {
        remote_host: remote.to_string(),
        bus_id: Some(selected.device.bus_id.clone()),
        vid_pid: extract_vid_pid(&selected.device.description),
    };
    auto_detach_matching_ports(runner, &target)?;
    attach_remote_device(runner, remote, tcp_port, &selected.device.bus_id)?;
    Ok(format!(
        "Attached remote USB device {} from {}",
        selected.device.bus_id, remote
    ))
}

fn detach_attached_remote_devices_on_exit(
    runner: &impl CommandRunner,
    remote: &str,
    tcp_port: u16,
) -> Result<(), String> {
    let states = load_remote_device_states(runner, remote, tcp_port)?;
    let mut detached = Vec::<String>::new();

    for port in states
        .iter()
        .filter_map(|state| state.attached_port.as_deref())
    {
        if detached.iter().any(|seen| seen == port) {
            continue;
        }
        detach_port(runner, port)?;
        detached.push(port.to_string());
    }

    Ok(())
}

pub fn remote_device_states(
    remote: &str,
    devices: &[RemoteUsbDevice],
    ports: &[AttachedUsbPort],
) -> Vec<RemoteUsbDeviceState> {
    let mut used_ports = Vec::<String>::new();
    let mut states = devices
        .iter()
        .map(|device| {
            let vid_pid = extract_vid_pid(&device.description);
            let attached_port = ports
                .iter()
                .find(|port| port_matches_remote_device(remote, device, vid_pid.as_deref(), port))
                .map(|port| port.port.clone());
            if let Some(port) = &attached_port {
                used_ports.push(port.clone());
            }

            RemoteUsbDeviceState {
                device: device.clone(),
                attached_port,
            }
        })
        .collect::<Vec<_>>();

    for port in ports {
        if used_ports.contains(&port.port) || !attached_port_belongs_to_remote(remote, port) {
            continue;
        }

        states.push(RemoteUsbDeviceState {
            device: RemoteUsbDevice {
                bus_id: format!("attached-port-{}", port.port),
                description: attached_port_description(port),
            },
            attached_port: Some(port.port.clone()),
        });
    }

    states
}

pub fn format_remote_device_state(state: &RemoteUsbDeviceState) -> String {
    let status = state
        .attached_port
        .as_deref()
        .map(|port| format!("[x] port {port}"))
        .unwrap_or_else(|| "[ ]".to_string());

    format!(
        "{status} | {} | {}",
        state.device.bus_id, state.device.description
    )
}

fn port_matches_remote_device(
    remote: &str,
    device: &RemoteUsbDevice,
    vid_pid: Option<&str>,
    port: &AttachedUsbPort,
) -> bool {
    let host_bus_matches = port.remote_host.as_deref() == Some(remote)
        && port.remote_bus_id.as_deref() == Some(device.bus_id.as_str());
    let unknown_host_vid_pid_matches = port.remote_host.is_none()
        && vid_pid.is_some_and(|vid_pid| port.vid_pid.as_deref() == Some(vid_pid));

    host_bus_matches || unknown_host_vid_pid_matches
}

fn attached_port_belongs_to_remote(remote: &str, port: &AttachedUsbPort) -> bool {
    port.remote_host.as_deref() == Some(remote)
        || (port.remote_host.is_none() && port.vid_pid.is_some())
}

fn attached_port_description(port: &AttachedUsbPort) -> String {
    match port.vid_pid.as_deref() {
        Some(vid_pid) => format!("Attached USB/IP device ({vid_pid})"),
        None => "Attached USB/IP device".into(),
    }
}

pub fn parse_usbip_list_output(stdout: &str) -> Vec<RemoteUsbDevice> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("Exportable")
                || trimmed.starts_with('=')
                || trimmed.starts_with('-')
                || trimmed.starts_with(':')
            {
                return None;
            }

            let (bus_id, description) = trimmed.split_once(':')?;
            let bus_id = bus_id.trim();
            let description = description.trim();

            if bus_id.is_empty() || description.is_empty() {
                return None;
            }

            Some(RemoteUsbDevice {
                bus_id: bus_id.to_string(),
                description: description.to_string(),
            })
        })
        .collect()
}

pub fn parse_usbip_port_output(stdout: &str) -> Vec<AttachedUsbPort> {
    let mut ports = Vec::new();
    let mut current: Option<AttachedUsbPort> = None;

    for line in stdout.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Port ") {
            if let Some(port) = current.take() {
                ports.push(port);
            }

            let port = rest
                .split_once(':')
                .map(|(value, _)| value.trim())
                .unwrap_or(rest.trim())
                .to_string();

            current = Some(AttachedUsbPort {
                port,
                remote_host: None,
                remote_bus_id: None,
                vid_pid: None,
            });
            continue;
        }

        let Some(port) = current.as_mut() else {
            continue;
        };

        if port.vid_pid.is_none() {
            port.vid_pid = extract_vid_pid(trimmed);
        }

        if let Some((host, bus_id)) = extract_usbip_url(trimmed) {
            port.remote_host = Some(host);
            port.remote_bus_id = Some(bus_id);
        }
    }

    if let Some(port) = current {
        ports.push(port);
    }

    ports
}

pub fn ports_to_detach(ports: &[AttachedUsbPort], target: &AttachTarget) -> Vec<String> {
    ports
        .iter()
        .filter(|port| should_detach(port, target))
        .map(|port| port.port.clone())
        .collect()
}

pub fn ports_to_detach_for_missing_remote_devices(
    remote: &str,
    devices: &[RemoteUsbDevice],
    ports: &[AttachedUsbPort],
) -> Vec<String> {
    ports
        .iter()
        .filter(|port| port.remote_host.as_deref() == Some(remote))
        .filter(|port| {
            port.remote_bus_id
                .as_deref()
                .is_none_or(|bus_id| !devices.iter().any(|device| device.bus_id == bus_id))
        })
        .map(|port| port.port.clone())
        .collect()
}

fn should_detach(port: &AttachedUsbPort, target: &AttachTarget) -> bool {
    let bus_matches = target
        .bus_id
        .as_deref()
        .is_some_and(|bus_id| port.remote_bus_id.as_deref() == Some(bus_id));
    let vid_pid_matches = target
        .vid_pid
        .as_deref()
        .is_some_and(|vid_pid| port.vid_pid.as_deref() == Some(&vid_pid.to_ascii_lowercase()));

    if port.remote_host.as_deref() != Some(target.remote_host.as_str()) {
        return port.remote_host.is_none() && vid_pid_matches;
    }

    if target.bus_id.is_none() && target.vid_pid.is_none() {
        return true;
    }

    bus_matches || vid_pid_matches
}

pub fn extract_vid_pid(line: &str) -> Option<String> {
    let start = line.find('(')?;
    let end = line[start + 1..].find(')')? + start + 1;
    let value = &line[start + 1..end];

    if value.len() == 9
        && value.as_bytes().get(4) == Some(&b':')
        && value.chars().all(|ch| ch == ':' || ch.is_ascii_hexdigit())
    {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

fn extract_usbip_url(line: &str) -> Option<(String, String)> {
    let marker = "usbip://";
    let start = line.find(marker)? + marker.len();
    let url = &line[start..];
    let (host_port, bus_id) = url.split_once('/')?;
    let host = host_port
        .split_once(':')
        .map(|(host, _)| host)
        .unwrap_or(host_port)
        .trim();
    let bus_id = bus_id.split_whitespace().next()?.trim();

    if host.is_empty() || bus_id.is_empty() {
        None
    } else {
        Some((host.to_string(), bus_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::client_session_lock_path;

    #[test]
    fn client_session_lock_path_is_shared_by_remote_and_port() {
        let path = client_session_lock_path("10.10.61.72", 3240);
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("lusbip-client-10.10.61.72-3240.lock")
        );
    }

    #[test]
    fn client_session_lock_path_sanitizes_unsafe_remote_chars() {
        let path = client_session_lock_path("fe80::1%eth0", 3240);
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("lusbip-client-fe80__1_eth0-3240.lock")
        );
    }
}
