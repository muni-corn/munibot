// @generated automatically by Diesel CLI.

diesel::table! {
    ai_attachments (id) {
        id -> Bigint,
        conversation_id -> Bigint,
        message_id -> Nullable<Bigint>,
        #[max_length = 64]
        media_type -> Varchar,
        byte_size -> Integer,
        #[max_length = 64]
        sha256 -> Char,
        data -> Mediumblob,
        created_at -> Datetime,
    }
}

diesel::table! {
    ai_conversations (id) {
        id -> Bigint,
        #[max_length = 32]
        platform -> Varchar,
        #[max_length = 255]
        scope_key -> Varchar,
        #[max_length = 64]
        persona_id -> Varchar,
        owner_user_id -> Nullable<Bigint>,
        #[max_length = 255]
        title -> Nullable<Varchar>,
        summary -> Nullable<Text>,
        summary_tokens -> Integer,
        archived_at -> Nullable<Datetime>,
        created_at -> Datetime,
        last_active_at -> Datetime,
    }
}

diesel::table! {
    ai_memories (id) {
        id -> Bigint,
        user_id -> Bigint,
        #[max_length = 128]
        key -> Varchar,
        value -> Text,
        created_at -> Datetime,
        updated_at -> Datetime,
    }
}

diesel::table! {
    ai_messages (id) {
        id -> Bigint,
        conversation_id -> Bigint,
        seq -> Integer,
        #[max_length = 16]
        role -> Varchar,
        content -> Longtext,
        token_count -> Integer,
        created_at -> Datetime,
    }
}

diesel::table! {
    ai_rate_limits (id) {
        id -> Bigint,
        #[max_length = 16]
        scope_type -> Varchar,
        scope_id -> Nullable<Bigint>,
        window_start -> Datetime,
        request_count -> Integer,
        token_count -> Bigint,
    }
}

diesel::table! {
    ai_spend_caps (id) {
        id -> Bigint,
        #[max_length = 16]
        scope_type -> Varchar,
        scope_id -> Nullable<Bigint>,
        #[max_length = 16]
        period -> Varchar,
        limit_micros -> Bigint,
        current_micros -> Bigint,
        reset_at -> Datetime,
    }
}

diesel::table! {
    ai_tool_calls (id) {
        id -> Bigint,
        conversation_id -> Nullable<Bigint>,
        #[max_length = 64]
        tool_name -> Varchar,
        input -> Nullable<Text>,
        output -> Nullable<Text>,
        duration_ms -> Bigint,
        #[max_length = 16]
        status -> Varchar,
        created_at -> Datetime,
    }
}

diesel::table! {
    ai_usage (id) {
        id -> Bigint,
        conversation_id -> Nullable<Bigint>,
        user_id -> Nullable<Bigint>,
        guild_id -> Nullable<Bigint>,
        #[max_length = 32]
        provider -> Varchar,
        #[max_length = 128]
        model -> Varchar,
        #[max_length = 64]
        persona_id -> Varchar,
        input_tokens -> Bigint,
        output_tokens -> Bigint,
        cost_micros -> Bigint,
        iterations -> Integer,
        succeeded -> Bool,
        created_at -> Datetime,
    }
}

diesel::table! {
    ai_user_settings (user_id) {
        user_id -> Bigint,
        memory_opt_in -> Bool,
        created_at -> Datetime,
        updated_at -> Datetime,
    }
}

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
    user_permissions (id) {
        id -> Bigint,
        user_id -> Bigint,
        #[max_length = 64]
        permission -> Varchar,
        created_at -> Datetime,
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
    ai_rate_limits,
    ai_spend_caps,
    ai_tool_calls,
    ai_usage,
    ai_user_settings,
    autodelete_timers,
    community_links,
    guild_configs,
    guild_payouts,
    guild_wallets,
    linked_accounts,
    quotes,
    user_permissions,
    users,
);
