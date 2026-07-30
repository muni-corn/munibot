# Milestone 6 — hardening

**Outcome:** munibot's AI features are safe to leave running in public, on a budget you choose, with
enough observability to diagnose anything that goes wrong — and no longer reachable only by people who
happen to have a Discord account.

Almost nothing in this milestone adds a capability. Most of it protects the capabilities already built,
protects the people using them, and protects your wallet. The exception is sign-in: the web companion
ships Discord-OAuth-gated in milestone 2, and this is where that stops being true.

**Phase 23, commits 188 through 204.**

## What moved out of this milestone

Two groups were pulled forward into milestone 2, because the web chat surface makes them urgent rather
than eventual:

| Moved                           | To                   | Why                                                                                                                                          |
| ------------------------------- | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Rate limiting and spend caps    | milestone 2 phase 14 | A public web chat has no invite gate and no channel gate. One signed-in stranger can open unlimited conversations, so the caps ship with it. |
| Crisis recognition and response | milestone 2 phase 13 | A companion-first bot is one people will actually confide in. This has to exist before he is public, not in a later hardening pass.          |

## On sequencing

What remains is listed last but should not all be done last. One item is a prerequisite for public
exposure and should be pulled forward the moment the feature it guards exists:

| Item                         | Pull forward to                                        |
| ---------------------------- | ------------------------------------------------------ |
| Injection resistance testing | before the research persona gets `web_fetch` in public |

The rest is genuinely finishing work.

---

## Phase 23 — safety, sign-in, administration, and observability

| #   | Commit                                                        | Description                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 188 | `feat(ai): add abuse detection and cooldowns`                 | Detect repeated near-identical prompts, prompt-injection signatures, and rapid persona switching to farm free tokens. Escalating cooldowns per user, recorded for review. Log every trip rather than silently dropping, so false positives are discoverable.                                                                                                                                                                                          |
| 189 | `feat(ai): add provider moderation pre and post check`        | Run inbound prompts and outbound responses through the provider's moderation endpoint where one exists, with the persona choosing the policy. A moderation failure is fail-closed for tier 4 personas and fail-open with a warning for chat, since a moderation outage should not silence munibot entirely.                                                                                                                                           |
| 190 | `feat(ai): add safety event auditing`                         | Record every rate-limit trip, spend refusal, moderation block, and crisis trigger in an `ai_safety_events` table, with content excluded and only a hash retained. Enough to tune the systems, not enough to become a surveillance log.                                                                                                                                                                                                                |
| 191 | `feat(auth): add additional sign in providers`                | Break the Discord-OAuth-only gate the web companion ships with. GitHub and email sign-in against the same internal `users.id`, reusing the `linked_accounts` `(provider, provider_user_id)` unique index rather than inventing a second identity model. A companion nobody can reach without a Discord account is a companion with an arbitrary door on it.                                                                                           |
| 192 | `feat(auth): add account linking and unlinking`               | Let one person attach several providers to a single `users.id`, and detach one safely, refusing to remove the last remaining sign-in method. This is what makes the memory promise survive a change of platform — the reason `users.id` was the key all along.                                                                                                                                                                                        |
| 193 | `feat(auth): add oauth state parameter and csrf protection`   | Close the documented gap at `docs/gui.md:132`: the Discord flow has no `state` parameter. Add one, verified against the session, for every provider. Should not reach public exposure without it.                                                                                                                                                                                                                                                     |
| 194 | `feat(auth): replace the has permission stub`                 | `HasPermission::has` at `munibot_api/src/auth/server.rs:64-69` currently always returns `false`, noted as a gap in `docs/notes/gui-configuration-research.md:55-65`. Replace it with a real operator flag so the administrative pages below can be gated on something.                                                                                                                                                                                |
| 195 | `feat(db): add ai guild settings columns`                     | Add `ai_enabled`, `ai_default_persona`, and `ai_channel_mode` to `guild_configs`, plus an `ai_channel_allowlist` table, so milestone 1's Discord surface can be enabled per guild instead of globally. **Use the whole-row upsert at `operations.rs:33`, never `REPLACE INTO`**, which deletes and reinserts the row and would silently null out `logging_channel` on every AI save — documented at `docs/notes/gui-configuration-research.md:31-53`. |
| 196 | `feat(api): add guild ai settings server functions`           | `get_guild_ai_settings` and `set_guild_ai_settings`, guild-admin gated via `munibot_api/src/auth/guild.rs:20`, following the logging settings slice exactly.                                                                                                                                                                                                                                                                                          |
| 197 | `feat(gui): add guild ai settings page`                       | A section under the existing guild settings layout: an enable toggle, a default persona selector, and a channel allowlist editor. A new `li` in `munibot_gui/src/pages/guild_settings.rs:13` plus a route variant in `app.rs:37`.                                                                                                                                                                                                                     |
| 198 | `feat(api): add conversation transcript server function`      | `get_ai_transcript(conversation_id)` returning messages with their tool calls, with the bot's own reasoning blocks stripped. Operator-gated for any conversation, owner-gated for your own. Paginated from the start.                                                                                                                                                                                                                                 |
| 199 | `feat(gui): add conversation transcript viewer`               | Render a transcript with tool calls collapsible and their inputs and outputs inspectable. The fastest way to understand why a persona behaved oddly, and the audit surface behind the memory-wipe promise.                                                                                                                                                                                                                                            |
| 200 | `feat(gui): add operator usage dashboard`                     | Global spend over time, token totals, and a breakdown by persona, model, and user. The user-facing half already exists from milestone 2 phase 14; this is the view that catches a problem across everyone at once.                                                                                                                                                                                                                                    |
| 201 | `test(ai): add prompt injection resistance suite`             | A corpus of injection payloads delivered through every untrusted channel: user messages, web page content, GitHub issue bodies, and tool output. Assert that no payload causes a tier escalation, a tool call outside the persona's allowlist, or a system-prompt disclosure. Runs against `MockProvider`, so it is fast and free.                                                                                                                    |
| 202 | `test(ai): add budget and cancellation stress tests`          | Assert that every budget limit terminates a loop, that cancellation never leaks a running tool or an orphaned container, and that a provider hanging indefinitely is bounded by the wall-clock limit. The failure modes that only appear in production, tested deterministically.                                                                                                                                                                     |
| 203 | `feat(ai): add tracing spans across the harness and pipeline` | Instrument every crate with `#[instrument(skip_all, fields(...))]` carrying persona, model, iteration, conversation, and pipeline identifiers, using `.instrument(span)` across every `tokio::spawn` rather than `.entered()`. Update the span table in `docs/tracing.md` in the same commit, as that document requires.                                                                                                                              |
| 204 | `docs(ai): add operator runbook`                              | `docs/ai-operations.md`: required and optional environment variables, how to add a provider, how to write a persona, how to interpret the dashboards, how to abort a pipeline, what each safety system does and how to tune it, and what to do when spend spikes. Written for you at 3am, not for a reader at leisure.                                                                                                                                |

---

## Definition of done

- Someone can sign up and talk to munibot without owning a Discord account.
- Linking a second provider to one account keeps the same memories and conversations; unlinking the
  last sign-in method is refused.
- Abusive usage patterns earn an escalating cooldown, logged rather than silently dropped.
- Every injection payload in the corpus fails to escalate privileges or leak the system prompt.
- An operator can read any conversation's transcript, with tool calls inspectable, and see global spend
  broken down by persona, model, and user.
- A guild admin can turn munibot's Discord AI on or off for their own server without affecting logging.
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
