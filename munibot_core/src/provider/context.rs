use munibot_config::{ConfigScope, FieldError};
use thiserror::Error;

use crate::db::DbPool;

/// Everything a `ConfigurationProvider` needs to read or write settings for
/// one scope: which database to use, which scope (e.g. a specific discord
/// guild) the operation applies to, and who's asking.
pub struct ConfigContext {
    pub db: DbPool,
    pub scope: ConfigScope,
    /// The signed-in user performing this operation, if any. `None` for
    /// operations not attributable to a specific user (e.g. a slash
    /// command's write going through the same provider).
    pub actor: Option<i64>,
}

/// An error reading or writing a provider's configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A patch failed to apply: coercion failures, constraint violations,
    /// or both, aggregated -- see `munibot_config::Configurable::with_patch`.
    #[error("invalid configuration")]
    Invalid(Vec<FieldError>),

    #[error("database error: {0}")]
    Database(#[from] diesel::result::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<Vec<FieldError>> for ConfigError {
    fn from(errors: Vec<FieldError>) -> Self {
        Self::Invalid(errors)
    }
}

#[cfg(test)]
mod tests {
    use munibot_config::ConfigPath;

    use super::ConfigError;

    #[test]
    fn test_field_errors_convert_into_invalid() {
        let errors = vec![munibot_config::FieldError::new(
            ConfigPath::root().field("channel"),
            "bad value",
        )];
        let error: ConfigError = errors.into();
        assert!(matches!(error, ConfigError::Invalid(_)));
    }

    #[test]
    fn test_diesel_error_converts_into_database() {
        let diesel_error = diesel::result::Error::NotFound;
        let error: ConfigError = diesel_error.into();
        assert!(matches!(error, ConfigError::Database(_)));
    }

    #[test]
    fn test_anyhow_error_converts_into_other() {
        let anyhow_error = anyhow::anyhow!("boom");
        let error: ConfigError = anyhow_error.into();
        assert!(matches!(error, ConfigError::Other(_)));
    }
}
