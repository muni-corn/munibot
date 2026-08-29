DROP TABLE IF EXISTS `ai_channel_allowlist` ;

ALTER TABLE `guild_configs`
DROP COLUMN `ai_enabled`,
DROP COLUMN `ai_default_persona`,
DROP COLUMN `ai_channel_mode` ;
