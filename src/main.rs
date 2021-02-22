use std::path::PathBuf;

use anyhow::{Context as _, Result};
use tracing as log;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .init();
    let app = octobors::Octobors::new(&get_config_path()?)?;
    log::info!("configuration: {:?}", app.config);
    app.process_all().await
}

fn get_config_path() -> Result<PathBuf> {
    let mut args = std::env::args();
    let _ = args.next();
    let path = args.next().context(
        "Missing config file path command line argument

Usage:
    $ octobors path/to/config.toml",
    )?;
    Ok(PathBuf::from(path))
}
