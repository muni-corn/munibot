# munibot AI — overview

munibot is growing an AI agent harness: provider-agnostic, tool-using, and multi-persona. It's meant
to serve casual conversation, emotional support, creative writing, research, and real software
development.

munibot is **a companion first**, and supremely useful second. He earns the title of the universe's
most lovable bot by being someone worth talking to, and keeps it by also being genuinely good at
programming and research. Where those two goals compete, the companion wins.

His **primary surface is munibot's own web chat page**. That's where a conversation list, streamed
replies, rendered code, visible tool work, and an auditable memory panel can all exist at once.
Discord comes last, as a plain chat surface onto the same munibot. Twitch is out.

This plan supersedes the `municode` project. Its provider-agnostic client, tools, agent loop,
sandbox, and issue-to-pull-request pipeline are all absorbed here. Its twelve agent prompts become
personas munibot can delegate to from conversation, long before the pipeline automates them.

---

## Where things stand

The AI was once built out through what was then milestone 7, then deliberately **stripped back** to a
temporary chat (`9b4a38e1`), so features can be rebuilt and tested one piece at a time. These plans
build up from that baseline, not from the old code.

What exists today:

| Area             | What's there                                                                                                                                                                                                                                                     |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `munibot_ai`     | `types` (text and thinking content only, with no tool or image blocks), `provider` (`Provider`, `MockProvider`, `RigProvider`, `ProviderResolver`), `config` (`AiConfig { enabled, model }`), and `conversation` (`Chat::reply`: one provider call, text stream) |
| `munibot_api`    | `chat_stream` (`POST /api/ai/chat/stream`, server-sent events); `ChatEvent` is `TextDelta`, `Finished`, or `Failed`; `validate_conversation` caps the history the browser sends                                                                                  |
| `munibot_gui`    | `/chat`: history kept in page state, markdown with highlight.js code blocks and copy buttons, a composer, retry                                                                                                                                                  |
| `munibot` binary | builds `Arc<Chat>` outside the `MUNIBOT_DISABLE_BOTS` guard and hands it to the GUI server as an `Extension`                                                                                                                                                     |
| Sign-in          | Discord, GitHub, and email, with account linking and unlinking and OAuth `state` checks                                                                                                                                                                          |
| Infrastructure   | CI (`.github/workflows/build-and-test.yml`: build, clippy, treefmt, `cargo test`); `devenv test` runs `cargo nextest run`                                                                                                                                        |

What doesn't exist: tools, the agent loop, persistence of any AI data, limits, personas, memory,
delegation, a sandbox, projects, the pipeline, permissions, and any Discord AI. `docs/chat.md`
describes the current chat accurately.

**The strip also reverted unrelated main-branch work**: the guild-config upsert, Discord token
refresh, the guild cache, the test fixture, and more. [`docs/plans/loose-ends.md`](../loose-ends.md)
records it and has a recipe to restore it. Do at least its commit 1.6 before milestone 03.

## Milestone map

| #                                     | Milestone              | You can try                                                                 | Plan depth  | Status      |
| ------------------------------------- | ---------------------- | --------------------------------------------------------------------------- | ----------- | ----------- |
| [01](milestone-01-agent-loop.md)      | the agent loop         | munibot uses tools in `/chat`, visibly, within a budget, with a stop button | full recipe | not started |
| [02](milestone-02-research.md)        | web research           | he searches and reads the web, citing sources                               | full recipe | not started |
| [03](milestone-03-conversations.md)   | saved conversations    | a conversation list that survives restarts, with titles and compaction      | full recipe | not started |
| [04](milestone-04-affordability.md)   | affordability          | per-user and global limits, `/usage`, operators, a pause switch             | full recipe | not started |
| [05](milestone-05-personas.md)        | personas               | `companion.md`, a persona picker, a catalogue                               | outline     | not started |
| [06](milestone-06-memory-and-care.md) | memory and care        | opt-in memory with a panel; a reviewed crisis path                          | outline     | not started |
| [07](milestone-07-specialists.md)     | specialists and senses | delegation to an engineering team; images                                   | outline     | not started |
| [08](milestone-08-sandbox.md)         | sandbox                | developers get munibot running code in a rootless container                 | outline     | not started |
| [09](milestone-09-projects.md)        | projects               | conversations bound to real repositories, with project-built images         | outline     | not started |
| [10](milestone-10-autonomous.md)      | autonomous development | a request, from chat or GitHub, becomes a pull request                      | outline     | not started |
| [11](milestone-11-hardening.md)       | hardening              | abuse handling, moderation, tracing, transcripts, the full runbook          | outline     | not started |
| [12](milestone-12-discord.md)         | munibot in Discord     | chat with him in a server or DM; developers can start work from there       | outline     | not started |

