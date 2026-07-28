# Milestone 5 — hardening

**Outcome:** munibot's AI features are safe to leave running in public servers, on a budget you
choose, with enough observability to diagnose anything that goes wrong.

Nothing in this milestone adds a capability. Everything in it protects the capabilities already built,
protects the people using them, and protects your wallet.

**Phase 18, commits 152 through 164.**

## On sequencing

This milestone is listed last but should not be done last. Three items are prerequisites for public
exposure and should be pulled forward the moment the feature they guard exists:

| Item                                   | Pull forward to                                        |
| -------------------------------------- | ------------------------------------------------------ |
| Rate limiting and spend caps (152–155) | before any public guild is enabled                     |
| The crisis response path (158–159)     | before the companion persona reaches anyone but you    |
| Injection resistance testing (161)     | before the research persona gets `web_fetch` in public |

The rest is genuinely finishing work.

---

## Phase 18 — safety, cost control, and observability

| #   | Commit                                                        | Description                                                                                                                                                                                                                                                                                                                        |
| --- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 152 | `feat(db): add ai rate limit and spend cap tables`            | Migration for `ai_rate_limits` (scope, window start, request count, token count) and `ai_spend_caps` (scope, period, limit micros, current micros, reset at). Scope is a discriminated key covering user, guild, repository, and global, so one mechanism serves every level.                                                      |
| 153 | `feat(ai): add rate limiter with sliding window`              | A sliding-window limiter over the database with a small in-memory cache, checked **before** the provider call. Separate limits for requests, tokens, and concurrent turns per scope. Exceeding one returns a friendly lowercase refusal naming when the window resets, per the error style in `AGENTS.md`.                         |
| 154 | `feat(ai): add spend cap enforcement with kill switch`        | Track spend per user, guild, and globally against configured caps. At 80 percent, warn in the log and the dashboard. At 100 percent, refuse new turns for that scope while letting in-flight ones finish. A global cap that aborts is the last line of defence against a runaway loop, so it is checked first.                     |
| 155 | `feat(gui): add spend cap configuration and alerts`           | Configure caps per guild and globally, and show current usage against each. Surface a prominent warning at 80 percent. The dashboard from phase 13 shows what was spent; this decides what may be spent.                                                                                                                           |
| 156 | `feat(ai): add abuse detection and cooldowns`                 | Detect repeated near-identical prompts, prompt-injection signatures, and rapid persona switching to farm free tokens. Escalating cooldowns per user, recorded for review. Log every trip rather than silently dropping, so false positives are discoverable.                                                                       |
| 157 | `feat(ai): add provider moderation pre and post check`        | Run inbound prompts and outbound responses through the provider's moderation endpoint where one exists, with the persona choosing the policy. A moderation failure is fail-closed for tier 4 personas and fail-open with a warning for chat, since a moderation outage should not silence munibot entirely.                        |
| 158 | `feat(ai): add crisis detection classifier`                   | A dedicated small-model classifier for self-harm, suicidal ideation, abuse disclosure, and acute distress, run on inbound messages for personas with `MemoryPolicy::User`. Returns a severity, not a boolean. Tuned to over-trigger rather than under-trigger, because the asymmetry of harm here is enormous.                     |
| 159 | `feat(ai): add crisis response path with resources`           | On a positive crisis signal, bypass the normal turn and respond from a reviewed, non-generated template: acknowledge, do not diagnose, do not counsel, and surface real region-appropriate crisis resources. Configurable resource list. **Write this response with care, and never let a model improvise it.**                    |
| 160 | `feat(ai): add safety event auditing`                         | Record every rate-limit trip, spend refusal, moderation block, and crisis trigger in an `ai_safety_events` table, with content excluded and only a hash retained. Enough to tune the systems, not enough to become a surveillance log.                                                                                             |
| 161 | `test(ai): add prompt injection resistance suite`             | A corpus of injection payloads delivered through every untrusted channel: user messages, web page content, GitHub issue bodies, and tool output. Assert that no payload causes a tier escalation, a tool call outside the persona's allowlist, or a system-prompt disclosure. Runs against `MockProvider`, so it is fast and free. |
| 162 | `test(ai): add budget and cancellation stress tests`          | Assert that every budget limit terminates a loop, that cancellation never leaks a running tool or an orphaned container, and that a provider hanging indefinitely is bounded by the wall-clock limit. The failure modes that only appear in production, tested deterministically.                                                  |
| 163 | `feat(ai): add tracing spans across the harness and pipeline` | Instrument every crate with `#[instrument(skip_all, fields(...))]` carrying persona, model, iteration, conversation, and pipeline identifiers, using `.instrument(span)` across every `tokio::spawn` rather than `.entered()`. Update the span table in `docs/tracing.md` in the same commit, as that document requires.           |
| 164 | `docs(ai): add operator runbook`                              | `docs/ai-operations.md`: required and optional environment variables, how to add a provider, how to write a persona, how to interpret the dashboards, how to abort a pipeline, what each safety system does and how to tune it, and what to do when spend spikes. Written for you at 3am, not for a reader at leisure.             |

---

## Definition of done

- A user hammering munibot is rate limited with a clear, kind message.
- Hitting a guild spend cap refuses new turns without breaking in-flight ones.
- The global kill switch demonstrably stops all spending.
- Every injection payload in the corpus fails to escalate privileges or leak the system prompt.
- A simulated crisis message produces the reviewed template response, never a generated one.
- Every request is traceable end to end from a single identifier.
- The runbook is good enough that someone who is not you can operate this.

## Ongoing work beyond this plan

- **Prompt iteration.** Personas are never finished. Expect to keep tuning `companion.md` for as long
  as munibot exists, and treat prompt changes as product changes worth reviewing.
- **Model updates.** New models arrive constantly. Keeping the pricing table and persona model
  references current is routine maintenance, made cheap by the fact that both are configuration.
- **Additional forges.** `munibot_vcs` exists so that adding Forgejo or GitLab is one crate
  implementing two traits, with no change to the pipeline.
- **Cost optimisation.** Prompt caching, cheaper models for cheap roles, and smarter compaction are
  the obvious levers once you have real usage data to aim them with.
