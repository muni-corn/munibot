// ai.rs: builds the chat service for the running server.

use std::sync::Arc;

use anyhow::Context;
use munibot_ai::{AiConfig, Chat};
use tracing::info;

/// Builds the chat service, or `None` when `ai.enabled` isn't `true` in
/// config.
///
/// # Errors
///
/// Fails startup, rather than leaving a chat page that can never answer, if
/// `ai.enabled` is `true` but no `ai.model` is set, or if rig can't connect to
/// the model (it isn't written as `vendor/model`, rig doesn't know the
/// vendor, or its api key isn't in the environment).
pub fn build(config: &AiConfig) -> anyhow::Result<Option<Arc<Chat>>> {
    if !config.enabled {
        info!("ai.enabled is false; chat is turned off");
        return Ok(None);
    }

    let model = config.model.as_deref().context(
        "ai.enabled is true but ai.model isn't set :< add something like `model = \
         \"anthropic/claude-opus-5\"` to the [ai] section of the config file",
    )?;
    let chat = Chat::connect(model).with_context(|| {
        format!(
            "couldn't connect to ai.model `{model}` :< it should be written as `vendor/model`, \
             like `anthropic/claude-opus-5`"
        )
    })?;

    info!(model, "chat is turned on");
    Ok(Some(Arc::new(chat)))
}
