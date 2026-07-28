# Milestone 4 — autonomous development

**Outcome:** someone opens an issue on a repository munibot watches, and munibot triages it,
researches the codebase, plans the work, writes tests, implements them, reviews itself, commits
granularly, and opens a pull request — asking you in Discord whenever it needs a decision.

This is `municode`'s entire purpose, rebuilt inside munibot with one significant advantage: munibot
already lives where you talk. When the architect needs clarification, it does not post a comment into
the void and wait. It asks you in Discord and resumes on your reply.

**Phases 15 through 17, commits 116 through 151.**

---

## The pipeline

Every box is a fresh, stateless agent invocation. Context arrives only as explicit template
variables. This is the central defence against context rot, and the reason the whole system is built
out of small personas rather than one long-running agent.

```
issue event
    │
    ▼
Issue Analyst ──Skip──> done
    │ NeedsMoreInfo ──> ask, await reply
    │ Proceed
    ▼
Codebase Researcher ──ResearchComplete──> Software Architect <──RequestPlanChanges──┐
                                              │ RequestPlanHelp ──> ask             │
                                              │ CreatePlan                          │
                                              ▼                                     │
                                       Architecture Reviewer ────────────────────────┘
                                              │ ApprovePlan
                                              ▼
                       ┌──────────────> Project Manager
                       │                      │ StartTaskTests
                       │                      ▼
                       │              Test Engineer <──RequestTestChanges──┐
                       │                      │ SubmitTests                │
                       │                      ▼                            │
                       │              Test Reviewer ─────────────────────────┘
                       │                      │ ApproveTests
                       │                      ▼
                       │                  Builder <──RequestCodeChanges──┐
                       │                      │ SubmitCode                │
                       │                      ▼                           │
                       │              Code Reviewer ────────────────────────┘
                       │                      │ ApproveCode
                       │                      ▼
                       └───CommitComplete── Commit Crafter

Project Manager ──BeginFinalReview──> Final Code Reviewer
                                              │ RequestCodeChanges ──> fix subtask ──> Project Manager
                                              │ ProjectComplete
                                              ▼
                                         PR Author ──> pull request opened, munibot stops
```

**Nothing merges.** munibot opens a pull request and a human decides.

### Porting the prompts

The eleven prompts in `municode/docs/agent-prompts/` are high quality and port over nearly verbatim.
They carry known defects that must be fixed during the port rather than inherited:

- `architecture-reviewer.md` line 88 has a stray `git config --unset-all --local core.hooksPath`
  spliced mid-sentence.
- `architecture-reviewer.md` requires a `strengths` field in its `ApprovePlan` table that its own JSON
  example omits.
- `project-manager.md` refers to `StartTask` in its context-enrichment section; the action is
  `StartTaskTests`.
- `code-reviewer.md` references an `implementation_issues` field that `SubmitTests` does not have. The
  real field is `assumptions`.
- `overview.md` uses `TestsWritten` and `ReadyForReview` where the prompts use `SubmitTests` and
  `SubmitCode`. The prompts win.

The twelfth prompt, Issue Analyst, exists only as a sketch at `municode/docs/plan.md` lines 889–929
and must be written in full to the same standard as the other eleven.

Every role becomes a `Persona` with `handoff` set, which means the pipeline reuses the harness
unchanged. No new execution machinery.

---

## Phase 15 — `munibot_vcs` and `munibot_github`

VCS-agnostic traits first, then the GitHub App implementation. You have said other forges are coming,
so the abstraction is worth having from the start rather than retrofitted.

