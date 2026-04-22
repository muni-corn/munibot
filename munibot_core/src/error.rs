use thiserror::Error;

/// Core error type for munibot. Contains only platform-agnostic variants.
/// Platform-specific errors (discord, twitch) are defined in their respective
/// crates and should implement `From<MuniBotError>`.
#[derive(Error, Debug)]
pub enum MuniBotError {
    #[error("parsing failure :< {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("request failed :< {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("missing token :<")]
    MissingToken,

    #[error("error with database :< {0}")]
    DbError(#[from] diesel::result::Error),

    #[error("error loading config :< {0}, {1}")]
    LoadConfig(String, anyhow::Error),

    #[error("couldn't parse duration :< {0}")]
    DurationParseError(#[from] humantime::DurationError),

    #[error("something went wrong :< {0}")]
    Other(String),
}

impl From<anyhow::Error> for MuniBotError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
    }
}