Each milestone ends with something you can actually use. Each builds only on the ones before it. Keep
the status column current as work lands.

## Before going public

The chat is signed-in only, but anyone can sign in. Don't advertise it, or open it to people you
don't know, until all of these exist:

1. **Spend limits.** Milestone 04.
2. **The care path.** Milestone 06. A companion people confide in has to handle crisis before
   strangers can reach him, not after.
3. **The prompt injection suite.** Milestone 11 commit 11.1, pulled forward and written against the
   tools that exist at that point.
4. **Email sign-in throttling.** Loose ends commit 2.7. Otherwise the sign-in form is a free mail
   relay.

---

## How these plans are written

Plans are **commit recipes**: a sequence of atomic commits, each leaving the workspace compiling and
its tests passing.

- **Full recipes** give every commit an implementation summary and its entire commit message: the
  subject, plus a body explaining _why_.
- **Outlines** give the design and a provisional commit list with one-line summaries. Before starting
  an outlined milestone, expand it into a full recipe and re-check every claim against the code. The
  milestone before it will have changed things.
- **Numbering is per milestone** (`3.14` is milestone 03's fourteenth commit). The old global numbering
  drifted with every unplanned commit.
- **Order is dependency order**:
  1. types, structures, and interfaces;
  2. configuration;
  3. behaviour;
  4. surfaces.
- **No code comment ever names a milestone, phase, or commit number.** "This arrives in milestone 6" in
  code is wrong the day plans change. Comments explain why the code is the way it is now.
- **Dependencies are added with `cargo add`** in the commit that first uses them, never by editing a
  manifest by hand.
- **Commit scopes follow the repository's rules.** A single file takes its base name (`event` for
  `chat/event.rs`). Several files in one directory take that directory (`harness`, `tools`, `memory`).
  Several directories within one crate take the crate's short name (`ai`, `api`, `gui`, `core`,
  `discord`). Changes spanning crates omit the scope.
- **When reality disagrees with a plan, the plan changes.** Record the deviation in the milestone file,
  briefly, as part of the work.

## Guiding decisions

| Decision              | Choice                                                                        | Rationale                                                                                                                                                                                                           |
| --------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Provider abstraction  | Our own `Provider` trait over `rig-core`                                      | rig covers many providers, but its `CompletionModel` isn't object-safe, so our trait is what makes runtime provider choice possible, and it absorbs rig's pre-1.0 churn. See `docs/notes/rig-provider-findings.md`. |
| Agent loop            | Hand-rolled, streaming-first                                                  | We need budgets, cancellation, tool events, and handoffs. rig's `Agent` gives none of that control.                                                                                                                 |
| Service handle        | `Chat`, grown in place                                                        | It's already what the API and GUI hold. Rename only if it genuinely outgrows the name.                                                                                                                              |
| Unit of configuration | The persona                                                                   | A model, a prompt, tools, and a budget, with fields added by the milestones that need them. Chat personas and pipeline roles are the same type.                                                                     |
| Structure             | One `munibot_ai` crate with modules, plus three small crates                  | Module privacy gives the boundaries a crate split would. The tool agent and forge crates stay separate for reasons that have nothing to do with being "AI".                                                         |
| Persistence           | MySQL through `diesel-async`                                                  | Matches the rest of munibot. Pipelines use an append-only event log.                                                                                                                                                |
| Limits                | Computed from `ai_usage`, plus in-memory reservations                         | One source of truth for spend, and running turns reserve their worst case so concurrent turns can't overshoot.                                                                                                      |
| Permissions           | `user_permissions` with a recorded source                                     | Config sync can revoke exactly what config granted, without touching manual grants.                                                                                                                                 |
| Search                | Exa                                                                           | Neural search and extraction in one API built for models.                                                                                                                                                           |
| Sandbox               | Rootless podman through `bollard`; a static tool agent mounted into any image | Strong isolation, and one mechanism for every project image.                                                                                                                                                        |
| Forge integration     | A GitHub App                                                                  | Per-repository installation, scoped permissions, better rate limits, and a real bot identity.                                                                                                                       |
| Persona selection     | A visible picker per conversation, never automatic routing                    | An invisible personality switch is wrong for a companion.                                                                                                                                                           |
| Specialist access     | Delegation through a `delegate` tool                                          | munibot stays the conversational partner and reports back in his own voice, reusing the tool system's gating, accounting, and visibility.                                                                           |
| Starting work         | A `start_work` tool used on munibot's own judgement, developers only          | The same path from web chat and Discord. Every run gets a public issue, a cost ceiling, and a human gate before merge.                                                                                              |
| Memory                | Opt-in, per user, fully auditable                                             | `remember` and `forget`, plus a panel to see and wipe everything.                                                                                                                                                   |
| Discord               | A plain chat surface, no slash commands, last                                 | The same munibot, reachable where people already are, once everything he can do exists.                                                                                                                             |

