-- one row per uploaded image. `message_id` starts NULL at upload time (see
-- commit 125's own upload/send_message split - an image is uploaded before
-- the message that will reference it exists at all) and is filled in once
-- that message is actually persisted; an attachment uploaded but never sent
-- stays orphaned rather than being cleaned up automatically for now.
--
-- `data` holds the raw image bytes directly - deliberately not base64
-- inlined into `ai_messages.content`: a one-megabyte image becomes roughly
-- 1.4 MB of base64 in a JSON column, and that table is read on every single
-- turn. Base64 only ever happens transiently, when actually building a
-- provider request. `MEDIUMBLOB` caps a single row at 16 MB regardless of
-- whatever smaller limit the upload path itself enforces, as a hard backstop.
--
-- Real object storage (S3-compatible, keyed by `sha256` so two people
-- pasting the same screenshot share one blob) is the obvious next move once
-- this table's own size becomes a real operational concern - not solved
-- prematurely here.
CREATE TABLE IF NOT EXISTS `ai_attachments` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`conversation_id` BIGINT NOT NULL,
`message_id` BIGINT,
`media_type` VARCHAR (64) NOT NULL,
`byte_size` INT NOT NULL,
`sha256` CHAR (64) NOT NULL,
`data` MEDIUMBLOB NOT NULL,
`created_at` DATETIME NOT NULL,
KEY `ai_attachments_message` (`message_id`),
FOREIGN KEY (`conversation_id`) REFERENCES `ai_conversations` (`id`) ON DELETE CASCADE,
FOREIGN KEY (`message_id`) REFERENCES `ai_messages` (`id`) ON DELETE CASCADE
) ;
