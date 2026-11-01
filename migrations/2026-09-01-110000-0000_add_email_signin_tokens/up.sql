-- one row per outstanding email sign-in (magic link) request. `token_hash`
-- is a SHA-256 hex digest of the actual token mailed to the user, never
-- the token itself - the same reasoning `ai_safety_events.content_hash`
-- documents: a database leak alone must never hand out a working sign-in
-- link. single-use: a row is deleted the moment it's consumed, and an
-- expired, never-consumed row is simply left for the next request from
-- that address to replace (see upsert_email_signin_token).
CREATE TABLE IF NOT EXISTS `email_signin_tokens` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`email` VARCHAR (255) NOT NULL,
`token_hash` CHAR (64) NOT NULL,
`expires_at` DATETIME NOT NULL,
`created_at` DATETIME NOT NULL,
UNIQUE KEY `email_signin_tokens_email` (`email`),
UNIQUE KEY `email_signin_tokens_token_hash` (`token_hash`)
) ;