## Crate architecture

The target, with when each part arrives:

```
munibot_ai
  types         provider-neutral domain types               exists; tool blocks 01, images 07
  provider      Provider, mock, rig adapter, retry, pricing exists; retry and pricing 01
  tools         Tool, registry, context, tiers, built-ins   01, web tools 02
  harness       the agent loop, budgets, events, handoffs   01, handoffs 10
  memory        conversations, context, compaction, memory  03, user memory 06
  accounting    usage and tool-call recording               03
  limits        admission control                           04
  persona       templates, personas, registry               05
  sandbox       lifecycle, sessions, sandboxed tools        08
  project       projects, worktrees, images                 09
  pipeline      the autonomous state machine and executor   10
  config.rs     the [ai] section                            exists
  conversation  Chat, the service handle                    exists

munibot_toolagent   in-container rpc server, no munibot dependency   08
munibot_vcs         forge-agnostic traits and events                 10
munibot_github      GitHub App implementation                        10
```

```
munibot_ai
  types ──> provider ──> rig-core
  types ──> tools ──> harness ──> provider
  harness ──> memory, accounting, limits ──> munibot_core
  persona ──> all of the above
  sandbox ──> bollard        project ──> sandbox
  pipeline ──> project, munibot_vcs

munibot_github ──> munibot_vcs, octocrab
munibot_toolagent              (nothing from munibot, ever)
```

No `rig` type leaks past `provider`. `munibot_vcs` and `munibot_github` know nothing about AI.
`munibot_toolagent` defines its own wire types rather than depend on anything that would drag
`munibot_ai`'s dependency tree into an untrusted container.

Platform adapters live in the crates that own their platforms: `munibot_api/src/chat*` and
`munibot_gui/src/{pages,components}/chat*` for the web, and `munibot_discord` for Discord in
milestone 12.

## Tool risk tiers

A tool's authority comes from the **invoking human**, never from the model. Every risky tool
re-checks `ToolCtx` when it's called.

| Tier | Name          | Tools                                                  | Arrives    |
| ---- | ------------- | ------------------------------------------------------ | ---------- |
| 0    | `Safe`        | `current_time`; `remember`, `forget`; `delegate`       | 01, 06, 07 |
| 1    | `NetworkRead` | `web_search`, `web_fetch`                              | 02         |
| 2    | `BotData`     | reserved; no tools planned                             | —          |
| 3    | `Sandbox`     | `read`, `write`, `edit`, `bash`, `grep`, `glob`        | 08         |
| 4    | `Privileged`  | `manage_project`; `start_work`, `answer_work_question` | 09, 10     |

| Caller                               | Granted tier  |
| ------------------------------------ | ------------- |
| unlinked Discord member              | `Safe`        |
| signed in                            | `NetworkRead` |
| `developer` or `operator` permission | `Privileged`  |

A persona's tool selection can narrow this further, but never widen it. A delegated turn inherits the
human's tier unchanged. Forge operations like pushing a branch or opening a pull request are performed
by the pipeline's executor and are never offered to a model as tools.

## Safety model

**Prompt injection.** Search results, fetched pages, memories, issue bodies, and sandbox output all
pass through `wrap_untrusted` before a model sees them. Every system prompt states the same
instruction hierarchy: munibot's own instructions, then operator configuration, then the user, then
anything encountered while working. Specialists get a written brief, never the raw transcript.

**Cost.** Every turn has a budget, and every turn is admitted against per-user and global limits
before any provider call, with its worst case reserved. Pipeline runs have their own ceiling. A paused
switch stops everything and survives a restart.

**Output.** The web renders markdown, so its sanitisation boundary is the renderer, which drops raw
HTML. Discord output passes through `filter_output`. Tool previews are always shown as plain text.

