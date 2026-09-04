# Pipeline sandbox wiring gap

What the executor (`munibot_ai::pipeline::executor`) actually verified about
sandbox lifecycle, and what it didn't - written down so it isn't discovered
by surprise during rollout.

**Closed as of commit 236.** Everything this note originally described as
missing is built and wired into the real running server now: `pipeline_id`
(229-230), the real `SandboxLifecycle` (231, below), the launcher
(232), webhook dispatch (233), sourcing triggers from the project store
instead of a hardcoded empty list (234), constructing the launcher in
`munibot::ai::build` and injecting it as `WebhookConfig::dispatch` (235),
and resuming every non-terminal pipeline at boot (236). A webhook delivery
can start a real pipeline run today, against a real project's real
worktree and image, in a real running server.

Phase 27 (commits 239-243) verified the pieces this needed, but not all
together in one run - see the `SandboxLifecycle` section below for exactly
what commit 243 did and did not cover, and why.

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
without a real working tool agent inside a real container.

Commit 243's webhook-to-`Complete` test (below) does **not** close that
remaining gap, and should not be read as having done so: its
`MockProvider` responds with a `NotActionable`/`Skip` analysis, and
`advance.rs`'s own state table sends `Skip` straight to `Complete` without
ever entering a role that calls `provision` - so that test never
constructs a real `ProjectSandboxLifecycle`, never touches podman, and
proves the webhook/registry/dispatch wiring only. `provision`'s own
resolution logic is exercised (without a container) by
`ProjectSandboxLifecycle`'s own tests, above; a real, compiled tool agent
talking through a real container is exercised, apart from any pipeline, by
commit 240's `test_the_full_chain_a_real_tool_call_round_trips_through_a_real_container`;
and a project's real image getting built is exercised, apart from any
sandbox or pipeline, by commit 241's `project_image_build.rs`. No test
anywhere yet drives a pipeline role past `Researching` into a state that
provisions `ProjectSandboxLifecycle` for real and runs a tool call through
the resulting container - that combination remains genuinely unverified,
and would need a `MockProvider` response landing on `RecommendedAction`
other than `Skip` plus a project with a real, already-built image.

`PipelineLauncher` (`munibot_gui::server::pipeline_launcher`) is that
launcher: `launch` resolves which project a matched trigger's own
repository belongs to and constructs one of these for real, sharing one
`Arc<dyn SandboxLifecycle>` with `PipelineRegistry::try_start` so an abort
tears down the exact same instance the run loop provisioned through, never
a second one that never provisioned anything.

## Pipeline monitor page's own scope (phase 22, commit 181)

The monitor page's per-pipeline detail view shows one row per persisted
`PipelineEvent` - "every agent invocation" in the sense that each event is
exactly what one role's own handoff produced. It does not show "every tool
call" a turn made along the way: those are recorded in `ai_tool_calls`,
now keyed by `pipeline_id` as well as `conversation_id` (commit 229-230
gave a pipeline turn its own audit trail) - showing them on the monitor
page itself is still unbuilt, a straightforward read against
`list_tool_calls_for_conversation`'s own `pipeline_id`-filtered sibling
(which does not exist yet either). `PipelineRegistry::is_running` is real
and, as of commit 235, reflects an actually-running pipeline correctly -
this page's own summaries were simply never revisited to take advantage
of it.
