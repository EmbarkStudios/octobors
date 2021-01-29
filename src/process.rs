use std::collections::{HashMap, HashSet};

use crate::{client_request, context};
use anyhow::{Context as _, Error, Result};
use chrono::{DateTime, Duration, Utc};
use models::{
    pulls::{PullRequest, ReviewState},
    IssueState, StatusState,
};
use octocrab::models;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct PR {
    pub id: u64,
    pub number: u64,
    pub commit_sha: String,
    draft: bool,
    state: models::IssueState,
    updated_at: DateTime<Utc>,
    labels: HashSet<String>,
    has_description: bool,
}

impl PR {
    pub fn from_octocrab_pull_request(pr: PullRequest) -> Self {
        let labels = pr
            .labels
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.name)
            .collect();
        Self {
            id: pr.id,
            number: pr.number,
            commit_sha: pr.head.sha,
            draft: pr.draft,
            state: pr.state,
            updated_at: pr.updated_at.unwrap_or(pr.created_at),
            has_description: pr.body.unwrap_or_default() != "",
            labels,
        }
    }
}

pub struct Analyzer<'a> {
    pr: &'a PR,
    client: &'a context::Client,
    config: &'a context::Config,
    // We optionally keep a local version of these fields using `RemoteData`
    // so we can pre-set the data with values in order to not hit the GitHub
    // API in unit tests
    reviews: RemoteData<Vec<Review>>,
    statuses: RemoteData<HashMap<String, StatusState>>,
}

impl<'a> Analyzer<'a> {
    pub fn new(pr: &'a PR, client: &'a context::Client, config: &'a context::Config) -> Self {
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
            log::info!("PR #{}: Draft, nothing to do", pr.number);
            return Ok(actions);
        }

        if pr.state == IssueState::Closed {
            log::info!("PR #{}: Closed, nothing to do", pr.number);
            return Ok(actions);
        }

        if pr.updated_at < Utc::now() - Duration::minutes(60) {
            log::info!("PR #{}: Inactive for over 1 hour, nothing to do", pr.number);
            return Ok(actions);
        }

        // TODO: check if the no-merge label is there

        // Now that the basic checks have been passed we can gather information
        // from the GitHub API in order to do the full check. We do this second
        // so that we use the GitHub API as little as possible, we don't want to
        // hit the rate limit.
        let statuses_passed = self.pr_statuses_passed().await?;
        let pr_approved = self.pr_approved().await?;
        let mut description_ok = true;

        // TODO: ensure we are not within the grace period

        // Assign the "reviewed" label if there is one and the PR is approved
        if let Some(label) = &self.config.reviewed_label {
            actions.set_label(label, Presence::should_be_present(pr_approved));
        }

        // Assign the "needs-description" label if there is one and the PR lacks one
        if let Some(label) = &self.config.needs_description_label {
            description_ok = self.pr.has_description;
            actions.set_label(label, Presence::should_be_present(!self.pr.has_description));
        }

        // Assign the "ci-passed" label if CI passed
        actions.set_label(
            &self.config.ci_passed_label,
            Presence::should_be_present(statuses_passed),
        );

        actions.set_merge(
            !self.block_merge_label_applied() && description_ok && pr_approved && statuses_passed,
        );

