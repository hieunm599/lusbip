pub async fn run(remote: Option<&str>, tcp_port: u16) -> Result<(), String> {
    let remote = remote.ok_or_else(|| {
        "Client UI cần --remote, ví dụ: lusbip client --remote 10.10.61.72 --tcp-port 3241"
            .to_string()
    })?;

    crate::client::run_remote_control_tui(remote, tcp_port)
}
