use std::{fs, io::ErrorKind, path::Path};

use log::{info, warn};
use poise::serenity_prelude::UserId;
use serde::{Deserialize, Serialize};

use crate::MuniBotError;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub discord: DiscordConfig,
    pub twitch: TwitchConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiscordConfig {
    #[serde(default)]
    pub invite_link: Option<String>,

    #[serde(default)]
    pub ventriloquists: Vec<UserId>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TwitchConfig {
    #[serde(default = "default_twitch_user")]
    pub twitch_user: String,

    #[serde(default)]
    pub initial_channels: Vec<String>,
}

impl Config {
    /// Reads the config from the file if it exists, otherwise writes the
    /// default config to the file and loads that.
    pub fn read_or_write_default_from<P: AsRef<Path>>(path: P) -> Result<Self, Box<MuniBotError>> {
        let p = path.as_ref();

        // check if the path exists
        if !p.exists() {
            // construct default config
            let default = Config::default();

            // format it into a toml string
            let toml_string = toml::to_string_pretty(&default).map_err(|e| {
                MuniBotError::LoadConfig(
                    "couldn't format default config with toml".to_owned(),
                    e.into(),
                )
            })?;

            // write the default config string
            if let Err(e) = fs::write(p, toml_string) {
                warn!(
                    "hi there! i wanted to write my default configuration file to {}, but i can't.",
                    p.display(),
                );
                match e.kind() {
                    ErrorKind::NotFound => {
                        warn!("does its parent directory exist?\n");
                    }
                    ErrorKind::PermissionDenied => {
                        warn!("do you (or i) have permission to write to it?\n");
                    }
                    _ => warn!("(here's the error: {})\n", e),
                }
            } else {
                // notify we wrote the file
                info!(
                    "hi! i'm munibot! i've written my default configuration file to {} for you :3 <3",
                    p.display()
                );
            }

            // and return the config
            Ok(default)
        } else {
            // read the file to a string
            let raw_string = fs::read_to_string(p).map_err(|e| {
                MuniBotError::LoadConfig(
                    format!("couldn't read contents of {}", p.display()),
                    e.into(),
                )
            })?;

            // parse the string as toml
            let config = toml::from_str(&raw_string).map_err(|e| {
                MuniBotError::LoadConfig(
                    format!("couldn't parse toml from {}", p.display()),
                    e.into(),
                )
            })?;

            // notify we read the config
            info!("hiya! configuration has been read from {} ^u^", p.display());

            // return the config
            Ok(config)
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            discord: DiscordConfig {
                invite_link: None,
                ventriloquists: vec![],
            },
            twitch: TwitchConfig {
                twitch_user: default_twitch_user(),
                initial_channels: Vec::new(),
            },
        }
    }
}

fn default_twitch_user() -> String {
    "muni__bot".to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::Config;

    #[test]
    fn test_default_config_discord_invite_link_is_none() {
        let config = Config::default();
        assert!(
            config.discord.invite_link.is_none(),
            "default invite link should be None"
        );
    }

    #[test]
    fn test_default_config_ventriloquists_is_empty() {
        let config = Config::default();
        assert!(
            config.discord.ventriloquists.is_empty(),
            "default ventriloquists list should be empty"
        );
    }

    #[test]
    fn test_default_config_twitch_user() {
        let config = Config::default();
        assert_eq!(config.twitch.twitch_user, "muni__bot");
    }

    #[test]
    fn test_default_config_initial_channels_is_empty() {
        let config = Config::default();
        assert!(
            config.twitch.initial_channels.is_empty(),
            "default initial_channels should be empty"
        );
    }

    #[test]
    fn test_toml_roundtrip() {
        let original = Config::default();
        let toml_str = toml::to_string_pretty(&original).expect("failed to serialize config");
        let parsed: Config = toml::from_str(&toml_str).expect("failed to deserialize config");
        assert_eq!(parsed.twitch.twitch_user, original.twitch.twitch_user);
        assert_eq!(parsed.discord.invite_link, original.discord.invite_link);
    }

    #[test]
    fn test_read_or_write_default_creates_file_when_missing() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");

        assert!(!path.exists(), "file should not exist before call");
        let config = Config::read_or_write_default_from(&path)
            .expect("should succeed even when file is missing");

        // the default should be returned
        assert_eq!(config.twitch.twitch_user, "muni__bot");
        // the file should now exist
        assert!(path.exists(), "config file should have been written");
    }

    #[test]
    fn test_read_or_write_default_reads_existing_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");

        // write a custom TOML manually
        let toml_content = r#"
[discord]
invite_link = "https://discord.gg/example"
ventriloquists = []

[twitch]
twitch_user = "custom_bot_user"
initial_channels = ["mychannel"]
"#;
        fs::write(&path, toml_content).expect("failed to write test config");

        let config = Config::read_or_write_default_from(&path).expect("should read existing file");

        assert_eq!(config.twitch.twitch_user, "custom_bot_user");
        assert_eq!(
            config.discord.invite_link.as_deref(),
            Some("https://discord.gg/example")
        );
        assert_eq!(config.twitch.initial_channels, vec!["mychannel"]);
    }
}
