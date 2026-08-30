# Operating munibot's AI features

This is written for you at 3am, not for a reader at leisure. If something is on fire, skip to
[when spend spikes](#what-to-do-when-spend-spikes) or [aborting a pipeline](#aborting-a-pipeline).
Otherwise, read top to bottom.

## Environment variables

Every AI-adjacent variable lives in `secretspec.toml`. Nothing below is required just to run
munibot at all -- `ai.enabled = false` (the default) boots without any of it.

### Required regardless of AI

| Variable                 | Purpose                                                 |
| ------------------------ | ------------------------------------------------------- |
| `DATABASE_URL`           | Database connection string                              |
| `REDIS_URL`              | Backs gui login sessions                                |
| `MUNIBOT_BASE_URL`       | Public base URL, used to build every oauth redirect URI |
| `DISCORD_APPLICATION_ID` | Discord app ID (also the oauth2 client ID)              |
| `DISCORD_CLIENT_SECRET`  | Discord client secret                                   |
| `DISCORD_PUBLIC_KEY`     | Discord public key                                      |
| `DISCORD_TOKEN`          | Discord bot token                                       |
| `TWITCH_CLIENT_ID`       | Twitch client ID                                        |
| `TWITCH_CLIENT_SECRET`   | Twitch client secret                                    |

### AI model providers (all optional)

Every key here is optional: munibot boots without any of them, and a persona that references an
unconfigured provider fails **at startup**, naming which variable is missing -- never silently at
the first real turn.

| Variable             | Enables                                                               |
| -------------------- | --------------------------------------------------------------------- |
| `ANTHROPIC_API_KEY`  | Personas on `anthropic:` models                                       |
| `OPENAI_API_KEY`     | Personas on `openai:` models, **and** provider moderation (see below) |
| `OPENROUTER_API_KEY` | Personas on `openrouter:` models                                      |
| `EXA_API_KEY`        | The `web_search` and `web_fetch` tools                                |

`ollama:` models need no key at all -- `ollama::Client::from_env()` defaults to an unauthenticated
local server.

### GitHub (two unrelated pairs -- do not confuse them)

| Variable                     | Authenticates                                                                                            |
| ---------------------------- | -------------------------------------------------------------------------------------------------------- |
| `GITHUB_APP_ID`              | The autonomous pipeline itself, against repositories it's installed into                                 |
| `GITHUB_APP_PRIVATE_KEY`     | Same App, for minting installation tokens                                                                |
| `GITHUB_WEBHOOK_SECRET`      | Verifying GitHub's own webhook deliveries                                                                |
| `GITHUB_BOT_LOGIN`           | Not a credential -- the App's own login, so munibot ignores its own comments. Defaults to `munibot[bot]` |
| `GITHUB_OAUTH_CLIENT_ID`     | A **separate** OAuth App, signing a human in ("sign in with github")                                     |
| `GITHUB_OAUTH_CLIENT_SECRET` | Same OAuth App                                                                                           |

`GITHUB_BOT_LOGIN` is read at `munibot_gui/src/server.rs:67` but is **not yet declared in
`secretspec.toml`** -- so `secretspec` will not prompt for it and will not warn about it. Set it in
the environment directly until that's fixed.

### Email sign-in (optional; unset `SMTP_HOST` disables it entirely)

| Variable            | Purpose                                                               |
| ------------------- | --------------------------------------------------------------------- |
| `SMTP_HOST`         | SMTP relay host -- unset means email sign-in never appears configured |
| `SMTP_PORT`         | Defaults to `587`                                                     |
| `SMTP_USERNAME`     | SMTP auth username                                                    |
| `SMTP_PASSWORD`     | SMTP auth password                                                    |
| `SMTP_FROM_ADDRESS` | Defaults to `no-reply@<SMTP_HOST>`                                    |

### Twitch (optional)

`TWITCH_TOKEN` and `VENTR_ALLOWLIST` are optional; the latter is a list of Discord user IDs allowed
to use ventriloquism, unrelated to AI.

## Adding a new AI provider

One match arm, in `munibot_ai/src/provider/rig/resolve.rs::build_provider`:

1. Confirm `rig-core` ships a client for it (`rig_core::providers::<name>`), implementing
   `CompletionClient` + `ProviderClient`.
2. Add a match arm following the existing shape:
   ```rust
   "<name>" => {
       let client = <name>::Client::from_env().map_err(|error| config_error(provider, error))?;
       Ok(Arc::new(RigProvider::new(provider, client.completion_model(model))))
   }
   ```
3. `from_env()` reads that provider's own conventional environment variable (check its own
   rig-core module for the exact name) -- add it to `secretspec.toml` as `required = false`.
