pub async fn run(
    remote: Option<&str>,
    tcp_port: u16,
    background: bool,
    agent_child: bool,
) -> Result<(), String> {
    let remote = remote.ok_or_else(|| {
        "Client UI cần --remote, ví dụ: lusbip client --remote 10.10.61.72 --tcp-port 3241"
            .to_string()
    })?;

    let endpoint = crate::client_agent::ClientEndpoint::new(remote, tcp_port);
    if agent_child {
        return crate::client_agent::run_background_agent(endpoint);
    }
    if background {
        return crate::client_agent::spawn_background_agent(&endpoint);
    }
    if crate::client_agent::background_agent_is_live(&endpoint) {
        return crate::client_agent::run_controller_tui(&endpoint);
    }
    crate::client::run_remote_control_tui(remote, tcp_port)
}
