use dioxus::{html::keyboard_types::Key, prelude::*};

/// The growing textarea a conversation is driven from: enter sends,
/// shift+enter inserts a newline.
///
/// `disabled` should cover a whole reply, not just the instant of sending, so
/// a second message can't be sent while munibot is still answering the first.
/// The draft is cleared as soon as it is handed to `on_send`.
#[component]
pub fn Composer(disabled: bool, on_send: EventHandler<String>) -> Element {
    let mut draft = use_signal(String::new);
    let is_empty = draft.read().trim().is_empty();

    let mut submit = move || {
        let text = draft.read().trim().to_string();
        if disabled || text.is_empty() {
            return;
        }
        draft.set(String::new());
        on_send.call(text);
    };

    rsx! {
        div { class: "border-t border-slate-800",
            div { class: "flex items-end gap-2 p-4",
                textarea {
                    class: "textarea w-full resize-none",
                    style: "field-sizing: content; max-height: 16rem;",
                    placeholder: "message munibot...",
                    disabled,
                    value: draft,
                    oninput: move |event| draft.set(event.value()),
                    onkeydown: move |event| {
                        if event.key() == Key::Named(NamedKey::Enter) && !event.modifiers().shift() {
                            event.prevent_default();
                            submit();
                        }
                    },
                }
                button {
                    class: "btn btn-primary",
                    title: "send",
                    disabled: disabled || is_empty,
                    onclick: move |_| submit(),
                    i { class: "ph-duotone ph-paper-plane-right" }
                }
            }
        }
    }
}
