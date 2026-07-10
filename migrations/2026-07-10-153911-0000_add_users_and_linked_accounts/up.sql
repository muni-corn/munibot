-- Your SQL goes here
CREATE TABLE IF NOT EXISTS `users` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`display_name` VARCHAR (255) NOT NULL,
`avatar_url` VARCHAR (255),
`created_at` DATETIME NOT NULL
) ;

-- one row per provider account (discord, and eventually twitch/github) linked
-- to a munibot user. a user can have multiple linked accounts; a provider
-- account can only ever belong to one user.
CREATE TABLE IF NOT EXISTS `linked_accounts` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`user_id` BIGINT NOT NULL,
`provider` VARCHAR (32) NOT NULL,
`provider_user_id` VARCHAR (64) NOT NULL,
`username` VARCHAR (255) NOT NULL,
`access_token` TEXT NOT NULL,
`refresh_token` TEXT,
`token_expires_at` DATETIME,
`created_at` DATETIME NOT NULL,
`updated_at` DATETIME NOT NULL,
UNIQUE KEY `linked_accounts_provider_account` (`provider`, `provider_user_id`),
FOREIGN KEY (`user_id`) REFERENCES `users` (`id`)
) ;