4. Update the "supported providers are..." error message in the same function so it names the new
   one (a test asserts this message lists every supported provider).
5. Add a pricing entry to `munibot_ai/src/provider/pricing.toml` for every model you'll actually
   use, keyed as `"provider:model"` (the same string `ModelRef::to_string()` produces):
   ```toml
   ["newprovider:some-model"]
   input_per_mtok = 3.00
   output_per_mtok = 15.00
   ```
   **A model missing from this table silently costs `$0.00`** -- `estimate_cost` logs a warning
   and moves on rather than failing the turn. If a persona's spend looks suspiciously low, this is
   the first thing to check.

`ProviderResolver` itself needs no changes -- it dispatches to `build_provider` generically and
caches per `provider:model` string, never per-provider alone (a second model on the same provider
must never share a cached client with the first).

## Writing a persona

A persona is one `[ai.personas.<id>]` table. Nothing is required except `prompt`:

```toml
[ai.personas.my-persona]
model = "anthropic:claude-opus-5"   # falls back to ai.default_model if unset
prompt = "my-persona.md"             # required; resolved against ai.prompt_dir, else embedded
display_name = "My Persona"          # falls back to the id string
description = "what this persona is for, shown in pickers"
temperature = 0.7
tools = ["tier0", "web_search"]      # see RiskTier below; default is none
memory = "user"                      # "none" (default) | "conversation" | "user"
sandbox = "required"                 # "forbidden" (default) | "optional" | "required"
delegable = true                     # default false -- can other personas hand off to this one?
moderation_fail_closed = true        # see "provider moderation" below; default resolves from tools

[ai.personas.my-persona.budget]
max_iterations = 20
max_cost_usd = 1.0
max_wall_clock = "60s"
max_tool_retries = 3
```

An operator's own `[ai.personas.<id>]` **entirely replaces** an embedded default of the same id --
there is no field-by-field merge, so overriding one field of `companion` means repeating every
field you want to keep.

`tools` names either individual tool names, or a whole tier via its shorthand:

| Shorthand | `RiskTier`    | Meaning                                                                                     |
| --------- | ------------- | ------------------------------------------------------------------------------------------- |
| `tier0`   | `Safe`        | No side effects: the clock, a scratchpad, opt-in memory                                     |
| `tier1`   | `NetworkRead` | Read-only network: web search, fetching a URL                                               |
| `tier2`   | `BotData`     | munibot's own data, scoped to the invoking user                                             |
| `tier3`   | `Sandbox`     | Filesystem and shell, inside a container only                                               |
| `tier4`   | `Privileged`  | Real-world consequences (opening a PR, moderating a user); never reachable from public chat |
| `all`     | —             | Every registered tool, regardless of tier                                                   |

A prompt file is looked up in `ai.prompt_dir` first (if configured and the file actually exists
there), falling back to munibot's own embedded copy under `munibot_ai/prompts/`. Adding a brand
new (not overriding an existing) persona means both a prompt file **and** a line in
`embedded_prompt()` (`munibot_ai/src/persona/registry.rs`) if you want it embedded rather than
requiring `prompt_dir` in every deployment.

See `munibot_ai/src/persona/registry.rs::embedded_personas` for every shipped persona's own exact
configuration, as a set of working examples.

## Interpreting the dashboards

