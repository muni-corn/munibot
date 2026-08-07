# Milestone 3 — specialists and senses

**Outcome:** munibot stays the person you are talking to, but can bring in a specialist — including a
real software engineering team — when you ask him to. And he can look at what you show him.

Milestone 2 gave munibot a home on the web and made him someone worth talking to. This milestone makes
him someone who can _get things done_ without ever handing your conversation to a stranger.

## Delegation, not routing

Automatic routing was dropped for a reason: silently swapping which persona answers you is
disorienting, and for a companion it is actively harmful. You think you are talking to munibot, and
then munibot is briefly somebody else.

Delegation inverts that. **munibot remains the conversational partner throughout.** When you ask him to
dig into a research question, plan a coding project, review a diff, critique a drawing, or brainstorm a
piece of writing, he calls a specialist the way he calls any other tool, waits for the result, and comes
back to you in his own voice — saying plainly that he brought someone in, and never passing their work
off as his own.

Mechanically this is a `delegate` tool, which means it inherits everything the tool system already
guarantees: tier gating, budget accounting, cancellation, audit rows, and a visible activity indicator.
That reuse is the whole argument for the design.

**Phases 15 through 17, commits 109 through 129.**

---

## The five problems delegation creates, and what already solves them

Every one of these is a real hazard, and in each case milestone 1 already built the mechanism:

| Problem                                                                                                                                                    | Solved by                                                                                                                                                                                                                                           |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tier escalation.** A companion limited to `NetworkRead` delegating to a builder persona configured for `Sandbox` would be privilege escalation by proxy. | The nested `ToolCtx` inherits `granted_tier` **unchanged**. Authority comes from the invoking human, never from the persona's wishes, and `ToolCtx::require_tier` already enforces it at the point of use.                                          |
| **Budget double-spend.** Ten delegations in one batch, each handed the parent's full remaining budget, multiplies spend tenfold.                           | `Tool::is_serial()`, built in milestone 1 for persistent shell sessions, already forces same-tool calls to run one at a time. The delegate tool sets it, so delegations cannot race each other for the same remaining budget.                       |
| **Infinite recursion.** A specialist delegating back to the companion never terminates.                                                                    | `ToolCtx` gains a `delegation_depth`, and the tool refuses past a configured maximum.                                                                                                                                                               |
| **Delegating to the wrong thing.** An orchestration-only role must not be summonable mid-conversation.                                                     | A `delegable` flag on `Persona`, defaulting to **false**, so a role becomes reachable from chat only when someone says so in configuration. The tool's input schema enumerates only delegable persona ids, so the model cannot name an invalid one. |
| **A dependency cycle.** A tool that runs a turn needs the thing that runs turns: `Harness -> ToolRegistry -> DelegateTool -> Harness`.                     | A `Delegator` trait that `Ai` implements, inverting the dependency exactly the way `ProviderSource` inverted provider resolution in milestone 2 phase 11.                                                                                           |

**Ordering note:** this milestone lands _after_ milestone 2's spend caps, not before. One user message
can now fan out into several nested turns, so the caps have to already exist.

---

## Phase 15 — delegation

