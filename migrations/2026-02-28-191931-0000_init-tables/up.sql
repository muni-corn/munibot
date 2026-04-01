-- Your SQL goes here
CREATE TABLE IF NOT EXISTS `guild_configs` (
`guild_id` BIGINT NOT NULL PRIMARY KEY,
`logging_channel` BIGINT
) ;

CREATE TABLE IF NOT EXISTS `autodelete_timers` (
`channel_id` BIGINT NOT NULL PRIMARY KEY,
`guild_id` BIGINT NOT NULL,
`duration_secs` BIGINT NOT NULL,
`last_cleaned` DATETIME NOT NULL,
`last_message_id_cleaned` BIGINT NOT NULL,
`mode` VARCHAR (32) NOT NULL
) ;

CREATE TABLE IF NOT EXISTS `guild_wallets` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`guild_id` BIGINT NOT NULL,
`user_id` BIGINT NOT NULL,
`balance` BIGINT UNSIGNED NOT NULL DEFAULT 0,
UNIQUE KEY `guild_wallets_guild_user` (`guild_id`, `user_id`)
) ;

CREATE TABLE IF NOT EXISTS `guild_payouts` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`guild_id` BIGINT NOT NULL,
`user_id` BIGINT NOT NULL,
`balance` BIGINT UNSIGNED NOT NULL DEFAULT 0,
`last_payout` DATETIME NOT NULL,
UNIQUE KEY `guild_payouts_guild_user` (`guild_id`, `user_id`)
) ;

-- must be created before quotes (FK dependency)
CREATE TABLE IF NOT EXISTS `community_links` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`twitch_streamer_id` VARCHAR (64) UNIQUE,
`discord_guild_id` BIGINT UNIQUE
) ;

CREATE TABLE IF NOT EXISTS `quotes` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`community_id` BIGINT NOT NULL,
`sequential_id` INTEGER NOT NULL,
`created_at` DATETIME NOT NULL,
`quote` TEXT NOT NULL,
`invoker` VARCHAR (255) NOT NULL,
`stream_category` VARCHAR (255) NOT NULL,
`stream_title` VARCHAR (255) NOT NULL,
UNIQUE KEY `quotes_community_sequential` (`community_id`, `sequential_id`),
FOREIGN KEY (`community_id`) REFERENCES `community_links` (`id`)
) ;
