use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use crate::{
    context,
    review::{Approval, CommentEffect, Review, Reviews},
};
use anyhow::{Context as _, Error, Result};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule;
use models::{pulls::PullRequest, IssueState, StatusState};
use octocrab::models::{self};
use tracing as log;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct Pr {
    pub id: u64,
    pub author: String,
    pub number: u64,
    pub commit_sha: String,
    pub draft: bool,
    pub state: models::IssueState,
    pub updated_at: DateTime<Utc>,
    pub labels: HashSet<String>,
    pub has_description: bool,
    pub requested_reviewers_remaining: usize,
}

impl Pr {
    pub fn from_octocrab_pull_request(pr: PullRequest) -> Self {
        let labels = pr
            .labels
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.name)
            .collect();
        Self {
            id: *pr.id,
            author: pr.user.login,
            number: pr.number,
            commit_sha: pr.head.sha,
            draft: pr.draft,
            state: pr.state,
            updated_at: pr.updated_at.unwrap_or(pr.created_at),
            has_description: pr.body.unwrap_or_default() != "",
            requested_reviewers_remaining: pr.requested_reviewers.len(),
            labels,
        }
    }
}

pub struct Analyzer<'a> {
    pr: &'a Pr,
    client: &'a context::Client,
    config: &'a context::RepoConfig,
    // We optionally keep a local version of these fields using `RemoteData`
    // so we can pre-set the data with values in order to not hit the GitHub
    // API in unit tests
    reviews: RemoteData<Vec<Review>>,
    statuses: RemoteData<HashMap<String, StatusState>>,
}

impl<'a> Analyzer<'a> {
    pub fn new(pr: &'a Pr, client: &'a context::Client, config: &'a context::RepoConfig) -> Self {
        Self {
            pr,
            client,
            config,
            reviews: RemoteData::Remote,
            statuses: RemoteData::Remote,
        }
    }

    /// Analyze a PR to determine what actions need to be undertaken.
    pub async fn required_actions(&self) -> Result<Actions> {
        let pr = &self.pr;
        let mut actions = Actions::noop();

        if pr.draft {
            log::info!("Draft, nothing to do");
            return Ok(actions);
        }

        if pr.state == IssueState::Closed {
            log::info!("Closed, nothing to do");
            return Ok(actions);
        }

        if pr.updated_at < Utc::now() - Duration::minutes(60) {
            log::info!("Inactive for over 60 minutes, nothing to do");
            return Ok(actions);
        }

        let block_on_reviews = self.requires_reviews();

        if block_on_reviews && pr.requested_reviewers_remaining != 0 {
            log::info!("Waiting on reviewers, nothing to do");
            return Ok(actions);
        }

        // Now that the basic checks have been passed we can gather information
        // from the GitHub API in order to do the full check. We do this second
        // so that we use the GitHub API as little as possible, we don't want to
        // hit the rate limit.

        let statuses_passed = self.pr_statuses_passed().await?;

        let pr_approved = self.pr_approved(block_on_reviews).await?;

        if let Some(label) = &self.config.reviewed_label {
            actions.set_label(label, Presence::should_be_present(pr_approved));
        }

        let description_ok = if let Some(label) = &self.config.needs_description_label {
            actions.set_label(label, Presence::should_be_present(!self.pr.has_description));
            self.pr.has_description
        } else {
            true
        };

        if let Some(label) = &self.config.ci_passed_label {
            actions.set_label(label, Presence::should_be_present(statuses_passed));
        }

        // All requirements for merge-able PR.
        let mut should_merge = !self.merge_blocked_by_label()
            && self.outside_grace_period()
            && description_ok
            && pr_approved
            && statuses_passed;

        let has_maintenance_label = self.has_maintenance_label();

        if should_merge && has_maintenance_label {
            should_merge = self.within_maintenance_time();
        }

        actions.set_merge(should_merge);

        Ok(actions)
    }

    async fn pr_approved(&self, review_required: bool) -> Result<bool> {
        let reviews = self.get_pr_reviews().await?;
        log::debug!(reviews = ?reviews, "Got PR reviews");

        let review_required = if review_required {
            Approval::Required
        } else {
            Approval::Optional
        };
        let comment_effect = if self.config.comment_requests_change {
            CommentEffect::RequestsChange
        } else {
            CommentEffect::Ignore
        };

        let reviews = Reviews::new(self.pr.author.clone(), comment_effect).record_reviews(reviews);

        if reviews.approved(review_required) {
            Ok(true)
        } else {
            log::info!("Not yet approved by review");
            Ok(false)
        }
    }

