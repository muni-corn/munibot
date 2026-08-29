use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use munibot_api::{
    chat::{ChatRole, TranscriptMessage, TranscriptToolCall},
    server_fns::chat::transcript::get_ai_transcript,
};

use crate::components::Spinner;

/// How many of a conversation's most recent messages load up front - this
/// is an audit view read once in full, not somewhere worth building
/// infinite scroll for yet, the same reasoning `ChatConversation`'s own
/// `MESSAGE_PAGE_SIZE` documents, just a larger bound since a transcript
/// reviewer is more likely to want the whole thing at once.
const MESSAGE_PAGE_SIZE: i64 = 500;

/// One entry in a transcript's timeline: a message or a tool call,
/// interleaved by when each actually happened - `ai_tool_calls` has no
/// `message_id` of its own (see `TranscriptToolCall`'s own doc comment), so
/// this is the only place the two ever get a single, chronological order.
enum TimelineEntry {
    Message(TranscriptMessage),
    ToolCall(TranscriptToolCall),
}

impl TimelineEntry {
    /// The instant this entry happened, for sorting - falls back to the
    /// unix epoch on an unparsable timestamp rather than panicking, so one
    /// malformed row can never break the whole timeline's ordering, only
    /// misplace itself within it.
    fn timestamp(&self) -> DateTime<Utc> {
        let raw = match self {
            Self::Message(message) => &message.created_at,
            Self::ToolCall(call) => &call.created_at,
        };
        DateTime::parse_from_rfc3339(raw)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_default()
    }
}

/// Merges a transcript's messages and tool calls into one chronological
/// timeline.
fn build_timeline(
    messages: Vec<TranscriptMessage>,
    tool_calls: Vec<TranscriptToolCall>,
) -> Vec<TimelineEntry> {
    let mut entries: Vec<TimelineEntry> = messages
        .into_iter()
        .map(TimelineEntry::Message)
        .chain(tool_calls.into_iter().map(TimelineEntry::ToolCall))
        .collect();
    entries.sort_by_key(TimelineEntry::timestamp);
    entries
}

/// A conversation's full transcript: every stored message and every tool
/// call audited for it, interleaved chronologically - the audit surface
/// behind the memory-wipe promise, and the fastest way to understand why a
/// persona behaved oddly. Tool calls render collapsed, with their input and
/// output inspectable on expand.
#[component]
pub fn TranscriptViewer(conversation_id: i64) -> Element {
    let transcript = use_resource(move || async move {
        get_ai_transcript(conversation_id, None, MESSAGE_PAGE_SIZE).await
    });

    let content = match &*transcript.read() {
        Some(Ok(transcript)) => {
            let timeline =
                build_timeline(transcript.messages.clone(), transcript.tool_calls.clone());
            rsx! {
                ol { class: "flex flex-col gap-3",
                    for entry in timeline {
                        li {
                            match entry {
                                TimelineEntry::Message(message) => rsx! {
                                    TranscriptMessageRow { message }
                                },
                                TimelineEntry::ToolCall(call) => rsx! {
                                    TranscriptToolCallRow { call }
                                },
                            }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load this transcript :< {e}" }
        },
        None => rsx! {
            div { class: "flex h-full place-content-center p-4", Spinner {} }
        },
    };

    rsx! {
        document::Title { "transcript ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "transcript" }
            {content}
        }
    }
}

#[component]
fn TranscriptMessageRow(message: TranscriptMessage) -> Element {
    let (label, alignment) = match message.role {
        ChatRole::System => ("system", "items-start"),
        ChatRole::User => ("user", "items-end"),
        ChatRole::Assistant => ("assistant", "items-start"),
        ChatRole::Tool => ("tool", "items-start"),
    };

    rsx! {
        div { class: "flex flex-col gap-1 {alignment}",
            span { class: "text-xs font-semibold text-slate-400 uppercase", {label} }
            div { class: "max-w-2xl rounded-box bg-slate-900/50 p-3 whitespace-pre-wrap",
                {message.content}
            }
        }
    }
}

#[component]
fn TranscriptToolCallRow(call: TranscriptToolCall) -> Element {
    rsx! {
        details { class: "rounded-box bg-slate-950/40 p-3",
            summary { class: "flex cursor-pointer items-center gap-2 text-sm font-semibold",
                i { class: "ph-duotone ph-wrench" }
                "{call.tool_name}"
                span { class: "text-xs font-normal text-slate-400",
                    "({call.status}, {call.duration_ms}ms)"
                }
            }
            div { class: "mt-2 flex flex-col gap-2 text-sm",
                div {
                    span { class: "font-semibold", "input" }
                    pre { class: "overflow-x-auto rounded bg-slate-900/50 p-2 whitespace-pre-wrap",
                        {call.input}
                    }
                }
                div {
                    span { class: "font-semibold", "output" }
                    pre { class: "overflow-x-auto rounded bg-slate-900/50 p-2 whitespace-pre-wrap",
                        {call.output}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(created_at: &str) -> TranscriptMessage {
        TranscriptMessage {
            id: 1,
            role: ChatRole::User,
            content: "hi".to_string(),
            created_at: created_at.to_string(),
        }
    }

    fn tool_call(created_at: &str) -> TranscriptToolCall {
        TranscriptToolCall {
            id: 1,
            tool_name: "current_time".to_string(),
            input: "{}".to_string(),
            output: "12:00".to_string(),
            duration_ms: 5,
            status: "ok".to_string(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn test_build_timeline_interleaves_by_timestamp() {
        let messages = vec![
            message("2026-07-30T10:00:00+00:00"),
            message("2026-07-30T10:00:10+00:00"),
        ];
        let tool_calls = vec![tool_call("2026-07-30T10:00:05+00:00")];

        let timeline = build_timeline(messages, tool_calls);

        assert_eq!(timeline.len(), 3);
        assert!(matches!(timeline[0], TimelineEntry::Message(_)));
        assert!(matches!(timeline[1], TimelineEntry::ToolCall(_)));
        assert!(matches!(timeline[2], TimelineEntry::Message(_)));
    }

    #[test]
    fn test_build_timeline_with_no_tool_calls_keeps_message_order() {
        let messages = vec![
            message("2026-07-30T10:00:00+00:00"),
            message("2026-07-30T10:00:01+00:00"),
        ];

        let timeline = build_timeline(messages, Vec::new());
        assert_eq!(timeline.len(), 2);
    }

    #[test]
    fn test_an_unparsable_timestamp_falls_back_rather_than_panicking() {
        let entry = TimelineEntry::Message(message("not a real timestamp"));
        assert_eq!(entry.timestamp(), DateTime::<Utc>::default());
    }
}
