# munibot AI agent harness — overview

munibot is gaining a full agent harness: a provider-agnostic, tool-using, multi-persona AI system
that serves casual conversation, emotional support, creative writing, deep research, and autonomous
software development.

munibot is **a companion first**, and supremely useful second. The ordering matters: he earns the
title of the universe's most lovable bot by being someone worth talking to, and keeps it by also being
genuinely good at programming and research. Where those two goals compete, the companion wins.

His **primary surface is munibot's own web chat page**, because that is where a persistent
conversation list, streamed replies, rendered code, visible tool work, and an auditable memory panel
can all exist at once. Discord is a secondary surface that stops at what milestone 1 built.

This plan supersedes the `municode` project. Everything `municode` planned is absorbed here: the
provider-agnostic LLM client, the tool suite, the agent loop, the container sandbox, and the
multi-agent pipeline that turns an issue into a pull request.

`municode`'s **twelve agent prompts are kept in full and promoted**, not merely mined for precedent.
They are a complete software engineering team — architect, reviewers, project manager, issue analyst,
codebase researcher, builder, test engineer, commit crafter, PR author — and munibot delegates to them
from ordinary conversation, starting in milestone 3, rather than only reaching them through an
autonomous pipeline. Eleven exist at `municode/docs/agent-prompts/`; the twelfth, Issue Analyst, is a
draft at `municode/docs/plan.md` lines 889–929.

They split by whether a role needs a filesystem: the six advisory roles land in milestone 3 phase 16,
the six hands-on roles in milestone 4 phase 19 once a sandbox exists to give them hands. By the time
milestone 5 assembles the autonomous pipeline, every agent in it has already been exercised
interactively — which turns the riskiest milestone in this plan from a first outing for twelve untested
prompts into an orchestration problem over known-good parts.

## Milestone map

| Milestone                                                    | Outcome                                                            | Phases | Commits |
| ------------------------------------------------------------ | ------------------------------------------------------------------ | ------ | ------- |
| [1 — conversation](milestone-1-conversation.md)              | munibot holds a real conversation in Discord                       | 0–8    | 1–66    |
| [2 — the companion on the web](milestone-2-web-companion.md) | A web chat page with persistence, memory, and real usefulness      | 9–14   | 67–108  |
| [3 — specialists and senses](milestone-3-specialists.md)     | He delegates to an engineering team, and can see what you show him | 15–17  | 109–129 |
| [4 — sandbox](milestone-4-sandbox.md)                        | munibot reads, writes, and runs code in a container                | 18–19  | 130–151 |
| [5 — autonomous development](milestone-5-autonomous.md)      | munibot answers a GitHub issue with a working pull request         | 20–22  | 152–182 |
| [6 — hardening](milestone-6-hardening.md)                    | Safe, affordable, and observable in public                         | 23     | 183–199 |

Around 199 commits total. Each commit is one logical change that leaves the workspace compiling.

Commit numbers are sequence labels, not promises. Milestone 1 already landed three commits its own
table never planned for, and each milestone's internal numbering is rebased against reality when work
on it actually begins — so treat the ranges for milestones 3 through 5 as approximate until then.

## Guiding decisions

