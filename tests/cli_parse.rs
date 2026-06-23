use lusbip::cli::{Cli, Commands};
use lusbip::client::{DoctorReport, ubuntu_client_packages};

#[test]
fn parses_tui_command_with_remote_defaults() {
    let cli = Cli::parse_from([
        "lusbip",
        "tui",
        "--remote",
        "10.10.61.72",
        "--tcp-port",
        "3241",
    ]);

    assert!(matches!(
        cli.command,
        Commands::Tui(args)
            if args.remote.as_deref() == Some("10.10.61.72") && args.tcp_port == 3241
    ));
}

#[test]
fn parses_client_command_as_interactive_ui() {
    let cli = Cli::parse_from([
        "lusbip",
        "client",
        "--remote",
        "10.10.61.72",
        "--tcp-port",
        "3241",
    ]);

    assert!(matches!(
        cli.command,
        Commands::Client(args)
            if args.remote.as_deref() == Some("10.10.61.72") && args.tcp_port == 3241
    ));
}

#[test]
fn parses_attach_command_with_remote_and_bus_id() {
    let cli = Cli::parse_from([
        "lusbip",
        "attach",
        "--remote",
        "10.10.61.72",
        "--bus-id",
        "1-1",
        "--tcp-port",
        "3241",
    ]);

    assert!(matches!(
        cli.command,
        Commands::Attach(args)
            if args.remote == "10.10.61.72"
                && args.bus_id.as_deref() == Some("1-1")
                && args.tcp_port == 3241
    ));
}

#[test]
fn parses_server_defaults() {
    let cli = Cli::parse_from(["lusbip", "server"]);

    assert!(matches!(
        cli.command,
        Commands::Server(args) if args.host == "0.0.0.0" && args.port == 3240 && !args.background
    ));
}

#[test]
fn parses_server_background_mode() {
    let cli = Cli::parse_from(["lusbip", "server", "--background"]);

    assert!(matches!(
        cli.command,
        Commands::Server(args) if args.background
    ));
}

#[test]
fn parses_detach_command_with_port() {
    let cli = Cli::parse_from(["lusbip", "detach", "-p", "00"]);

    assert!(matches!(
        cli.command,
        Commands::Detach(args) if args.port == "00"
    ));
}

#[test]
fn parses_status_command_with_remote_and_tcp_port() {
    let cli = Cli::parse_from([
        "lusbip",
        "status",
        "--remote",
        "10.10.61.72",
        "--tcp-port",
        "3241",
    ]);

    assert!(matches!(
        cli.command,
        Commands::Status(args)
            if args.remote.as_deref() == Some("10.10.61.72") && args.tcp_port == 3241
    ));
}

#[test]
fn parses_doctor_command_with_remote_and_tcp_port() {
    let cli = Cli::parse_from([
        "lusbip",
        "doctor",
        "--remote",
        "10.10.61.72",
        "--tcp-port",
        "3241",
    ]);

    assert!(matches!(
        cli.command,
        Commands::Doctor(args)
            if args.remote.as_deref() == Some("10.10.61.72") && args.tcp_port == 3241 && !args.fix
    ));
}

#[test]
fn parses_doctor_fix_command() {
    let cli = Cli::parse_from(["lusbip", "doctor", "--fix"]);

    assert!(matches!(
        cli.command,
        Commands::Doctor(args) if args.fix && args.remote.is_none() && args.tcp_port == 3240
    ));
}

#[test]
fn ubuntu_client_packages_include_kernel_specific_tools_and_modules() {
    assert_eq!(
        ubuntu_client_packages("6.8.0-124-generic"),
        vec![
            "usbip",
            "linux-tools-generic",
            "linux-tools-6.8.0-124-generic",
            "linux-modules-extra-6.8.0-124-generic"
        ]
    );
}

#[test]
fn doctor_report_fails_when_any_required_check_fails() {
    let report = DoctorReport {
        usbip_available: true,
        sudo_cached: false,
        usbip_port_readable: true,
        remote_export_readable: Some(true),
    };

    assert!(!report.is_ok());
}
