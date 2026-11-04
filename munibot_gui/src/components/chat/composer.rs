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
                    if event.key() == Key::Named(NamedKey::Enter) && !event.modifiers().shift() {
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