- **`/usage`** -- your own spend and turn count, plus (operators only) service-wide totals and a
  breakdown by persona, model, user, and the last 30 days. This is where you go first when
  [spend spikes](#what-to-do-when-spend-spikes).
- **`/transcript/:conversation_id`** -- one conversation's full history, messages and tool calls
  interleaved chronologically, tool calls collapsible with their input/output inspectable.
  Operator-gated for any conversation; anyone can read their own. This is the fastest way to
  understand why a persona behaved oddly, or to investigate anything a safety event flagged.
- **`/pipelines`** -- every autonomous pipeline run, live via SSE: state, current subtask, elapsed
  time, and an abort button. `/pipelines/:id` shows one run's full event log. **Nothing can start a
  run yet** -- see "the autonomous pipeline is not yet wired up" below. The page itself works; it
  just has nothing to show.
- **`/dashboard/:guild_id/ai`** -- per-guild: AI on/off, default persona, and a channel allowlist.
  Guild-admin gated, not operator-only -- a server owner manages their own server's settings here.
- **`/account`** -- a signed-in user's own linked sign-in providers, with unlink buttons. Point a
  confused user here rather than doing it for them.

## The autonomous pipeline is not yet wired up

Everything this document says about pipelines describes a subsystem that is **fully built, fully
tested, and has no production caller**. Concretely, as things stand:

- Nothing in any binary constructs an `Executor`, so **no pipeline can start**. `WebhookConfig` is
  hardcoded to `triggers: Vec::new()` and `dispatch: None` (`munibot_gui/src/server.rs:71-72`), so a
  webhook delivery is verified, normalized, and then dropped for want of a trigger to match.
- `PipelineRegistry::is_running` therefore always answers `false`, so `/pipelines` will list any rows
  that exist but never show a run as live, and the abort button never renders.
- `resume_all` exists but is **never called at boot**, so the resume behaviour described below does
  not happen yet.
- Nothing builds `munibot-sandbox:latest`. Until you run
  `podman build -t munibot-sandbox:latest -f Containerfile .` by hand, every sandboxed persona fails
  at container creation.

`docs/plans/ai/milestone-7-projects.md` is the plan that closes all of this. Read `/pipelines` and
the section below as documentation of intended behaviour, not of current behaviour.

## Aborting a pipeline

`abort_pipeline_action(pipeline_id)`, operator-gated, from the `/pipelines` list or detail page (a
single, unconfirmed button -- a run worth aborting is, by definition, one you already decided
needed to stop). It cancels the pipeline's own turn and stops its container, and reports back
whether it was actually running here at all (aborting one already finished, or resumed by a
different process, is a no-op, not an error).

If the button doesn't work (the process that owns it crashed, say), the pipeline is _intended_ to
resume automatically the next time munibot starts, every non-terminal pipeline being replayed from
its own event log on boot -- which would make restarting the process a valid, if blunt, way to regain
control of a pipeline whose owning process is itself unresponsive. **`resume_all` is not called at
startup yet**, so today a restart leaves a non-terminal pipeline sitting in its last persisted state
instead. See the section above.

## Safety systems

Every system below writes a trip to `ai_safety_events` (except abuse detection, which has its own
richer table) -- `event_type`, `scope_type`/`scope_id`, a short stable `reason` string, and
(only where there's real content to hash) a `content_hash`. **Never raw message content.** Query
this table first for anything below; it's indexed on `(event_type, created_at)`.

### Abuse detection

Screens **behaviour**, per signed-in user only (never guild or global): repeated near-identical
prompts, a known prompt-injection phrasing, or rapid persona switching. One trip imposes an
escalating cooldown (doubling each strike, capped, forgiven after a day of clean behaviour) in its
own table, `ai_abuse_cooldowns` (`scope_type`, `scope_id`, `strike_count`, `cooldown_until`,
`last_reason`). Every trip is logged via `tracing::warn!` before anything else, specifically so a
false positive is discoverable.

Tune via `[ai.abuse]` (all optional, humantime durations):

```toml
[ai.abuse]
cooldown_base = "1m"           # default 60s
cooldown_max = "1h"            # default 1h
cooldown_reset_after = "24h"   # default 1 day
duplicate_threshold = 3        # repeats before it trips; default 3
duplicate_window = "2m"        # default 120s
persona_switch_threshold = 4   # distinct personas before it trips; default 4
persona_switch_window = "1m"   # default 60s
```

**Investigating a false positive:** read `ai_abuse_cooldowns` for the scope (its `last_reason` is
one of `"repeated near-identical prompts"`, `"a known prompt-injection phrasing"`, or `"rapid
persona switching"`), then grep logs around `cooldown_until - cooldown_secs` for the matching
`tracing::warn!`. A store failure fails open (allows the turn, logs a warning) -- a database hiccup
never itself blocks someone.

### Provider moderation

Runs inbound/outbound content through the model provider's own moderation endpoint. **Only OpenAI
ships one today** (`omni-moderation-latest`), so this needs `OPENAI_API_KEY` -- unset, moderation
is simply off, not a startup failure. Flagged content **always** refuses the turn, regardless of
policy; the policy only governs what happens when the _check itself_ fails to run:

- **fail-open** (default for most personas) -- lets the turn through, logs a warning. Right choice
  for casual chat: an outage should never silence munibot entirely.
- **fail-closed** (default once a persona's tools reach `RiskTier::Privileged`) -- refuses the
  turn outright. Right choice for anything with real-world consequences.

Override per persona with `moderation_fail_closed = true`/`false`; leave it unset to use the
tier-based default.

**Investigating:** `ai_safety_events WHERE event_type = 'moderation'` -- `reason` holds either the
flagged category list (e.g. `"violence, harassment"`) or the check-failure error text.
`content_hash` lets you confirm a suspected message matches without ever storing it. A spike in
check-failure rows (not flagged-content rows) means check `OPENAI_API_KEY`'s validity and quota.

### Rate limits

Per-scope (user, guild, global) request/token/concurrency limits, checked before every provider
call. Concurrency is checked purely in memory; request/token counts live in `ai_rate_limits`
(current window only, not history).

```toml
[ai.rate_limits.user]
max_requests = 20
window = "1m"
max_tokens = 50000
max_concurrent_turns = 2

[ai.rate_limits.global]
max_requests = 200
window = "1m"
```

A scope not mentioned stays unlimited. What a refused user sees: _"you're sending messages a
little too fast"_, _"that's used up the token budget for now"_, or _"munibot's already working on
a lot with you at once"_, each naming a retry time where relevant.

**Investigating:** `ai_safety_events WHERE event_type = 'rate_limit'` for trip history;
`ai_rate_limits` for a scope's live counter if you need to see what it's at _right now_.

### Spend caps

Per-user and global only -- **no guild scope** (a guild's own members are already covered
individually by their own user caps). Refuses new turns once a scope's period spend reaches its
limit; anything already running finishes normally. Warns at 80% of cap, before it ever refuses
anything -- the earliest signal you'll see.

```toml
[ai.spend_caps.user]
max_usd = 5.0
period = "monthly"

[ai.spend_caps.global]
max_usd = 500.0
period = "monthly"
```

What a refused user sees: \*"that's hit the spend cap for now :< it resets `<timestamp>`"`.

**State:** `ai_spend_caps` (`scope_type`, `scope_id`, `period`, `limit_micros`, `current_micros`,
`reset_at`). Trips also land in `ai_safety_events` as `event_type = 'spend_cap'`.

### Crisis classifier

Screens every inbound message, but **only** for a persona with `memory = "user"` (today, that's
just `companion`). A cheap, one-shot classification into `none` / `low` / `elevated` / `severe`.
`elevated` or above bypasses the normal turn for a **reviewed, never-generated** response listing
whatever real crisis resources are configured -- never anything the model itself writes.

Configure real contact information (there is no sensible default):

```toml
[[ai.crisis_resources]]
name = "a crisis line"
contact = "call or text 988"
```

There's no separate model knob -- it reuses whatever model the default persona resolves to. A
classifier failure (provider error, unparsable output) always falls back to `none`, never
escalates on plumbing failure.

**Investigating:** `ai_safety_events WHERE event_type = 'crisis'` -- `reason` names the severity,
`content_hash` lets you confirm which message without storing it. The crisis-bypassed reply is
stored in the conversation like any other, so the transcript viewer shows full context.

### Safety event auditing

The shared `ai_safety_events` table itself, described throughout this section. No config knob --
wired unconditionally whenever `ai.enabled`. Deliberately excludes content by design: _"enough to
tune the systems, not enough to become a surveillance log."_ An audit write failing can never
affect whether the safety check it's recording actually did its job.

## What to do when spend spikes

1. Open `/usage` as an operator. The **breakdown** section is the whole point of this page.
2. Check **by user** first. One outlier user is the most common cause, and the easiest to act on --
   if you find one, check `ai_abuse_cooldowns` for them and consider tightening `[ai.abuse]`
   thresholds, or lowering their effective spend cap.
3. Check **by persona** and **by model**. A specific persona or model costing far more than
   expected usually means either a genuinely popular feature, or a runaway loop hammering one
   model repeatedly. Cross-reference `munibot_ai/src/provider/pricing.toml` -- an _unpriced_ model
   reads as **zero** cost, which can mask where a spike is actually coming from just as easily as
   cause one.
4. Check **last 30 days** to bound _when_ it started -- a step change points at one change
   (a new persona, a config edit, an incident); a gradual climb points at organic growth.
5. Pull the affected users'/personas' conversation transcripts (`/transcript/:conversation_id`) to
   see the actual turns.
6. If it's one conversation looping, check that persona's own budget
   (`max_iterations`/`max_cost_usd`/`max_wall_clock`/`max_tool_retries`) is tight enough. **A spend
   cap is the last line of defense, not the first** -- a well-configured per-turn budget should
   catch a runaway loop long before the cap ever needs to.

## Further reading

- `docs/plans/ai/milestone-6-hardening.md` -- the plan this document itself was written to satisfy
  (commit 199), and the plan for everything else in this milestone.
- `docs/plans/ai/milestone-7-projects.md` -- projects, workspaces, and the wiring that makes the
  pipeline sections above describe reality. Its opening table is the current, evidenced list of
  everything in the AI stack that is built but not connected.
- `docs/plans/ai/overview.md` -- the full architecture, including the "Risks" section this
  document exists to make actionable.
- `docs/tracing.md` -- how to actually watch any of this happen, via `RUST_LOG`.
