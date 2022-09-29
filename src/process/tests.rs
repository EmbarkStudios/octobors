use chrono::Duration;
use models::IssueState;
use octocrab::models::pulls::ReviewState;

use super::*;

fn make_context() -> (Pr, context::Client, context::RepoConfig) {
    let client = context::Client::new("token".to_string(), "org".to_string(), None, &[]).unwrap();

    let config = context::RepoConfig {
        name: "the-project".to_string(),
        needs_description_label: Some("needs-description".to_string()),
        required_statuses: vec!["status1"].into_iter().map(String::from).collect(),
        ci_passed_label: Some("ci-passed".to_string()),
        reviewed_label: Some("reviewed".to_string()),
        block_merge_label: Some("block-merge".to_string()),
        automerge_grace_period: Some(10),
        trivial_review_label: None,
        merge_method: context::MergeMethod::Rebase,
        comment_requests_change: false,
    };

    let pr = Pr {
        id: 13482,
        author: "author".to_owned(),
        number: 1,
        commit_sha: "somesha".to_string(),
        draft: false,
        state: models::IssueState::Open,
        updated_at: Utc::now() - Duration::seconds(50),
        labels: HashSet::new(),
        has_description: true,
        requested_reviewers_remaining: 0,
    };

    (pr, client, config)
}

fn make_analyzer<'a>(
    pr: &'a Pr,
    client: &'a context::Client,
    config: &'a context::RepoConfig,
) -> Analyzer<'a> {
    let mut analyzer = Analyzer::new(pr, client, config);
    analyzer.reviews = RemoteData::Local(vec![
        review("1", ReviewState::Commented),
        review("2", ReviewState::Approved),
        review("3", ReviewState::Commented),
    ]);
    analyzer.statuses = RemoteData::Local(
        vec![
            ("status1".to_string(), StatusState::Success),
            ("status2".to_string(), StatusState::Failure),
        ]
        .into_iter()
        .collect(),
    );
    analyzer
}

