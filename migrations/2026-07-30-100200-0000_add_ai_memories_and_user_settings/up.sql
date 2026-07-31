-- one row per remembered fact, opted into and owned by one user. keyed on the
-- internal `users.id`, not a raw platform snowflake, so memory survives a
-- user linking a second platform account -- see the identity trap documented
-- at docs/notes/gui-configuration-research.md:91-108.
CREATE TABLE IF NOT EXISTS `ai_memories` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`user_id` BIGINT NOT NULL,
`key` VARCHAR (128) NOT NULL,
`value` TEXT NOT NULL,
`created_at` DATETIME NOT NULL,
`updated_at` DATETIME NOT NULL,
UNIQUE KEY `ai_memories_user_key` (`user_id`, `key`),
FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE
) ;

-- one row per user who has ever touched an ai memory-related setting.
-- `memory_opt_in` defaults to false: memory is opt-in, never assumed.
CREATE TABLE IF NOT EXISTS `ai_user_settings` (
`user_id` BIGINT NOT NULL PRIMARY KEY,
`memory_opt_in` BOOLEAN NOT NULL DEFAULT FALSE,
`created_at` DATETIME NOT NULL,
`updated_at` DATETIME NOT NULL,
FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE
) ;
