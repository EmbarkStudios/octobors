use octocrab::models::pulls::ReviewState;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Review {
    pub user_name: String,
    pub state: ReviewState,
}

impl Review {
    pub fn from_octocrab_review(review: &octocrab::models::pulls::Review) -> Option<Self> {
        Some(Self {
            user_name: review.user.login.clone(),
            state: review.state?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Approved,
    ChangeRequested,
}

#[derive(Debug, Clone, Default)]
pub struct Reviews {
    latest: HashMap<String, Status>,
}

impl Reviews {
    pub fn new() -> Self {
        Self {
            latest: HashMap::new(),
        }
    }

    /// Check whether all the reviews are approving.
    pub fn approved(&self, approval_required: bool) -> bool {
        let mut approved = !approval_required;
        for (user, review) in self.latest.iter() {
            tracing::info!(user = %user, review = ?review, "review");
            match review {
                Status::Approved => approved = true,
                Status::ChangeRequested => return false,
            }
        }
        approved
    }

    pub fn record_reviews(mut self, reviews: Vec<Review>) -> Self {
        for review in reviews.into_iter() {
            self.record(review);
        }
        self
    }

    /// Review a new review. Approve and ChangeRequested reviews overwrite
    /// existing review state for the reviewer.
    fn record(&mut self, review: Review) {
        let status = match &review.state {
            ReviewState::Approved => Some(Status::Approved),
            ReviewState::ChangesRequested => Some(Status::ChangeRequested),
            _ => None,
        };
        if let Some(status) = status {
            let _ = self.latest.insert(review.user_name, status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(name: &str, state: ReviewState) -> Review {
        Review {
            user_name: name.to_string(),
            state,
        }
    }

    #[test]
    fn empty() {
        let reviews = Reviews::new();
        assert!(!reviews.approved(true));
        assert!(reviews.approved(false));
    }

    #[test]
    fn commented() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(true));
        assert!(reviews.approved(false));
    }

    #[test]
    fn approve() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::Approved));
        assert!(reviews.approved(true));
        assert!(reviews.approved(false));
    }

    #[test]
    fn disapprove() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::ChangesRequested));
        assert!(!reviews.approved(true));
        assert!(!reviews.approved(false));
    }

    #[test]
    fn disapprove_then_approve() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::ChangesRequested));
        reviews.record(review("a", ReviewState::Approved));
        assert!(reviews.approved(true));
        assert!(reviews.approved(false));
    }

    #[test]
    fn approve_then_disapprove() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("a", ReviewState::ChangesRequested));
        assert!(!reviews.approved(true));
        assert!(!reviews.approved(false));
    }

    #[test]
    fn disapprove_then_comment() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::ChangesRequested));
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(true));
        assert!(!reviews.approved(false));
    }

    #[test]
    fn approve_then_comment() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("a", ReviewState::Commented));
        assert!(reviews.approved(true));
        assert!(reviews.approved(false));
    }

    #[test]
    fn approve_and_disapprove() {
        let mut reviews = Reviews::new();
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("b", ReviewState::ChangesRequested));
        assert!(!reviews.approved(true));
        assert!(!reviews.approved(false));

        let mut reviews = Reviews::new();
        reviews.record(review("d", ReviewState::ChangesRequested));
        reviews.record(review("c", ReviewState::Approved));
        assert!(!reviews.approved(true));
        assert!(!reviews.approved(false));
    }
}
