use std::path::PathBuf;

use anyhow::{Context as _, Result};

#[tokio::main]
async fn main() -> Result<()> {
    hook_logger()?;
    let app = octobors::Octobors::new(&get_config_path()?)?;
    log::debug!("configuration: {:#?}", app.config);
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

fn hook_logger() -> Result<()> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            let command = match record.level() {
                log::Level::Debug => "debug",
                log::Level::Warn => "warning",
                log::Level::Error => "error",
                // Info level messages are just emitted directly to stdout
                log::Level::Info | log::Level::Trace => {
                    return out.finish(format_args!("{}", message));
                }
            };

            out.finish(format_args!("::{}::{}", command, message))
        })
        .level(log::LevelFilter::Warn)
        // The actions UI will automatically filter out debug level events unless
        // the user has configured their workflow for debugging
        .level_for("octobors", log::LevelFilter::Debug)
        .chain(std::io::stdout())
        // Apply globally
        .apply()
        .context("unable to configure logging")
}
