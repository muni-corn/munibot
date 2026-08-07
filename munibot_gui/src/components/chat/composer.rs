use base64::{Engine, engine::general_purpose::STANDARD};
use dioxus::{
    html::{HasFileData, keyboard_types::Key},
    prelude::*,
};
use munibot_api::{
    chat::{ALLOWED_MEDIA_TYPES, MAX_ATTACHMENT_BYTES},
    server_fns::chat::{attachment::upload_attachment, message::send_message},
};

use crate::pages::chat::ChatDrafts;

/// The element id the paste listener (see [`Composer`]'s own doc comment)
/// looks itself up by, and the drop target for drag-and-drop.
const DROP_ZONE_ID: &str = "composer-drop-zone";

/// One image on its way into the conversation, from pick, drag, or paste,
/// through to being usable as `send_message`'s own `attachment_ids`.
#[derive(Clone, PartialEq)]
struct PendingAttachment {
    /// Distinguishes entries added and removed out of order - an index
    /// would drift the moment two uploads race or one is removed before
    /// another resolves.
    key: u64,
    /// A `data:` URL built locally from the same bytes just read, so the
    /// thumbnail shows immediately with no round trip back to the server -
    /// `/attachments/{id}` (for a message already sent) is a different
    /// concern.
    preview_url: String,
    status: AttachmentStatus,
}

#[derive(Clone, PartialEq)]
enum AttachmentStatus {
    Uploading,
    Uploaded {
        id: i64,
    },
    /// Carries why, so a rejected upload says something more useful than a
    /// generic failure - same reasoning as `ChatError::AttachmentRejected`
    /// itself, which is usually where `reason` comes from verbatim.
    Failed {
        reason: String,
    },
}

