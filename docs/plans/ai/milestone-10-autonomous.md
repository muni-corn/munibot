# Milestone 10 — autonomous development

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 09](milestone-09-projects.md).

**Outcome:** munibot turns a request into a pull request. A request can arrive two ways:

- **From conversation.** You tell munibot "the parser chokes on empty files in cocoa, can you fix
  it?". He decides it's real work, opens an issue, starts a run, and tells you so with a link.
- **From GitHub.** Someone opens or labels an issue on a project munibot watches.

Either way, the run triages the work, researches the codebase, plans, writes tests, implements,
reviews itself, commits granularly, and opens a pull request. Whenever it needs a decision, it asks,
and munibot relays the question in your conversation. Nothing ever merges automatically.

---

## Starting point

After milestone 09:

- Projects, worktrees, and per-project images exist.
- All twelve engineering roles are delegable personas, already used interactively. Their prompts
  carry no output contracts yet.
- `Permission::Developer` grants `Privileged`.
- `manage_project` is the only `Privileged` tool.

From the code as it stands:

- There's no `munibot_vcs` or `munibot_github` crate, and no `octocrab`.
- There's no webhook route.
- The only GitHub integration is OAuth sign-in (`munibot_api/src/oauth/github.rs`), an **OAuth App**
  with the `read:user` scope. That's separate from the GitHub **App** this milestone adds, which has
  its own id, private key, and webhook secret.

## What changed from the previous autonomous plan

The pre-strip pipeline was a complete, tested library that nothing in production ever started:

- no launcher;
- no real sandbox lifecycle;
- `resume_all` was never called;
- webhook triggers were hardcoded empty;
- pipeline turns recorded no usage, and fabricated a `ConversationId` from the pipeline id.

This plan wires each of those as part of building it. Every phase ends with something that runs.

## The pipeline

Every box is a fresh, stateless invocation of a persona with a handoff schema attached. Context
arrives only through explicit template variables. That's the central defence against context rot.

```
request (issue event or start_work)
    │
    ▼
Issue Analyst ──Skip──> done
    │ NeedsMoreInfo ──> ask, await reply
    │ Proceed
    ▼
Codebase Researcher ──> Software Architect <──RequestPlanChanges──┐
                              │ RequestPlanHelp ──> ask           │
                              │ CreatePlan                        │
                              ▼                                   │
                        Architecture Reviewer ────────────────────┘
                              │ ApprovePlan
                              ▼
               ┌──────> Project Manager ──BeginFinalReview──> Final Code Reviewer
               │              │ StartTaskTests                    │ RequestCodeChanges ──> fix subtask
               │              ▼                                   │ ProjectComplete
               │        Test Engineer <──> Test Reviewer          ▼
               │              │ ApproveTests                   PR Author ──> pull request, stop
               │              ▼
               │           Builder <──> Code Reviewer
               │              │ ApproveCode
               │              ▼
               └──CommitComplete── Commit Crafter
```

### Starting work from conversation

`start_work` is a tier `Privileged` tool, so only developers and operators can reach it. It takes a
project, a kind (`fix` or `feature`), a title, and a self-contained brief:

1. Opens an issue on the project's repository through the GitHub App, so every run has a public
   record and one entry point.
2. Starts a run bound to that issue, using the calling conversation as its interaction channel.
3. Returns the issue link and the run's monitor link.

A run per issue is unique while it's live, so the webhook for the issue `start_work` just opened
can't start a second run.

`companion.md` and `coder.md` learn when to use it:

- Start when someone clearly asks for a change to a registered project.
- Ask first when the request is ambiguous, or the project isn't clear.
- Always say out loud that work has started.
- Never start work as a side effect of answering a question.

### Questions come back through munibot

When a run needs input, it enters `AwaitingInput` and persists, so waiting costs nothing and
survives a restart. The question appears in the originating conversation as a card. While a
question is pending, that conversation's turns see it in context and have an `answer_work_question`
tool. munibot relays your answer in his own words and confirms it was sent.

