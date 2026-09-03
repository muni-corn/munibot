-- which pipeline run a usage or tool-call record was made under, if any --
-- nullable, since every record predating this milestone (and every one a
-- plain chat turn ever writes) has no pipeline behind it at all.
--
-- ai_usage: ON DELETE SET NULL, the same reasoning conversation_id and
-- user_id already use here -- a spend record must survive independently
-- for billing and reporting even if the pipeline row it was attributed to
-- were ever removed.
--
-- ai_tool_calls: ON DELETE CASCADE, the same reasoning conversation_id
-- already uses here -- a tool call belongs entirely to the run that made
-- it and is meaningless without it.
ALTER TABLE `ai_usage`
ADD COLUMN `pipeline_id` BIGINT NULL,
ADD CONSTRAINT `ai_usage_pipeline_id_fk`
FOREIGN KEY (`pipeline_id`) REFERENCES `ai_pipelines` (`id`) ON DELETE SET NULL ;

ALTER TABLE `ai_tool_calls`
ADD COLUMN `pipeline_id` BIGINT NULL,
ADD CONSTRAINT `ai_tool_calls_pipeline_id_fk`
FOREIGN KEY (`pipeline_id`) REFERENCES `ai_pipelines` (`id`) ON DELETE CASCADE ;
