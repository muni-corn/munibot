use std::time::{Duration, Instant};

use futures::StreamExt;
use munibot_ai::{
    Ai, AiError, AiTurnRequest,
    harness::HarnessEvent,
    persona::{OutputLimits, filter_output},
};
use poise::serenity_prelude::{ChannelId, EditMessage, Error as SerenityError, Http, Message};
use tracing::warn;

use super::DISCORD_MESSAGE_LIMIT;

/// The minimum time between edits to the in-progress placeholder, to respect
/// Discord's five-edits-per-five-seconds-per-channel rate limit with
/// headroom for other bot activity in the same channel.
const MIN_EDIT_INTERVAL: Duration = Duration::from_secs(1);

/// Something went wrong rendering a streamed turn to Discord.
///
/// Deliberately its own small type rather than
/// [`crate::error::DiscordHandlerError`]
/// or [`crate::error::MunibotDiscordError`] directly: this renderer is meant
/// to be usable from both an event handler and a slash command, which use
/// those two different error types, and each can convert this one into its
/// own.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("the ai turn failed :< {0}")]
    Ai(#[from] AiError),
    // boxed: serenity::Error is large enough on its own to blow up this whole enum's
    // size, the same reason MunibotDiscordError::Serenity boxes it too
    #[error("discord rejected a message :< {0}")]
    Discord(#[from] Box<SerenityError>),
}

impl From<SerenityError> for RenderError {
    fn from(error: SerenityError) -> Self {
        Self::Discord(Box::new(error))
    }
}

/// Runs `request` as a streamed turn, rendering it into a placeholder Discord
/// message that is edited no faster than once per second as text arrives,
/// with one final edit once the turn finishes.
///
/// If the finished reply is longer than Discord's own message limit, it is
/// split across follow-up messages at paragraph boundaries rather than
/// truncated - unlike an in-progress edit, which is capped with
/// [`filter_output`]'s own ellipsis truncation since it is never the final
/// word on what the reply says.
pub async fn render_streamed_reply(
    http: &Http,
    channel_id: ChannelId,
    ai: &Ai,
    request: AiTurnRequest,
) -> Result<(), RenderError> {
    let mut events = ai.turn_streamed(request).await?;

    let mut placeholder = channel_id.say(http, "_thinking..._").await?;
    let mut buffer = String::new();
    let mut active_tools: Vec<String> = Vec::new();
    let mut last_edit = Instant::now();
    let mut final_text: Option<String> = None;

    while let Some(event) = events.next().await {
        match event {
            HarnessEvent::TextDelta(text) => {
                buffer.push_str(&text);
                maybe_edit(
                    http,
                    &mut placeholder,
                    &active_tools,
                    &buffer,
                    &mut last_edit,
                )
                .await;
            }
            HarnessEvent::ToolStarted { name } => {
                active_tools.push(name);
                maybe_edit(
                    http,
                    &mut placeholder,
                    &active_tools,
                    &buffer,
                    &mut last_edit,
                )
                .await;
            }
            HarnessEvent::ToolFinished { name, .. } => {
                if let Some(position) = active_tools.iter().position(|active| active == &name) {
                    active_tools.remove(position);
                }
                maybe_edit(
                    http,
                    &mut placeholder,
                    &active_tools,
                    &buffer,
                    &mut last_edit,
                )
                .await;
            }
            HarnessEvent::TurnFinished { .. } => {
                final_text = Some(buffer.clone());
            }
            HarnessEvent::Failed(error) => {
                warn!(%error, "ai turn failed mid-stream");
                final_text = Some(if buffer.is_empty() {
                    "something went wrong on my end, sorry :<".to_string()
                } else {
                    buffer.clone()
                });
            }
            _ => {}
        }
    }

    render_final(
        http,
        channel_id,
        &mut placeholder,
        &final_text.unwrap_or(buffer),
    )
    .await
}

/// Edits the placeholder if [`MIN_EDIT_INTERVAL`] has passed since the last
/// one, resetting the timer when it does.
///
/// Shared by every event that can change what the preview should show - a
/// text delta, a tool starting, or a tool finishing - so all three respect
/// the same rate limit rather than each needing its own throttle check.
async fn maybe_edit(
    http: &Http,
    message: &mut Message,
    active_tools: &[String],
    buffer: &str,
    last_edit: &mut Instant,
) {
    if last_edit.elapsed() < MIN_EDIT_INTERVAL {
        return;
    }
    edit_preview(http, message, active_tools, buffer).await;
    *last_edit = Instant::now();
}

/// Best-effort: a failed intermediate edit is logged and skipped rather than
/// aborting the stream, since the next edit (or the final one) will simply
/// carry more text than the last successful edit showed.
async fn edit_preview(http: &Http, message: &mut Message, active_tools: &[String], buffer: &str) {
    let content = build_preview(active_tools, buffer);
    let preview = filter_output(&content, OutputLimits::new(DISCORD_MESSAGE_LIMIT));
    if let Err(error) = message
        .edit(http, EditMessage::new().content(preview))
        .await
    {
        warn!(%error, "couldn't edit ai response preview");
    }
}

/// Builds the in-progress preview: an italic status line naming every
/// currently running tool, above whatever assistant text has arrived so far.
/// Replaced entirely by the finished text once the turn ends - a tool that
/// already finished has nothing left to show here.
fn build_preview(active_tools: &[String], buffer: &str) -> String {
    if active_tools.is_empty() {
        return buffer.to_string();
    }

    let status = format!("_using {}..._", active_tools.join(", "));
    if buffer.is_empty() {
        status
    } else {
        format!("{status}\n\n{buffer}")
    }
}

/// Renders the finished reply: one last edit to the placeholder holding the
/// first chunk, and a follow-up message per additional chunk.
async fn render_final(
    http: &Http,
    channel_id: ChannelId,
    placeholder: &mut Message,
    full_text: &str,
) -> Result<(), RenderError> {
    // no length limit here - the chunking below is what enforces Discord's own
    // limit, at a paragraph boundary rather than filter_output's mid-word ellipsis
    let filtered = filter_output(full_text, OutputLimits::new(usize::MAX));
    let mut chunks = split_into_chunks(&filtered, DISCORD_MESSAGE_LIMIT).into_iter();

    let first = chunks.next().unwrap_or_default();
    placeholder
        .edit(http, EditMessage::new().content(first))
        .await?;

    for chunk in chunks {
        channel_id.say(http, chunk).await?;
    }

    Ok(())
}

/// Splits `text` into chunks of at most `limit` characters each, preferring a
/// blank-line paragraph boundary, then a single newline, and finally a hard
/// character split if a single paragraph itself exceeds `limit`.
fn split_into_chunks(text: &str, limit: usize) -> Vec<String> {
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.chars().count() <= limit {
            chunks.push(remaining.to_string());
            break;
        }

        let boundary = best_break_point(remaining, limit);
        let (chunk, rest) = remaining.split_at(boundary);
        let chunk = chunk.trim_end();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = rest.trim_start();
    }

    chunks
}

