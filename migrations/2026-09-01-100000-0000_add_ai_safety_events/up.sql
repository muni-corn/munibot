-- one row per trip of any of munibot's ai safety systems: a rate limit
-- refusal, a spend cap refusal, a moderation block, or a crisis classifier
-- trigger. `scope_type`/`scope_id` is the same discriminated key
-- `ai_rate_limits` documents ('user' | 'guild' | 'global', NULL scope_id
-- for the global row).
--
-- deliberately excludes raw content: `content_hash` is a one-way SHA-256
-- digest, present only when there was meaningfully "content" to hash at all
-- (a rate limit or spend cap trip has none). enough to confirm two events
-- came from the same repeated message, or to compare against a hash an
-- operator already has from elsewhere, without this table ever becoming a
-- second place a user's own words end up stored - see the milestone 6 plan's
-- own framing: "enough to tune the systems, not enough to become a
-- surveillance log."
CREATE TABLE IF NOT EXISTS `ai_safety_events` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`event_type` VARCHAR (16) NOT NULL,
`scope_type` VARCHAR (16) NOT NULL,
`scope_id` BIGINT,
`reason` VARCHAR (255) NOT NULL,
`content_hash` CHAR (64),
`created_at` DATETIME NOT NULL,
KEY `ai_safety_events_type_created` (`event_type`, `created_at`)
) ;