**Duty of care.** Every chat message is classified. An acute message gets a reviewed, never-generated
response with real resources, and the model is never asked to improvise it.

**No automatic merges.** munibot opens pull requests. A human decides.

## Database schema

Every table keys people by the internal `users.id`, with a real foreign key. Never by a raw platform
id, which is the trap `docs/notes/gui-configuration-research.md` documents. The one exception is
`ai_usage.discord_user_id`, for Discord members who never linked an account.

| Table                                    | Added | Later changes                                                       |
| ---------------------------------------- | ----- | ------------------------------------------------------------------- |
| `ai_conversations`                       | 03    | `persona_id` 05; `project_id` 09; `platform`, `scope_key` 12        |
| `ai_messages`                            | 03    |                                                                     |
| `ai_usage`                               | 03    | `persona_id` 05; `pipeline_id` 10; `guild_id`, `discord_user_id` 12 |
| `ai_tool_calls`                          | 03    | `pipeline_id` 10                                                    |
| `user_permissions`                       | 04    |                                                                     |
| `ai_runtime_settings`                    | 04    |                                                                     |
| `ai_memories`, `ai_user_settings`        | 06    |                                                                     |
| `ai_attachments`                         | 07    |                                                                     |
| `ai_projects`, `ai_workspaces`           | 09    | triggers 10                                                         |
| `ai_pipelines`, `ai_pipeline_events`     | 10    |                                                                     |
| `ai_safety_events`                       | 11    |                                                                     |
| `guild_ai_settings`, `guild_ai_channels` | 12    |                                                                     |

Per-user content cascades with its user. Spend history (`ai_usage`) uses `ON DELETE SET NULL`, so
deleting people or conversations never falsifies global spend. `linked_accounts` still lacks a
cascade, which loose ends commit 2.1 fixes.

## Testing strategy

- **`MockProvider` is the most important test enabler in the plan.** It scripts text, tool calls, and
  errors, so the harness, delegation, and pipeline are all testable offline.
- **No unit test touches the network.** Live checks, like milestone 02's Exa check, are `#[ignore]`d
  and run by hand.
- **Unit tests are colocated** in `#[cfg(test)] mod tests` at the bottom of the file, per `AGENTS.md`.
- **Database tests use `TestDb`**, shared across crates through `munibot_core`'s `test-util` feature
  from milestone 03.
- **Sandbox and image tests are gated** behind `sandbox-integration`, run by `devenv shell
test-sandbox`, and excluded from CI, which has no rootless podman.
- **Pure functions get table tests**: admission, branch naming, strategy detection, the pipeline's
  `advance`, `is_public`, and `filter_output`.
- `devenv test` runs `cargo nextest run`, and CI runs `cargo test`. Both must pass.

## Risks

1. **Cost is the real operational risk.** A tool loop can spend a budget in minutes. That's why limits
   come in milestone 04, before personas, specialists, or anything else multiplies spend.
2. **`rig-core` is pre-1.0.** Its documentation already describes removed APIs. The `Provider` boundary
   and `convert.rs` are where its churn is absorbed.
3. **Prompt quality is the product.** `companion.md` deserves real time, and never stops being worked
   on.
4. **The sandbox runs hostile code.** Milestone 08's posture is designed to be checked, not trusted:
   the full-chain integration test is part of the milestone, not a follow-up.
5. **Model judgement starts real work.** `start_work` is gated, costed, ceilinged, and public by
   construction, so a wrong call is cheap and visible.
6. **The strip's collateral damage.** Until the loose-ends restoration lands, some notes describe code
   that doesn't compile in, and the local test fixture is broken.

## Decisions still open

1. **Vector memory.** Key-value facts first. rig's vector stores are there if semantic recall ever
   earns its keep.
2. **A network allowlist for the sandbox.** It needs a real filtering proxy to mean anything.
3. **Warm container pools.** Wait for a real complaint about startup time.
4. **Whether the base sandbox image survives** once real projects bring their own images.
5. **Starting brand-new projects.** `start_work` changes existing repositories. Creating a repository
   from a conversation is a bigger trust decision.
6. **Bot-data tools.** Tier 2 is reserved. Reading someone's balance or quotes on request is a
   privacy choice to make on purpose.
7. **Extended thinking with tools.** Needs thinking-block signatures carried through history first.
8. **Plans after milestone 04 will change.** They're outlines on purpose. Expand each one when its
   turn comes, against the code as it is then.