Runs started by a webhook have no conversation, so they ask by posting an issue comment, and a
maintainer's reply resumes them. Milestone 12 adds a Discord thread adapter through the same
`InteractionAdapter` trait.

### What's reused

| Need                                      | Reused                                                                                                                   |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| GitHub App JWTs and installation tokens   | octocrab: `OctocrabBuilder::app`, `installation`, and its cached `installation_token_with_buffer`                        |
| Webhook signature verification            | octocrab's `verification` module: HMAC-SHA256 over the raw body, compared in constant time                               |
| Typed webhook payloads                    | octocrab's `WebhookEvent::try_from_header_and_body`                                                                      |
| Issues, comments, branches, pull requests | octocrab's REST client                                                                                                   |
| Structured agent output                   | rig-agent's output-tool mode (`AgentBuilder::output_schema` with `OutputMode::Tool`), with schemas derived by `schemars` |
| Validating that output in full            | `rig_core::tool::validate`, since rig-agent itself only checks top-level required fields                                 |
| Driving each agent                        | the persona agents and hooks from milestones 01 to 08                                                                    |

**Why `munibot_vcs` exists at all.** The pipeline should never see a GitHub payload, so that a second
forge is one crate implementing two traits. That's a bet on a second forge arriving. If it never
does, `munibot_vcs` is an indirection over octocrab with one implementation, and the pipeline could
use octocrab's types directly. It's kept small (types, events, two traits) so the bet is cheap either
way.

The pipeline's own types (states, roles, plans, subtasks, handoff payloads) describe munibot's
process. No library models them.

### Spend

Pipeline turns record usage and tool calls with a `pipeline_id`, attributed to whoever started the
run. Webhook-started runs are attributed to no one, but the global cap still applies. Each run has
its own cost ceiling (`[ai.pipeline].max_cost_usd`), checked before every agent invocation. A run
that would exceed it stops in `Failed` with the reason. A cap that aborts is safer than an alert that
notifies.

## Configuration

```toml
[ai.pipeline]
max_cost_usd = 15.0
max_concurrent = 2
max_concurrent_per_project = 1

[ai.github]
# app id, private key, and webhook secret come from the environment
bot_login = "munibot[bot]"
```

Triggers live on the project row (`trigger_mode`, `trigger_label`), editable on the projects page.
There's no TOML list of repositories to keep in sync.

## Commits

### Forge crates

| #     | Commit                                                      | Summary                                                                                                                                                |
| ----- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 10.1  | `build(secretspec): declare github app credentials`         | `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, and `GITHUB_WEBHOOK_SECRET`, all optional.                                                                  |
| 10.2  | `build(vcs): create the vcs crate`                          | No forge-specific dependency, ever.                                                                                                                    |
| 10.3  | `feat(vcs): add repository and issue types`                 | `Forge`, `RepoRef`, `IssueRef`, `Issue`, and `Comment`, displayed in `owner/name#number` form.                                                         |
| 10.4  | `feat(vcs): add normalized forge events`                    | `IssueOpened`, `IssueLabeled`, `IssueCommented`, and `PullRequestReviewed`.                                                                            |
| 10.5  | `feat(vcs): add trigger types`                              | `TriggerMode` and a pure `matches(&ForgeEvent)`, with table tests.                                                                                     |
| 10.6  | `feat(vcs): add forge traits`                               | `IssueSource` (fetch, comments, post, create) and `PullRequestTarget` (branch, push, open). Both object-safe.                                          |
| 10.7  | `build(github): create the github crate`                    | octocrab, with its JWT support on the `jwt-rust-crypto` backend, so nix doesn't build aws-lc.                                                          |
| 10.8  | `feat(github): add app authentication`                      | octocrab's app builder and its cached installation tokens, refreshed a few minutes before they expire.                                                 |
| 10.9  | `feat(github): verify webhook signatures`                   | octocrab's `verification` module over the **raw** body, before anything is parsed.                                                                     |
| 10.10 | `feat(github): normalize webhook payloads`                  | From octocrab's typed `WebhookEvent` into `ForgeEvent`. Ignores event types munibot doesn't act on, and filters out events munibot's own app authored. |
| 10.11 | `feat(github): implement the forge traits`                  | `GitHubForge` over an installation token.                                                                                                              |
| 10.12 | `test(github): add a wiremock forge suite`                  | Idempotent branch reuse, a rejected push, and opening a pull request end to end.                                                                       |
| 10.13 | `feat(project): clone private repositories through the app` | A `CloneCredentials` implementation using installation tokens through `git credential approve` on stdin. Never in a URL, never in argv.                |
| 10.14 | `feat(project): register projects from app installations`   | An installation's repositories become projects on first use, behind an `auto_register` setting. When it's off, the skip is logged with its reason.     |

