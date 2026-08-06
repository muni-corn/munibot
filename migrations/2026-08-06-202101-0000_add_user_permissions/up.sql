-- one row per permission granted to a munibot user. `permission` is the
-- snake_case string form of `munibot_core::permission::Permission` (e.g.
-- "operator"), matching axum_session_auth's own `HasPermission` convention
-- of checking plain permission tokens rather than a bitmask.
CREATE TABLE IF NOT EXISTS `user_permissions` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`user_id` BIGINT NOT NULL,
`permission` VARCHAR (64) NOT NULL,
`created_at` DATETIME NOT NULL,
UNIQUE KEY `user_permissions_user_permission` (`user_id`, `permission`),
FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE
) ;
