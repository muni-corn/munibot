/// Why a turn was refused by [`crate::limits::SpendCapEnforcer::check`].
#[derive(Debug, Clone, thiserror::Error)]
#[error("that's hit the spend cap for now :< it resets {reset_at}")]
pub struct SpendCapError {
    /// Pre-formatted (RFC 3339) rather than a raw timestamp - this message
    /// is meant to be read directly, not reformatted downstream.
    pub reset_at: String,
}
