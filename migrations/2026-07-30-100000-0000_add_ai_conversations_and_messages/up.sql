-- one row per conversation munibot is holding, on any surface.
--
-- `platform` plus `scope_key` identifies where a conversation lives: a discord
-- channel, or an opaque per-conversation key on the web. `owner_user_id`,
-- `title`, and `archived_at` exist for the web surface only -- a web
-- conversation belongs to one person and needs a name in a sidebar, while a
-- discord channel's conversation has neither, so all three are NULL there.
CREATE TABLE IF NOT EXISTS `ai_conversations` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`platform` VARCHAR (32) NOT NULL,
`scope_key` VARCHAR (255) NOT NULL,
`persona_id` VARCHAR (64) NOT NULL,
`owner_user_id` BIGINT,
`title` VARCHAR (255),
`summary` TEXT,
`summary_tokens` INT NOT NULL DEFAULT 0,
`archived_at` DATETIME,
`created_at` DATETIME NOT NULL,
`last_active_at` DATETIME NOT NULL,
UNIQUE KEY `ai_conversations_scope` (`platform`, `scope_key`),
KEY `ai_conversations_owner_recent` (`owner_user_id`, `last_active_at`),
FOREIGN KEY (`owner_user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE
) ;

-- one row per message, ordered within a conversation by `seq`. `content` holds
-- a JSON-encoded Vec<ContentBlock>, so tool calls and their results survive a
-- restart intact rather than being flattened to text.
CREATE TABLE IF NOT EXISTS `ai_messages` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`conversation_id` BIGINT NOT NULL,
`seq` INT NOT NULL,
`role` VARCHAR (16) NOT NULL,
`content` JSON NOT NULL,
`token_count` INT NOT NULL DEFAULT 0,
`created_at` DATETIME NOT NULL,
UNIQUE KEY `ai_messages_conversation_seq` (`conversation_id`, `seq`),
FOREIGN KEY (`conversation_id`) REFERENCES `ai_conversations` (`id`) ON DELETE CASCADE
) ;
