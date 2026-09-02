-- one row per project munibot has cloned: a git repository, one directory
-- on disk, one container image built by one of three strategies. see
-- docs/plans/ai/milestone-7-projects.md for the full design. `image_tag`,
-- `image_source_hash`, `image_built_at`, and `image_error` together are
-- the persisted form of `munibot_ai::project::ImageStatus` -- exactly one
-- of "unbuilt" (all four null), "built" (tag/hash/built_at set, error
-- null), or "failed" (error set, the others null) at any time.
CREATE TABLE IF NOT EXISTS `ai_projects` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`slug` VARCHAR (64) NOT NULL,
`display_name` VARCHAR (255) NOT NULL,
`forge` VARCHAR (16) NOT NULL,
`repo_owner` VARCHAR (255) NOT NULL,
`repo_name` VARCHAR (255) NOT NULL,
`clone_url` VARCHAR (1024) NOT NULL,
`default_branch` VARCHAR (255) NOT NULL,
`local_path` VARCHAR (1024) NOT NULL,
`image_strategy` VARCHAR (16) NOT NULL,
`image_tag` VARCHAR (255) NULL,
`image_source_hash` CHAR (16) NULL,
`image_built_at` DATETIME NULL,
`image_error` TEXT NULL,
`network_policy` VARCHAR (16) NOT NULL,
`trigger_mode` VARCHAR (16) NULL,
`trigger_label` VARCHAR (255) NULL,
`created_at` DATETIME NOT NULL,
`updated_at` DATETIME NOT NULL,
UNIQUE KEY `ai_projects_slug` (`slug`)
) ;

-- one row per workspace: a git worktree of a project, on its own branch.
-- persistent -- it survives a run, unlike the ephemeral sandbox container
-- over it, and is only ever collected by age (`[ai.projects].workspace_ttl`),
-- never by scope exit. `pipeline_id`/`conversation_id` are plain bigints,
-- not foreign keys into `ai_pipelines`/`ai_conversations`: a workspace can
-- outlive the pipeline or conversation that created it (that is the whole
-- point of a workspace persisting after a crash), so there is nothing to
-- cascade.
CREATE TABLE IF NOT EXISTS `ai_workspaces` (
`id` BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
`project_id` BIGINT NOT NULL,
`name` VARCHAR (255) NOT NULL,
`branch` VARCHAR (255) NOT NULL,
`path` VARCHAR (1024) NOT NULL,
`pipeline_id` BIGINT NULL,
`conversation_id` BIGINT NULL,
`created_at` DATETIME NOT NULL,
`last_used_at` DATETIME NOT NULL,
UNIQUE KEY `ai_workspaces_project_name` (`project_id`, `name`),
CONSTRAINT `ai_workspaces_project_id_fk`
FOREIGN KEY (`project_id`) REFERENCES `ai_projects` (`id`) ON DELETE CASCADE
) ;

-- which project a pipeline run belongs to, if any. nullable and
-- ON DELETE SET NULL: removing a project must not delete the historical
-- record of pipelines that ran against it -- only stop attributing them
-- to a project that no longer exists.
ALTER TABLE `ai_pipelines`
ADD COLUMN `project_id` BIGINT NULL,
ADD CONSTRAINT `ai_pipelines_project_id_fk`
FOREIGN KEY (`project_id`) REFERENCES `ai_projects` (`id`) ON DELETE SET NULL ;

-- which project a conversation is bound to, if any -- the chat project
-- picker. same ON DELETE SET NULL reasoning as `ai_pipelines.project_id`.
ALTER TABLE `ai_conversations`
ADD COLUMN `project_id` BIGINT NULL,
ADD CONSTRAINT `ai_conversations_project_id_fk`
FOREIGN KEY (`project_id`) REFERENCES `ai_projects` (`id`) ON DELETE SET NULL ;
