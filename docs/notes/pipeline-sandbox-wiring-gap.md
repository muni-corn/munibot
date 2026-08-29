# Pipeline sandbox wiring gap

What the executor (`munibot_ai::pipeline::executor`) actually verified about
sandbox lifecycle, and what it didn't - written down so it isn't discovered
by surprise during rollout.

## What's verified

- The executor's own loop mechanics: provisioning exactly once on entering
  `Researching`, tearing down on reaching a terminal state or pausing on
  `AwaitingUserInput`, and every event persisting before the next iteration
  begins - all against a `SandboxLifecycle` mock (`NoSandbox` for the happy
  path, a counting fake for the lifecycle assertions themselves). See
  `Executor::run`'s own tests.
- Every (role, state) pairing and every handoff-to-event conversion, against
  `MockAgentDispatcher` - no model call, no sandbox, no forge.

## What's still only a trait, not a real implementation

`SandboxLifecycle`'s real implementation - the one that actually calls
`munibot_ai::sandbox::provision_if_needed`, checks out the repository via
`Sandbox::checkout` using a `munibot_github::GitHubForge`'s own
`clone_url_with_token`, and layers the resulting tools onto what
`HarnessDispatcher` runs with - does not exist yet. Building it needs:

- A live `IssueSource`/`PullRequestTarget` (a real `GitHubForge`, in
  practice) threaded into whatever constructs the executor for a real run.
- Deciding which branch to check out (`resolve_branch_name`, already built)
  before `Sandbox::checkout` has anything to check out.
- Handling `provision_if_needed`'s own failure modes (podman unavailable,
  clone failure) as `ExecutorError::Sandbox` in a way the pipeline monitor
  page (a later commit) can actually surface to a maintainer.

None of this is hard given everything already built - `provision_if_needed`
and `Sandbox::checkout` both already exist and are independently tested
(see `docs/notes/sandbox-verification-gaps.md`) - it is just not yet the
one thing that turns "the executor can drive a scripted mock through every
state" into "the executor can actually build software". Do this before a
real repository is ever pointed at this pipeline.

## Pipeline monitor page's own scope (phase 22, commit 181)

The monitor page's per-pipeline detail view shows one row per persisted
`PipelineEvent` - "every agent invocation" in the sense that each event is
exactly what one role's own handoff produced. It does not show "every tool
call" a turn made along the way: those are recorded in `ai_tool_calls`
today, keyed by `conversation_id`, and a pipeline turn has no
`conversation_id` at all (`HarnessDispatcher` never records usage or tool
calls anywhere). Linking a turn's own tool-call audit trail back to the
pipeline that ran it needs either a `pipeline_id` column somewhere in that
recording path, or folding it into the event log itself - neither exists
yet. `PipelineRegistry::is_running` is real, but nothing in the current
binary actually starts a pipeline running in the first place (see above),
so `running` on every summary is `false` until that wiring lands too.