| #   | Commit                                                         | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 109 | `feat(persona): add delegation policy to persona types`        | A `delegable: bool` on `Persona` and `PersonaConfig`, defaulting to `false`. Types before the code that uses them, per the ordering rule in `AGENTS.md`. Defaulting closed means orchestration-only roles are excluded by construction rather than by remembering to exclude them.                                                                                                                                                                                                                                                                                                                                                                                            |
| 110 | `feat(tools): add delegation depth and budget to tool context` | `ToolCtx` gains `delegation_depth: usize` and the parent turn's remaining budget, so a nested turn is bounded by what is actually left rather than handed a fresh allowance. Both are plain data on an existing struct, so this is unit-testable without a provider.                                                                                                                                                                                                                                                                                                                                                                                                          |
| 111 | `feat(ai): add delegator trait on the service handle`          | A `Delegator` trait with one method — run a turn for a named persona with a task brief and a bounded budget — implemented by `Ai`. This breaks the `Harness`/tool cycle, and it is the same seam `ProviderSource` uses, so tests can substitute a fake delegator returning a canned result with no provider and no network.                                                                                                                                                                                                                                                                                                                                                   |
| 112 | `feat(tools): add delegate tool`                               | `delegate` taking a `persona` and a `task`, returning the specialist's final text as the tool result. Tier `Safe` — it grants no new authority, since the nested context inherits the invoker's tier unchanged. `is_serial()` is `true`. Refuses an unknown or non-delegable persona, and refuses past the depth cap, both as recoverable `ToolOutcome::Err` so the model can adjust rather than the turn dying.                                                                                                                                                                                                                                                              |
| 113 | `feat(ai): pass a self contained brief rather than history`    | The specialist receives the companion's written brief, **not the conversation transcript**. Cheaper, and a real prompt-injection boundary: a specialist holding `web_fetch` never sees raw conversation content, so a payload pasted into chat cannot reach it. **Already done**, in commit 111's own `Ai::delegate`: it builds a fresh single-message `History` from `task` alone, never touching `assemble_context` or the session store - there was never a version of this that sent the transcript to begin with. `test_delegate_sends_only_the_task_never_a_conversation_history` covers it. Commit 117 is where the companion learns to write briefs that stand alone. |
| 114 | `feat(harness): add delegation events`                         | `HarnessEvent::DelegationStarted { persona, task }` and `DelegationFinished { persona, ok }`. Delegation must never be invisible — that was the entire objection to automatic routing, and it would be absurd to reintroduce it here.                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| 115 | `feat(api): add delegation to the chat wire types`             | Extend `ChatEvent` and its `HarnessEvent` mapping from milestone 2 phase 11. The mapping is a pure function and is tested directly.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| 116 | `feat(gui): add delegation display in the chat`                | "munibot asked the code reviewer to look at this", with the specialist's own tool activity nested underneath and collapsible. Builds on the tool activity strip from milestone 2 phase 12 rather than inventing a second progress idiom.                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| 117 | `feat(persona): teach the companion to delegate`               | `companion.md` learns when to bring someone in: when you ask, or when a task plainly exceeds conversation. Instructed to say so out loud, to write briefs that stand alone, and to **never present a specialist's work as his own**. Also instructed not to delegate reflexively — most questions deserve an answer from munibot himself, sometimes with a search.                                                                                                                                                                                                                                                                                                            |
| 118 | `test(ai): add delegation safety suite`                        | The security-critical commit. Assert that a nested turn cannot exceed the invoker's granted tier, that a delegation chain terminates at the depth cap, that many delegations cannot collectively outspend the parent turn's budget, and that a non-delegable persona is unreachable. Runs against `MockProvider` and a fake `Delegator`, so it is fast, free, and offline.                                                                                                                                                                                                                                                                                                    |

---

## Phase 16 — the advisory engineering team

Programming is meant to be integral to munibot, not a milestone-4 afterthought. The `municode` project
already wrote a full software engineering team as eleven prompts in
`municode/docs/agent-prompts/`, plus a twelfth (Issue Analyst) sketched at `municode/docs/plan.md`
lines 889–929. They are genuinely good and they are the roster.

Those twelve split cleanly by **whether the role needs hands**:

| Needs nothing but text — lands here                                                                                                                                                                                                                                                     | Needs a filesystem — lands in milestone 4 phase 19                                                      |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `software-architect` (requirements to a plan), `architecture-reviewer` (reviews the plan), `project-manager` (picks the next subtask), `code-reviewer` (reviews a diff or pasted code), `test-reviewer` (reviews tests before implementation), Issue Analyst (an issue to requirements) | `codebase-researcher`, `builder`, `test-engineer`, `final-code-reviewer`, `commit-crafter`, `pr-author` |

This ordering is a real win beyond just being earlier: by the time milestone 5 builds the autonomous
pipeline, **every agent it composes has already been exercised interactively** through chat delegation.
The riskiest milestone in the plan stops being a first outing for twelve untested prompts.

