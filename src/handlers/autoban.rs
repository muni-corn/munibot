use twitch_irc::message::ServerMessage;

use crate::{
    config::Config,
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

pub struct AutoBanHandler;

#[async_trait::async_trait]
impl TwitchMessageHandler for AutoBanHandler {
    /// Handle a new message from chat. Returns `true` if something was done to
    /// handle the message, or `false` if the message was ignored (or if the
    /// message is allowed to also be handled by other handlers).
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        _client: &MuniBotTwitchIRCClient,
        agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let (user_login, channel_login, msg_content) = match message {
            ServerMessage::Join(join_msg) => (
                join_msg.user_login.to_lowercase(),
                join_msg.channel_login.as_str(),
                "",
            ),
            ServerMessage::Privmsg(privmsg) => (
                privmsg.sender.login.to_lowercase(),
                privmsg.channel_login.as_str(),
                privmsg.message_text.as_str(),
            ),
            _ => return Ok(false),
        };

        if user_login.contains("isapred") || user_login.contains("isabadstreamer") {
            yeet_user(
                agent,
                &user_login,
                channel_login,
                "user is suspected of harassment",
            )
            .await?;
            Ok(true)
        } else if matches_scam_message(msg_content).map_err(|e| {
            TwitchHandlerError::Other(format!("couldn't sanitize homoglyphed message: {e}"))
        })? {
            yeet_user(agent, &user_login, channel_login, "likely viewer scam bot").await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

async fn yeet_user(
    agent: &TwitchAgent<'_>,
    user_login: &str,
    channel_login: &str,
    reason: &str,
) -> Result<(), TwitchHandlerError> {
    let Some(broadcaster_id) = agent.get_user_from_login(channel_login).await? else {
        return Err(TwitchHandlerError::Other(format!(
            "could not get broadcaster id for channel {channel_login}"
        )));
    };

    let Some(ban_user_id) = agent.get_user_from_login(user_login).await? else {
        return Err(TwitchHandlerError::Other(format!(
            "could not get user id for user {user_login} on channel {channel_login}"
        )));
    };

    agent
        .ban_user(&ban_user_id.id, reason, &broadcaster_id.id)
        .await?;

    Ok(())
}

fn matches_scam_message(msg_content: &str) -> Result<bool, decancer::Error> {
    // match by "cured" message (free of homoglyphs)
    let cured_string = decancer::cure!(msg_content)?;

    Ok(!cured_string
        .find_multiple(["cheap viewers on", "best viewers on"])
        .is_empty())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_matches_scam_message() {
        let msg = "𝕔𝕙𝕖𝕒𝕡 𝕧𝕚𝕖𝕨𝕖𝕣𝕤 𝕠𝕟 scam.url";
        assert!(super::matches_scam_message(msg).unwrap());

        let msg = "b︢e︢st v︢ie︢we︣rs o︣n scam.url";
        assert!(super::matches_scam_message(msg).unwrap());
    }
}
