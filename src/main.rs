use anyhow::{Context as _, Error};
use octobors::process;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opts {
    /// The Github API token to use for all requests
    #[structopt(long, env = "GITHUB_TOKEN")]
    token: String,
    #[structopt(long = "event")]
    event_path: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    real_main().await.map_err(|e| {
        log::error!("{:#}", e);
        e
    })
}

async fn real_main() -> Result<(), Error> {
    hook_logger()?;

    let args = Opts::from_args();

    let action_event = octobors::context::deserialize_action_context(args.event_path.as_ref())?;

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(args.token)
        .build()
        .context("failed to create client")?;

    let client = octobors::context::Client::new(client)?;
    let config = process::Config::deserialize()?;

    process::process_event(client, action_event, config).await?;

    Ok(())
}

fn hook_logger() -> Result<(), Error> {
    let global_level = std::env::var("ACTIONS_STEP_DEBUG")
        .map(|val| match val.as_str() {
            "true" | "1" => log::LevelFilter::Debug,
            _ => log::LevelFilter::Info,
        })
        .unwrap_or(log::LevelFilter::Info);

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
        .level_for("octobors", global_level)
        .chain(std::io::stdout())
        // Apply globally
        .apply()
        .context("unable to configure logging")
}