        return Ok(actions);
    }

    async fn pr_approved(&self) -> Result<bool> {
        let review_not_required = self.config.reviewed_label.is_none();
        let mut waiting = false;
        let mut approved = review_not_required;
        let latest_reviews_per_person = self
            .get_pr_reviews()
            .await?
            .into_iter()
            .map(|review| (review.user_id, review.state))
            .collect::<HashMap<_, _>>();
        for review in latest_reviews_per_person.values() {
            match review {
                ReviewState::Approved => approved = true,
                ReviewState::Pending | ReviewState::ChangesRequested => waiting = true,
                _ => (),
            }
        }
        Ok(approved && !waiting)
    }

    async fn pr_statuses_passed(&self) -> Result<bool> {
        let statuses = self.get_pr_statuses().await?;
        for required in &self.config.required_statuses {
            if statuses.get(required) != Some(&StatusState::Success) {
                return Ok(false);
            }
        }
        return Ok(true);
    }

    fn block_merge_label_applied(&self) -> bool {
        match &self.config.block_merge_label {
            None => false,
            Some(label) => self.pr.labels.contains(label),
        }
    }

    async fn get_pr_reviews(&self) -> Result<Vec<Review>> {
        match &self.reviews {
            RemoteData::Local(reviews) => Ok(reviews.clone()),
            RemoteData::Remote => {
                let mut reviews = Vec::new();
                let page = client_request!(self.client, pulls)
                    .list_reviews(self.pr.number)
                    .await
                    .context("Could not get reviews for PR")?;
                let mut page = Some(page);
                while let Some(previous) = page {
                    reviews.extend(previous.items.iter().flat_map(Review::from_octocrab_review));
                    page = self.client.inner.get_page(&previous.next).await?;
                }
                Ok(reviews)
            }
        }
    }

    async fn get_pr_statuses(&self) -> Result<HashMap<String, StatusState>> {
        match &self.statuses {
            RemoteData::Local(statuses) => Ok(statuses.clone()),
            RemoteData::Remote => Ok(client_request!(self.client, repos)
                .combined_status_for_ref(&octocrab::params::repos::Reference::Commit(
                    self.pr.commit_sha.clone(),
                ))
                .await
                .context("Could not get statuses for commit")?
                .statuses
                .into_iter()
                .flat_map(|status| Some((status.context?, status.state)))
                .collect()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Review {
    user_id: i64,
    state: ReviewState,
}

impl Review {
    fn from_octocrab_review(review: &octocrab::models::pulls::Review) -> Option<Self> {
        Some(Self {
            user_id: review.user.id,
            state: review.state?,
        })
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
    merge: bool,
    add_labels: HashSet<String>,
    remove_labels: HashSet<String>,
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

async fn process_pr(_client: &context::Client, _cfg: &context::Config) -> Result<(), Error> {
    // add_labels(client, pr_number, &mut labels, &labels_to_add).await?;
    // remove_labels(client, pr_number, &mut labels, &labels_to_remove).await?;

    // if can_merge {
    //     log::warn!(
    //         "PR #{} has met all automerge requirements, queuing for merge...",
    //         pr_number
    //     );
    //     crate::merge::queue(&client, pr.pr, &cfg).await?;
    // }

    Ok(())
}

#[inline]
fn has_label(labels: &[String], name: &str) -> Option<usize> {
    labels.iter().position(|label| label == name)
}

/// Adds one or more labels to the PR. Only adds labels that aren't already present.
async fn add_labels(
    client: &context::Client,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_add: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), Error> {
    // Only add the label(s) that are not actually present
    let to_add: Vec<_> = to_add
        .into_iter()
        .filter_map(|new_label| match has_label(labels, new_label.as_ref()) {
            None => Some(new_label.as_ref().to_owned()),
            _ => None,
        })
        .collect();

    if to_add.is_empty() {
        return Ok(());
    }

    log::debug!("Adding labels {:?}", to_add);

    let ih = client_request!(client, issues);

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
async fn remove_labels(
    client: &context::Client,
    pr_number: u64,
    labels: &mut Vec<String>,
    to_remove: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Debug>,
) -> Result<(), Error> {
    // Only remove the label(s) that are actually present
    let to_remove: Vec<_> = to_remove
        .into_iter()
        .filter_map(|old_label| {
            has_label(&labels, old_label.as_ref()).map(|i| {
                labels.remove(i);
                old_label
            })
        })
        .collect();

    if to_remove.is_empty() {
        return Ok(());
    }

    log::debug!("Removing labels {:?}", to_remove);

    let ih = client_request!(client, issues);

    for old_label in to_remove {
        if let Err(e) = ih.remove_label(pr_number, old_label.as_ref()).await {
            log::debug!("Error removing label '{}': {:#}", old_label.as_ref(), e);
        }
    }

    Ok(())
}
