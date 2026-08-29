use dioxus::prelude::*;
use munibot_api::{
    chat::PersonaSummary,
    server_fns::{
        chat::persona::list_personas,
        settings::{
            ai::{get_guild_ai_settings, set_guild_ai_settings},
            channels::get_guild_channels,
            discord::get_discord_invite_link,
        },
    },
    settings::{
        CHANNEL_MODE_ALL, CHANNEL_MODE_ALLOWLIST, ChannelSummary, GuildAiSettings, SettingsError,
    },
};

use crate::components::{
    Spinner,
    settings::{InviteMunibotPrompt, SaveBar, SettingsRow, SettingsSection},
};

/// A guild's ai settings: whether munibot's discord ai surface (mentions,
/// replies, dms) is enabled at all, which persona answers by default, and
/// which channels it may operate in.
#[component]
pub fn GuildAiSettingsPage(guild_id: String) -> Element {
    let channels_guild_id = guild_id.clone();
    let channels = use_resource(move || {
        let guild_id = channels_guild_id.clone();
        async move { get_guild_channels(guild_id).await }
    });

    let settings_guild_id = guild_id.clone();
    let settings = use_resource(move || {
        let guild_id = settings_guild_id.clone();
        async move { get_guild_ai_settings(guild_id).await }
    });

    let personas = use_resource(list_personas);

    // only fetched for the BotNotInGuild case below - see LoggingSettingsPage's
    // own doc comment for why fetching this unconditionally is fine
    let invite_link = use_resource(get_discord_invite_link);

    let mut enabled = use_signal(|| false);
    let mut default_persona = use_signal(|| None::<String>);
    let mut channel_mode = use_signal(|| CHANNEL_MODE_ALL.to_string());
    let mut channel_allowlist = use_signal(Vec::<String>::new);

    let mut saved = use_signal(|| None::<GuildAiSettings>);
    let mut seeded = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut save_error = use_signal(|| None::<String>);

    use_effect(move || {
        if *seeded.read() {
            return;
        }
        if let Some(Ok(loaded)) = &*settings.read() {
            enabled.set(loaded.enabled);
            default_persona.set(loaded.default_persona.clone());
            channel_mode.set(loaded.channel_mode.clone());
            channel_allowlist.set(loaded.channel_allowlist.clone());
            saved.set(Some(loaded.clone()));
            seeded.set(true);
        }
    });

    let current = move || GuildAiSettings {
        enabled: *enabled.read(),
        default_persona: default_persona.read().clone(),
        channel_mode: channel_mode.read().clone(),
        channel_allowlist: channel_allowlist.read().clone(),
    };

    let dirty = Some(current()) != *saved.read();

    let save_guild_id = guild_id.clone();
    let on_save = move |_| {
        let guild_id = save_guild_id.clone();
        let settings = current();
        spawn(async move {
            saving.set(true);
            save_error.set(None);
            match set_guild_ai_settings(guild_id, settings).await {
                Ok(result) => {
                    enabled.set(result.enabled);
                    default_persona.set(result.default_persona.clone());
                    channel_mode.set(result.channel_mode.clone());
                    channel_allowlist.set(result.channel_allowlist.clone());
                    saved.set(Some(result));
                }
                Err(error) => save_error.set(Some(error.to_string())),
            }
            saving.set(false);
        });
    };

    let on_discard = move |_| {
        if let Some(saved) = &*saved.read() {
            enabled.set(saved.enabled);
            default_persona.set(saved.default_persona.clone());
            channel_mode.set(saved.channel_mode.clone());
            channel_allowlist.set(saved.channel_allowlist.clone());
        }
    };

    let content = match (&*channels.read(), &*settings.read(), &*personas.read()) {
        (Some(Ok(channels)), Some(Ok(_)), Some(Ok(personas))) => rsx! {
            SettingsSection {
                title: "munibot".to_string(),
                description: Some("let people mention, reply to, or dm munibot in this server.".to_string()),
                SettingsRow { label: "enabled".to_string(), description: None,
                    input {
                        r#type: "checkbox",
                        class: "toggle toggle-primary",
                        checked: *enabled.read(),
                        onchange: move |event| enabled.set(event.checked()),
                    }
                }
                SettingsRow {
                    label: "default persona".to_string(),
                    description: Some(
                        "which persona answers when nobody's picked one for this conversation."
                            .to_string(),
                    ),
                    PersonaSelect {
                        personas: personas.clone(),
                        value: default_persona.read().clone(),
                        on_change: move |value| default_persona.set(value),
                    }
                }
                SettingsRow {
                    label: "channels".to_string(),
                    description: Some("every channel, or only ones you pick below.".to_string()),
                    select {
                        class: "select w-full",
                        value: channel_mode.read().clone(),
                        onchange: move |event| channel_mode.set(event.value()),
                        option { value: CHANNEL_MODE_ALL, "every channel" }
                        option { value: CHANNEL_MODE_ALLOWLIST, "only these channels" }
                    }
                }
                if *channel_mode.read() == CHANNEL_MODE_ALLOWLIST {
                    ChannelAllowlist {
                        channels: channels.clone(),
                        selected: channel_allowlist.read().clone(),
                        on_change: move |value| channel_allowlist.set(value),
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
        (Some(Err(SettingsError::BotNotInGuild)), ..) => {
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
        (Some(Err(_)), ..) | (_, Some(Err(_)), _) => rsx! {
            div { class: "alert alert-error", "couldn't load this server's ai settings :<" }
        },
        _ => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "ai settings ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "ai" }
            {content}
        }
    }
}

/// A `<select>` for choosing a guild's default persona, or none (falling
/// back to the service-wide default) - the same shape as `ChannelSelect`,
/// over personas instead of channels.
#[component]
fn PersonaSelect(
    personas: Vec<PersonaSummary>,
    value: Option<String>,
    on_change: EventHandler<Option<String>>,
) -> Element {
    rsx! {
        select {
            class: "select w-full",
            value: value.unwrap_or_default(),
            onchange: move |event| {
                let selected = event.value();
                on_change.call(if selected.is_empty() { None } else { Some(selected) });
            },
            option { value: "", "(service default)" }
            for persona in personas {
                option { value: "{persona.id}", {persona.display_name} }
            }
        }
    }
}

/// A checkbox per channel, for picking the set `ai_channel_mode:
/// "allowlist"` actually consults.
#[component]
fn ChannelAllowlist(
    channels: Vec<ChannelSummary>,
    selected: Vec<String>,
    on_change: EventHandler<Vec<String>>,
) -> Element {
    rsx! {
        ul { class: "flex max-h-64 flex-col gap-1 overflow-y-auto rounded-box bg-slate-950/40 p-3",
            for channel in channels {
                li { key: "{channel.id}",
                    label { class: "flex items-center gap-2",
                        input {
                            r#type: "checkbox",
                            class: "checkbox checkbox-sm",
                            checked: selected.contains(&channel.id),
                            onchange: {
                                let channel_id = channel.id.clone();
                                let selected = selected.clone();
                                move |event: Event<FormData>| {
                                    let mut next = selected.clone();
                                    if event.checked() {
                                        if !next.contains(&channel_id) {
                                            next.push(channel_id.clone());
                                        }
                                    } else {
                                        next.retain(|id| id != &channel_id);
                                    }
                                    on_change.call(next);
                                }
                            },
                        }
                        "#{channel.name}"
                    }
                }
            }
        }
    }
}
