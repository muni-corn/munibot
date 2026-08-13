use dioxus::prelude::*;
use munibot_api::{
    server_fns::settings::{
        discord::get_discord_invite_link, logging::set_guild_logging_settings,
        logging_page::get_guild_logging_page,
    },
    settings::{GuildLoggingSettings, SettingsError},
};

use crate::components::{
    Spinner,
    settings::{ChannelSelect, InviteMunibotPrompt, SaveBar, SettingsRow, SettingsSection},
};

/// A guild's logging settings: which channel (if any) server events are
/// logged to.
#[component]
pub fn LoggingSettingsPage(guild_id: String) -> Element {
    let page_guild_id = guild_id.clone();
    let page = use_resource(move || get_guild_logging_page(page_guild_id.clone()));

    // only fetched for the BotNotInGuild case below, so it's fine that this
    // makes a request up front regardless -- it's a single cheap read of an
    // env-derived config value, not a per-guild call
    let invite_link = use_resource(get_discord_invite_link);

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
        if let Some(Ok(loaded)) = &*page.read() {
            selected.set(loaded.settings.channel_id.clone());
            saved.set(loaded.settings.channel_id.clone());
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

    let content = match &*page.read() {
        Some(Ok(page)) => rsx! {
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
                        channels: page.channels.clone(),
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
        Some(Err(SettingsError::BotNotInGuild)) => {
            let invite_link = invite_link
                .read()
                .as_ref()
                .and_then(|link| link.as_ref().ok())
                .cloned()
                .flatten();
            rsx! {
                InviteMunibotPrompt { invite_link }
            }
        }
        Some(Err(_)) => rsx! {
            div { class: "alert alert-error", "couldn't load this server's logging settings :<" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "logging settings ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "logging" }
            {content}
        }
    }
}
