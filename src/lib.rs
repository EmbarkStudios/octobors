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
        let prs = self.client.get_pull_requests().await?;

        dbg!(&prs[0]);

        Ok(())
    }
}
