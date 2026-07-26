use dioxus::prelude::*;
use munibot_api::{
    server_fns::settings::{
        channels::get_guild_channels,
        logging::{get_guild_logging_settings, set_guild_logging_settings},
    },
    settings::GuildLoggingSettings,
};

use crate::components::{
    Spinner,
    settings::{ChannelSelect, SaveBar, SettingsRow, SettingsSection},
};

/// A guild's logging settings: which channel (if any) server events are
/// logged to.
#[component]
pub fn LoggingSettingsPage(guild_id: String) -> Element {
    let channels_guild_id = guild_id.clone();
    let channels = use_resource(move || {
        let guild_id = channels_guild_id.clone();
        async move { get_guild_channels(guild_id).await }
    });

    let settings_guild_id = guild_id.clone();
    let settings = use_resource(move || {
        let guild_id = settings_guild_id.clone();
        async move { get_guild_logging_settings(guild_id).await }
    });

    // the form's current selection, and the last-saved value it's compared
    // against for dirty tracking -- both stay `None` (and the form stays
    // unseeded) until `settings` loads
    let mut selected = use_signal(|| None::<String>);
    let mut saved = use_signal(|| None::<String>);
    let mut seeded = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut save_error = use_signal(|| None::<String>);

    use_effect(move || {
        if *seeded.read() {
            return;
        }
        if let Some(Ok(loaded)) = &*settings.read() {
            selected.set(loaded.channel_id.clone());
            saved.set(loaded.channel_id.clone());
            seeded.set(true);
        }
    });

    let dirty = *selected.read() != *saved.read();

    let save_guild_id = guild_id.clone();
    let on_save = move |_| {
        let guild_id = save_guild_id.clone();
        let channel_id = selected.read().clone();
        spawn(async move {
            saving.set(true);
            save_error.set(None);
            match set_guild_logging_settings(guild_id, GuildLoggingSettings { channel_id }).await {
                Ok(result) => {
                    selected.set(result.channel_id.clone());
                    saved.set(result.channel_id);
                }
                Err(error) => save_error.set(Some(error.to_string())),
            }
            saving.set(false);
        });
    };

    let on_discard = move |_| selected.set(saved.read().clone());

    let content = match (&*channels.read(), &*settings.read()) {
        (Some(Ok(channels)), Some(Ok(_))) => rsx! {
            SettingsSection {
                title: "logging".to_string(),
                description: Some(
                    "send server events -- joins, leaves, message edits and deletions -- to a channel."
                        .to_string(),
                ),
                SettingsRow {
                    label: "log channel".to_string(),
                    description: Some("leave this empty to turn logging off.".to_string()),
                    ChannelSelect {
                        channels: channels.clone(),
                        value: selected.read().clone(),
                        none_label: "off".to_string(),
                        on_change: move |value| selected.set(value),
                    }
                }
            }
            if let Some(message) = &*save_error.read() {
                div { class: "alert alert-error", "couldn't save that :< {message}" }
            }
            SaveBar {
                dirty,
                saving: *saving.read(),
                on_save,
                on_discard,
            }
        },
        (Some(Err(_)), _) | (_, Some(Err(_))) => rsx! {
            div { class: "alert alert-error", "couldn't load this server's logging settings :<" }
        },
        _ => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "logging settings ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "font-black text-2xl", "logging" }
            {content}
        }
    }
}
