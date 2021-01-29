pub mod context;
mod merge;
pub mod process;

use anyhow::Result;

pub struct Octobors {
    pub config: context::Config,
    pub client: context::Client,
}

impl Octobors {
    pub fn new() -> Result<Self> {
        let client = context::Client::new_from_env()?;
        let config = context::Config::deserialize()?;

        Ok(Self { client, config })
    }

    pub async fn process_pull_requests(&self) -> Result<()> {
        log::info!("Processing pull requests");

        futures::future::try_join_all(
            self.client
                .get_pull_requests()
                .await?
                .into_iter()
                .map(|pr| self.process(pr)),
        )
        .await?;

        log::info!("Done");
        Ok(())
    }

    async fn process(&self, pr: octocrab::models::pulls::PullRequest) -> Result<()> {
        let pr = process::PR::from_octocrab_pull_request(pr);
        log::info!("PR #{}: Processing", pr.number);

        // Analyze the PR to determine if there is anything we need to do
        let actions = process::Analyzer::new(&pr, &self.client, &self.config)
            .required_actions()
            .await?;
        log::info!("PR #{}: {:?}", pr.number, actions);

        // TODO: apply labels
        // TODO: merge if needed

        Ok(())
    }
}
