-- one row per completed turn, written on failure too: a turn that errored on
-- iteration nine still cost money, and a usage table that only records
-- successes understates spend exactly when something is going wrong.
--
-- cost is stored in micros (millionths of a dollar) as an integer rather than
-- a float, so summing a month of rows does not accumulate rounding error.
CREATE TABLE IF NOT EXISTS `ai_usage` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`conversation_id` BIGINT,
`user_id` BIGINT,
`guild_id` BIGINT,
`provider` VARCHAR (32) NOT NULL,
`model` VARCHAR (128) NOT NULL,
`persona_id` VARCHAR (64) NOT NULL,
`input_tokens` BIGINT NOT NULL DEFAULT 0,
`output_tokens` BIGINT NOT NULL DEFAULT 0,
`cost_micros` BIGINT NOT NULL DEFAULT 0,
`iterations` INT NOT NULL DEFAULT 0,
`succeeded` BOOLEAN NOT NULL DEFAULT TRUE,
`created_at` DATETIME NOT NULL,
-- per-user first: with the web as the primary surface this is the common
-- query, and per-guild is the secondary one
KEY `ai_usage_user_time` (`user_id`, `created_at`),
KEY `ai_usage_guild_time` (`guild_id`, `created_at`),
FOREIGN KEY (`conversation_id`) REFERENCES `ai_conversations` (`id`) ON DELETE SET NULL,
FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE SET NULL
) ;

-- one row per tool invocation, with input and output truncated. the only way
-- to debug a bad tool loop after the fact, and what the web chat's tool
-- activity strip reads back when rendering a past conversation.
CREATE TABLE IF NOT EXISTS `ai_tool_calls` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`conversation_id` BIGINT,
`tool_name` VARCHAR (64) NOT NULL,
`input` TEXT,
`output` TEXT,
`duration_ms` BIGINT NOT NULL DEFAULT 0,
`status` VARCHAR (16) NOT NULL,
`created_at` DATETIME NOT NULL,
KEY `ai_tool_calls_conversation` (`conversation_id`, `created_at`),
FOREIGN KEY (`conversation_id`) REFERENCES `ai_conversations` (`id`) ON DELETE CASCADE
) ;
