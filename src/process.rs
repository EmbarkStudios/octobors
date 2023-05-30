use std::collections::{HashMap, HashSet};

use crate::{
    context,
    review::{Approval, CommentEffect, Review, Reviews},
};
use anyhow::{Context as _, Error, Result};
use chrono::{DateTime, Duration, Utc};
use models::{pulls::PullRequest, IssueState, StatusState};
use octocrab::models::{self};
use tracing as log;

#[cfg(test)]
mod tests;

#[derive(PartialEq, Eq, Hash)]
enum BlockReason {
    /// The PR is in the draft status.
    DraftPr,
    /// The PR is closed.
    ClosedPr,
    /// The PR has been inactive for over 60 minutes.
    InactivePr,
    /// The PR has reviewers set, and they haven't given a review yet.
    MissingReviews,
    /// The PR is waiting for a PR approval, and a label requires approvals.
    MissingReviewApproval { from_users: Vec<String> },
    /// The CI is not done running yet, or it's failing.
    CiNotPassing,
    /// The PR lacks a description, and a label requires a description.
    MissingDescription,
    /// The merge is blocked by a label.
    BlockedByLabel,
    /// The merge is blocked to prevent accidental graphite merge.
    BlockProtectionGraphite,
    /// The PR is inside a grace period.
    InsideGracePeriod,
}

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

enum PrApprovalStatus {
    Approved,
    MissingReview { from_users: Vec<String> },
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
    pr_comments: Vec<octocrab::models::issues::Comment>,
}

impl<'a> Analyzer<'a> {
    pub async fn new(
        pr: &'a Pr,
        client: &'a context::Client,
        config: &'a context::RepoConfig,
    ) -> anyhow::Result<Analyzer<'a>> {
        let pr_comments = client
            .get_pull_request_comments(config.name.as_str(), pr.number)
            .await?;

