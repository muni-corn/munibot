-- lets milestone 1's discord ai surface (until now, always on wherever
-- munibot is invited at all - see munibot_discord/src/handlers/ai.rs's own
-- doc comment) be enabled per guild instead of globally, with an optional
-- channel allowlist.
--
-- added as columns on the existing `guild_configs` row, not a new
-- `ai_guild_settings` table, since a guild's ai settings and its logging
-- settings are both just "this guild's config" - splitting them into
-- separate tables would only mean joining them back together on every read
-- that needs both. this is exactly why `upsert_guild_config` (see
-- operations.rs) must always be called through the guild_configs-scoped
-- setters that read-then-merge the *other* concern's columns first, never
-- with only one concern's fields populated: this row now has two settings'
-- worth of columns sharing one primary key, and a REPLACE INTO -- or a
-- naively-constructed whole-row upsert missing the other concern's current
-- values -- would silently null one out on every save of the other.
ALTER TABLE `guild_configs`
ADD COLUMN `ai_enabled` BOOLEAN NOT NULL DEFAULT FALSE,
ADD COLUMN `ai_default_persona` VARCHAR (64),
ADD COLUMN `ai_channel_mode` VARCHAR (16) NOT NULL DEFAULT 'all' ;

-- one row per channel an ai-enabled guild has explicitly allowed, consulted
-- only when that guild's own `ai_channel_mode` is `'allowlist'` - a guild
-- left on the default `'all'` mode never needs a row here at all.
CREATE TABLE IF NOT EXISTS `ai_channel_allowlist` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`guild_id` BIGINT NOT NULL,
`channel_id` BIGINT NOT NULL,
`created_at` DATETIME NOT NULL,
UNIQUE KEY `ai_channel_allowlist_guild_channel` (`guild_id`, `channel_id`)
) ;
