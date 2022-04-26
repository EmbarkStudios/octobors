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

#[derive(Debug, Clone)]
pub struct Reviews {
    /// Latest review given by author.
    /// Doesn't include the PR's author in it.
    review_by_nick: HashMap<String, Status>,

    /// What should be the effect of a comment review?
    comment_effect: CommentEffect,

    /// PR author's nickname.
    author: String,
}

pub enum Approval {
    Required,
    Optional,
}

#[derive(Debug, Clone, Copy)]
pub enum CommentEffect {
    RequestsChange,
    Ignore,
}

impl Reviews {
    pub fn new(author: impl Into<String>, comment_effect: CommentEffect) -> Self {
        Self {
            review_by_nick: HashMap::new(),
            author: author.into(),
            comment_effect,
        }
    }

    /// Check whether all the reviews are approving.
    pub fn approved(&self, approval_required: Approval) -> bool {
        let mut approved = matches!(approval_required, Approval::Optional);
        for (user, review) in self.review_by_nick.iter() {
            tracing::info!(user = %user, review = ?review, "review");
            match review {
                Status::Approved => approved = true,
                Status::ChangeRequested => return false,
            }
        }
        approved
    }

    pub fn record_reviews(mut self, reviews: Vec<Review>) -> Self {
        for review in reviews {
            self.record(review);
        }
        self
    }

    /// Review a new review. `Approved` and `ChangeRequested` reviews overwrite
    /// existing review state for the reviewer.
    fn record(&mut self, review: Review) {
        // Self-reviews shouldn't be taken into account.
        if review.user_name == self.author {
            return;
        }

        let status = match (review.state, self.comment_effect) {
            (ReviewState::Approved, _) => Some(Status::Approved),
            (ReviewState::ChangesRequested, _) => Some(Status::ChangeRequested),
            (ReviewState::Commented, CommentEffect::RequestsChange) => {
                if let Some(Status::Approved) = self.review_by_nick.get(&review.user_name) {
                    // As a very special case, don't count comments that are newer than an approval
                    // review as request for changes.
                    None
                } else {
                    Some(Status::ChangeRequested)
                }
            }
            _ => None,
        };

        if let Some(status) = status {
            let _ = self.review_by_nick.insert(review.user_name, status);
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
        let reviews = Reviews::new("example", CommentEffect::Ignore);
        assert!(!reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));
    }

    #[test]
    fn commented() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));

        let mut reviews = Reviews::new("example", CommentEffect::RequestsChange);
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));

        // A self-comment shouldn't have any influence.
        let mut reviews = Reviews::new("example", CommentEffect::RequestsChange);
        reviews.record(review("example", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));
    }

    #[test]
    fn approve() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::Approved));
        assert!(reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));
    }

    #[test]
    fn disapprove() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::ChangesRequested));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));
    }

    #[test]
    fn disapprove_then_approve() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::ChangesRequested));
        reviews.record(review("a", ReviewState::Approved));
        assert!(reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));
    }

    #[test]
    fn approve_then_disapprove() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("a", ReviewState::ChangesRequested));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));
    }

    #[test]
    fn disapprove_then_comment() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::ChangesRequested));
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));

        let mut reviews = Reviews::new("example", CommentEffect::RequestsChange);
        reviews.record(review("a", ReviewState::ChangesRequested));
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));
    }

    #[test]
    fn approve_then_comment() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("a", ReviewState::Commented));
        assert!(reviews.approved(Approval::Required));
        assert!(reviews.approved(Approval::Optional));

        let mut reviews = Reviews::new("example", CommentEffect::RequestsChange);
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("a", ReviewState::Commented));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));
    }

    #[test]
    fn approve_and_disapprove() {
        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("a", ReviewState::Approved));
        reviews.record(review("b", ReviewState::ChangesRequested));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));

        let mut reviews = Reviews::new("example", CommentEffect::Ignore);
        reviews.record(review("d", ReviewState::ChangesRequested));
        reviews.record(review("c", ReviewState::Approved));
        assert!(!reviews.approved(Approval::Required));
        assert!(!reviews.approved(Approval::Optional));
    }
}