| Decision              | Choice                                                      | Rationale                                                                                                                                                                                                                                                                                               |
| --------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Provider abstraction  | In-house `Provider` trait over a `rig-core` backend         | rig covers 20+ providers with embeddings and vector stores. Its `CompletionModel` is not object-safe, so our trait is what makes runtime provider selection possible at all — see `docs/notes/ai-preflight-findings.md`. It also absorbs rig's pre-1.0 churn and leaves room for a hand-rolled backend. |
| Agent loop            | Hand-rolled, not rig's `Agent`                              | We need budgets, cancellation, structured handoffs, and event streaming. Use rig's low-level `CompletionModel`; own everything above it.                                                                                                                                                                |
| Unit of configuration | The **persona**                                             | A persona is a model, a system prompt, a tool allowlist, a budget, and an optional handoff schema. Chat personas and pipeline agent roles are the same type.                                                                                                                                            |
| Structure             | One `munibot_ai` crate with internal modules, plus adapters | A project this size does not need ten `Cargo.toml`s. Rust's module privacy enforces the same internal boundaries a crate split would, at a fraction of the ceremony. Forge integration and the tool agent binary stay separate — see below.                                                             |
| Persistence           | MySQL through `diesel-async`                                | Matches existing munibot infrastructure. Pipelines use an append-only event log.                                                                                                                                                                                                                        |
| Sandbox               | Rootless podman through `bollard`, tools over a Unix socket | Strong isolation for untrusted generated code, and it matches how the deployment already works.                                                                                                                                                                                                         |
| Search                | Exa                                                         | Neural search with content extraction in one API, designed for model consumption.                                                                                                                                                                                                                       |
| Forge integration     | A proper GitHub App                                         | Per-repository installation, scoped permissions, far better rate limits, and a real bot identity on pull requests. Worth the extra setup over a token.                                                                                                                                                  |
| Primary surface       | munibot's own web chat page                                 | A conversation list, streamed replies, rendered code, visible tool activity, and an auditable memory panel need room that a chat platform does not give. Discord stays as a secondary surface at milestone 1's feature set; Twitch AI is dropped.                                                       |
| Persona selection     | A visible picker, plus a companion who has his own tools    | An invisible personality switch is actively bad for a companion, and giving the companion `web_search`/`web_fetch` removes most of the reason to switch at all. The sticky auto-router is deferred rather than built — see milestone 2's deferred list.                                                 |
| Specialist access     | Delegation through a `delegate` tool, never a persona swap  | munibot stays the conversational partner and calls a specialist the way he calls any other tool, then reports back in his own voice. Reuses tier gating, budget accounting, cancellation, auditing, and the activity indicator that already exist, and keeps the relationship continuous.               |
| Memory                | Opt-in per user, with full user control                     | `remember` and `forget` tools, plus a web panel to list, edit, delete, and wipe. An opt-in you cannot audit is not really consent, so the panel ships in the same milestone as the tools.                                                                                                               |

## Crate architecture

One crate holds everything ai-specific except forge integration and the in-container tool agent,
which stay separate for reasons that have nothing to do with being "ai":

```
munibot_ai            everything below, as modules of one crate
  ai::types              provider-neutral domain types; serde and schemars only
  ai::provider           Provider trait; rig-backed and mock implementations; retry classification
  ai::tools              Tool trait, ToolRegistry, ToolCtx, risk tiers, built-in tools
  ai::harness            the agent loop: model to tools to handoff, with budgets and events
  ai::memory             SessionStore and MemoryStore traits, compaction, diesel implementations
  ai::sandbox            podman lifecycle, repository checkout, RPC client, sandboxed tools
  ai::pipeline           multi-agent state machine: issue to research to plan to build to review to PR
  ai::persona            persona type, prompt template engine, and the persona registry
  (crate root)           the `[ai]` config section and the `Ai` service handle

munibot_toolagent     [bin] in-container RPC server executing filesystem and shell tools
munibot_vcs           VCS-agnostic traits: IssueSource, PullRequestTarget, normalized webhooks
munibot_github        octocrab-backed VCS implementation and webhook verification
```

`munibot_toolagent` and `munibot_vcs`/`munibot_github` are separate crates on purpose:

- **`munibot_toolagent`** ships into an untrusted container and must stay small regardless of what
  `munibot_ai`'s own dependency tree grows into (diesel-async, bollard, rig, ...). Bundling it as a
  module would mean every container build either pulls in all of that or fights a feature-flag matrix
  to avoid it. A tiny standalone crate sidesteps the problem entirely.
- **`munibot_vcs`/`munibot_github`** are not an ai concern. They are forge integration that the
  pipeline happens to consume, and could serve a plain (non-ai) `/issue` command just as well. Keeping
  them separate matches the existing split by concern (`munibot_core` / `munibot_discord` /
  `munibot_twitch`) rather than by feature.

### Dependency graph

