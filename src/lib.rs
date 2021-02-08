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
            .context("failed to read GITHUB_TOKEN environment variable")?
            .trim()
            .to_string();
        let contents = std::fs::read_to_string(path)?;
        let config: context::Config = toml::from_str(contents.as_str())?;
        let client = context::Client::new(token, config.owner.to_string())?;

        Ok(Self { client, config })
    }

    pub async fn process_all(&self) -> Result<()> {
        for repo in self.config.repos.iter() {
            RepoProcessor::new(&self.config, &self.client, repo)
                .process()
                .await?;
        }
        Ok(())
    }
}

pub struct RepoProcessor<'a> {
    pub config: &'a context::Config,
    pub client: &'a context::Client,
    pub repo_config: &'a context::RepoConfig,
}

impl<'a> RepoProcessor<'a> {
    pub fn new(
        config: &'a context::Config,
        client: &'a context::Client,
        repo_config: &'a context::RepoConfig,
    ) -> Self {
        Self {
            config,
            client,
            repo_config,
        }
    }

    pub async fn process(&self) -> Result<()> {
        log::info!("{} Processing", self.repo_config.name);
        let futures = self
            .client
            .get_pull_requests(&self.repo_config.name)
            .await?
            .into_iter()
            .map(|pr| self.process_pr(pr));
        futures::future::try_join_all(futures).await?;
        log::info!("{} Done", self.repo_config.name);
        Ok(())
    }

    async fn process_pr(&self, pr: octocrab::models::pulls::PullRequest) -> Result<()> {
        let pr = PR::from_octocrab_pull_request(pr);
        let log = |msg: &str| log::info!("{}:{} {}", self.repo_config.name, pr.number, msg);
        log("Processing");

        let actions = Analyzer::new(&pr, &self.client, self.repo_config)
            .required_actions()
            .await?;
        log(&format!("{:?}", &actions));

        if self.config.dry_run {
            log("Dry run, doing nothing");
        } else {
            self.apply(actions, &pr).await?;
            log("Actions applied");
        }

        Ok(())
    }

    pub async fn apply(&self, actions: Actions, pr: &PR) -> Result<()> {
        let mut labels = pr.labels.iter().cloned().collect();
        let client = &self.client;
        let num = pr.number;
        process::remove_labels(
            client,
            &self.repo_config.name,
            num,
            &mut labels,
            actions.remove_labels.into_iter(),
        )
        .await?;
        process::add_labels(
            client,
            &self.repo_config.name,
            num,
            &mut labels,
            actions.add_labels.into_iter(),
        )
        .await?;

        if actions.merge {
            log::info!("PR #{}: Attempting to merge", pr.number);
            merge::queue(&self.client, pr, self.repo_config).await?;
        }
        Ok(())
    }
}