/// Finds the best byte offset at or before `limit` characters into `text` to
/// break a chunk, preferring a paragraph break, then a line break, then a
/// hard cut. Always at least `1`, so a caller advances on every call
/// regardless of how degenerate the input is.
fn best_break_point(text: &str, limit: usize) -> usize {
    let byte_limit = text
        .char_indices()
        .nth(limit)
        .map_or(text.len(), |(index, _)| index);
    let window = &text[..byte_limit];

    let boundary = if let Some(pos) = window.rfind("\n\n") {
        pos + 2
    } else if let Some(pos) = window.rfind('\n') {
        pos + 1
    } else {
        byte_limit
    };

    boundary.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_active_tools_and_no_text_yields_an_empty_preview() {
        assert_eq!(build_preview(&[], ""), "");
    }

    #[test]
    fn test_no_active_tools_shows_only_the_buffer() {
        assert_eq!(build_preview(&[], "hello there"), "hello there");
    }

    #[test]
    fn test_one_active_tool_with_no_text_yet_shows_only_the_status_line() {
        assert_eq!(
            build_preview(&["web_search".to_string()], ""),
            "_using web_search..._"
        );
    }

    #[test]
    fn test_one_active_tool_with_text_shows_both() {
        let preview = build_preview(&["web_search".to_string()], "here's what i found so far");
        assert_eq!(
            preview,
            "_using web_search..._\n\nhere's what i found so far"
        );
    }

    #[test]
    fn test_multiple_active_tools_are_all_named() {
        let tools = vec!["web_search".to_string(), "web_fetch".to_string()];
        assert_eq!(
            build_preview(&tools, ""),
            "_using web_search, web_fetch..._"
        );
    }

    #[test]
    fn test_short_text_is_a_single_chunk() {
        let chunks = split_into_chunks("hello", 2000);
        assert_eq!(chunks, vec!["hello".to_string()]);
    }

    #[test]
    fn test_text_at_exactly_the_limit_is_a_single_chunk() {
        let text = "a".repeat(10);
        let chunks = split_into_chunks(&text, 10);
        assert_eq!(chunks, vec![text]);
    }

    #[test]
    fn test_splits_at_a_paragraph_boundary_when_possible() {
        let text = format!("{}\n\n{}", "a".repeat(8), "b".repeat(8));
        let chunks = split_into_chunks(&text, 10);
        assert_eq!(chunks, vec!["a".repeat(8), "b".repeat(8)]);
    }

    #[test]
    fn test_falls_back_to_a_single_newline_when_no_paragraph_break_exists() {
        let text = format!("{}\n{}", "a".repeat(8), "b".repeat(8));
        let chunks = split_into_chunks(&text, 10);
        assert_eq!(chunks, vec!["a".repeat(8), "b".repeat(8)]);
    }

    #[test]
    fn test_falls_back_to_a_hard_split_when_a_single_paragraph_exceeds_the_limit() {
        let text = "a".repeat(25);
        let chunks = split_into_chunks(&text, 10);

        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 10));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn test_every_chunk_respects_the_limit_across_many_paragraphs() {
        let paragraphs: Vec<String> = (0..20).map(|n| format!("paragraph number {n}")).collect();
        let text = paragraphs.join("\n\n");

        let chunks = split_into_chunks(&text, 40);

        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 40));
        // rejoining with the same separator the input used should reconstruct it,
        // proving no paragraph was dropped or duplicated
        assert_eq!(chunks.join("\n\n"), text);
    }

    #[test]
    fn test_a_zero_limit_still_terminates_rather_than_looping_forever() {
        // a degenerate input that should never occur in practice, but must not hang
        let chunks = split_into_chunks(&"a".repeat(5), 0);
        assert_eq!(chunks.concat(), "a".repeat(5));
    }

    #[test]
    fn test_empty_text_yields_a_single_empty_chunk() {
        let chunks = split_into_chunks("", 10);
        assert_eq!(chunks, vec![String::new()]);
    }

    #[test]
    fn test_multibyte_text_is_split_on_a_character_boundary_not_a_byte_boundary() {
        // each "é" is two bytes but one character; a byte-based split would either
        // panic on a non-boundary slice or produce mangled output
        let text = "é".repeat(15);
        let chunks = split_into_chunks(&text, 10);

        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 10));
        assert_eq!(chunks.concat(), text);
    }
}