```
munibot_ai (lib.rs exposes the Ai service handle, built on everything below)
  ai::types
    ├── ai::provider ──> rig-core
    ├── ai::tools
    │     └── ai::harness ──> ai::provider
    │           ├── ai::memory ──> munibot_core
    │           ├── ai::sandbox ──> bollard
    │           └── ai::pipeline ──> munibot_vcs
    └── ai::persona ──> all of the above, plus munibot_core

munibot_vcs
  └── munibot_github ──> octocrab

munibot_toolagent     (no munibot dependency at all — see below)
```

Within `munibot_ai`, dependencies flow strictly downward through module visibility: `ai::types`
depends on nothing but `serde`, `schemars`, and `thiserror`, and no other module may leak a `rig` type
past `ai::provider`. `munibot_vcs` and `munibot_github` know nothing about AI.

**`munibot_toolagent` takes no munibot dependency at all**, not even `munibot_ai::types`. Before the
consolidation it could lean on `munibot_ai_types` for its RPC wire types (`ToolRequest`,
`ToolResponse`) at zero cost, because that crate had no heavy dependencies of its own. Now that those
types live inside `munibot_ai`, depending on the crate at all — for any single module — pulls
`rig-core`, `bollard`, and everything else `munibot_ai` needs into the tool agent's build, since Cargo
does not compile a crate's dependencies per-module. `munibot_toolagent` instead defines its own tiny
copy of the wire protocol types. A few lines of duplication buys a hard isolation boundary for the one
binary that runs inside a container an attacker's generated code can reach.

Platform adapters live in the crates that already own those platforms:

- **`munibot_api/src/chat/` and `munibot_gui/src/pages/chat/` — the primary surface.** Neither crate
  depends on `munibot_ai` today, and `Arc<Ai>` is currently built inside `munibot`'s bot-startup guard
  rather than being reachable from the GUI server; milestone 2 phase 11 fixes both.
- `munibot_discord/src/handlers/ai.rs` and `munibot_discord/src/commands/ai.rs` — secondary, frozen at
  milestone 1's feature set.
- `munibot_twitch/` gets no AI adapter. Twitch cannot edit messages, so it needs an entirely separate
  buffered-chunking renderer, and it is not a surface where a companion is worth showing off.

### Workspace layout

Crates stay flat at the workspace root, as they are today. This plan adds three: `munibot_ai`,
`munibot_toolagent`, and `munibot_vcs`/`munibot_github` (two), taking the workspace from six top-level
directories to ten. This keeps `diesel.toml`, the `embed_migrations!` path in
`munibot_core/src/db.rs`, and `nix/build.nix` untouched.

## The persona abstraction

Everything hinges on this type. One definition serves the companion, the writer, the researcher, and
all eleven pipeline agent roles.

```rust
pub struct Persona {
    /// Stable identifier used in config, commands, and the database.
    pub id: PersonaId,
    pub display_name: String,
    /// Shown in the persona picker, the catalogue, and the `delegate` tool's schema.
    pub description: String,
    /// Provider and model, resolved at runtime from a string like `anthropic:claude-opus-5`.
    pub model: ModelRef,
    pub params: ModelParams,
    pub system_prompt: PromptTemplate,
    pub tools: ToolSelection,
    pub budget: Budget,
    /// Structured terminal output. Left `None` for chat and for delegation, where the
    /// specialist simply answers; the pipeline attaches one when it needs a
    /// machine-readable result. This is what lets one prompt file serve both.
    pub handoff: Option<HandoffSchema>,
    pub memory: MemoryPolicy,
    pub sandbox: SandboxPolicy,
    /// Whether munibot may bring this persona in mid-conversation via `delegate`.
    /// Defaults to `false`, so a role is reachable only when configuration says so.
    pub delegable: bool,
}
```

Personas are declared in TOML, with prompts in separate markdown files so they can be edited without
recompiling and without TOML escaping. The companion and the whole engineering team ship as an
**embedded default roster** (`PersonaRegistry::embedded_personas`) - the minimum an operator ever
has to write is:

