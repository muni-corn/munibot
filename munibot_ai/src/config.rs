//! The `[ai]` section of munibot's configuration file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Settings for munibot's conversation interface.
///
/// Deliberately **not** a field of `munibot_core::Config`: this crate has no
/// dependency on `munibot_core`, so [`AiConfig::load_from_file`] does its own
/// parse of the same file, reading only the `[ai]` table and ignoring every
/// other section.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiConfig {
    /// Defaults to `false`, so a config file that never mentions `[ai]` boots
    /// with the conversation interface off rather than silently on.
    #[serde(default)]
    pub enabled: bool,

    /// The model every conversation talks to, as a rig catalog reference:
    /// `vendor/model`, like `anthropic/claude-opus-5`. Required once
    /// `enabled` is `true`.
    ///
    /// Kept as the string the operator wrote. rig's catalog is what decides
    /// whether it names a real model, when the chat service connects at
    /// startup, so parsing it here as well would only duplicate that check
    /// with worse error messages.
    #[serde(default)]
    pub model: Option<String>,
}

/// Why the `[ai]` section couldn't be loaded.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("couldn't read '{}' :< does the file have read permissions? ({source})", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("couldn't parse the [ai] section of '{}' :< {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl AiConfig {
    /// Loads the `[ai]` section from the same configuration file
    /// `munibot_core::Config` reads.
    ///
    /// A missing file, or a file with no `[ai]` table at all, yields
    /// [`AiConfig::default`] (`enabled: false`) rather than an error, so a
    /// deployment without AI configured boots completely unchanged.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] if the file exists but can't be read or
    /// parsed.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        struct ConfigFile {
            #[serde(default)]
            ai: AiConfig,
        }

        let path = path.as_ref();
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };

        let file: ConfigFile = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        Ok(file.ai)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(contents: &str) -> Result<AiConfig, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        AiConfig::load_from_file(&path)
    }

    #[test]
    fn test_default_config_is_disabled() {
        let config = AiConfig::default();
        assert!(!config.enabled, "ai should be off unless configured");
        assert!(config.model.is_none(), "no model should be assumed");
    }

    #[test]
    fn test_missing_file_yields_the_default() {
        let config = AiConfig::load_from_file("/nonexistent/path/does/not/exist.toml").unwrap();
        assert_eq!(config, AiConfig::default());
    }

    #[test]
    fn test_file_without_an_ai_table_yields_the_default() {
        assert_eq!(load("[discord]\n").unwrap(), AiConfig::default());
    }

    #[test]
    fn test_loads_enabled_and_model() {
        let config = load("[ai]\nenabled = true\nmodel = \"anthropic/claude-opus-5\"\n").unwrap();
        assert!(config.enabled);
        assert_eq!(config.model.as_deref(), Some("anthropic/claude-opus-5"));
    }

    #[test]
    fn test_ignores_sections_it_doesnt_know() {
        let config = load(
            "[ai]\nenabled = true\nmodel = \"openai/gpt-5\"\n[ai.rate_limits.user]\nmax_requests \
             = 5\n",
        )
        .unwrap();
        assert!(config.enabled);
    }

    #[test]
    fn test_malformed_toml_is_a_parse_error() {
        assert!(matches!(
            load("[ai\nenabled ="),
            Err(ConfigError::Parse { .. })
        ));
    }
}
