use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use crate::client::{
    attach_remote_device, attached_port_belongs_to_remote, detach_port, format_remote_device_state,
    query_attached_ports, query_remote_devices, remote_device_states,
};
use crate::process::StdCommandRunner;
use crate::tui::{TuiItem, run_action_list};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEndpoint {
    remote: String,
    tcp_port: u16,
}

impl ClientEndpoint {
    pub fn new(remote: &str, tcp_port: u16) -> Self {
        Self {
            remote: remote.to_string(),
            tcp_port,
        }
    }

    pub fn remote(&self) -> &str {
        &self.remote
    }

    pub fn tcp_port(&self) -> u16 {
        self.tcp_port
    }

    pub fn runtime_dir(&self) -> PathBuf {
        let remote = self
            .remote
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        std::env::temp_dir().join(format!("lusbip-client-agent-{remote}-{}", self.tcp_port))
    }

    fn pid_path(&self) -> PathBuf {
        self.runtime_dir().join("pid")
    }
    fn socket_path(&self) -> PathBuf {
        self.runtime_dir().join("control.sock")
    }
    fn status_path(&self) -> PathBuf {
        self.runtime_dir().join("status")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    Status,
    Toggle { bus_id: String },
    Shutdown,
}

impl ControlRequest {
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim_end_matches(['\r', '\n']);
        match input {
            "STATUS" => Ok(Self::Status),
            "SHUTDOWN" => Ok(Self::Shutdown),
            _ => {
                let bus_id = input
                    .strip_prefix("TOGGLE\t")
                    .filter(|bus_id| !bus_id.is_empty())
                    .ok_or_else(|| "Invalid client-agent control request".to_string())?;
                Ok(Self::Toggle {
                    bus_id: percent_decode(bus_id)?,
                })
            }
        }
    }
}