```toml
[ai]
enabled = true
default_model = "anthropic:claude-opus-5"
```

That alone resolves every embedded persona - companion, researcher, coder, writer, the six
engineering-team roles, and the critic - against the same model, with `default_persona` falling
back to `companion` automatically. `[ai.personas.<id>]` is only for **overriding** one: an operator's own entry for an id
replaces the embedded default entirely (not a field-by-field merge), and any id they never mention
keeps the embedded one unchanged. A persona whose model can't be resolved - no `default_model` set
and no explicit override - is skipped rather than failing startup, since it was never something the
operator explicitly asked to work.

The full example below shows every knob available, overriding personas that want something
different from the shared default:

```toml
[ai]
default_persona = "companion"
default_model = "anthropic:claude-opus-5"
prompt_dir = "/etc/muni_bot/prompts" # optional; defaults to embedded prompts
max_delegation_depth = 2

# real, reviewed resources the crisis response path surfaces on a positive
# signal - never a model's own invention. region-appropriate, not exhaustive:
# an operator adds whichever regions their community actually needs.
[[ai.crisis_resources]]
name = "988 Suicide & Crisis Lifeline"
contact = "call or text 988 (US)"

[[ai.crisis_resources]]
name = "Samaritans"
contact = "116 123 (UK and Ireland)"

# every scope is unlimited unless configured; checked before the provider
# call, global first, since it is the last defence against a runaway loop
[ai.rate_limits.user]
max_requests = 20
window = "1m"
max_tokens = 50000
max_concurrent_turns = 2

[ai.rate_limits.global]
max_requests = 200
window = "1m"

# per user and globally only - a guild's own members are already covered
# individually by their own user-level caps. at 80 percent, warns in the
# log; at 100 percent, refuses new turns for that scope
[ai.spend_caps.user]
max_usd = 5.0
period = "monthly"

[ai.spend_caps.global]
max_usd = 500.0
period = "monthly"

[ai.personas.companion]
prompt = "companion.md"
description = "warm, playful conversation and emotional support"
temperature = 1.0
# the companion carries research tools himself, so most questions need no specialist
tools = ["tier0", "web_search", "web_fetch", "delegate"]
# companion.md renders {{memories}}; without this, he simply has none to draw on
memory = "user"

[ai.personas.researcher]
# a per-persona model override - anything left unset here still falls back
# to ai.default_model, this is the exception, not the pattern
model = "anthropic:claude-sonnet-5"
prompt = "researcher.md"
description = "multi-step research with citations"
tools = ["tier0", "tier1"]
delegable = true
budget = { max_iterations = 30, max_cost_usd = 2.0 }

[ai.personas.software-architect]
prompt = "software-architect.md"
description = "turns a request into a detailed, buildable plan"
tools = ["tier0"]
delegable = true
budget = { max_iterations = 15, max_cost_usd = 1.0 }

[ai.personas.issue-analyst]
prompt = "issue-analyst.md"
description = "triages an issue and works out what it needs before anyone builds it"
tools = ["tier0", "tier1"]
delegable = true

[ai.personas.code-reviewer]
prompt = "code-reviewer.md"
description = "reviews a diff or pasted code against the project's standards"
tools = ["tier0"]
delegable = true

[ai.personas.test-reviewer]
prompt = "test-reviewer.md"
description = "reviews pasted tests as a specification, before anything is built against them"
tools = ["tier0"]
delegable = true

[ai.personas.architecture-reviewer]
prompt = "architecture-reviewer.md"
description = "critiques a plan against completeness, ordering, and instruction quality"
tools = ["tier0"]
delegable = true

[ai.personas.project-manager]
prompt = "project-manager.md"
description = "given a plan and what's done, decides what to work on next"
tools = ["tier0"]
delegable = true

[ai.personas.critic]
prompt = "critic.md"
description = "critiques a drawing, design, or screenshot: what works, what doesn't, what to try next"
delegable = true

[ai.personas.builder]
prompt = "builder.md"
description = "implements one subtask in a checked-out repository"
tools = ["tier0", "tier3"]
delegable = true
sandbox = "required"
```

