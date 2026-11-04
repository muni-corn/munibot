// @generated automatically by Diesel CLI.

diesel::table! {
    autodelete_timers (channel_id) {
        channel_id -> Bigint,
        guild_id -> Bigint,
        duration_secs -> Bigint,
        last_cleaned -> Datetime,
        last_message_id_cleaned -> Bigint,
        #[max_length = 32]
        mode -> Varchar,
    }
}

diesel::table! {
    community_links (id) {
        id -> Bigint,
        #[max_length = 64]
        twitch_streamer_id -> Nullable<Varchar>,
        discord_guild_id -> Nullable<Bigint>,
    }
}

diesel::table! {
    email_signin_tokens (id) {
        id -> Bigint,
        #[max_length = 255]
        email -> Varchar,
        #[max_length = 64]
        token_hash -> Char,
        expires_at -> Datetime,
        created_at -> Datetime,
    }
}

diesel::table! {
    guild_configs (guild_id) {
        guild_id -> Bigint,
        logging_channel -> Nullable<Bigint>,
    }
}

diesel::table! {
    guild_payouts (id) {
        id -> Bigint,
        guild_id -> Bigint,
        user_id -> Bigint,
        balance -> Unsigned<Bigint>,
        last_payout -> Datetime,
    }
}

diesel::table! {
    guild_wallets (id) {
        id -> Bigint,
        guild_id -> Bigint,
        user_id -> Bigint,
        balance -> Unsigned<Bigint>,
    }
}

diesel::table! {
    linked_accounts (id) {
        id -> Bigint,
        user_id -> Bigint,
        #[max_length = 32]
        provider -> Varchar,
        #[max_length = 64]
        provider_user_id -> Varchar,
        #[max_length = 255]
        username -> Varchar,
        access_token -> Text,
        refresh_token -> Nullable<Text>,
        token_expires_at -> Nullable<Datetime>,
        created_at -> Datetime,
        updated_at -> Datetime,
    }
}

diesel::table! {
    quotes (id) {
        id -> Bigint,
        community_id -> Bigint,
        sequential_id -> Integer,
        created_at -> Datetime,
        quote -> Text,
        #[max_length = 255]
        invoker -> Varchar,
        #[max_length = 255]
        stream_category -> Varchar,
        #[max_length = 255]
        stream_title -> Varchar,
    }
}

diesel::table! {
    users (id) {
        id -> Bigint,
        #[max_length = 255]
        display_name -> Varchar,
        #[max_length = 255]
        avatar_url -> Nullable<Varchar>,
        created_at -> Datetime,
    }
}

diesel::joinable!(ai_attachments -> ai_conversations (conversation_id));
diesel::joinable!(ai_attachments -> ai_messages (message_id));
diesel::joinable!(ai_conversations -> users (owner_user_id));
diesel::joinable!(ai_memories -> users (user_id));
diesel::joinable!(ai_messages -> ai_conversations (conversation_id));
diesel::joinable!(ai_pipeline_events -> ai_pipelines (pipeline_id));
diesel::joinable!(ai_tool_calls -> ai_conversations (conversation_id));
diesel::joinable!(ai_usage -> ai_conversations (conversation_id));
diesel::joinable!(ai_usage -> users (user_id));
diesel::joinable!(ai_user_settings -> users (user_id));
diesel::joinable!(linked_accounts -> users (user_id));
diesel::joinable!(quotes -> community_links (community_id));
diesel::joinable!(user_permissions -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    ai_attachments,
    ai_conversations,
    ai_memories,
    ai_messages,
    ai_pipeline_events,
    ai_pipelines,
    ai_rate_limits,
    ai_spend_caps,
    ai_tool_calls,
    ai_usage,
    ai_user_settings,
    autodelete_timers,
    community_links,
    email_signin_tokens,
    guild_configs,
    guild_payouts,
    guild_wallets,
    linked_accounts,
    quotes,
    user_permissions,
    users,
);
