//! Error type for the Riot pipeline. "Game not running" is a normal state, not an
//! error — it is represented by `TrackerSnapshot::not_running`, never by `Error`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("lockfile not found (Riot Client not running)")]
    LockfileMissing,

    #[error("failed to parse lockfile: {0}")]
    LockfileParse(String),

    #[error("local API not ready yet")]
    NotReady,

    #[error("auth token expired / bad claims")]
    BadClaims,

    #[error("resource not found (state-transition race)")]
    ResourceNotFound,

    /// An HTTP 429, carrying the server's `Retry-After` delay in seconds when it sent a usable
    /// one, so the backoff can be exactly as long as we were asked to wait.
    #[error("rate limited")]
    RateLimited(Option<u64>),

    #[error("http error: {0}")]
    Http(String),

    #[error("websocket error: {0}")]
    WebSocket(String),

    /// A well-formed JSON body whose CONTENT is unusable (e.g. a match payload with no
    /// players). Distinct from `Json`, which is a serde shape failure.
    #[error("malformed payload: {0}")]
    MalformedPayload(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
