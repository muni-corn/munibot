/// Why a turn was refused by [`crate::limits::RateLimiter::check`].
///
/// Distinct from [`crate::types::AiError`]: this is never the harness's own
/// concern, and a caller needs to match on *why* a turn was refused (to
/// decide whether retrying later makes sense) rather than get one opaque
/// error variant for every reason.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RateLimitError {
    /// `retry_after` is pre-formatted (`humantime::format_duration`) rather
    /// than a raw `Duration`, since this message is meant to be read
    /// directly, not reformatted downstream.
    #[error("you're sending messages a little too fast :< try again in about {retry_after}")]
    TooManyRequests { retry_after: String },
    #[error("that's used up the token budget for now :< try again in about {retry_after}")]
    TooManyTokens { retry_after: String },
    #[error(
        "munibot's already working on a lot with you at once :< let one finish before starting \
         another"
    )]
    TooManyConcurrentTurns,
}