`delegable` defaults to `false`, so a persona becomes reachable mid-conversation only when someone says
so. A delegated turn always inherits the invoking human's granted tier, so marking `builder` delegable
does not let a chat user reach `tier3` tools they were never authorized for.

Default prompts ship embedded via `include_str!` so nix builds and container deployments work with no
extra files, and `prompt_dir` overrides them for live iteration. The persona configuration itself -
not just the prompt text - ships embedded too (`PersonaRegistry::embedded_personas`), which is what
makes the minimal `default_model`-only config at the top of this section work at all.

## Tool risk tiers

Chat users are untrusted and sometimes adversarial. Tools are tiered, and a tool's authority derives
from the **invoking human**, never from the model's request. Every tier 2 and above tool re-checks
permissions from `ToolCtx` at invocation time.

| Tier | Tools                                                                      | Availability                             |
| ---- | -------------------------------------------------------------------------- | ---------------------------------------- |
| 0    | `current_time`, `todo_write`, `remember`, `forget`, `delegate`             | always                                   |
| 1    | `web_search`, `web_fetch`                                                  | per-persona allowlist                    |
| 2    | `get_user_profile`, `read_recent_messages`, `search_quotes`, `get_balance` | scoped to the invoking user              |
| 3    | `read`, `write`, `edit`, `bash`, `grep`, `glob`                            | coding personas, inside a container only |
| 4    | `create_pull_request`, `comment_on_issue`, `send_message`, `timeout_user`  | pipeline roles with explicit grants only |

Tier 4 is never reachable from public chat.

`delegate` sits at tier 0 because it grants no authority of its own: a delegated turn inherits the
invoking human's `granted_tier` unchanged, so it can never reach a tool the invoker could not have
reached directly. Delegating to a persona configured for tier 3 tools gets a turn that simply cannot
call them. This is the invariant milestone 3 phase 15's safety suite exists to enforce.

## Safety model

### Prompt injection

Both user messages **and tool results** are wrapped in delimiters and explicitly labeled as
untrusted data. Fetched web pages and GitHub issue bodies are the highest-risk injection vectors in
the entire system, because a research or pipeline persona reads attacker-authored text with tools
still attached.

The instruction hierarchy is fixed and stated in every system prompt: system instructions outrank
operator configuration, which outranks the invoking user, which outranks any content encountered
while working.

### Abuse and cost

Per-user and per-guild rate limits plus token and cost ceilings, enforced from the database _before_
the provider call. An open Discord bot running a thirty-iteration research loop is the fastest way to
lose money in this design, so budget enforcement lands in phase 4, well before public exposure.

### Output filtering

Responses pass through mention stripping, length caps, and `decancer` before they reach a platform.
The web surface applies its own rendering rules instead: markdown is rendered rather than escaped, so
the sanitisation boundary there is the markdown renderer, not `filter_output`.

### Duty of care

The companion persona needs more than a friendly prompt. **Milestone 2 phase 13** adds an explicit
crisis path: a classifier that recognises self-harm and acute distress, a prompt instruction that
forbids the model from handling it alone, and a reviewed, never-generated response that surfaces real
resources. This is a requirement, not a nice-to-have, and it moved forward out of the hardening
milestone precisely because a companion-first bot is one people will actually confide in — that has to
be handled before he is public, not after.

## Database schema

All new tables key users by the internal `users.id`, with a real foreign key.

```
ai_conversations   (id, platform, scope_key, persona_id, summary, summary_tokens,
                    owner_user_id NULL, title NULL, archived_at NULL,
                    created_at, last_active_at)
ai_messages        (id, conversation_id, seq, role, content JSON, token_count, created_at)
ai_memories        (id, user_id, key, value, created_at, updated_at)
ai_usage           (id, conversation_id, guild_id, user_id, provider, model,
                    input_tokens, output_tokens, cost_micros, created_at)
ai_tool_calls      (id, conversation_id, tool_name, input JSON, output JSON,
                    duration_ms, status, created_at)
ai_rate_limits     (scope, window_start, request_count, token_count)
ai_spend_caps      (scope, period, limit_micros, current_micros, reset_at)
ai_pipelines       (id, forge, repo, issue_number, state, branch, created_at, updated_at)
ai_pipeline_events (id, pipeline_id, seq, event JSON, created_at)
ai_user_settings   (user_id, memory_opt_in, created_at, updated_at)
```

