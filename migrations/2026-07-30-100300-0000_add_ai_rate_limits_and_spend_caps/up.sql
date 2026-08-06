-- one counter row per scope, checked before every provider call so a
-- runaway loop or an abusive signed-in stranger is refused before it costs
-- anything. `scope_type` is a discriminated key ('user' | 'guild' | 'global')
-- covering every level this needs to check with one mechanism, rather than
-- three separate tables; `scope_id` is NULL for the single global row.
--
-- a fixed window with lazy reset, not a genuine sliding log: the window
-- simply starts over once `window_start` is old enough, rather than tracking
-- every individual request's own timestamp. this is what makes checking a
-- limit an O(1) lookup by primary key regardless of how much history exists,
-- which a live aggregate query over a growing `ai_usage` table would not be.
CREATE TABLE IF NOT EXISTS `ai_rate_limits` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`scope_type` VARCHAR (16) NOT NULL,
`scope_id` BIGINT,
`window_start` DATETIME NOT NULL,
`request_count` INT NOT NULL DEFAULT 0,
`token_count` BIGINT NOT NULL DEFAULT 0,
UNIQUE KEY `ai_rate_limits_scope` (`scope_type`, `scope_id`)
) ;

-- one row per scope per configured spend period (e.g. "monthly"), tracking
-- spend against a limit that resets on its own schedule. `limit_micros` is
-- stored per row rather than only read from config, so a future per-scope
-- override (a higher cap for one user) needs no schema change - today every
-- row is simply populated from the same configured value.
CREATE TABLE IF NOT EXISTS `ai_spend_caps` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`scope_type` VARCHAR (16) NOT NULL,
`scope_id` BIGINT,
`period` VARCHAR (16) NOT NULL,
`limit_micros` BIGINT NOT NULL,
`current_micros` BIGINT NOT NULL DEFAULT 0,
`reset_at` DATETIME NOT NULL,
UNIQUE KEY `ai_spend_caps_scope` (`scope_type`, `scope_id`, `period`)
) ;
