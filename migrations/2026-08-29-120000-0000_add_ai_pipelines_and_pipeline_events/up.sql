-- one row per autonomous pipeline run, identified by the issue that
-- triggered it. deliberately holds only identity -- the forge, the
-- repository, the issue number, and when the run started -- and never a
-- mutable "current state" column: `PipelineState` is always a fold over
-- `ai_pipeline_events`, recomputed by replay, so there is nothing here for
-- a crash mid-run to leave inconsistent.
CREATE TABLE IF NOT EXISTS `ai_pipelines` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`forge` VARCHAR (16) NOT NULL,
`owner` VARCHAR (255) NOT NULL,
`repo_name` VARCHAR (255) NOT NULL,
`issue_number` BIGINT UNSIGNED NOT NULL,
`created_at` DATETIME NOT NULL,
KEY `ai_pipelines_repo_issue` (`forge`, `owner`, `repo_name`, `issue_number`)
) ;

-- the event log itself. `payload` is a json-encoded event, the same
-- json-encoded-text convention `ai_messages.content` already uses rather
-- than a native json column. the unique index on (`pipeline_id`, `seq`)
-- is what makes this log append-only and gap-free: two concurrent writers
-- racing for the same `seq` fail one of them outright rather than
-- silently interleaving, and `replay` can trust that folding events
-- `seq` 0..n in order reconstructs the real history exactly once.
CREATE TABLE IF NOT EXISTS `ai_pipeline_events` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`pipeline_id` BIGINT NOT NULL,
`seq` INT NOT NULL,
`event_type` VARCHAR (64) NOT NULL,
`payload` LONGTEXT NOT NULL,
`created_at` DATETIME NOT NULL,
UNIQUE KEY `ai_pipeline_events_pipeline_seq` (`pipeline_id`, `seq`),
FOREIGN KEY (`pipeline_id`) REFERENCES `ai_pipelines` (`id`) ON DELETE CASCADE
) ;
