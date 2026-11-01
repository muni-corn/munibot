//! The `[ai]` section of munibot's configuration file.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::{AiError, ModelRef};

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

    /// The model every conversation talks to, written as `provider:model`,
    /// like `anthropic:claude-opus-5`. Required once `enabled` is `true`.
    ///
    /// Also accepted as `default_model`, the key earlier configs used.
    #[serde(default, alias = "default_model")]
    pub model: Option<ModelRef>,
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
    /// Returns [`AiError::Config`] if the file exists but can't be read or
    /// parsed.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, AiError> {
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
            Err(error) => {
                return Err(AiError::Config(format!(
                    "couldn't read '{}' :< {error}. does the file have read permissions?",
                    path.display()
                )));
            }
        };

        let file: ConfigFile = toml::from_str(&raw).map_err(|error| {
            AiError::Config(format!(
                "couldn't parse the [ai] section of '{}' :< {error}",
                path.display()
            ))
        })?;
        Ok(file.ai)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[discord]\n").unwrap();

        assert_eq!(
            AiConfig::load_from_file(&path).unwrap(),
            AiConfig::default()
        );
    }

    #[test]
    fn test_loads_enabled_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ai]\nenabled = true\nmodel = \"anthropic:claude-opus-5\"\n",
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        assert!(config.enabled);
        assert_eq!(
            config.model,
            Some(ModelRef::new("anthropic", "claude-opus-5"))
        );
    }

    #[test]
    fn test_accepts_the_legacy_default_model_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ai]\nenabled = true\ndefault_model = \"openai:gpt-5\"\n",
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        assert_eq!(config.model, Some(ModelRef::new("openai", "gpt-5")));
    }

    #[test]
    fn test_ignores_sections_it_doesnt_know() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ai]\nenabled = true\nmodel = \"openai:gpt-5\"\n[ai.rate_limits.user]\nmax_requests \
             = 5\n",
        )
        .unwrap();

        assert!(AiConfig::load_from_file(&path).unwrap().enabled);
    }

    #[test]
    fn test_malformed_toml_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ai\nenabled =").unwrap();

        assert!(matches!(
            AiConfig::load_from_file(&path),
            Err(AiError::Config(_))
        ));
    }
}
