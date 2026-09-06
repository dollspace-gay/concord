use super::{Deserialize, Serialize};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SearchQueryPlan {
    pub(super) text: Option<String>,
    pub(super) channel: Option<String>,
    pub(super) sender: Option<String>,
    pub(super) has_attachment: bool,
    pub(super) has_link: bool,
    pub(super) before: Option<String>,
    pub(super) after: Option<String>,
    pub(super) after_inclusive: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct SearchContinuationClaims {
    pub(super) exp: i64,
    pub(super) credential_id: String,
    pub(super) fingerprint: String,
    pub(super) authorization_version: i64,
    pub(super) before_created_at: String,
    pub(super) before_message_id: String,
    pub(super) position: i64,
}

#[derive(Debug)]
pub struct SearchResultsPage {
    pub results: Vec<crate::engine::events::SearchResultMessage>,
    pub total_count: i64,
    pub offset: i64,
    pub next_continuation: Option<String>,
    pub restarted: bool,
    pub stamp: crate::engine::authorization::AuthorizationStamp,
}

pub struct SearchMessagesRequest<'a> {
    pub server_id: &'a str,
    pub query: &'a str,
    pub channel_name: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
    pub continuation: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("INVALID_INPUT: {0}")]
    InvalidInput(String),
    #[error("INVALID_CONTINUATION: invalid search continuation")]
    InvalidContinuation,
    #[error("DEPENDENCY_UNAVAILABLE: search dependency unavailable")]
    DependencyUnavailable(#[source] crate::engine::authorization::AuthorizationError),
    #[error("RESOURCE_UNAVAILABLE: resource unavailable")]
    ResourceUnavailable,
}

impl SearchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::InvalidContinuation => "INVALID_CONTINUATION",
            Self::DependencyUnavailable(_) => "DEPENDENCY_UNAVAILABLE",
            Self::ResourceUnavailable => "RESOURCE_UNAVAILABLE",
        }
    }

    pub(super) fn from_authorization(
        error: crate::engine::authorization::AuthorizationError,
    ) -> Self {
        match error {
            crate::engine::authorization::AuthorizationError::Unavailable => {
                Self::ResourceUnavailable
            }
            other => Self::DependencyUnavailable(other),
        }
    }
}
