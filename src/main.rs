use lusbip::cli::{Cli, Commands};

#[tokio::main]
async fn main() {
    env_logger::init();

    let cli = Cli::parse_from(std::env::args_os());
    let result = match cli.command {
        Commands::List => lusbip::usb::print_usb_devices().await,
        Commands::Server(args) => {
            lusbip::server::run_server(
                &args.host,
                args.port,
                args.vid,
                args.pid,
                args.bus_id.as_deref(),
            )
            .await
        }
        Commands::Client(args) => lusbip::app_tui::run(args.remote.as_deref(), args.tcp_port).await,
        Commands::Attach(args) => {
            lusbip::client::run_attach(&args.remote, args.tcp_port, args.bus_id.as_deref())
        }
        Commands::Detach(args) => lusbip::client::run_detach(&args.port),
        Commands::Status(args) => lusbip::client::run_status(args.remote.as_deref(), args.tcp_port),
        Commands::Doctor(args) => {
            lusbip::client::run_doctor(args.remote.as_deref(), args.tcp_port, args.fix)
        }
        Commands::Tui(args) => lusbip::app_tui::run(args.remote.as_deref(), args.tcp_port).await,
    };

    if let Err(err) = result {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
