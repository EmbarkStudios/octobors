use chrono::Duration;
use models::IssueState;

use super::*;

fn make_context() -> (context::Client, context::Config) {
    let client =
        context::Client::new("token".to_string(), "org".to_string(), "repo".to_string()).unwrap();
    let config = context::Config {
        needs_description_label: Some("needs-description".to_string()),
        required_statuses: vec!["status1"].into_iter().map(String::from).collect(),
        ci_passed_label: "ci-passed".to_string(),
        reviewed_label: Some("reviewed".to_string()),
        block_merge_label: Some("block-merge".to_string()),
        automerge_grace_period: Some(1000),
        merge_method: octocrab::params::pulls::MergeMethod::Rebase,
    };
    (client, config)
}

fn make_analyzer<'a>(client: &'a context::Client, config: &'a context::Config) -> Analyzer<'a> {
    let pr = PR {
        id: 1,
        draft: false,
        state: models::IssueState::Open,
        updated_at: Utc::now(),
        labels: HashSet::new(),
        has_description: true,
    };
    let mut analyzer = Analyzer::new(pr, client, config);
    analyzer.reviews = RemoteData::Fetched(vec![
        ReviewState::Pending,
        ReviewState::Approved,
        ReviewState::Commented,
    ]);
    analyzer.statuses = RemoteData::Fetched(
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
    let (client, config) = make_context();
    let mut analyzer = make_analyzer(&client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed".to_string(), true)
            .set_label("ci-passed".to_string(), true)
            .set_label("needs-description".to_string(), false)
    );
}

#[tokio::test]
async fn draft_pr_actions() {
    let (client, config) = make_context();
    let mut analyzer = make_analyzer(&client, &config);
    analyzer.pr.draft = true;
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn closed_pr_actions() {
    let (client, config) = make_context();
    let mut analyzer = make_analyzer(&client, &config);
    analyzer.pr.state = IssueState::Closed;
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn stale_pr_actions() {
    let (client, config) = make_context();
    let mut analyzer = make_analyzer(&client, &config);
    analyzer.pr.updated_at = Utc::now() - Duration::minutes(61);
    assert_eq!(analyzer.required_actions().await.unwrap(), Actions::noop());
}

#[tokio::test]
async fn no_description_pr_actions() {
    let (client, config) = make_context();
    let mut analyzer = make_analyzer(&client, &config);
    analyzer.pr.has_description = false;
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(false)
            .set_label("reviewed".to_string(), true)
            .set_label("ci-passed".to_string(), true)
            .set_label("needs-description".to_string(), true)
    );
}

#[tokio::test]
async fn no_description_none_required_pr_actions() {
    let (client, mut config) = make_context();
    config.needs_description_label = None;
    let mut analyzer = make_analyzer(&client, &config);
    analyzer.pr.has_description = false;
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("reviewed".to_string(), true)
            .set_label("ci-passed".to_string(), true)
    );
}

#[tokio::test]
async fn no_reviewed_label_pr_actions() {
    let (client, mut config) = make_context();
    config.reviewed_label = None;
    let mut analyzer = make_analyzer(&client, &config);
    assert_eq!(
        analyzer.required_actions().await.unwrap(),
        *Actions::noop()
            .set_merge(true)
            .set_label("ci-passed".to_string(), true)
            .set_label("needs-description".to_string(), false)
    );
}

#[tokio::test]
async fn required_ci_not_passed_pr_actions() {
    macro_rules! assert_ci_failed_actions {
        ($cases:expr) => {{
            let (client, mut config) = make_context();
            config.reviewed_label = None;
            let mut analyzer = make_analyzer(&client, &config);
            analyzer.statuses = RemoteData::Fetched($cases.into_iter().collect());
            assert_eq!(
                analyzer.required_actions().await.unwrap(),
                *Actions::noop()
                    .set_merge(false)
                    .set_label("ci-passed".to_string(), false)
                    .set_label("needs-description".to_string(), false)
            );
        }};
    }

    assert_ci_failed_actions!(vec![("status1".to_string(), StatusState::Error)]);

    assert_ci_failed_actions!(vec![("status1".to_string(), StatusState::Failure)]);

    assert_ci_failed_actions!(vec![("status1".to_string(), StatusState::Pending)]);

    assert_ci_failed_actions!(vec![
        ("status2".to_string(), StatusState::Success),
        ("status1".to_string(), StatusState::Error),
    ]);

    assert_ci_failed_actions!(vec![
        ("status1".to_string(), StatusState::Failure),
        ("status2".to_string(), StatusState::Success),
    ]);

    assert_ci_failed_actions!(vec![
        ("status1".to_string(), StatusState::Pending),
        ("status2".to_string(), StatusState::Success),
    ]);

    assert_ci_failed_actions!(vec![]);

    assert_ci_failed_actions!(vec![("status2".to_string(), StatusState::Success)]);
}
