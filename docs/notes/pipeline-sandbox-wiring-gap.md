# Pipeline sandbox wiring gap

What the executor (`munibot_ai::pipeline::executor`) actually verified about
sandbox lifecycle, and what it didn't - written down so it isn't discovered
by surprise during rollout.

**Partially closed as of commit 231.** `pipeline_id` (229-230) and the real
`SandboxLifecycle` (231, below) are done. Still open, all in
`docs/plans/ai/milestone-7-projects.md` phase 26: the launcher that
constructs an `Executor` at all (232), webhook dispatch (233), and resume at
boot (236) - nothing in any binary calls any of this yet, so it remains
exactly as untested against a real repository as it was before this note's
own first version, just for a different reason now (no caller, rather than
no implementation).

## What's verified

- The executor's own loop mechanics: provisioning exactly once on entering
  `Researching`, tearing down on reaching a terminal state or pausing on
  `AwaitingUserInput`, and every event persisting before the next iteration
  begins - all against a `SandboxLifecycle` mock (`NoSandbox` for the happy
  path, a counting fake for the lifecycle assertions themselves). See
  `Executor::run`'s own tests.
- Every (role, state) pairing and every handoff-to-event conversion, against
  `MockAgentDispatcher` - no model call, no sandbox, no forge.

## `SandboxLifecycle`'s real implementation (commit 231)

`ProjectSandboxLifecycle` (`munibot_ai::pipeline::sandbox`) is the real
implementation: `provision` resolves a dedicated worktree for the run via
`ProjectManager::workspace_for(project, branch, WorkspaceOwner::Pipeline(id))`,
resolves the project's own built image (refusing outright, as
`ExecutorError::Sandbox`, if that image failed to build - never silently
falling back to the generically configured one), and provisions a sandbox
over that worktree at `SandboxPolicy::Required` - eagerly, unlike a chat
persona's `Optional` sandbox, since a pipeline role that reaches a
sandboxed state cannot do its job at all without one. `teardown` drops the
session but never the worktree itself, the same "persistent workspace"
guarantee `crate::project::collect_unused_workspaces` and chat's own
project-bound conversations rely on.

This is a different, simpler mechanism than this note originally
described: no in-container clone via `Sandbox::checkout` at all - the
worktree is already checked out on the _host_, by `ProjectManager`, and
`WorkspaceSession::over_workspace` bind-mounts it in, the same path chat's
own project-bound conversations use (commit 226).

Tested against a real `ProjectManager` (an in-memory store, a local git
upstream, no network) for the workspace-resolution mechanics, deliberately
without a real working tool agent inside a real container - that full
round trip, against a real project's real built image, is commit 243's own
job (`docs/plans/ai/milestone-7-projects.md` phase 27).

Still needed before any of this runs for real: the launcher that resolves
which project a run belongs to and actually constructs one of these
(commit 232) - nothing in any binary does yet.

## Pipeline monitor page's own scope (phase 22, commit 181)

The monitor page's per-pipeline detail view shows one row per persisted
`PipelineEvent` - "every agent invocation" in the sense that each event is
exactly what one role's own handoff produced. It does not show "every tool
call" a turn made along the way: those are recorded in `ai_tool_calls`,
now keyed by `pipeline_id` as well as `conversation_id` (commit 229-230
gave a pipeline turn its own audit trail) - showing them on the monitor
page itself is still unbuilt, a straightforward read against
`list_tool_calls_for_conversation`'s own `pipeline_id`-filtered sibling
(which does not exist yet either). `PipelineRegistry::is_running` is real,
but nothing in the current binary actually starts a pipeline running in
the first place (see above), so `running` on every summary is `false`
until that wiring lands too.