| #   | Commit                                                            | Description                                                                                                                                                                                                                                                                                                                      |
| --- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 116 | `build(deps): add forge integration dependencies`                 | Add `octocrab`, `jsonwebtoken`, `hmac`, `sha2`, and `subtle` to `[workspace.dependencies]`. `subtle` is for constant-time signature comparison and is not optional.                                                                                                                                                              |
| 117 | `build(vcs): create vcs crate skeleton`                           | Create `munibot_vcs/` depending on `async-trait`, `serde`, `thiserror`, `chrono`, and `url`. No forge-specific dependency belongs here, ever.                                                                                                                                                                                    |
| 118 | `feat(vcs): add repository and issue reference types`             | `Forge` (`GitHub`, with room for others), `RepoRef { forge, owner, name }`, `IssueRef { repo, number }`, `Issue { reference, title, body, author, labels, state }`, and `Comment`. Serde plus `Display` implementations that produce the conventional `owner/name#number` form.                                                  |
| 119 | `feat(vcs): add normalized forge event types`                     | `ForgeEvent`: `IssueOpened`, `IssueLabeled { label }`, `IssueCommented { comment }`, `PullRequestReviewed`. Every forge normalizes into this, so the pipeline never sees a GitHub payload.                                                                                                                                       |
| 120 | `feat(vcs): add trigger configuration types`                      | `TriggerMode` (`AllIssues`, `Label(String)`, `Keyword(String)`, `Any(Vec<TriggerMode>)`) and `RepoTriggerConfig { repo, mode, enabled }`, with a `matches(&ForgeEvent) -> bool` that is a pure function with table-driven tests. Repository owners choose their own trigger style.                                               |
| 121 | `feat(vcs): add issue source and pull request traits`             | `#[async_trait] IssueSource` with `fetch_issue`, `list_comments`, and `post_comment`; `PullRequestTarget` with `create_branch`, `push`, `open_pull_request`, and `clone_url_with_token`. Both object-safe, so the pipeline holds `Arc<dyn IssueSource>`.                                                                         |
| 122 | `build(github): create github crate skeleton`                     | Create `munibot_github/` depending on `munibot_vcs`, `octocrab`, `jsonwebtoken`, `hmac`, `sha2`, `subtle`, `axum`, and `tracing`.                                                                                                                                                                                                |
| 123 | `feat(github): add app authentication with installation tokens`   | Mint a short-lived JWT from `GITHUB_APP_ID` and `GITHUB_APP_PRIVATE_KEY`, exchange it for a per-installation access token, and cache tokens in a `RwLock<HashMap<InstallationId, (String, Instant)>>` refreshing a few minutes before the one-hour expiry. Add all three variables plus the webhook secret to `secretspec.toml`. |
| 124 | `feat(github): add webhook signature verification`                | Verify the `X-Hub-Signature-256` HMAC against `GITHUB_WEBHOOK_SECRET` using `subtle` for constant-time comparison over the **raw** body, before any parsing. Reject missing or malformed signatures. Tests use known-good vectors. Timing-safe comparison here is not a nicety.                                                  |
| 125 | `feat(github): add webhook payload normalization`                 | Parse the `X-GitHub-Event` header and body into a `ForgeEvent`, ignoring event types munibot does not act on rather than erroring. Filter out events authored by munibot's own App identity, which is how you avoid an infinite comment loop on day one.                                                                         |
| 126 | `feat(github): add issue source and pull request implementations` | `GitHubForge` implementing both `munibot_vcs` traits over octocrab with an installation token. `clone_url_with_token` returns a URL suitable for a credential helper, never one that gets logged.                                                                                                                                |
| 127 | `feat(gui): add forge webhook endpoint`                           | An axum `POST /webhooks/github` route in `munibot_gui/src/server.rs`, verifying the signature, normalizing the event, checking it against `RepoTriggerConfig`, and handing off to the pipeline registry. Returns 202 immediately and does all work in a spawned task with a `tracing` span, per `docs/tracing.md`.               |

---

## Phase 16 — `munibot_ai_pipeline`

Types, then persistence, then the pure state machine, then the prompts, then the executor.

