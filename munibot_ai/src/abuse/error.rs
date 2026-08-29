/// Why a turn was refused by [`crate::abuse::AbuseDetector::check`].
///
/// Distinct from [`crate::limits::RateLimitError`]/`SpendCapError`: those
/// are about cost, this is about behaviour, and a caller may reasonably
/// want to react differently (there is no sense retrying sooner for either
/// of those, but a cooldown is, by definition, temporary).
#[derive(Debug, Clone, thiserror::Error)]
#[error("that tripped munibot's abuse detection ({reason}) :< try again in about {retry_after}")]
pub struct AbuseError {
    /// A short, human-readable description of what tripped detection -
    /// never raw message content, see [`crate::abuse::AbuseSignal::reason`].
    pub reason: String,
    /// Pre-formatted (`humantime::format_duration`) rather than a raw
    /// `Duration`, the same choice `RateLimitError`'s own `retry_after`
    /// makes, since this message is meant to be read directly.
    pub retry_after: String,
}
