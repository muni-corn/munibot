use crate::chat::{ChatError, ChatMessage, ChatRole};

/// The most messages one conversation may hold.
///
/// The browser keeps the conversation and sends all of it every turn, so the
/// server bounds what it will forward to a paid model.
pub const MAX_CONVERSATION_MESSAGES: usize = 200;

/// The most characters one conversation may hold in total, across every
/// message.
pub const MAX_CONVERSATION_CHARS: usize = 100_000;

/// Checks that `messages` is a conversation munibot can answer: not empty,
/// within [`MAX_CONVERSATION_MESSAGES`] and [`MAX_CONVERSATION_CHARS`], and
/// ending with something a person said.
///
/// # Errors
///
/// Returns [`ChatError::InvalidConversation`], with a reason fit to show
/// verbatim, if any of those doesn't hold.
pub fn validate_conversation(messages: &[ChatMessage]) -> Result<(), ChatError> {
    let Some(last) = messages.last() else {
        return Err(ChatError::InvalidConversation(
            "there's nothing to reply to yet :< send a message first".to_string(),
        ));
    };

    if last.role != ChatRole::User || last.content.trim().is_empty() {
        return Err(ChatError::InvalidConversation(
            "munibot can only reply to a message from you :<".to_string(),
        ));
    }

    if messages.len() > MAX_CONVERSATION_MESSAGES {
        return Err(ChatError::InvalidConversation(format!(
            "this conversation has run past {MAX_CONVERSATION_MESSAGES} messages :< start a new \
             one to keep chatting"
        )));
    }

    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    if total_chars > MAX_CONVERSATION_CHARS {
        return Err(ChatError::InvalidConversation(
            "this conversation has gotten too long :< start a new one to keep chatting".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_conversation_ending_in_a_user_message_is_valid() {
        let messages = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello!"),
            ChatMessage::user("how are you?"),
        ];
        assert!(validate_conversation(&messages).is_ok());
    }

    #[test]
    fn test_an_empty_conversation_is_rejected() {
        assert!(matches!(
            validate_conversation(&[]),
            Err(ChatError::InvalidConversation(_))
        ));
    }

    #[test]
    fn test_a_conversation_ending_in_an_assistant_message_is_rejected() {
        let messages = vec![ChatMessage::user("hi"), ChatMessage::assistant("hello!")];
        assert!(validate_conversation(&messages).is_err());
    }

    #[test]
    fn test_a_blank_final_message_is_rejected() {
        let messages = vec![ChatMessage::user("   \n")];
        assert!(validate_conversation(&messages).is_err());
    }

    #[test]
    fn test_too_many_messages_is_rejected() {
        let messages: Vec<_> = (0..=MAX_CONVERSATION_MESSAGES)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user("hi")
                } else {
                    ChatMessage::assistant("hello")
                }
            })
            .collect();
        assert_eq!(messages.len(), MAX_CONVERSATION_MESSAGES + 1);
        assert!(validate_conversation(&messages).is_err());
    }

    #[test]
    fn test_too_many_characters_is_rejected() {
        let messages = vec![ChatMessage::user("a".repeat(MAX_CONVERSATION_CHARS + 1))];
        assert!(validate_conversation(&messages).is_err());
    }

    #[test]
    fn test_exactly_the_character_limit_is_allowed() {
        let messages = vec![ChatMessage::user("a".repeat(MAX_CONVERSATION_CHARS))];
        assert!(validate_conversation(&messages).is_ok());
    }

    #[test]
    fn test_characters_are_counted_not_bytes() {
        // each of these is several bytes but one character
        let messages = vec![ChatMessage::user("🦄".repeat(MAX_CONVERSATION_CHARS))];
        assert!(validate_conversation(&messages).is_ok());
    }
}