| #   | Commit                                                        | Description                                                                                                                                                                                                                                                                                                                                                          |
| --- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 128 | `build(ai_pipeline): create pipeline crate skeleton`          | Create `munibot_ai_pipeline/` depending on `munibot_ai_types`, `munibot_ai_harness`, `munibot_ai_sandbox`, `munibot_vcs`, `munibot_core`, `tokio`, and `tracing`.                                                                                                                                                                                                    |
| 129 | `feat(ai_pipeline): add pipeline state types`                 | `PipelineId` and `PipelineState`: `Triaging`, `Researching`, `Planning`, `ReviewingPlan`, `TestWriting { subtask }`, `TestReviewing { subtask }`, `Building { subtask }`, `ReviewingCode { subtask }`, `Committing { subtask }`, `FinalReview`, `AwaitingFixSubtask`, `WritingPr`, `AwaitingUserInput { request }`, `Complete`, `Failed { reason }`.                 |
| 130 | `feat(ai_pipeline): add agent role and handoff types`         | `AgentRole` with all twelve variants, and a handoff payload type per role — `ResearchComplete`, `CreatePlan`, `ApprovePlan`, `RequestPlanChanges`, `StartTaskTests`, `SubmitTests`, `ApproveTests`, `SubmitCode`, `ApproveCode`, `CommitComplete`, `ProjectComplete`, `PullRequestReady`, `IssueAnalysis`. Each derives `JsonSchema` so the harness can validate it. |
| 131 | `feat(ai_pipeline): add plan and subtask types`               | `Plan { summary, subtasks }` and `Subtask { id, title, description, instructions, commit_message, files_affected, dependencies, status }` with `SubtaskStatus` (`Pending`, `TestsWritten`, `TestsApproved`, `InProgress`, `ReviewPending`, `Approved`, `Committed`). Mirrors the schema the architect prompt already emits.                                          |
| 132 | `feat(db): add pipeline and pipeline event tables`            | Migration for `ai_pipelines` and `ai_pipeline_events` with a unique index on `(pipeline_id, seq)` making the event log append-only and gap-free. Regenerate the diesel schema.                                                                                                                                                                                       |
| 133 | `feat(ai_pipeline): add pipeline store with event sourcing`   | `PipelineStore` trait with an in-memory implementation for tests and a diesel one for production. `append_event` and `replay(pipeline_id) -> PipelineState`. State is always a fold over events, never a mutated column, so recovery is a replay.                                                                                                                    |
| 134 | `feat(ai_pipeline): add pipeline advance transition function` | `Pipeline::advance(state, event) -> Result<PipelineState>` as a **pure function** with no I/O, validating that each transition is legal from the current state. Exhaustively table-driven tests, including every rejection loop and every illegal transition. This function is the specification.                                                                    |
| 135 | `feat(ai_pipeline): add issue analyst role and prompt`        | Write `prompts/pipeline/issue-analyst.md` in full, to the standard of the other eleven: classify as bug, feature, question, or not actionable; attempt reproduction; return `IssueAnalysis` with a recommended action. Runs in a minimal triage sandbox before a full one is provisioned, so a junk issue costs almost nothing.                                      |
| 136 | `feat(ai_pipeline): add researcher and architect prompts`     | Port `codebase-researcher.md` and `software-architect.md`. Preserve the instruction-quality rule verbatim — the demand for `return AppError::NotFound with the message ...` over "add appropriate error handling" is what makes the plans executable by isolated builders.                                                                                           |
| 137 | `feat(ai_pipeline): add plan and code reviewer prompts`       | Port `architecture-reviewer.md`, `code-reviewer.md`, and `final-code-reviewer.md`, fixing the four documented defects. Keep the severity gates exactly as written: critical, major, and minor block; a bare nit does not.                                                                                                                                            |
| 138 | `feat(ai_pipeline): add test engineer and reviewer prompts`   | Port `test-engineer.md` and `test-reviewer.md`. Tests are written **before** the implementation and the builder may not modify them; preserve that framing and the mutation-testing mindset in the reviewer, which is the sharpest quality gate in the whole pipeline.                                                                                               |
| 139 | `feat(ai_pipeline): add builder and project manager prompts`  | Port `builder.md` and `project-manager.md`. Preserve the precedence rule that tests outrank instructions on conflict, and the project manager's `context` field as the only channel carrying cross-subtask memory to an isolated builder.                                                                                                                            |
| 140 | `feat(ai_pipeline): add commit crafter and pr author prompts` | Port `commit-crafter.md` and `pr-author.md`. Adapt both to munibot's own conventions in `AGENTS.md` and `.versionrc`: stage files individually, never amend, re-stage after a formatter hook rewrites a file, no co-author footers, and a scope matching the repository's scope regex.                                                                               |
| 141 | `feat(ai_pipeline): add agent dispatcher over the harness`    | `AgentDispatcher` trait with `invoke_agent(role, context) -> AgentOutput`, and a `HarnessDispatcher` mapping each `AgentRole` to its persona, prompt template, and handoff schema, then delegating to `Harness::run_turn`. A mock dispatcher makes the entire executor testable with no model calls.                                                                 |
| 142 | `feat(ai_pipeline): add branch naming with idempotent reuse`  | `munibot/{issue_number}-{slug}` with the slug lowercased, dash-separated, alphanumeric, and capped at 60 characters. Reuse an existing branch for the same issue; on collision with unrelated work, append an attempt suffix. Pure function, thoroughly tested, because a wrong branch name here means a wrong pull request.                                         |
| 143 | `feat(ai_pipeline): add executor loop with persistence`       | Dispatch the agent for the current state, append the resulting event, advance, persist, repeat until `Complete` or `Failed`. Provision the sandbox on entering `Researching` and tear it down on exit. Every iteration is durable, so a crash resumes rather than restarts.                                                                                          |