`ai_conversations.owner_user_id`, `title`, and `archived_at` exist for the web surface: a web
conversation belongs to one person and needs a name in a sidebar, while a Discord channel's
conversation has neither. They are `NULL` for channel-scoped conversations, and `owner_user_id` is a
real foreign key with `ON DELETE CASCADE` so deleting a user takes their conversations with it.

`docs/notes/gui-configuration-research.md` documents a real trap here: `linked_accounts.user_id`
holds the internal `users.id`, while `guild_wallets.user_id` holds a raw Discord snowflake with no
foreign key. Every table above uses the internal identifier so that memory and usage records survive
a user linking a second platform account.

`ai_pipeline_events` is append-only. A pipeline's state is a fold over its events, which makes crash
recovery a replay rather than a repair.

## Testing strategy

- **`MockProvider` is the single most important test enabler in this plan.** It replays scripted
  responses, including tool calls, so the entire harness, delegation, and pipeline are testable with no
  network access. It ships in phase 2, before the loop that consumes it.
- Unit tests are colocated in `#[cfg(test)] mod tests` at the bottom of the implementation file, per
  `AGENTS.md`.
- Harness tests cover tool dispatch, parallel calls, malformed tool arguments and the retry path,
  every budget limit, cancellation mid-turn, and handoff schema validation.
- `Pipeline::advance` is a pure function, so the state machine gets table-driven tests.
- Store tests reuse the existing `TestDb` fixture in `munibot_core/tests/common/mod.rs`, which
  creates and drops a scratch database per test.
- Sandbox integration tests require podman and are gated behind a feature flag so `devenv test` stays
  green without it.
- No unit test touches the network. Ever.

## Risks

1. **Cost is the real operational risk.** A public bot with an agent loop can burn a budget in
   minutes. Budgets are enforced in phase 4, capped per user in phase 14, and hardened in phase 23; do not
   expose a research
   persona publicly before then.
2. **`rig-core` is pre-1.0** and releases frequently, and its API has already shifted enough that the
   published documentation site describes a removed API. The `Provider` trait boundary is what makes
   that survivable. Keep rig types out of every module except `ai::provider`, and confine the
   conversion code to one file within it.
3. **The toolchain is nightly**, because `munibot_discord` uses `#![feature(never_type)]`. rig and
   bollard are both verified to build on it; see `docs/notes/ai-preflight-findings.md`.
4. **Podman in production** means the NixOS module in `nix/nixos.nix` needs podman and socket
   configuration. It is not currently installed in the development environment either, so container
   behaviour is entirely unverified until milestone 4.
5. **Prompt quality is the product.** The `municode` prompts are genuinely good and port over nearly
   verbatim, but they carry known defects: a stray shell command spliced into a sentence in
   `architecture-reviewer.md`, a `StartTask` versus `StartTaskTests` naming drift in
   `project-manager.md`, a phantom `implementation_issues` field referenced in `code-reviewer.md`,
   and an `ApprovePlan` schema that requires a `strengths` field its own example omits. Fix these
   during the port in phases 16 and 19 rather than inheriting them.
6. **Autonomous pull requests need a human gate.** Nothing in milestone 5 merges anything. munibot
   opens a pull request and stops.

## Decisions still open

1. **Vector memory** — long-term memory starts as plain key-value facts, which works well and needs
   no embeddings. rig brings vector store support along for free if semantic recall becomes
   worthwhile later.
2. **Milestones 3 through 5 will change.** They are planned in full here because you asked for it,
   but expect to revise them once milestone 1 is in your hands and you have actually talked to him.
