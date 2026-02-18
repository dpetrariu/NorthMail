use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Graph API error {status}: {body}")]
    ApiError { status: u16, body: String },

    #[error("Failed to parse response: {0}")]
    ParseError(String),
}

pub type GraphResult<T> = Result<T, GraphError>;
