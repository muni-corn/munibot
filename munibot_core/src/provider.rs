// munibot_core::provider: the ConfigurationProvider trait, its
// registration, and the plumbing that lets the gui talk to any registered
// provider without knowing which ones exist.
//
// named `provider`, not `config`: munibot_core::config already exists (the
// file-based bot Config/DiscordConfig/TwitchConfig) and is an unrelated
// concern -- see docs/notes/config-schema-implementation-notes.md.

pub mod context;
pub mod dyn_provider;
pub mod typed;

pub use context::{ConfigContext, ConfigError};
pub use dyn_provider::DynConfigProvider;
pub use typed::{ConfigurationProvider, RefOption};
