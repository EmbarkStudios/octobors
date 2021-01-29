pub mod context;
mod merge;
pub mod process;

use anyhow::Result;
use process::{Actions, Analyzer, PR};

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
        let pr = PR::from_octocrab_pull_request(pr);
        log::info!("PR #{}: Processing", pr.number);

        let actions = Analyzer::new(&pr, &self.client, &self.config)
            .required_actions()
            .await?;
        log::info!("PR #{}: {:?}", pr.number, actions);

        self.apply(actions, &pr).await?;

        log::info!("PR #{}: Done", pr.number);
        Ok(())
    }

    pub async fn apply(&self, actions: Actions, pr: &PR) -> Result<()> {
        let mut labels = pr.labels.iter().cloned().collect();
        let client = &self.client;
        let num = pr.number;
        process::remove_labels(client, num, &mut labels, actions.remove_labels.into_iter()).await?;
        process::add_labels(client, num, &mut labels, actions.add_labels.into_iter()).await?;

        if actions.merge {
            log::info!("PR #{}: Attempting to merge", pr.number);
            merge::queue(&self.client, pr, &self.config).await?;
        }
        Ok(())
    }
}