#[tokio::test]
async fn ok_pr_actions() {
    let (pr, client, config) = make_context();
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn merge_blocked_by_label() {
    let (mut pr, client, mut config) = make_context();
    config.block_merge_label = Some("blocked!".to_string());
    pr.labels.insert("blocked!".to_string());
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(false)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn trivial_merge_not_blocked_on_pending_reviews() {
    let (mut pr, client, mut config) = make_context();

    // It's trivial
    config.trivial_review_label = Some("trivial :)".to_string());
    pr.labels.insert("trivial :)".to_string());

    // But a few reviews are pending
    pr.requested_reviewers_remaining = 42;

    let analyzer = make_analyzer(&pr, &client, &config);

    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn trivial_merge_blocked_on_requested_changes() {
    let (mut pr, client, mut config) = make_context();

    // It's trivial
    config.trivial_review_label = Some("trivial :)".to_string());
    pr.labels.insert("trivial :)".to_string());

    // But a few reviews are pending
    pr.requested_reviewers_remaining = 41;

    let mut analyzer = make_analyzer(&pr, &client, &config);

    // But one reviewer was like meh
    analyzer.reviews = RemoteData::Local(vec![review("1", ReviewState::ChangesRequested)]);

    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(false)
            .set_label("reviewed", Presence::Absent)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn trivial_merge_with_approval() {
    let (mut pr, client, mut config) = make_context();

    // It's trivial
    config.trivial_review_label = Some("trivial :)".to_string());
    pr.labels.insert("trivial :)".to_string());

    // But a few reviews are pending
    pr.requested_reviewers_remaining = 41;

    let mut analyzer = make_analyzer(&pr, &client, &config);

    // But one reviewer was satisfied
    analyzer.reviews = RemoteData::Local(vec![review("1", ReviewState::Approved)]);

    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn draft_pr_actions() {
    let (mut pr, client, config) = make_context();
    pr.draft = true;
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn closed_pr_actions() {
    let (mut pr, client, config) = make_context();
    pr.state = IssueState::Closed;
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn stale_pr_actions() {
    let (mut pr, client, config) = make_context();
    pr.updated_at = Utc::now() - Duration::minutes(61);
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn no_description_pr_actions() {
    let (mut pr, client, config) = make_context();
    pr.has_description = false;
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(false)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Present)
    );
}

#[tokio::test]
async fn no_description_none_required_pr_actions() {
    let (mut pr, client, mut config) = make_context();
    config.needs_description_label = None;
    pr.has_description = false;
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed", Presence::Present)
            .set_label("ci-passed", Presence::Present)
    );
}

#[tokio::test]
async fn review_not_required_if_label_not_configured() {
    use ReviewState::{Approved, ChangesRequested, Commented};

    #[track_caller]
    async fn assert_approved(approved: bool, cases: Vec<Review>) {
        let (pr, client, mut config) = make_context();
        config.reviewed_label = None;
        let mut analyzer = make_analyzer(&pr, &client, &config);
        analyzer.reviews = RemoteData::Local(cases);
        assert_eq!(
            analyzer.required_actions().await.unwrap(),
            *Actions::noop()
                .set_merge(approved)
                .set_label("ci-passed", Presence::Present)
                .set_label("needs-description", Presence::Absent)
        );
    }

    assert_approved(true, vec![]).await;
    assert_approved(true, vec![review("1", Approved)]).await;
    assert_approved(true, vec![review("1", Commented)]).await;
    assert_approved(false, vec![review("1", ChangesRequested)]).await;
}

#[tokio::test]
async fn changes_requested_still_blocks_if_label_not_configured() {
    let (pr, client, mut config) = make_context();
    config.reviewed_label = None;
    let mut analyzer = make_analyzer(&pr, &client, &config);
    analyzer.reviews = RemoteData::Local(vec![Review {
        user_name: "me".to_string(),
        state: ReviewState::ChangesRequested,
    }]);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(false)
            .set_label("ci-passed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn no_ci_passed_label() {
    let (pr, client, mut config) = make_context();
    config.ci_passed_label = None;
    let analyzer = make_analyzer(&pr, &client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed", Presence::Present)
            .set_label("needs-description", Presence::Absent)
    );
}

#[tokio::test]
async fn required_ci_not_passed_pr_actions() {
    macro_rules! assert_ci_failed_actions {
        ($cases:expr) => {{
            let (pr, client, mut config) = make_context();
            config.required_statuses = vec!["required1".to_string(), "required2".to_string()];
            let mut analyzer = make_analyzer(&pr, &client, &config);
            analyzer.statuses = RemoteData::Local(
                $cases
                    .into_iter()
                    .map(|(a, b): (&str, StatusState)| (a.to_string(), b))
                    .collect(),
            );
            assert_eq!(
                analyzer.required_actions().await.unwrap(),
                *Actions::noop()
                    .set_merge(false)
                    .set_label("reviewed", Presence::Present)
                    .set_label("ci-passed", Presence::Absent)
                    .set_label("needs-description", Presence::Absent)
            );
        }};
    }

    // No passes
    assert_ci_failed_actions!(vec![]);

    // One pass, other missing
    assert_ci_failed_actions!(vec![("required1", StatusState::Success)]);
    assert_ci_failed_actions!(vec![("required2", StatusState::Success)]);

    // One passed, other failed

    assert_ci_failed_actions!(vec![
        ("required2", StatusState::Success),
        ("required1", StatusState::Error)
    ]);
    assert_ci_failed_actions!(vec![
        ("required2", StatusState::Success),
        ("required1", StatusState::Failure)
    ]);
    assert_ci_failed_actions!(vec![
        ("required2", StatusState::Success),
        ("required1", StatusState::Pending)
    ]);
    assert_ci_failed_actions!(vec![
        ("required1", StatusState::Success),
        ("required2", StatusState::Error)
    ]);
    assert_ci_failed_actions!(vec![
        ("required1", StatusState::Success),
        ("required2", StatusState::Failure)
    ]);
    assert_ci_failed_actions!(vec![
        ("required1", StatusState::Success),
        ("required2", StatusState::Pending)
    ]);

    // Failing statuses with a non-required pass

    assert_ci_failed_actions!(vec![("not-required", StatusState::Success)]);

    assert_ci_failed_actions!(vec![
        ("not-required", StatusState::Success),
        ("required1", StatusState::Error),
        ("required2", StatusState::Success),
    ]);

    assert_ci_failed_actions!(vec![
        ("required1", StatusState::Failure),
        ("not-required", StatusState::Success),
        ("required2", StatusState::Success),
    ]);

    assert_ci_failed_actions!(vec![
        ("required1", StatusState::Pending),
        ("not-required", StatusState::Success),
        ("required2", StatusState::Success),
    ]);
}

#[tokio::test]
async fn review_approval_pr_actions() {
    use ReviewState::{Approved, ChangesRequested, Commented};
    macro_rules! assert_approved {
        ($approved:expr, $cases:expr) => {{
            let (pr, client, config) = make_context();
            let mut analyzer = make_analyzer(&pr, &client, &config);
            analyzer.reviews = RemoteData::Local($cases);
            assert_eq!(
                analyzer.required_actions().await.unwrap(),
                *Actions::noop()
                    .set_merge($approved)
                    .set_label("reviewed", Presence::should_be_present($approved))
                    .set_label("ci-passed", Presence::Present)
                    .set_label("needs-description", Presence::Absent)
            );
        }};
    }

    // No reviews
    assert_approved!(false, vec![]);

    // One non-approving review
    assert_approved!(false, vec![review("1", ChangesRequested)]);
    assert_approved!(false, vec![review("1", Commented)]);

    // One person approved, another disapproves
    assert_approved!(
        false,
        vec![review("1", Approved), review("2", ChangesRequested)]
    );

    // One person approves, another comments
    assert_approved!(true, vec![review("1", Approved), review("2", Commented)]);

    // One person approves
    assert_approved!(true, vec![review("1", Approved)]);

    // One person disapproves and then later approves
    assert_approved!(
        true,
        vec![review("1", ChangesRequested), review("1", Approved)]
    );

    // One person disapproves and then later comments
    assert_approved!(
        false,
        vec![review("1", ChangesRequested), review("1", Commented)]
    );

    // One person approves and then later comments
    assert_approved!(true, vec![review("1", Approved), review("1", Commented)]);
}

fn review(user_name: &str, state: ReviewState) -> Review {
    Review {
        user_name: user_name.to_string(),
        state,
    }
}

#[tokio::test]
async fn grace_period_prevents_merge() {
    macro_rules! assert_merge {
        ($grace_period:expr, $updated_seconds_ago:expr, $merge:expr) => {{
            let (mut pr, client, mut config) = make_context();
            config.automerge_grace_period = $grace_period;
            pr.updated_at = Utc::now() - Duration::seconds($updated_seconds_ago);
            let analyzer = make_analyzer(&pr, &client, &config);
            assert_eq!(
                analyzer.required_actions().await.unwrap(),
                *Actions::noop()
                    .set_merge($merge)
                    .set_label("reviewed", Presence::Present)
                    .set_label("ci-passed", Presence::Present)
                    .set_label("needs-description", Presence::Absent)
            );
        }};
    }

    // No grace period means it will always merge
    assert_merge!(None, 0, true);
    assert_merge!(None, -1, true);

    // Falling within a grace period means it will not merge
    assert_merge!(Some(1), 0, false);

    // Falling after a grace period means it will merge
    assert_merge!(Some(1), 2, true);
}

#[tokio::test]
async fn requested_reviews() {
    macro_rules! assert_merge {
        ($requested_reviewers:expr, $merge:expr) => {{
            let (mut pr, client, config) = make_context();
            pr.requested_reviewers_remaining = $requested_reviewers;
            let analyzer = make_analyzer(&pr, &client, &config);
            assert_eq!(
                analyzer.required_actions().await.unwrap().merge,
                Actions::noop().set_merge($merge).merge
            );
        }};
    }

    assert_merge!(0, true);
    assert_merge!(1, false);
    assert_merge!(2, false);
    assert_merge!(3, false);
}
