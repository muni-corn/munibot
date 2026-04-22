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

diesel::joinable!(quotes -> community_links (community_id));

diesel::allow_tables_to_appear_in_same_query!(
    autodelete_timers,
    community_links,
    guild_configs,
    guild_payouts,
    guild_wallets,
    quotes,
);