    async fn pr_statuses_passed(&self) -> Result<bool> {
        let statuses = self.get_pr_statuses().await?;
        log::debug!(statuses = ?statuses, "Got PR statuses");
        for required in &self.config.required_statuses {
            if statuses.get(required) != Some(&StatusState::Success) {
                log::info!("Required status `{}` has not passed", required);
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn merge_blocked_by_label(&self) -> bool {
        self.config
            .block_merge_label
            .as_ref()
            .map_or(false, |label| {
                if self.pr.labels.contains(label) {
                    log::info!("Merge blocked by label");
                    true
                } else {
                    false
                }
            })
    }

    fn has_maintenance_label(&self) -> bool {
        self.config
            .maintenance_label
            .as_ref()
            .map_or(false, |label| {
                self.pr.labels.contains(label)
            })
    }

    fn within_maintenance_time(&self) -> bool {
        self.config
            .maintenance_time
            .as_ref()
            .and_then(|maintenance_time| {
                let schedule = Schedule::from_str(maintenance_time).unwrap();
                schedule.upcoming(Utc).next()
            })
            .map_or(false,|maintenance_time| {
                let current_time = Utc::now();

                if maintenance_time > current_time {
                    log::info!("Maintenance time");
                    true
                } else {
                    false
                }
            })
    }

    fn requires_reviews(&self) -> bool {
        // Either there's a trivial label, and the PR contains it, so reviews are optional.
        if let Some(ref trivial_label) = self.config.skip_review_label {
            if self.pr.labels.contains(trivial_label) {
                log::info!("Not blocking on reviews because of trivial review label");
                return false;
            }
        }

        // Or there's a review label, and that makes review mandatory.
        self.config.reviewed_label.is_some()
    }

    fn outside_grace_period(&self) -> bool {
        match &self.config.automerge_grace_period {
            None => true,
            Some(grace_period) => {
                if Utc::now() - Duration::seconds(*grace_period as i64) > self.pr.updated_at {
                    true
                } else {
                    log::info!("Within grace period, not merging");
                    false
                }
            }
        }
    }

    async fn get_pr_reviews(&self) -> Result<Vec<Review>> {
        match &self.reviews {
            RemoteData::Local(reviews) => Ok(reviews.clone()),
            RemoteData::Remote => Ok(self
                .client
                .get_pull_request_reviews(self.config.name.as_str(), self.pr.number)
                .await?
                .iter()
                .flat_map(Review::from_octocrab_review)
                .collect()),
        }
    }

    async fn get_pr_statuses(&self) -> Result<HashMap<String, StatusState>> {
        match &self.statuses {
            RemoteData::Local(statuses) => Ok(statuses.clone()),
            RemoteData::Remote => Ok(self
                .client
                .get_pull_request_statuses(&self.config.name, self.pr)
                .await?
                .into_iter()
                .flat_map(|status| Some((status.context?, status.state)))
                .collect()),
        }
    }
}

pub enum RemoteData<T> {
    Remote,
    Local(T),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Presence {
    Present,
    Absent,
}

impl Presence {
    fn should_be_present(should: bool) -> Self {
        if should {
            Self::Present
        } else {
            Self::Absent
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Actions {
    pub merge: bool,
    pub add_labels: HashSet<String>,
    pub remove_labels: HashSet<String>,
}

impl Actions {
    pub fn noop() -> Self {
        Self::default()
    }

    pub fn set_label(&mut self, label: &str, precence: Presence) -> &mut Self {
        match precence {
            Presence::Present => self.add_labels.insert(label.to_string()),
            Presence::Absent => self.remove_labels.insert(label.to_string()),
        };
        self
    }

    pub fn set_merge(&mut self, should_merge: bool) -> &mut Self {
        self.merge = should_merge;
        self
    }
}

#[inline]
fn has_label(labels: &[String], name: &str) -> Option<usize> {
    labels.iter().position(|label| label == name)
}

/// Adds one or more labels to the PR. Only adds labels that aren't already present.
pub async fn add_labels(
    client: &context::Client,
    repo: &str,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_add: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), Error> {
    // Only add the label(s) that are not actually present
    let to_add: Vec<_> = to_add
        .into_iter()
        .filter_map(|new_label| match has_label(labels, new_label.as_ref()) {
            None => Some(new_label.as_ref().to_owned()),
            Some(_) => None,
        })
        .collect();

    if to_add.is_empty() {
        return Ok(());
    }

    log::debug!("#{}: Adding labels {:?}", pr_number, to_add);

    let ih = client.inner.issues(&client.owner, repo);

    ih.add_labels(pr_number, &to_add)
        .await
        .with_context(|| format!("failed to add label(s): {:?}", to_add))?;

    for new_label in to_add {
        labels.push(new_label);
    }

    Ok(())
}

/// Removes one or more labels from the PR. Only removes labels that are actually
/// on the PR.
pub async fn remove_labels(
    client: &context::Client,
    repo: &str,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_remove: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Debug>,
) -> Result<(), Error> {
    // Only remove the label(s) that are actually present
    let to_remove: Vec<_> = to_remove
        .into_iter()
        .filter_map(|old_label| {
            has_label(labels, old_label.as_ref()).map(|i| {
                labels.remove(i);
                old_label
            })
        })
        .collect();

    if to_remove.is_empty() {
        return Ok(());
    }

    log::debug!("#{}: Removing labels {:?}", pr_number, to_remove);

    let ih = client.inner.issues(&client.owner, repo);

    for old_label in to_remove {
        if let Err(e) = ih.remove_label(pr_number, old_label.as_ref()).await {
            log::debug!("Error removing label '{}': {:#}", old_label.as_ref(), e);
        }
    }

    Ok(())
}
