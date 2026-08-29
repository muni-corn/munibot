-- one row per scope that has ever tripped abuse detection: how many times
-- (`strike_count`), how long it is cooling down for right now
-- (`cooldown_until`), and what tripped it last (`last_reason`), so an
-- operator reviewing this table later can see why without any message
-- content ever being stored here at all.
--
-- `scope_type`/`scope_id` mirrors `ai_rate_limits`' own discriminated-key
-- convention ('user' | 'guild' | 'global', NULL scope_id for the single
-- global row) - see that migration's comment for why one shared shape
-- serves every scope level rather than a separate table per level. In
-- practice only ever 'user' rows exist today: abuse detection (repeated
-- prompts, injection probing, persona-switching) is inherently a single
-- person's behaviour, not a guild's or the whole service's, but the same
-- shape is kept anyway rather than a table that could only ever have one
-- discriminant.
CREATE TABLE IF NOT EXISTS `ai_abuse_cooldowns` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`scope_type` VARCHAR (16) NOT NULL,
`scope_id` BIGINT,
`strike_count` INT NOT NULL DEFAULT 0,
`cooldown_until` DATETIME NOT NULL,
`last_reason` VARCHAR (64) NOT NULL,
`last_tripped_at` DATETIME NOT NULL,
UNIQUE KEY `ai_abuse_cooldowns_scope` (`scope_type`, `scope_id`)
) ;
