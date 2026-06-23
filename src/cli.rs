use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "lusbip")]
#[command(about = "Share and attach USB devices over IP", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::parse_from(itr)
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    List,
    Server(ServerArgs),
    Client(TuiArgs),
    Attach(AttachArgs),
    Detach(DetachArgs),
    Status(StatusArgs),
    Doctor(StatusArgs),
    Tui(TuiArgs),
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long, value_parser = parse_hex_u16)]
    pub vid: Option<u16>,
    #[arg(long, value_parser = parse_hex_u16)]
    pub pid: Option<u16>,
    #[arg(long)]
    pub bus_id: Option<String>,
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    #[arg(short, long, default_value_t = 3240)]
    pub port: u16,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    #[arg(short, long)]
    pub remote: String,
    #[arg(short, long)]
    pub bus_id: Option<String>,
    #[arg(long, default_value_t = 3240)]
    pub tcp_port: u16,
}

#[derive(Debug, Args)]
pub struct DetachArgs {
    #[arg(short, long)]
    pub port: String,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(short, long)]
    pub remote: Option<String>,
    #[arg(long, default_value_t = 3240)]
    pub tcp_port: u16,
}

#[derive(Debug, Args)]
pub struct TuiArgs {
    #[arg(short, long)]
    pub remote: Option<String>,
    #[arg(long, default_value_t = 3240)]
    pub tcp_port: u16,
}

pub fn parse_hex_u16(value: &str) -> Result<u16, String> {
    let trimmed = value.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);

    u16::from_str_radix(hex, 16).map_err(|_| format!("Invalid hex value: {value}"))
}
