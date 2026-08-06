use dioxus::prelude::*;
use munibot_api::{
    chat::ChatEvent,
    server_fns::chat::{conversation::get_conversation_messages, stream::chat_stream},
};

use crate::components::{
    Spinner,
    chat::{
        composer::Composer,
        message_list::MessageList,
        tool_activity::{ToolActivityEntry, ToolActivityResult, record_finished, record_started},
        turn_failure::{TurnFailure, TurnFailureBanner},
    },
};

/// How many of a conversation's most recent messages load up front.
///
/// Loading older history a page at a time (the cursor
/// `get_conversation_messages` already accepts for it) isn't part of this phase
/// yet -- a companion conversation is rarely long enough for one page to
/// matter, and it's straightforward to add once it does.
const MESSAGE_PAGE_SIZE: i64 = 100;

/// One conversation's transcript and composer.
#[component]
pub fn ChatConversation(conversation_id: i64) -> Element {
    let mut messages = use_resource(move || async move {
        get_conversation_messages(conversation_id, None, MESSAGE_PAGE_SIZE).await
    });
    // `Some("")` the instant a turn starts, growing as text deltas arrive, and
    // `None` once it's over -- see MessageList's own doc comment for why this
    // can't just be another entry in `messages` itself
    let mut live_reply = use_signal(|| None::<String>);
    let mut tool_activity = use_signal(Vec::<ToolActivityEntry>::new);
    let mut turn_failure = use_signal(|| None::<TurnFailure>);
    // the last message id a turn was run for, so `retry` can re-ask munibot
    // to answer the same, already-persisted message rather than sending a
    // new one
    let mut last_message_id = use_signal(|| None::<i64>);

    let mut run_turn = move |message_id: i64| {
        turn_failure.set(None);
        live_reply.set(Some(String::new()));
        tool_activity.set(Vec::new());

        spawn(async move {
            match chat_stream(message_id).await {
                Ok(mut stream) => {
                    while let Some(event) = stream.recv().await {
                        match event {
                            Ok(ChatEvent::TextDelta(text)) => {
                                if let Some(reply) = live_reply.write().as_mut() {
                                    reply.push_str(&text);
                                }
                            }
                            Ok(ChatEvent::ToolStarted { name }) => {
                                record_started(&mut tool_activity.write(), name);
                            }
                            Ok(ChatEvent::ToolFinished {
                                name,
                                duration_ms,
                                ok,
                                result,
                            }) => {
                                record_finished(
                                    &mut tool_activity.write(),
                                    &name,
                                    ToolActivityResult {
                                        ok,
                                        duration_ms,
                                        result,
                                    },
                                );
                            }
                            Ok(ChatEvent::Failed { message, kind }) => {
                                turn_failure.set(Some(TurnFailure::from_event(kind, message)));
                            }
                            // not shown yet: TurnStarted/IterationComplete/Thinking/Handoff
                            // are persona-driven signals nothing here renders
                            Ok(_) => {}
                            Err(error) => {
                                turn_failure.set(Some(TurnFailure::from_transport_error(
                                    error.to_string(),
                                )));
                                break;
                            }
                        }
                    }
                }
                Err(error) => turn_failure.set(Some(TurnFailure::from_chat_error(&error))),
            }

            // cleared together, not separately: see MessageList's doc comment
            // on why the strip must disappear alongside the live reply rather
            // than being stranded once the persisted transcript reloads below
            live_reply.set(None);
            tool_activity.set(Vec::new());
            messages.restart();
        });
    };

    let on_sent = move |message_id: i64| {
        // the user's own message is already durably persisted by the time
        // send_message returns, so it's safe to reload the transcript right
        // away rather than waiting for the reply too
        messages.restart();
        last_message_id.set(Some(message_id));
        run_turn(message_id);
    };

    let retry = move |_| {
        if let Some(message_id) = *last_message_id.read() {
            run_turn(message_id);
        }
    };

    let content = match &*messages.read() {
        Some(Ok(loaded)) => rsx! {
            MessageList {
                messages: loaded.clone(),
                live_reply: live_reply.read().clone(),
                tool_activity: tool_activity.read().clone(),
            }
        },
        Some(Err(e)) => rsx! {
            div { class: "p-4 text-sm text-error", "couldn't load this conversation :< {e}" }
        },
        None => rsx! {
            div { class: "flex h-full place-content-center p-4", Spinner {} }
        },
    };

    rsx! {
        document::Title { "chat ~ munibot" }
        div { class: "flex h-full flex-col",
            div { class: "grow overflow-y-auto", {content} }
            if let Some(failure) = turn_failure.read().clone() {
                TurnFailureBanner { failure, on_retry: retry }
            }
            Composer {
                conversation_id,
                disabled: live_reply.read().is_some(),
                on_sent,
            }
        }
    }
}