---

## Phase 17 — interaction, concurrency, and observability

Where munibot's advantage over a headless pipeline actually materialises.

| #   | Commit                                                      | Description                                                                                                                                                                                                                                                                                                       |
| --- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 144 | `feat(ai_pipeline): add interaction adapter trait`          | `InteractionAdapter` with `request_input(InteractionRequest) -> InteractionResponse` and `notify(pipeline_id, message)`. A `RequestPlanHelp` or `RequestBuildHelp` handoff moves the pipeline to `AwaitingUserInput` and persists, so waiting costs nothing and survives a restart.                               |
| 145 | `feat(ai_pipeline): add github comment interaction adapter` | Post the question as an issue comment and resume when a maintainer replies, matching on the comment thread. The fallback adapter when no Discord channel is configured for a repository.                                                                                                                          |
| 146 | `feat(discord): add pipeline interaction adapter`           | **The reason this lives in munibot.** Open a thread in a configured Discord channel, post the question with context, and resume the pipeline on your reply. Verify the responder has permission over the repository before accepting an answer, since a thread is visible to a whole server.                      |
| 147 | `feat(ai_pipeline): add fix subtask synthesis`              | On `RequestCodeChanges` from the final reviewer, enter `AwaitingFixSubtask`, re-invoke the project manager with the full feedback, receive a `FixSubtask` carrying `review_feedback` and `parent_subtask_id`, and re-enter the test-and-build cycle. Without this the pipeline dead-ends on its own final review. |
| 148 | `feat(ai_pipeline): add concurrency limits and queue`       | `ConcurrencyConfig` with a global and a per-repository maximum, a FIFO queue for overflow, and a `PipelineRegistry` backed by database rows. `abort_pipeline` propagates cancellation into the harness and then stops the container. One runaway repository must not starve every other one.                      |
| 149 | `feat(ai_pipeline): add pipeline resume after restart`      | On startup, load every non-terminal pipeline, replay its events, re-provision its sandbox, and continue. Integration test: kill the process mid-build and assert the pipeline completes on restart. This is the payoff for event sourcing.                                                                        |
| 150 | `feat(gui): add pipeline monitor page`                      | Live pipeline list with state, current subtask, elapsed time, and accumulated cost, streamed over server-sent events. Per-pipeline detail showing the event log, every agent invocation, and every tool call. An unobservable autonomous system is an unusable one.                                               |
| 151 | `feat(discord): add pipeline status commands`               | `/pipeline list`, `/pipeline status <id>`, and `/pipeline abort <id>`, restricted to configured maintainers. Abort matters most: being able to stop a misbehaving run from your phone is worth more than any dashboard.                                                                                           |

---

## Definition of done

- Labelling an issue produces a pull request with granular, conventional commits and passing tests.
- A vague issue prompts munibot to ask a clarifying question in Discord and resume on the reply.
- A junk issue is closed out by the triage agent for a few cents.
- Killing munibot mid-pipeline resumes cleanly on restart.
- Two repositories can run concurrently without interfering.
- `/pipeline abort` stops a run and destroys its container within seconds.
- Nothing is ever merged automatically.

## Risks specific to this milestone

1. **Issue bodies are attacker-controlled.** Anyone can open an issue on a public repository, and that
   text reaches an agent holding filesystem and shell tools. The untrusted-content wrapper from phase
   5 is load-bearing here. Consider restricting triggers to maintainer-labelled issues by default.
2. **Cost per pipeline is high.** A dozen agent invocations across a multi-subtask plan is orders of
   magnitude more expensive than a chat turn. Per-pipeline cost ceilings are mandatory, and a spend
   cap that aborts is safer than an alert that notifies.
3. **Plan quality determines everything.** A bad plan wastes every downstream invocation. The
   architecture reviewer is the cheapest place to catch failure, so prefer rejecting plans too often
   over too rarely.
