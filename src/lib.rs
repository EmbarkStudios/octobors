pub mod context;
mod merge;
pub mod process;

use anyhow::{Context, Result};
use process::{Actions, Analyzer, PR};
use std::path::Path;

pub struct Octobors {
    pub config: context::Config,
    pub client: context::Client,
}

impl Octobors {
    pub fn new(path: &Path) -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .context("failed to read GITHUB_TOKEN environment variable")?;
        let contents = std::fs::read_to_string(path)?;
        let config: context::Config = toml::from_str(contents.as_str())?;
        let client = context::Client::new(token, config.owner.to_string())?;

        Ok(Self { client, config })
    }

    pub async fn process_all(&self) -> Result<()> {
        let futures = self.config.repos.iter().map(|repo| self.process_repo(repo));
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    pub async fn process_repo(&self, config: &context::RepoConfig) -> Result<()> {
        let futures = self
            .client
            .get_pull_requests(&config.name)
            .await?
            .into_iter()
            .map(|pr| self.process_pr(pr, config));
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    async fn process_pr(
        &self,
        pr: octocrab::models::pulls::PullRequest,
        config: &context::RepoConfig,
    ) -> Result<()> {
        let pr = PR::from_octocrab_pull_request(pr);
        log::info!("PR #{}: Processing", pr.number);

        let actions = Analyzer::new(&pr, &self.client, config)
            .required_actions()
            .await?;
        log::info!("PR #{}: {:?}", pr.number, actions);

        self.apply(actions, &pr, config).await?;

        log::info!("PR #{}: Done", pr.number);
        Ok(())
    }

    pub async fn apply(
        &self,
        actions: Actions,
        pr: &PR,
        config: &context::RepoConfig,
    ) -> Result<()> {
        let mut labels = pr.labels.iter().cloned().collect();
        let client = &self.client;
        let num = pr.number;
        process::remove_labels(
            client,
            &config.name,
            num,
            &mut labels,
            actions.remove_labels.into_iter(),
        )
        .await?;
        process::add_labels(
            client,
            &config.name,
            num,
            &mut labels,
            actions.add_labels.into_iter(),
        )
        .await?;

        if actions.merge {
            log::info!("PR #{}: Attempting to merge", pr.number);
            merge::queue(&self.client, pr, config).await?;
        }
        Ok(())
    }
}
