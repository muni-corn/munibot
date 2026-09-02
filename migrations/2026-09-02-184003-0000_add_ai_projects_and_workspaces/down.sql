ALTER TABLE `ai_conversations`
DROP FOREIGN KEY `ai_conversations_project_id_fk`,
DROP COLUMN `project_id` ;

ALTER TABLE `ai_pipelines`
DROP FOREIGN KEY `ai_pipelines_project_id_fk`,
DROP COLUMN `project_id` ;

DROP TABLE IF EXISTS `ai_workspaces` ;
DROP TABLE IF EXISTS `ai_projects` ;
