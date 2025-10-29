mod cli;
mod seed;
mod events;
mod process;
mod pod;
mod isolate;
mod store;
mod metrics;
mod volumes;
mod secrets;

use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // Parse and execute CLI
    cli::run()
}