/// What the paste listener's JavaScript hands back over `document::eval`'s
/// channel for each pasted image.
#[derive(serde::Deserialize)]
struct PastedImage {
    media_type: String,
    data: String,
}

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
///
/// Pasting a screenshot has to be a non-event, so it gets a listener of its
/// own rather than relying on dioxus's own paste event: `ClipboardData`
/// exposes nothing dioxus can hand back as a `FileData` (unlike a file
/// input's `FormData` or drag-and-drop's `DragData`), so image extraction
/// happens in real JavaScript through `document::eval`'s two-way channel,
/// the same escape hatch this file's own `CopyButton` neighbour already
/// uses. The listener is attached to the drop zone element itself (not
/// `document`), so it is torn down for free with that element on every
/// remount rather than needing to be manually removed.
#[component]
pub fn Composer(conversation_id: i64, disabled: bool, on_sent: EventHandler<i64>) -> Element {
    let mut drafts = use_context::<ChatDrafts>();
    let mut sending = use_signal(|| false);
    let mut attachments = use_signal(Vec::<PendingAttachment>::new);
    let next_key = use_signal(|| 0u64);
    let mut drag_hovered = use_signal(|| false);

    let draft = drafts
        .0
        .read()
        .get(&conversation_id)
        .cloned()
        .unwrap_or_default();

    let has_pending_upload = attachments
        .read()
        .iter()
        .any(|attachment| matches!(attachment.status, AttachmentStatus::Uploading));
    let is_empty = draft.trim().is_empty() && attachments.read().is_empty();

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
        let attachment_ids: Vec<i64> = attachments
            .read()
            .iter()
            .filter_map(|attachment| match attachment.status {
                AttachmentStatus::Uploaded { id } => Some(id),
                _ => None,
            })
            .collect();
        if text.trim().is_empty() && attachment_ids.is_empty() {
            return;
        }

        sending.set(true);
        spawn(async move {
            // the draft is left in place on failure, so nothing typed is ever
            // lost -- structural, ChatError-aware retry handling arrives in a
            // later commit
            if let Ok(message_id) = send_message(conversation_id, text, attachment_ids).await {
                drafts.0.write().remove(&conversation_id);
                attachments.write().clear();
                on_sent.call(message_id);
            }
            sending.set(false);
        });
    };

    let is_disabled = disabled || *sending.read() || has_pending_upload;

    rsx! {
        div {
            id: DROP_ZONE_ID,
            class: if *drag_hovered.read() { "border-t border-primary bg-primary/5" } else { "border-t border-slate-800" },
            ondragover: move |event| {
                event.prevent_default();
                drag_hovered.set(true);
            },
            ondragleave: move |_| drag_hovered.set(false),
            ondrop: move |event| {
                event.prevent_default();
                drag_hovered.set(false);
                let files = event.files();
                async move {
                    for file in files {
                        queue_upload(conversation_id, attachments, next_key, file).await;
                    }
                }
            },
            onmounted: move |_| {
                spawn(listen_for_pasted_images(conversation_id, attachments, next_key));
            },
            if !attachments.read().is_empty() {
                div { class: "flex flex-wrap gap-2 px-4 pt-4",
                    for attachment in attachments.read().iter().cloned() {
                        AttachmentThumbnail {
                            key: "{attachment.key}",
                            attachment: attachment.clone(),
                            on_remove: move |key| {
                                attachments.write().retain(|a| a.key != key);
                            },
                        }
                    }
                }
            }
            div { class: "flex items-end gap-2 p-4",
                label {
                    r#for: "composer-file-input",
                    class: "btn btn-ghost",
                    title: "attach an image",
                    i { class: "ph-duotone ph-paperclip" }
                }
                input {
                    r#type: "file",
                    id: "composer-file-input",
                    class: "hidden",
                    accept: ALLOWED_MEDIA_TYPES.join(","),
                    multiple: true,
                    onchange: move |event| {
                        let files = event.files();
                        async move {
                            for file in files {
                                queue_upload(conversation_id, attachments, next_key, file).await;
                            }
                        }
                    },
                }
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
}

/// Reads one picked or dropped file's bytes and hands them to
/// [`queue_upload_bytes`].
///
/// Split out only so [`Composer`] doesn't repeat the same three lines for
/// the file input and the drop zone, which otherwise differ only in how
/// they got their `Vec<FileData>` in the first place.
async fn queue_upload(
    conversation_id: i64,
    attachments: Signal<Vec<PendingAttachment>>,
    next_key: Signal<u64>,
    file: dioxus::html::FileData,
) {
    let media_type = file.content_type().unwrap_or_default();
    match file.read_bytes().await {
        Ok(bytes) => queue_upload_bytes(
            conversation_id,
            attachments,
            next_key,
            media_type,
            bytes.to_vec(),
        ),
        Err(error) => {
            push_failed(
                attachments,
                next_key,
                format!("couldn't read that file :< {error}"),
            );
        }
    }
}

/// Validates `bytes` against the same rules the server enforces, shows an
/// immediate local thumbnail, and spawns the real upload -- client-side
/// validation here is purely a faster no-round-trip rejection; the server
/// checks everything again regardless, and its message is what actually
/// reaches [`AttachmentStatus::Failed`] for anything this pass lets
/// through.
fn queue_upload_bytes(
    conversation_id: i64,
    mut attachments: Signal<Vec<PendingAttachment>>,
    mut next_key: Signal<u64>,
    media_type: String,
    bytes: Vec<u8>,
) {
    if !ALLOWED_MEDIA_TYPES.contains(&media_type.as_str()) {
        push_failed(
            attachments,
            next_key,
            format!("'{media_type}' isn't a supported image type -- try png, jpeg, gif, or webp"),
        );
        return;
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        push_failed(
            attachments,
            next_key,
            format!(
                "that image is {} bytes, over the {MAX_ATTACHMENT_BYTES} byte limit",
                bytes.len()
            ),
        );
        return;
    }

    let key = *next_key.read();
    next_key.set(key + 1);
    let data = STANDARD.encode(&bytes);
    attachments.write().push(PendingAttachment {
        key,
        preview_url: format!("data:{media_type};base64,{data}"),
        status: AttachmentStatus::Uploading,
    });

    spawn(async move {
        let outcome = upload_attachment(conversation_id, media_type, data).await;
        let mut list = attachments.write();
        if let Some(entry) = list.iter_mut().find(|entry| entry.key == key) {
            entry.status = match outcome {
                Ok(summary) => AttachmentStatus::Uploaded { id: summary.id },
                Err(error) => AttachmentStatus::Failed {
                    reason: error.to_string(),
                },
            };
        }
    });
}

/// Adds a failed entry with no preview, for a rejection caught before any
/// bytes were even readable as an image.
fn push_failed(
    mut attachments: Signal<Vec<PendingAttachment>>,
    mut next_key: Signal<u64>,
    reason: String,
) {
    let key = *next_key.read();
    next_key.set(key + 1);
    attachments.write().push(PendingAttachment {
        key,
        preview_url: String::new(),
        status: AttachmentStatus::Failed { reason },
    });
}

/// Registers a native `paste` listener on the drop zone element and queues
/// every pasted image it reports, for as long as that element stays
/// mounted.
///
/// See [`Composer`]'s own doc comment for why this needs real JavaScript at
/// all: `ClipboardData` has nothing dioxus can turn into a `FileData`, so
/// image extraction and base64 encoding both happen browser-side, with only
/// the finished base64 string ever crossing back over `document::eval`'s
/// channel.
async fn listen_for_pasted_images(
    conversation_id: i64,
    attachments: Signal<Vec<PendingAttachment>>,
    next_key: Signal<u64>,
) {
    let mut eval = document::eval(&format!(
        r#"
        const zone = document.getElementById('{DROP_ZONE_ID}');
        if (!zone) {{ return; }}
        zone.addEventListener('paste', async (event) => {{
            const items = event.clipboardData ? event.clipboardData.items : [];
            for (const item of items) {{
                if (!item.type.startsWith('image/')) {{ continue; }}
                const file = item.getAsFile();
                if (!file) {{ continue; }}
                const buffer = await file.arrayBuffer();
                const bytes = new Uint8Array(buffer);
                let binary = '';
                for (let i = 0; i < bytes.length; i++) {{
                    binary += String.fromCharCode(bytes[i]);
                }}
                dioxus.send({{ media_type: item.type, data: btoa(binary) }});
            }}
        }});
        "#
    ));

    // the channel closes when the drop zone (and its listener) is torn
    // down, which is the ordinary way this loop ends, not a failure worth
    // logging
    while let Ok(pasted) = eval.recv::<PastedImage>().await {
        match STANDARD.decode(&pasted.data) {
            Ok(bytes) => queue_upload_bytes(
                conversation_id,
                attachments,
                next_key,
                pasted.media_type,
                bytes,
            ),
            Err(error) => {
                push_failed(
                    attachments,
                    next_key,
                    format!("couldn't read a pasted image :< {error}"),
                );
            }
        }
    }
}

/// One thumbnail in the composer's pending-attachments strip: a preview,
/// a status indicator, and a remove button that works regardless of
/// whether the upload behind it ever finished.
#[component]
fn AttachmentThumbnail(attachment: PendingAttachment, on_remove: EventHandler<u64>) -> Element {
    let key = attachment.key;

    rsx! {
        div { class: "relative h-16 w-16 overflow-hidden rounded-box border border-slate-700 bg-slate-900",
            if attachment.preview_url.is_empty() {
                div { class: "flex h-full w-full items-center justify-center text-error",
                    i { class: "text-2xl ph-duotone ph-image-broken" }
                }
            } else {
                img {
                    class: "h-full w-full object-cover",
                    src: attachment.preview_url.clone(),
                    title: match &attachment.status {
                        AttachmentStatus::Failed { reason } => reason.clone(),
                        _ => String::new(),
                    },
                }
            }
            if matches!(attachment.status, AttachmentStatus::Uploading) {
                div { class: "absolute inset-0 flex items-center justify-center bg-slate-950/60",
                    i { class: "animate-spin text-lg text-white ph-duotone ph-circle-notch" }
                }
            }
            if let AttachmentStatus::Failed { reason } = &attachment.status {
                div {
                    class: "absolute inset-0 flex items-center justify-center bg-slate-950/60",
                    title: reason.clone(),
                    i { class: "text-lg text-error ph-duotone ph-x-circle" }
                }
            }
            button {
                class: "btn absolute -top-1 -right-1 btn-circle btn-error btn-xs",
                title: "remove",
                onclick: move |_| on_remove.call(key),
                i { class: "text-xs ph-duotone ph-x" }
            }
        }
    }
}
