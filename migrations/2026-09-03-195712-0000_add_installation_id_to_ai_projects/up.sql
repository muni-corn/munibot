-- which GitHub App installation authenticates this project, if any --
-- set once, at registration, from the InstallationEvent a repository's
-- webhook delivery carries. NULL for a project registered manually
-- (the operator projects page, or the manage_project tool against a
-- repository already reachable without one) - such a project can be
-- cloned once, but nothing can later re-authenticate as it to comment
-- on an issue or open a pull request. Never a foreign key: an
-- installation is GitHub's own concept, with nothing in this schema to
-- reference.
ALTER TABLE `ai_projects`
ADD COLUMN `installation_id` BIGINT NULL ;
