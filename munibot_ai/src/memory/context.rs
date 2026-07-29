use crate::{
    memory::{Conversation, SessionStore},
    types::{AiError, History, Message, Role},
};

/// Assembles the history to hand a provider for one turn.
///
/// Walks a conversation's stored messages newest-first, keeping whole messages
/// until `max_tokens` is spent, then restores oldest-first order - a message is
/// never partially kept, since a broken tool-use/tool-result pair would confuse
/// a provider expecting every call it made to be answered in the same request.
/// The single most recent message is always kept even if it alone exceeds
/// `max_tokens`, on the same principle the harness's own budget truncation
/// uses: some context beats none.
///
/// When the conversation carries a summary, it is prepended as a **user**-role
/// message, not a system-role one. Two reasons: the harness already sends a
/// persona's own system prompt as [`crate::harness::TurnRequest::system`], a
/// separate field from history entirely, so a system-role message injected here
/// would sit alongside it as a second, competing system turn on the wire; and
/// some providers require a conversation to open with a user turn, which a
/// system-role message at the front would violate.
pub async fn assemble_context(
    store: &dyn SessionStore,
    conversation: &Conversation,
    max_tokens: usize,
    counter: impl Fn(&str) -> usize,
) -> Result<History, AiError> {
    let history = store.history(conversation.id, None).await?;
    let messages = history.into_messages();

    let mut selected: Vec<Message> = Vec::new();
    let mut used_tokens = 0usize;

    for message in messages.into_iter().rev() {
        let message_tokens = message_tokens(&message, &counter);
        if used_tokens + message_tokens > max_tokens && !selected.is_empty() {
            break;
        }
        used_tokens += message_tokens;
        selected.push(message);
    }

    selected.reverse();

    // the token walk can land on the boundary between a tool result and the tool
    // call it answers, exactly as the in-memory store's own message cap can -
    // the same cascade fixes it
    while selected
        .first()
        .is_some_and(|message| message.role == Role::Tool)
    {
        selected.remove(0);
    }

    let mut result = Vec::with_capacity(selected.len() + 1);
    if let Some(summary) = &conversation.summary {
        result.push(Message::user(format!(
            "Here is a summary of earlier parts of this conversation that are no longer shown in \
             full: {summary}"
        )));
    }
    result.extend(selected);

    Ok(History::from(result))
}

