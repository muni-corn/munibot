//! munibot's conversation interface.
//!
//! A plain, temporary conversation with a model: [`Chat`] streams a reply to
//! whatever history it is handed and remembers nothing afterwards.
//!
//! Model access, messages, requests, streaming, usage, and errors are all
//! rig's own types, re-exported as [`rig`] so every crate that talks to the
//! chat service speaks the exact same version of them. This crate only adds
//! what rig doesn't have: munibot's configuration and the service itself.

pub mod config;
pub mod conversation;

pub use config::{AiConfig, ConfigError};
pub use conversation::Chat;
pub use rig_core as rig;
