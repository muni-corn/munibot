use async_trait::async_trait;
use twitch_irc::message::ServerMessage;

use crate::{
    config::Config,
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

pub struct ShoutoutHandler;

/// Builds a list of Twitch chat messages for a multi-shoutout command.
///
/// Splits targets across multiple messages if needed to stay under `max_len`
/// characters. Strips leading `@` from each target name.
fn build_multi_shoutout_messages(targets_raw: &str, max_len: usize) -> Vec<String> {
    let header = "go check out these cuties! :3";
    let mut messages = Vec::new();
    let mut current = header.to_string();

    for mut target in targets_raw.split_whitespace() {
        target = target.trim_start_matches('@');
        let link = format!(" https://twitch.tv/{target}");

        // if adding this link would exceed the limit, flush the current message
        if current.len() + link.len() >= max_len {
            messages.push(current);
            current = link.trim().to_string();
        } else {
            current.push_str(&link);
        }
    }

    if !current.is_empty() {
        messages.push(current);
    }

    messages
}

#[async_trait]
impl TwitchMessageHandler for ShoutoutHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        if let ServerMessage::Privmsg(msg) = message {
            // accept either !so or !shoutout
            if let Some(target) = msg
                .message_text
                .strip_prefix("!so ")
                .or_else(|| msg.message_text.strip_prefix("!shoutout "))
                // strip @ from the front of the username if it's there
                .map(|s| s.trim_start_matches('@'))
            {
                let message = format!(
                    "this is a PSA that you NEED to go check out {target} at https://twitch.tv/{target} ! :3 clearly they deserve the shoutout, so go follow them now >:c"
                );

                // send the message
                self.send_twitch_message(client, &msg.channel_login, &message)
                    .await?;

                Ok(true)
            } else if let Some(targets_raw) = msg.message_text.strip_prefix("!mso ") {
                // multi-shoutouts
                let messages = build_multi_shoutout_messages(targets_raw, 500);
                for message in messages {
                    self.send_twitch_message(client, &msg.channel_login, &message)
                        .await?;
                }

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_multi_shoutout_messages;

    #[test]
    fn test_single_target_produces_one_message() {
        let messages = build_multi_shoutout_messages("coolstreamer", 500);
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains("coolstreamer"),
            "expected target in message"
        );
    }

    #[test]
    fn test_at_symbol_stripped_from_target() {
        let messages = build_multi_shoutout_messages("@coolstreamer", 500);
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains("https://twitch.tv/coolstreamer"),
            "expected @ to be stripped; got '{}'",
            messages[0]
        );
    }

    #[test]
    fn test_multiple_targets_within_limit_produces_one_message() {
        let messages = build_multi_shoutout_messages("alice bob carol", 500);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("alice"));
        assert!(messages[0].contains("bob"));
        assert!(messages[0].contains("carol"));
    }

    #[test]
    fn test_many_targets_splits_when_over_limit() {
        // create enough long names that they won't all fit in 500 chars
        let targets = (0..20)
            .map(|i| format!("averylongnamethatishard{i:04}"))
            .collect::<Vec<_>>()
            .join(" ");
        let messages = build_multi_shoutout_messages(&targets, 500);
        assert!(
            messages.len() > 1,
            "expected multiple messages for many long targets"
        );
    }

    #[test]
    fn test_each_message_stays_within_limit() {
        let targets = (0..30)
            .map(|i| format!("streamer{i:05}"))
            .collect::<Vec<_>>()
            .join(" ");
        let messages = build_multi_shoutout_messages(&targets, 500);
        for (i, msg) in messages.iter().enumerate() {
            assert!(
                msg.len() < 500,
                "message {i} exceeds 500 chars (len={}): '{msg}'",
                msg.len()
            );
        }
    }

    #[test]
    fn test_empty_targets_produces_just_header() {
        let messages = build_multi_shoutout_messages("", 500);
        // empty input: only the header is flushed
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("cuties"));
    }
}