### Handoffs

| #     | Commit                                  | Summary                                                                                                                                                                                                                                                                                                                     |
| ----- | --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 10.15 | `feat(persona): add handoff schemas`    | `Persona.handoff: Option<schemars::Schema>`, unset for chat and delegation. A persona with one is built with rig-agent's `output_schema` in `OutputMode::Tool`.                                                                                                                                                             |
| 10.16 | `feat(turn): validate handoffs in full` | An `on_model_turn_finished` hook checks the output tool's arguments with `rig_core::tool::validate`, and answers a failure with `ModelTurnAction::Retry` and the problems as feedback, within a retry budget. Tool mode only, never a provider's native structured output, which can suppress tool calls on some providers. |

### Pipeline core

| #     | Commit                                                     | Summary                                                                                                                             |
| ----- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| 10.17 | `feat(pipeline): add pipeline state types`                 | `PipelineId`, and every state from `Triaging` through `Complete` and `Failed`, including `AwaitingInput`.                           |
| 10.18 | `feat(pipeline): add role and handoff payload types`       | Twelve roles, and a `JsonSchema` payload per handoff.                                                                               |
| 10.19 | `feat(pipeline): add plan and subtask types`               | Mirrors what the architect prompt emits.                                                                                            |
| 10.20 | `feat(db): add pipeline tables`                            | `ai_pipelines` (with project, issue, `started_by_user_id`, and conversation) and the append-only `ai_pipeline_events`.              |
| 10.21 | `feat(db): attribute usage and tool calls to pipelines`    | Nullable `pipeline_id` on `ai_usage` and `ai_tool_calls`.                                                                           |
| 10.22 | `feat(db): add triggers to projects`                       | `trigger_mode` and `trigger_label` on `ai_projects`.                                                                                |
| 10.23 | `feat(config): add pipeline and github settings`           | `[ai.pipeline]` and `[ai.github]`.                                                                                                  |
| 10.24 | `feat(pipeline): add the event-sourced store`              | In-memory and diesel implementations. State is always a fold over events.                                                           |
| 10.25 | `feat(pipeline): add the transition function`              | A pure `advance(state, event)` with exhaustive table tests. It is the specification.                                                |
| 10.26 | `feat(persona): attach handoff schemas to the team`        | The output contracts stripped out during the milestone 07 and 08 ports, supplied exactly once, here.                                |
| 10.27 | `feat(tools): let tools act for a pipeline`                | `Invoker` gains an optional `pipeline_id`, beside an optional conversation. No identifier is ever fabricated.                       |
| 10.28 | `feat(pipeline): add the agent dispatcher`                 | Maps a role to its persona and handoff, runs the persona's agent, and records usage and audit rows with the `pipeline_id`.          |
| 10.29 | `feat(pipeline): add the project-backed sandbox lifecycle` | A workspace on the run's branch via `ProjectManager::workspace_for`, from the project's image. Teardown keeps the worktree.         |
| 10.30 | `feat(pipeline): add the executor`                         | Dispatch, append, advance, persist, and repeat. The per-run cost ceiling and the global cap are checked before each invocation.     |
| 10.31 | `feat(pipeline): synthesize fix subtasks`                  | So the final review's change requests loop back to the project manager instead of dead-ending.                                      |
| 10.32 | `feat(pipeline): add the registry and concurrency limits`  | Global and per-project limits, a FIFO queue, one live run per issue, and abort dropping the running agent stream and the container. |

