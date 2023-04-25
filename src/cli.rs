#[derive(Debug, clap::Parser)]
pub struct Args {
    /// milliseconds per block (0 = instant)
    #[arg(long, default_value_t = 6000)]
    pub block_time: u64,
}