/// Estimates one message's token cost the same way [`History::token_estimate`]
/// does, but for a single message rather than a whole history - the walk in
/// [`assemble_context`] needs to know each message's cost individually as it
/// decides whether to keep it.
fn message_tokens(message: &Message, counter: &impl Fn(&str) -> usize) -> usize {
    History::from(vec![message.clone()]).token_estimate(counter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory::{ConversationScope, InMemorySessionStore},
        tools::Platform,
        types::{ContentBlock, rough_token_estimate},
    };

    async fn conversation_with(store: &InMemorySessionStore, messages: &[Message]) -> Conversation {
        let scope = ConversationScope::new(Platform::Discord, "channel-1");
        let conversation = store.load_or_create(&scope, "companion").await.unwrap();
        for message in messages {
            store
                .append(conversation.id, message.clone())
                .await
                .unwrap();
        }
        conversation
    }

    fn texts_of(history: &History) -> Vec<String> {
        history.iter().map(Message::text).collect()
    }

    #[tokio::test]
    async fn test_empty_conversation_yields_empty_context() {
        let store = InMemorySessionStore::new();
        let conversation = conversation_with(&store, &[]).await;

        let context = assemble_context(&store, &conversation, 1000, rough_token_estimate)
            .await
            .unwrap();
        assert!(context.is_empty());
    }

    #[tokio::test]
    async fn test_generous_budget_keeps_every_message() {
        let store = InMemorySessionStore::new();
        let messages = vec![
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
        ];
        let conversation = conversation_with(&store, &messages).await;

        let context = assemble_context(&store, &conversation, 10_000, rough_token_estimate)
            .await
            .unwrap();
        assert_eq!(texts_of(&context), vec![
            "one".to_string(),
            "two".to_string(),
            "three".to_string()
        ]);
    }

    #[tokio::test]
    async fn test_tight_budget_keeps_only_the_most_recent_messages() {
        let store = InMemorySessionStore::new();
        // each message is one word; a budget of one token (four characters) per
        // rough_token_estimate keeps roughly one short message
        let messages = vec![
            Message::user("aaaa"),
            Message::user("bbbb"),
            Message::user("cccc"),
        ];
        let conversation = conversation_with(&store, &messages).await;

        let context = assemble_context(&store, &conversation, 1, rough_token_estimate)
            .await
            .unwrap();

        assert_eq!(
            texts_of(&context),
            vec!["cccc".to_string()],
            "a tight budget should keep only the most recent message that fits"
        );
    }

    #[tokio::test]
    async fn test_zero_budget_still_keeps_the_single_most_recent_message() {
        let store = InMemorySessionStore::new();
        let messages = vec![
            Message::user("one"),
            Message::user("a much longer second message"),
        ];
        let conversation = conversation_with(&store, &messages).await;

        let context = assemble_context(&store, &conversation, 0, rough_token_estimate)
            .await
            .unwrap();

        assert_eq!(
            context.len(),
            1,
            "some context beats none - the most recent message should survive even a zero budget"
        );
        assert_eq!(texts_of(&context), vec![
            "a much longer second message".to_string()
        ]);
    }

    #[tokio::test]
    async fn test_selected_messages_remain_oldest_first() {
        let store = InMemorySessionStore::new();
        let messages = vec![
            Message::user("one"),
            Message::user("two"),
            Message::user("three"),
        ];
        let conversation = conversation_with(&store, &messages).await;

        let context = assemble_context(&store, &conversation, 10_000, rough_token_estimate)
            .await
            .unwrap();

        assert_eq!(texts_of(&context), vec![
            "one".to_string(),
            "two".to_string(),
            "three".to_string()
        ]);
    }

    #[tokio::test]
    async fn test_a_tool_use_and_result_pair_split_by_the_budget_is_dropped_together() {
        use serde_json::json;

        let store = InMemorySessionStore::new();
        let tool_call = Message::new(Role::Assistant, vec![ContentBlock::tool_use(
            "c1",
            "current_time",
            json!({}),
        )]);
        let tool_result = Message::tool_results(vec![ContentBlock::tool_result("c1", "12:00")]);

        // budget fits only the tool result on its own, not the tool call that precedes
        // it - proving the cascade removes the orphan rather than sending a
        // dangling result
        let conversation = conversation_with(&store, &[tool_call, tool_result]).await;

        let context = assemble_context(&store, &conversation, 1, rough_token_estimate)
            .await
            .unwrap();

        assert!(
            context
                .iter()
                .next()
                .is_none_or(|message| message.role != Role::Tool),
            "the context must never open with an orphaned tool result"
        );
    }

    #[tokio::test]
    async fn test_summary_is_prepended_as_a_user_message() {
        let store = InMemorySessionStore::new();
        let conversation = conversation_with(&store, &[Message::user("hi")]).await;
        store
            .set_summary(conversation.id, "we talked about cats".to_string())
            .await
            .unwrap();
        let conversation = store
            .load_or_create(
                &ConversationScope::new(Platform::Discord, "channel-1"),
                "companion",
            )
            .await
            .unwrap();

        let context = assemble_context(&store, &conversation, 10_000, rough_token_estimate)
            .await
            .unwrap();

        let first = context
            .iter()
            .next()
            .expect("should have a leading message");
        assert_eq!(
            first.role,
            Role::User,
            "the summary must be injected as a user message"
        );
        assert!(first.text().contains("we talked about cats"));
    }

    #[tokio::test]
    async fn test_no_summary_means_no_extra_leading_message() {
        let store = InMemorySessionStore::new();
        let conversation = conversation_with(&store, &[Message::user("hi")]).await;

        let context = assemble_context(&store, &conversation, 10_000, rough_token_estimate)
            .await
            .unwrap();

        assert_eq!(
            context.len(),
            1,
            "no summary should mean no message is added"
        );
    }
}