**The porting rule** matters more than any individual prompt. These were written for a pipeline, so they
mix role-and-standards prose (context-independent, valuable) with output-contract prose ("return JSON
shaped like…", "the builder will then…"). Only the former belongs in the prompt file. The latter belongs
in `Persona.handoff`, which is already `Option<HandoffSchema>` — left `None` for chat delegation so a
specialist simply answers, and set by the pipeline when a machine-readable result is required. One
prompt file therefore serves both, with no duplication and no chat-mode branching inside the prose.

| #   | Commit                                                                      | Description                                                                                                                                                                                                                                                                                                          |
| --- | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 119 | `feat(persona): port municode role prompts as handoff free role text`       | Establish the porting rule above and apply it to the first prompt as the worked example: strip the output contract out of the prose, express it as a `HandoffSchema` the pipeline attaches later, and keep the role, standards, and judgement in the markdown. Every commit below follows the pattern this one sets. |
| 120 | `feat(persona): add the software architect and issue analyst personas`      | `software-architect.md` turning requirements into a structured plan, and `issue-analyst.md` written up from the draft in `municode/docs/plan.md`. The entry point to the team: "help me plan this feature" is the most common programming request that needs no filesystem at all.                                   |
| 121 | `feat(persona): add the code and test reviewer personas`                    | `code-reviewer.md` and `test-reviewer.md`, both adapted to review pasted code as readily as a repository diff. Reviewing what someone pastes into a chat window is the single most useful programming thing munibot can do before he has a sandbox.                                                                  |
| 122 | `feat(persona): add the architecture reviewer and project manager personas` | `architecture-reviewer.md` critiquing a plan rather than code, and `project-manager.md` deciding what to do next. Both are pure text in, text out, and both are what make a _team_ rather than a pile of reviewers.                                                                                                  |
| 123 | `feat(persona): add the engineering team to the default configuration`      | Mark the six as `delegable`, give each a description the delegate tool's schema and the persona catalogue can show, and set budgets appropriate to their scope — a reviewer is cheap, an architect is not. Ships as embedded defaults so a fresh deployment has the whole team without writing any configuration.    |

---

## Phase 17 — seeing your work

"Critique a drawing" needs eyes. Most of this is already built and simply unreachable: milestone 1
shipped `ContentBlock::Image`, `ImageSource::{Base64, Url}`, and bidirectional conversion in
`munibot_ai/src/provider/rig/convert.rs:80,150,198` including MIME-to-media-type mapping. **No provider
work is required.** What is missing is a way to get an image in, somewhere to keep it, a check that the
model can actually see it, and someone worth showing it to.

| #   | Commit                                               | Description                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| --- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 124 | `feat(db): add ai attachment table`                  | `ai_attachments (id, conversation_id, message_id, media_type, byte_size, sha256, data, created_at)` with a hard size cap. Deliberately **not** base64 inlined into `ai_messages.content`: a one-megabyte image becomes roughly 1.4 MB of base64 in a JSON column, and the messages table is read on every single turn. Encoded to base64 only when building a provider request. Object storage is the obvious later move, noted in the migration. |
| 125 | `feat(api): add attachment wire types and upload`    | Upload as a separate call from `send_message`, validating media type and size server-side and returning an attachment id the message then references. Rejects anything that is not an image munibot can actually read, with a friendly reason.                                                                                                                                                                                                    |
| 126 | `feat(ai): add vision capability checking`           | Not every model accepts images. A persona whose model cannot see must refuse an attached image **with a clear explanation**, never silently drop it and answer as though it had looked — the failure mode that makes a user think munibot lied to them. Capability lives beside the pricing table, since both are per-model facts already loaded from configuration.                                                                              |
| 127 | `feat(gui): add image attachment in the composer`    | Paste, drag, or pick a file, with a thumbnail preview and a remove button before sending. Pasting a screenshot has to be a non-event — and a screenshot of a stack trace or a UI bug is the most common way a programming question arrives.                                                                                                                                                                                                       |
| 128 | `feat(gui): add image rendering in the message list` | Show attachments inline in the conversation, clickable to full size, so scrolling back shows what was actually discussed rather than a placeholder.                                                                                                                                                                                                                                                                                               |
| 129 | `feat(persona): add the critic persona`              | `critic.md`: critique drawings, designs, screenshots, and layouts. Specific and kind, structured as what works, what does not, and what to try next. Delegable. Names what it is looking at rather than praising vaguely, because vague praise is worthless to someone trying to improve.                                                                                                                                                         |

---

## Definition of done

- Asking munibot to plan a feature brings in the software architect, visibly, and he reports back in his
  own voice.
- Pasting a diff and asking "is this any good?" gets a real review from the code reviewer.
- He says out loud that he brought someone in, and never presents their work as his own.
- A delegated specialist cannot reach a tool the invoking human is not authorized for. Test-enforced,
  not merely intended.
- A delegation chain terminates at the depth cap instead of recursing.
- Many delegations in one turn cannot collectively outspend that turn's own budget.
- Showing munibot a drawing and asking what he thinks produces a specific, useful critique.
- Attaching an image to a persona whose model cannot see gets a clear refusal, never a confident answer
  about an image that was silently discarded.
- Most questions still get answered by munibot himself. Delegation is a tool, not a reflex.

## Deliberately deferred

- **The hands-on half of the team.** `codebase-researcher`, `builder`, `test-engineer`,
  `final-code-reviewer`, `commit-crafter`, and `pr-author` all need a filesystem to be worth anything,
  so they arrive with the sandbox in milestone 4 phase 19.
- **Automatic delegation.** He delegates when asked, or when a task plainly exceeds conversation.
  Deciding autonomously that you would rather talk to someone else is the routing mistake wearing a
  different hat.
- **Specialists orchestrating each other.** The depth cap exists partly to prevent this. Chained,
  stateful multi-agent work is milestone 5's pipeline, where it is the entire point.
- **Audio and video.** Images cover the use case that was actually asked for.