        Ok(Self {
            pr,
            client,
            config,
            reviews: RemoteData::Remote,
            statuses: RemoteData::Remote,
            pr_comments,
        })
    }

    async fn analyze_comments(
        &self,
        reasons: &HashSet<BlockReason>,
        actions: &mut Actions,
    ) -> Result<()> {
        const SIGIL: &str = "### Merge status";

        let user_id = self.client.get_bot_nick().await?;
        let bot_mention = format!("@{user_id}");

        let mut looking_for_response = false;

        for (author, body) in self.pr_comments.clone().into_iter().filter_map(|comment| {
            if let Some(body) = comment.body {
                Some((comment.user.login, body))
            } else {
                None
            }
        }) {
            if looking_for_response && author == user_id && body.starts_with(SIGIL) {
                log::trace!("Found response to the user asking why the PR is blocked");
                looking_for_response = false;
            }
            if author != user_id && body.contains(&bot_mention) {
                log::trace!("Found a comment asking mentioning the bot and asking why it's stuck");
                looking_for_response = true;
            }
        }

        if looking_for_response {
            let mut body = String::new();

            for reason in reasons {
                match reason {
                    BlockReason::DraftPr => {
                        body += "- This PR is a draft.\n";
                    }
                    BlockReason::ClosedPr => {
                        body += "- This PR is closed.\n";
                    }
                    BlockReason::InactivePr => {
                        // Probably the bot was inactive for too long, don't report here.
                    }
                    BlockReason::MissingReviews => {
                        body += "- Still waiting for requested reviewers to review this.\n";
                    }
                    BlockReason::MissingReviewApproval { from_users } => {
                        body += "- There are some missing review approvals";
                        if self.config.comment_requests_change {
                            body += " (and comments count as request-changes)";
                        }
                        if !from_users.is_empty() {
                            body += ". Missing approvals from: ";
                            body += &from_users
                                .iter()
                                .map(|nick| format!("@{nick}"))
                                .collect::<Vec<_>>()
                                .join(", ");
                        }
                        body += ".\n";
                    }
                    BlockReason::CiNotPassing => {
                        body += "- Github checks haven't passed yet.\n";
                    }
                    BlockReason::MissingDescription => {
                        body += "- This PR lacks a description.\n";
                    }
                    BlockReason::BlockedByLabel => {
                        body += &format!(
                            "- This PR is blocked by the '{}' label.\n",
                            self.config.block_merge_label.as_ref().unwrap()
                        );
                    }
                    BlockReason::BlockProtectionGraphite => {
                        body += &format!(
                            "- This PR is blocked by the '{}' label.\nThis label is added to protect the graphite stack from being automatically merged with dowstack PR's.\nIt is preferred to merge the stack from downstream to upstream branches.\nAnd manually rebase and push branches when downstack branches get merged.\nIf rebased on main, then you should remove the '{}' label.\n",
                            self.config.block_merge_label.as_ref().unwrap(),
                            self.config.block_merge_label.as_ref().unwrap()
                        );
                    }
                    BlockReason::InsideGracePeriod => {
                        body += "- In grace period; I'll retry in a bit.\n";
                    }
                }
            }

            if body.is_empty() {
                body += "Sorry, I was taking a nice little nap; will get back to work now!\n";
            }

            actions.post_comment(format!("{SIGIL}\n{body}"));
        }

        Ok(())
    }

    fn analyze_basic_checks(&self) -> HashSet<BlockReason> {
        let mut reasons = HashSet::new();
        let pr = &self.pr;
        if pr.draft {
            reasons.insert(BlockReason::DraftPr);
        }
        if pr.state == IssueState::Closed {
            reasons.insert(BlockReason::ClosedPr);
        }
        if pr.updated_at < Utc::now() - Duration::minutes(60) {
            reasons.insert(BlockReason::InactivePr);
        }
        let block_on_reviews = self.requires_reviews();
        if block_on_reviews && pr.requested_reviewers_remaining != 0 {
            reasons.insert(BlockReason::MissingReviews);
        }
        if self.merge_blocked_by_label() {
            reasons.insert(BlockReason::BlockedByLabel);
        }

        if self.merge_blocked_by_graphite() {
            reasons.insert(BlockReason::BlockProtectionGraphite);
        }

        if self.config.needs_description_label.is_some() && !self.pr.has_description {
            reasons.insert(BlockReason::MissingDescription);
        }
        if !self.outside_grace_period() {
            reasons.insert(BlockReason::InsideGracePeriod);
        }
        reasons
    }

    async fn analyze_extended_checks(
        &self,
        reasons: &mut HashSet<BlockReason>,
    ) -> anyhow::Result<()> {
        let statuses_passed = self.pr_statuses_passed().await?;
        if !statuses_passed {
            reasons.insert(BlockReason::CiNotPassing);
        }
        let pr_approved = self.pr_approved(self.requires_reviews()).await?;
        if let PrApprovalStatus::MissingReview { from_users } = pr_approved {
            reasons.insert(BlockReason::MissingReviewApproval { from_users });
        }
        Ok(())
    }

    /// Analyze a PR to determine what actions need to be undertaken.
    pub async fn required_actions(&self) -> Result<Actions> {
        let mut actions = Actions::noop();

        let mut block_reasons = self.analyze_basic_checks();
        if self.config.react_to_comments || block_reasons.is_empty() {
            // Now that the basic checks have been passed we can gather information
            // from the GitHub API in order to do the full check. We do this second
            // so that we use the GitHub API as little as possible, we don't want to
            // hit the rate limit.
            self.analyze_extended_checks(&mut block_reasons).await?;
        }

        if self.config.react_to_comments {
            self.analyze_comments(&block_reasons, &mut actions).await?;
        }

        let mut missing_review = false;
        let mut statuses_passed = true;
        let mut graphite_merge_protection: bool = false;

        for reason in &block_reasons {
            match reason {
                BlockReason::DraftPr => {
                    log::info!("Draft, nothing to do");
                    return Ok(actions);
                }
                BlockReason::ClosedPr => {
                    log::info!("Closed, nothing to do");
                    return Ok(actions);
                }
                BlockReason::InactivePr => {
                    log::info!("Inactive for over 60 minutes, nothing to do");
                    return Ok(actions);
                }
                BlockReason::MissingReviews => {
                    log::info!("Waiting on reviewers, nothing to do");
                    missing_review = true;
                }
                BlockReason::MissingReviewApproval { .. } => {
                    log::info!("Still waiting for a review approval");
                    missing_review = true;
                }
                BlockReason::CiNotPassing => {
                    log::info!("CI not passing yet");
                    statuses_passed = false;
                }
                BlockReason::MissingDescription => {
                    log::info!("Missing description");
                }
                BlockReason::BlockedByLabel => {
                    log::info!("Blocked by a block-merge label.");
                }
                BlockReason::BlockProtectionGraphite => {
                    log::info!("Blocked by graphite block-merge label.");
                    graphite_merge_protection = true;
                }
                BlockReason::InsideGracePeriod => {
                    log::info!("Still inside the grace period");
                }
            }
        }

        // Apply labels.
        if let Some(label) = &self.config.reviewed_label {
            let reviewed = !missing_review;
            actions.set_label(label, Presence::should_be_present(reviewed));
        }
        if let Some(label) = &self.config.ci_passed_label {
            actions.set_label(label, Presence::should_be_present(statuses_passed));
        }

        if let Some(label) = &self.config.needs_description_label {
            actions.set_label(label, Presence::should_be_present(!self.pr.has_description));
        }

        if graphite_merge_protection {
            if let (Some(graphite_label), Some(block_merge_label)) =
                (&self.config.graphite_label, &self.config.block_merge_label)
            {
                let contains_graphite_label = self.pr.labels.contains(graphite_label);
                let contains_block_merge_label = self.pr.labels.contains(block_merge_label);

                // Only set the block merge label once.
                if !contains_block_merge_label && !contains_graphite_label {
                    actions.set_label(block_merge_label, Presence::Present);
                }

                if !contains_graphite_label {
                    actions.set_label(graphite_label, Presence::Present);
                    let comment = format!("To safeguard the graphite branch from automated merges with dowstack pull requests, I have added the `{}` label. Feel free to ping me for more details!", block_merge_label);
                    actions.post_comment(comment);
                }
            }
        }

        // Conclude.
        actions.set_merge(block_reasons.is_empty());

        Ok(actions)
    }

    async fn pr_approved(&self, review_required: bool) -> Result<PrApprovalStatus> {
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
            Ok(PrApprovalStatus::Approved)
        } else {
            let from_users = reviews.missing_approvals_from_users();
            log::info!("Not yet approved by review");
            if !from_users.is_empty() {
                log::info!("\tWaiting for reviews from: {}", from_users.join(", "));
            }
            Ok(PrApprovalStatus::MissingReview { from_users })
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

    fn merge_blocked_by_graphite(&self) -> bool {
        let mut is_graphite_comment = false;
        for comment in self
            .pr_comments
            .iter()
            .filter_map(|x| x.body.clone())
        {
            is_graphite_comment |= comment.contains("Current dependencies on/for this PR:");
        }

        is_graphite_comment
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
    pub post_comment: Vec<String>,
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

    pub fn post_comment(&mut self, comment: String) -> &mut Self {
        self.post_comment.push(comment);
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

pub async fn post_comment(
    client: &context::Client,
    repo: &str,
    pr_number: u64,
    comment: String,
) -> anyhow::Result<()> {
    client
        .inner
        .issues(client.owner.clone(), repo)
        .create_comment(pr_number, comment)
        .await?;
    Ok(())
}
