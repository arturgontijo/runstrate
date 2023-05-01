#[derive(Debug, clap::Parser)]
pub struct Args {
    /// Websocket host
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    /// Websocket port
    #[arg(long, default_value_t = 9944)]
    pub port: u16,
    /// milliseconds per block (0 = instant)
    #[arg(long, default_value_t = 6000)]
    pub block_time: u64,
}
