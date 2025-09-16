use std::{fs, io::ErrorKind, path::Path};

use log::{info, warn};
use poise::serenity_prelude::UserId;
use serde::{Deserialize, Serialize};

use crate::MunibotError;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub db: DbConfig,
    pub discord: DiscordConfig,
    pub twitch: TwitchConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DbConfig {
    pub url: String,
    pub user: String,
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
    pub fn read_or_write_default_from<P: AsRef<Path>>(path: P) -> Result<Self, Box<MunibotError>> {
        let p = path.as_ref();

        // check if the path exists
        if !p.exists() {
            // construct default config
            let default = Config::default();

            // format it into a toml string
            let toml_string = toml::to_string_pretty(&default).map_err(|e| {
                MunibotError::LoadConfig(
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
                MunibotError::LoadConfig(
                    format!("couldn't read contents of {}", p.display()),
                    e.into(),
                )
            })?;

            // parse the string as toml
            let config = toml::from_str(&raw_string).map_err(|e| {
                MunibotError::LoadConfig(
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
            db: DbConfig {
                url: "127.0.0.1:7654".to_owned(),
                user: "muni_bot".to_owned(),
            },
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
