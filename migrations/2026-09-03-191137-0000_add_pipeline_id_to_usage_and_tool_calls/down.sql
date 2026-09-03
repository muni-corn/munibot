ALTER TABLE `ai_tool_calls`
DROP FOREIGN KEY `ai_tool_calls_pipeline_id_fk`,
DROP COLUMN `pipeline_id` ;

ALTER TABLE `ai_usage`
DROP FOREIGN KEY `ai_usage_pipeline_id_fk`,
DROP COLUMN `pipeline_id` ;
