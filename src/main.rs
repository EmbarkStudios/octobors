use octobors::process;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opts {
    /// The Github API token to use for all requests
    #[structopt(long, env = "GITHUB_TOKEN")]
    token: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    use anyhow::Context as _;

    let args = Opts::from_args();

    let webhook_payload: octobors::context::ActionContext = {
        let event_payload = std::fs::read_to_string(&args.context_path).with_context(|| {
            format!(
                "failed to read event data from '{}'",
                args.context_path.display()
            )
        })?;

        serde_json::from_str(&event_payload).context("failed to deserialize event payload")?
    };

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(args.token)
        .build()
        .context("failed to create client")?;

    process::process_event(client, webhook_payload, config).await?;

    Ok(())
}