### Wiring

| #     | Commit                                          | Summary                                                                                                                     |
| ----- | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 10.33 | `feat(pipeline): add the launcher`              | Forge, store, dispatcher, lifecycle, registry, and a spawned executor with `.instrument(span)`. The one place a run starts. |
| 10.34 | `feat(gui): add the github webhook route`       | Verify, normalize, match against project triggers, then launch. Returns 202 immediately.                                    |
| 10.35 | `feat(munibot): wire the pipeline at startup`   | Built only when the app credentials exist. Without them there's no launcher, and startup still succeeds.                    |
| 10.36 | `feat(munibot): resume unfinished runs at boot` | Replay, re-provision, and continue, before the webhook route accepts deliveries.                                            |
| 10.37 | `test(pipeline): add a launch integration test` | A signed webhook payload resolves a project, starts the executor, and is aborted, against `MockCompletionModel`.            |

### Interaction and starting work

| #     | Commit                                               | Summary                                                                                 |
| ----- | ---------------------------------------------------- | --------------------------------------------------------------------------------------- |
| 10.38 | `feat(pipeline): add the interaction adapter trait`  | `ask` and `notify`. `AwaitingInput` persists.                                           |
| 10.39 | `feat(pipeline): ask through issue comments`         | The adapter for webhook-started runs.                                                   |
| 10.40 | `feat(ai): ask through the originating conversation` | Question cards, pending questions in turn context, and the `answer_work_question` tool. |
| 10.41 | `feat(tools): add the start work tool`               | As described above. Tier `Privileged`.                                                  |
| 10.42 | `feat(persona): teach munibot when to start work`    | The judgement rules above, in `companion.md` and `coder.md`.                            |
| 10.43 | `feat(gui): show work started from a conversation`   | A card with the issue, the run's state, and a link to the monitor.                      |

### Observability

| #     | Commit                                     | Summary                                                                                                           |
| ----- | ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| 10.44 | `feat(api): add pipeline server functions` | List, detail with the event log, agent invocations, and tool calls, plus abort. Developer-gated.                  |
| 10.45 | `feat(gui): add the work monitor`          | `/work` shows live runs with their state, subtask, elapsed time, and cost, and lets you abort a run from a phone. |
| 10.46 | `docs: document autonomous work`           | The runbook covers app setup, triggers, `start_work`, costs, aborting, and resuming.                              |

## Definition of done

- Asking munibot, as a developer, to fix a described bug in a registered project opens an issue,
  starts a run, and he says so with links. The run ends in a pull request with conventional commits
  and passing tests.
- A vague request makes the run ask a question that appears in your conversation, and your answer,
  relayed by munibot, resumes it.
- Labelling an issue on GitHub starts a run with no conversation, and it asks through issue comments.
- A non-developer asking munibot to change code gets an explanation, and no run starts.
- A junk issue is closed out by triage for a few cents.
- Killing munibot mid-run resumes cleanly on restart.
- A run's spend appears on `/usage`, and a run that reaches its ceiling stops.
- Nothing is ever merged automatically.

## Risks

1. **Issue bodies are attacker-controlled.** On a public repository, anyone can write text that
   reaches an agent holding a shell. The untrusted wrapper is load-bearing here. Default triggers
   to maintainer-applied labels.
2. **A run is expensive.** Dozens of invocations per run. The ceiling is mandatory.
3. **Plan quality decides everything.** The architecture reviewer is the cheapest place to catch a
   bad plan. Prefer rejecting plans too often.
4. **`start_work` is a judgement call made by a model.** The prompt rules, the developer gate, the
   per-run ceiling, and the public issue together keep a wrong call cheap and visible.

## Deliberately deferred

- **Merging.** Never automatic. A human decides.
- **Other forges.** `munibot_vcs` exists so that Forgejo or GitLab is one crate implementing two
  traits.
- **Creating new repositories.** `start_work` changes existing projects. Starting a project from
  nothing is an open decision in [the overview](overview.md#decisions-still-open).