pub fn percent_encode(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\t', "%09")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

pub fn percent_decode(value: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        let code = [chars.next(), chars.next()];
        match code {
            [Some('2'), Some('5')] => output.push('%'),
            [Some('0'), Some('9')] => output.push('\t'),
            [Some('0'), Some('D')] => output.push('\r'),
            [Some('0'), Some('A')] => output.push('\n'),
            _ => return Err("Invalid percent encoding in client-agent request".to_string()),
        }
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlResponse {
    Snapshot(Vec<TuiItem>),
    Message(String),
    Error(String),
}

pub fn background_agent_is_live(endpoint: &ClientEndpoint) -> bool {
    fs::read_to_string(endpoint.pid_path())
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .is_some_and(process_exists)
}

fn process_exists(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn prepare_runtime_dir(endpoint: &ClientEndpoint) -> Result<(), String> {
    let runtime_dir = endpoint.runtime_dir();
    if runtime_dir.exists() {
        if background_agent_is_live(endpoint) {
            return Err(format!(
                "Background client agent already runs for {}:{}",
                endpoint.remote(),
                endpoint.tcp_port()
            ));
        }
        fs::remove_dir_all(&runtime_dir).map_err(|err| {
            format!(
                "Cannot remove stale client-agent state {}: {err}",
                runtime_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&runtime_dir).map_err(|err| {
        format!(
            "Cannot create client-agent state {}: {err}",
            runtime_dir.display()
        )
    })?;
    fs::write(endpoint.pid_path(), std::process::id().to_string())
        .map_err(|err| format!("Cannot write client-agent PID: {err}"))
}

fn remove_runtime_dir(endpoint: &ClientEndpoint) {
    let _ = fs::remove_dir_all(endpoint.runtime_dir());
}

pub fn spawn_background_agent(endpoint: &ClientEndpoint) -> Result<(), String> {
    if background_agent_is_live(endpoint) {
        let pid = fs::read_to_string(endpoint.pid_path()).unwrap_or_else(|_| "unknown".into());
        println!(
            "LUSBIP client agent already runs for {}:{} (pid {}).",
            endpoint.remote(),
            endpoint.tcp_port(),
            pid.trim()
        );
        return Ok(());
    }

    let exe =
        std::env::current_exe().map_err(|err| format!("Cannot locate current binary: {err}"))?;
    std::process::Command::new(exe)
        .arg("client")
        .arg("--remote")
        .arg(endpoint.remote())
        .arg("--tcp-port")
        .arg(endpoint.tcp_port().to_string())
        .arg("--background")
        .arg("--agent-child")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Cannot start background client agent: {err}"))?;

    for _ in 0..20 {
        if background_agent_is_live(endpoint) && endpoint.socket_path().exists() {
            println!(
                "LUSBIP client agent is running in background for {}:{}.",
                endpoint.remote(),
                endpoint.tcp_port()
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("Background client agent did not become ready".to_string())
}

pub fn run_background_agent(endpoint: ClientEndpoint) -> Result<(), String> {
    prepare_runtime_dir(&endpoint)?;
    let listener = UnixListener::bind(endpoint.socket_path())
        .map_err(|err| format!("Cannot bind client-agent control socket: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("Cannot configure client-agent socket: {err}"))?;

    let mut managed_ports = BTreeSet::<String>::new();
    let initial = snapshot(&endpoint);
    write_snapshot(&endpoint, &initial);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_request(&mut stream);
                let (response, stop) = match request {
                    Ok(ControlRequest::Status) => (snapshot_response(&endpoint), false),
                    Ok(ControlRequest::Toggle { bus_id }) => {
                        (toggle(&endpoint, &mut managed_ports, &bus_id), false)
                    }
                    Ok(ControlRequest::Shutdown) => match shutdown(&mut managed_ports) {
                        Ok(()) => (
                            ControlResponse::Message(
                                "Detached managed USB/IP ports and stopped background agent".into(),
                            ),
                            true,
                        ),
                        Err(err) => (ControlResponse::Error(err), false),
                    },
                    Err(err) => (ControlResponse::Error(err), false),
                };
                write_response(&mut stream, &response);
                write_snapshot(&endpoint, &snapshot(&endpoint));
                if stop {
                    remove_runtime_dir(&endpoint);
                    return Ok(());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(err) => {
                remove_runtime_dir(&endpoint);
                return Err(format!("Client-agent control socket failed: {err}"));
            }
        }
    }
}

pub fn run_controller_tui(endpoint: &ClientEndpoint) -> Result<(), String> {
    if !background_agent_is_live(endpoint) {
        return Err(format!(
            "No background client agent is running for {}:{}",
            endpoint.remote(),
            endpoint.tcp_port()
        ));
    }
    let title = format!(
        "LUSBIP - Background client ({}:{})",
        endpoint.remote(),
        endpoint.tcp_port()
    );
    let load_endpoint = endpoint.clone();
    let action_endpoint = endpoint.clone();
    let exit_endpoint = endpoint.clone();
    run_action_list(
        &title,
        move || snapshot_from_agent(&load_endpoint),
        move |item| match send_request(
            &action_endpoint,
            ControlRequest::Toggle { bus_id: item.id },
        )? {
            ControlResponse::Message(message) => Ok(message),
            ControlResponse::Error(err) => Err(err),
            ControlResponse::Snapshot(_) => Err("Invalid response to toggle request".into()),
        },
        move || match send_request(&exit_endpoint, ControlRequest::Shutdown)? {
            ControlResponse::Message(_) => Ok(()),
            ControlResponse::Error(err) => Err(err),
            ControlResponse::Snapshot(_) => Err("Invalid response to shutdown request".into()),
        },
    )
}

fn snapshot_from_agent(endpoint: &ClientEndpoint) -> Result<Vec<TuiItem>, String> {
    match send_request(endpoint, ControlRequest::Status)? {
        ControlResponse::Snapshot(items) => Ok(items),
        ControlResponse::Error(err) => Err(err),
        ControlResponse::Message(_) => Err("Invalid response to status request".into()),
    }
}

fn snapshot(endpoint: &ClientEndpoint) -> Result<Vec<TuiItem>, String> {
    let runner = StdCommandRunner;
    let ports = query_attached_ports(&runner)?;
    let devices = match query_remote_devices(&runner, endpoint.remote(), endpoint.tcp_port()) {
        Ok(devices) => devices,
        Err(err) => {
            if ports
                .iter()
                .any(|port| attached_port_belongs_to_remote(endpoint.remote(), port))
            {
                Vec::new()
            } else {
                return Err(err);
            }
        }
    };
    Ok(remote_device_states(endpoint.remote(), &devices, &ports)
        .iter()
        .map(|state| TuiItem {
            id: state.device.bus_id.clone(),
            label: format_remote_device_state(state),
        })
        .collect())
}

fn snapshot_response(endpoint: &ClientEndpoint) -> ControlResponse {
    snapshot(endpoint)
        .map(ControlResponse::Snapshot)
        .unwrap_or_else(ControlResponse::Error)
}

fn toggle(
    endpoint: &ClientEndpoint,
    managed_ports: &mut BTreeSet<String>,
    bus_id: &str,
) -> ControlResponse {
    let runner = StdCommandRunner;
    let ports = match query_attached_ports(&runner) {
        Ok(ports) => ports,
        Err(err) => return ControlResponse::Error(err),
    };

    let matching_port = ports.iter().find(|port| {
        port.remote_host.as_deref() == Some(endpoint.remote())
            && port.remote_bus_id.as_deref() == Some(bus_id)
    });

    if let Some(port) = matching_port {
        let port_num = &port.port;
        if !managed_ports.contains(port_num) {
            return ControlResponse::Error(format!(
                "USB/IP port {port_num} is not managed by this background session"
            ));
        }
        return match detach_port(&runner, port_num) {
            Ok(()) => {
                managed_ports.remove(port_num);
                ControlResponse::Message(format!("Detached USB/IP port {port_num}"))
            }
            Err(err) => ControlResponse::Error(err),
        };
    }

    let devices = match query_remote_devices(&runner, endpoint.remote(), endpoint.tcp_port()) {
        Ok(devices) => devices,
        Err(err) => return ControlResponse::Error(err),
    };
    let states = remote_device_states(endpoint.remote(), &devices, &ports);
    let Some(selected) = states.iter().find(|state| state.device.bus_id == bus_id) else {
        return ControlResponse::Error("Selected USB device is no longer available".into());
    };
    if let Some(port) = selected.attached_port.as_deref() {
        if !managed_ports.contains(port) {
            return ControlResponse::Error(format!(
                "USB/IP port {port} is not managed by this background session"
            ));
        }
        return match detach_port(&runner, port) {
            Ok(()) => {
                managed_ports.remove(port);
                ControlResponse::Message(format!("Detached USB/IP port {port}"))
            }
            Err(err) => ControlResponse::Error(err),
        };
    }
    if let Some(client) = selected.occupied_by.as_deref() {
        return ControlResponse::Error(format!("USB device {bus_id} is occupied by {client}"));
    }
    if let Err(err) = attach_remote_device(&runner, endpoint.remote(), endpoint.tcp_port(), bus_id)
    {
        return ControlResponse::Error(err);
    }
    match snapshot(endpoint) {
        Ok(items) => {
            if let Some(port) = items
                .iter()
                .find(|item| item.id == bus_id)
                .and_then(attached_port_from_label)
            {
                managed_ports.insert(port);
                ControlResponse::Message(format!(
                    "Attached remote USB device {bus_id} from {}",
                    endpoint.remote()
                ))
            } else {
                ControlResponse::Error(format!(
                    "Attached {bus_id}, but could not identify its local USB/IP port"
                ))
            }
        }
        Err(err) => ControlResponse::Error(err),
    }
}

fn attached_port_from_label(item: &TuiItem) -> Option<String> {
    item.label
        .strip_prefix("[x] port ")?
        .split_once(" | ")
        .map(|(port, _)| port.to_string())
}

fn shutdown(managed_ports: &mut BTreeSet<String>) -> Result<(), String> {
    let runner = StdCommandRunner;
    let mut failures = Vec::new();
    for port in managed_ports.clone() {
        match detach_port(&runner, &port) {
            Ok(()) => {
                managed_ports.remove(&port);
            }
            Err(err) => failures.push(format!("{port}: {err}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Could not detach managed USB/IP ports: {}",
            failures.join("; ")
        ))
    }
}

fn write_snapshot(endpoint: &ClientEndpoint, snapshot: &Result<Vec<TuiItem>, String>) {
    let body = match snapshot {
        Ok(items) => items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Err(err) => format!("Error: {err}"),
    };
    let temporary = endpoint.status_path().with_extension("tmp");
    if fs::write(&temporary, body).is_ok() {
        let _ = fs::rename(temporary, endpoint.status_path());
    }
}

fn read_request(stream: &mut UnixStream) -> Result<ControlRequest, String> {
    let mut line = String::new();
    BufReader::new(stream.try_clone().map_err(|err| err.to_string())?)
        .read_line(&mut line)
        .map_err(|err| format!("Cannot read client-agent request: {err}"))?;
    ControlRequest::parse(&line)
}

fn send_request(
    endpoint: &ClientEndpoint,
    request: ControlRequest,
) -> Result<ControlResponse, String> {
    let mut stream = UnixStream::connect(endpoint.socket_path())
        .map_err(|err| format!("Cannot connect to background client agent: {err}"))?;
    let request = match request {
        ControlRequest::Status => "STATUS\n".to_string(),
        ControlRequest::Shutdown => "SHUTDOWN\n".to_string(),
        ControlRequest::Toggle { bus_id } => format!("TOGGLE\t{}\n", percent_encode(&bus_id)),
    };
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("Cannot send client-agent request: {err}"))?;
    read_response(&mut stream)
}

fn write_response(stream: &mut UnixStream, response: &ControlResponse) {
    let body = match response {
        ControlResponse::Snapshot(items) => {
            let mut body = String::from("OK\n");
            for item in items {
                body.push_str(&format!(
                    "ITEM\t{}\t{}\n",
                    percent_encode(&item.id),
                    percent_encode(&item.label)
                ));
            }
            body.push_str("END\n");
            body
        }
        ControlResponse::Message(message) => format!("OK\t{}\n", percent_encode(message)),
        ControlResponse::Error(message) => format!("ERR\t{}\n", percent_encode(message)),
    };
    let _ = stream.write_all(body.as_bytes());
}

fn read_response(stream: &mut UnixStream) -> Result<ControlResponse, String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|err| err.to_string())?);
    let mut first = String::new();
    reader
        .read_line(&mut first)
        .map_err(|err| format!("Cannot read client-agent response: {err}"))?;
    let first = first.trim_end_matches(['\r', '\n']);
    if let Some(message) = first.strip_prefix("OK\t") {
        return Ok(ControlResponse::Message(percent_decode(message)?));
    }
    if let Some(message) = first.strip_prefix("ERR\t") {
        return Ok(ControlResponse::Error(percent_decode(message)?));
    }
    if first != "OK" {
        return Err("Invalid client-agent response".into());
    }
    let mut items = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|err| format!("Cannot read client-agent snapshot: {err}"))?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line == "END" {
            return Ok(ControlResponse::Snapshot(items));
        }
        let fields = line
            .strip_prefix("ITEM\t")
            .and_then(|value| value.split_once('\t'))
            .ok_or_else(|| "Invalid client-agent snapshot item".to_string())?;
        items.push(TuiItem {
            id: percent_decode(fields.0)?,
            label: percent_decode(fields.1)?,
        });
    }
}
