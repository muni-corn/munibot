use dioxus::prelude::*;

/// One conversation's transcript and composer.
///
/// A minimal shell for now: the message list, composer, and streaming reply
/// all arrive in their own later commits.
#[component]
pub fn ChatConversation(conversation_id: i64) -> Element {
    rsx! {
        document::Title { "chat ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            p { class: "text-slate-300", "conversation #{conversation_id}" }
        }
    }
}
