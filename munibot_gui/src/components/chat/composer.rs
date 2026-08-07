use dioxus::{html::keyboard_types::Key, prelude::*};
use munibot_api::server_fns::chat::message::send_message;

use crate::pages::chat::ChatDrafts;

/// The growing textarea a conversation is driven from: enter sends,
/// shift+enter inserts a newline, and the draft survives navigating away
/// and back (see [`ChatDrafts`]).
///
/// `disabled` is meant to cover a whole turn, not just this component's own
/// brief `send_message` round trip -- the parent sets it once streaming (a
/// later commit) is actually in flight. Until then, this component's own
/// internal `sending` is the only thing disabling it, covering the moment
/// between clicking send and the message finishing its persist, so a
/// double click can't submit the same draft twice.
#[component]
pub fn Composer(conversation_id: i64, disabled: bool, on_sent: EventHandler<i64>) -> Element {
    let mut drafts = use_context::<ChatDrafts>();
    let mut sending = use_signal(|| false);

    let draft = drafts
        .0
        .read()
        .get(&conversation_id)
        .cloned()
        .unwrap_or_default();
    let is_empty = draft.trim().is_empty();

    let mut submit = move || {
        if *sending.read() {
            return;
        }
        let text = drafts
            .0
            .read()
            .get(&conversation_id)
            .cloned()
            .unwrap_or_default();
        if text.trim().is_empty() {
            return;
        }

        sending.set(true);
        spawn(async move {
            // the draft is left in place on failure, so nothing typed is ever
            // lost -- structural, ChatError-aware retry handling arrives in a
            // later commit. attaching images is a later commit too, so this
            // is always empty for now.
            if let Ok(message_id) = send_message(conversation_id, text, Vec::new()).await {
                drafts.0.write().remove(&conversation_id);
                on_sent.call(message_id);
            }
            sending.set(false);
        });
    };

    let is_disabled = disabled || *sending.read();

    rsx! {
        div { class: "flex items-end gap-2 border-t border-slate-800 p-4",
            textarea {
                class: "textarea w-full resize-none",
                style: "field-sizing: content; max-height: 16rem;",
                placeholder: "message munibot...",
                disabled: is_disabled,
                value: draft,
                oninput: move |event| {
                    drafts.0.write().insert(conversation_id, event.value());
                },
                onkeydown: move |event| {
                    if event.key() == Key::Enter && !event.modifiers().shift() {
                        event.prevent_default();
                        submit();
                    }
                },
            }
            button {
                class: "btn btn-primary",
                disabled: is_disabled || is_empty,
                onclick: move |_| submit(),
                i { class: "ph-duotone ph-paper-plane-right" }
            }
        }
    }
}
