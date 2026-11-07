# Milestone 11 — hardening

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 10](milestone-10-autonomous.md). One item, the
injection suite, should be **pulled forward**, as explained below.

**Outcome:** munibot's AI is safe to leave running in public, on a budget you chose, with enough
observability to diagnose anything that goes wrong. Almost nothing here adds a capability. It
protects the capabilities already built, the people using them, and your wallet.

---

## What's no longer here

Several items from the pre-strip hardening plan are done, or have moved:

| Item                                                 | Where it went                                                               |
| ---------------------------------------------------- | --------------------------------------------------------------------------- |
| GitHub and email sign-in, account linking, unlinking | **Already implemented** (`munibot_api/src/oauth/`, `pages/account.rs`)      |
| OAuth `state` CSRF protection                        | **Already implemented** for Discord and GitHub (`oauth/routes.rs`)          |
| Replacing the `HasPermission` stub                   | Never existed after the strip. Milestone 04 builds permissions from scratch |
| Rate limits and spend caps                           | Milestone 04                                                                |
| Crisis recognition and response                      | Milestone 06                                                                |
| Guild AI settings                                    | Milestone 12, with the Discord surface they configure                       |
| Email sign-in throttling                             | [Loose ends](../loose-ends.md) commit 2.7                                   |

## Sequencing

Most of this is genuine finishing work. One item isn't:

| Item                                  | Pull forward to                                                                                                                  |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| The prompt injection resistance suite | Before `/chat` is opened to the public, which is after milestone 06. See [before going public](overview.md#before-going-public). |

Written against milestone 06's tool set, the suite covers user messages, search results, and fetched
pages. When it's pulled forward, extend it here to issue bodies, sandbox output, and delegation
briefs.

## Commits

| #     | Commit                                                        | Summary                                                                                                                                                                                                              |
| ----- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 11.1  | `test(ai): add the prompt injection resistance suite`         | Payloads delivered through every untrusted channel. Asserts no tier escalation, no tool call outside the selection, and no system-prompt disclosure. Runs against `MockProvider`, plus an opt-in live-model variant. |
| 11.2  | `feat(db): add safety events`                                 | `ai_safety_events`: kind, scope, and a content hash, never the content itself. Enough to tune, not enough to surveil.                                                                                                |
| 11.3  | `feat(ai): record safety events`                              | Limit refusals, care triggers, moderation blocks, and injection signatures.                                                                                                                                          |
| 11.4  | `feat(ai): add abuse detection and cooldowns`                 | Repeated near-identical prompts and injection signatures, met with escalating per-user cooldowns. Every trip is logged so false positives are discoverable.                                                          |
| 11.5  | `feat(ai): add provider moderation checks`                    | Inbound and outbound, where the provider offers moderation. Fail-closed for `Privileged` turns, fail-open with a warning for chat.                                                                                   |
| 11.6  | `test(ai): add budget and cancellation stress tests`          | Every limit terminates a loop. Cancellation never leaks a tool or a container. A provider that hangs forever is bounded by the wall clock.                                                                           |
| 11.7  | `feat(ai): add tracing spans across the harness and pipeline` | `#[instrument(skip_all, fields(...))]` with persona, model, iteration, conversation, and pipeline. `.instrument(span)` across every spawn. Updates `docs/tracing.md` in the same commit.                             |
| 11.8  | `feat(api): add transcript server functions`                  | Any conversation for an operator, with tool calls and reasoning stripped. Paginated.                                                                                                                                 |
| 11.9  | `feat(gui): add the transcript viewer`                        | Collapsible tool calls with their inputs and outputs.                                                                                                                                                                |
| 11.10 | `feat(gui): add usage breakdowns for operators`               | Spend over time, broken down by persona, model, user, and pipeline, on top of milestone 04's operator section.                                                                                                       |
| 11.11 | `feat(api): manage permissions from the gui`                  | Grant and revoke `manual` permissions. Operator-only.                                                                                                                                                                |
| 11.12 | `feat(gui): add the permissions page`                         | Who holds what, and whether config or a person granted it. Config grants are read-only here.                                                                                                                         |
| 11.13 | `docs: complete the operations runbook`                       | Every environment variable, adding a provider, writing a persona, reading the dashboards, aborting work, tuning each safety system, and what to do when spend spikes. Written for 3am.                               |

## Definition of done

- Every injection payload in the corpus fails to escalate, to call an unselected tool, or to leak the
  system prompt.
- Abusive patterns earn an escalating cooldown that's logged, not silently dropped.
- An operator can read any transcript with tool calls inspectable, and can see spend broken down
  every way that matters.
- Every request is traceable end to end from one identifier.
- Someone who isn't you could run munibot from the runbook alone.

## Ongoing work beyond this plan

- **Prompt iteration.** `companion.md` is never finished. Treat prompt changes as product changes.
- **Model updates.** The pricing table and persona models are configuration, so keeping them current
  is cheap.
- **Cost optimisation.** Prompt caching, cheaper models for cheap roles, and smarter compaction, once
  there's real usage data to aim them with.
