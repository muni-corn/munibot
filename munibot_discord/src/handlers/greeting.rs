use async_trait::async_trait;
use munibot_core::greeting::get_greeting_response;
use poise::serenity_prelude::{Context, FullEvent};

use crate::{
    DiscordFrameworkContext,
    handler::{DiscordEventHandler, DiscordHandlerError},
    utils::display_name_from_message,
};

/// Discord greeting handler. Uses munibot_core::greeting for the matching
/// logic, implementing only the Discord side.
pub struct GreetingHandler;

#[async_trait]
impl DiscordEventHandler for GreetingHandler {
    fn name(&self) -> &'static str {
        "greeting"
    }

    async fn handle_discord_event(
        &mut self,
        context: &Context,
        _framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        if let FullEvent::Message { new_message } = event {
            let msg = new_message;
            let author_name = display_name_from_message(msg, &context.http).await;

            if let Some(response) = get_greeting_response(&author_name, &msg.content)
                && msg.author.id != context.cache.current_user().id
            {
                msg.channel_id
                    .say(&context.http, response)
                    .await
                    .map_err(|e| DiscordHandlerError {
                        message: e.to_string(),
                        handler_name: self.name(),
                    })?;
            }
        };

        Ok(())
    }
}
