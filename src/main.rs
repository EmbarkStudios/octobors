use anyhow::{Context as _, Error, Result};

#[tokio::main]
async fn main() -> Result<(), Error> {
    real_main().await.map_err(|e| {
        log::error!("{:#}", e);
        e
    })
}

async fn real_main() -> Result<(), Error> {
    hook_logger()?;
    let app = octobors::Octobors::new()?;

    log::debug!("configuration: {:#?}", app.config);

    octobors::Octobors::new()?.process_pull_requests().await?;

    Ok(())
}

fn hook_logger() -> Result<(), Error> {
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
