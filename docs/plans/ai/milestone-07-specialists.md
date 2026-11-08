# Milestone 07 — specialists and senses

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 06](milestone-06-memory-and-care.md).

**Outcome:** munibot stays the one you're talking to, but can bring in a specialist when you ask,
including a real software engineering team that plans, reviews, and critiques. And he can look at
what you show him: a screenshot of a stack trace, a drawing, a design.

---

## Starting point

After milestone 06:

- Personas exist with a model, prompt, tools, budget, and memory policy.
- A conversation is bound to one persona.
- Turns are admitted against spend limits, with each turn reserving its persona's `max_cost_usd`.
- Each persona is a rig `Agent`, and the budget hook tracks a turn's spend, including tool costs
  reported as `ToolCost` result facts.

**Images need no new message types.** rig's `UserContent::Image` and `Image` already carry them, and
rig's providers already encode them. Whether a model can see comes from rig's catalog
(`ModelSpec.input.image`), so there's no capability table to maintain. What's missing is a way to get
an image in, somewhere to keep it, and a refusal for models that can't see.

The engineering team's prompts live in the `municode` repository: eleven at
`municode/docs/agent-prompts/`, plus an Issue Analyst sketch in `municode/docs/plan.md` (the section
headed "Issue Analyst agent prompt (draft)").

## Delegation, not routing

Silently swapping who answers you is disorienting, and wrong for a companion. Delegation turns that
around: **munibot stays the conversational partner throughout.** He calls a specialist the way he
calls any other tool, waits for the result, and comes back in his own voice. He says plainly that he
brought someone in, and never passes their work off as his own.

Because `delegate` is a tool, it inherits tier gating, budget accounting, cancellation, auditing, and
the activity strip. Five hazards, and what handles each:

| Hazard                                                                       | Handled by                                                                                                                                                                                                                                                  |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tier escalation.** Delegating to a persona configured with stronger tools. | The nested run gets the parent's `Invoker` unchanged, through rig's `ToolContext`. Authority comes from the human, never the persona.                                                                                                                       |
| **Budget double-spend.** Many delegations, each handed a fresh allowance.    | The budget's running totals move from the run's scratchpad into a shared value carried as a `ToolContext` scope, so a nested run spends from the parent turn's budget and admission reservation. The delegate tool takes a lock, so delegations can't race. |
| **Infinite recursion.**                                                      | A `delegation_depth` on the `Invoker`, refused past `[ai].max_delegation_depth` (default 1).                                                                                                                                                                |
| **Summoning the wrong thing.**                                               | `Persona.delegable`, defaulting to `false`. The tool's schema enumerates only delegable personas.                                                                                                                                                           |
| **A dependency cycle.** agents → `delegate` tool → agents.                   | The tool reaches persona agents through a handle filled in once the registry has built them.                                                                                                                                                                |

**Why not rig's `Agent::into_tool`.** It turns an agent into a tool that takes a prompt and inherits the
parent's `ToolContext`, but the nested run gets its own budget and nothing limits nesting. So
`delegate` is a rig `DynamicTool` of munibot's own that runs the chosen persona's agent with the
shared budget and a depth check.

**Specialists receive a brief, not the transcript.** That's cheaper, and it's a real injection
boundary: a specialist holding `web_fetch` never sees raw conversation content.

## The advisory engineering team

The twelve `municode` roles split by whether they need hands:

| Text in, text out: lands here                                                                                                                   | Needs a filesystem: lands in [milestone 08](milestone-08-sandbox.md)                                    |
| ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `software-architect`, `architecture-reviewer`, `project-manager`, `code-reviewer`, `test-reviewer`, `issue-analyst` (written up from the draft) | `codebase-researcher`, `builder`, `test-engineer`, `final-code-reviewer`, `commit-crafter`, `pr-author` |

By the time milestone 10 assembles the autonomous pipeline, every agent in it has already been
exercised interactively.

**The porting rule.** These prompts were written for a pipeline. They mix role-and-standards prose,
which is valuable anywhere, with output-contract prose ("return JSON shaped like…"). Only the former
goes in the markdown. The latter becomes `Persona.handoff` in milestone 10, which is left unset for
chat delegation, so a specialist simply answers. One prompt file serves both.

**Fix known defects during the port**, all verified in the `municode` sources:

- `architecture-reviewer.md` line 88 has `git config --unset-all --local core.hooksPath` spliced into
  a sentence.
- `architecture-reviewer.md` requires a `strengths` field (line 133) that its own JSON example omits.
- `project-manager.md` line 58 says `StartTask`, where the action is `StartTaskTests`.
- `code-reviewer.md` line 191 references `implementation_issues`, a field `SubmitTests` doesn't have.
  The real field is `assumptions`.
- `municode/docs/overview.md` uses `TestsWritten` and `ReadyForReview` where the prompts use
  `SubmitTests` and `SubmitCode`. The prompts win.

## Commits

### Delegation

| #    | Commit                                                           | Summary                                                                                                                                                                                                                                                                         |
| ---- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 7.1  | `feat(persona): add the delegable flag`                          | `delegable: bool` on `Persona` and `PersonaConfig`, defaulting to `false`.                                                                                                                                                                                                      |
| 7.2  | `feat(tools): add delegation depth to the invoker`               | `Invoker.delegation_depth`, and a nested invoker one level deeper with everything else unchanged.                                                                                                                                                                               |
| 7.3  | `feat(config): add the delegation depth limit`                   | `[ai].max_delegation_depth`, defaulting to 1.                                                                                                                                                                                                                                   |
| 7.4  | `feat(turn): add delegation events`                              | `DelegationStarted { persona, brief }`, `DelegationFinished { persona, ok }`, and nested tool events tagged with their delegation.                                                                                                                                              |
| 7.5  | `refactor(turn): share a turn's budget through its tool context` | The budget hook keeps its running totals in an `Arc` carried as a rig `ToolContext` scope, so a nested run can spend from it. No behaviour change.                                                                                                                              |
| 7.6  | `feat(tools): add the delegate tool`                             | A rig `DynamicTool` taking `persona` and `brief`, shelved at `Safe`. Runs the persona's agent from the brief alone, records usage with that persona, and takes a lock so delegations run one at a time. Unknown, non-delegable, and too-deep requests are model-visible errors. |
| 7.7  | `feat(event): carry delegation to the chat page`                 | Wire variants, and a tested mapping.                                                                                                                                                                                                                                            |
| 7.8  | `feat(gui): show delegation in the chat`                         | "munibot asked the code reviewer…", with the specialist's own activity nested and collapsible.                                                                                                                                                                                  |
| 7.9  | `feat(persona): teach the companion to delegate`                 | When to bring someone in, writing briefs that stand alone, always saying so, and never presenting their work as his own.                                                                                                                                                        |
| 7.10 | `test(ai): add the delegation safety suite`                      | No tier escalation, depth termination, shared budget, and non-delegable personas unreachable. Runs against `MockCompletionModel`.                                                                                                                                               |

### The advisory team

| #    | Commit                                                             | Summary                                                                                                                                                   |
| ---- | ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 7.11 | `feat(persona): ship default personas alongside configured ones`   | The registry merges embedded default personas under any configured ones, so a fresh deployment gets the team. Configuration can override or disable each. |
| 7.12 | `feat(persona): add the software architect and issue analyst`      | The worked example of the porting rule. Issue analyst is written in full to the same standard as the others.                                              |
| 7.13 | `feat(persona): add the code and test reviewers`                   | Adapted to review pasted code as readily as a diff.                                                                                                       |
| 7.14 | `feat(persona): add the architecture reviewer and project manager` | Applies the defect fixes listed above.                                                                                                                    |
| 7.15 | `feat(persona): make the advisory team delegable by default`       | Descriptions for the delegate schema and the catalogue, and budgets sized to scope: reviewers are cheap, the architect isn't.                             |

### Seeing your work

| #    | Commit                                                  | Summary                                                                                                                                                                                     |
| ---- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 7.16 | `feat(db): add the attachments table`                   | `ai_attachments (id, conversation_id, owner_user_id, media_type, byte_size, sha256, data, created_at)`, with a hard size cap. Never inlined into `ai_messages`.                             |
| 7.17 | `feat(memory): resolve stored images into rig messages` | Stored messages reference an attachment by id. Context assembly swaps each reference for rig's `Image` with base64 data just before sending, so the messages table never holds image bytes. |
| 7.18 | `feat(api): add image upload`                           | A separate upload call that validates type and size server-side and returns an id that `send_message` references.                                                                           |
| 7.19 | `feat(ai): refuse images a model can't see`             | Checks the persona's `ModelSpec.input.image` from rig's catalog. A clear refusal, never a silent drop followed by a confident answer about an image the model never received.               |
| 7.20 | `feat(gui): attach images in the composer`              | Paste, drag, or pick a file, with a thumbnail and a remove button.                                                                                                                          |
| 7.21 | `feat(gui): show images in the conversation`            | Inline, and clickable to full size.                                                                                                                                                         |
| 7.22 | `feat(persona): add the critic`                         | `critic.md`: specific, kind critique of drawings, designs, and screenshots. Delegable.                                                                                                      |
| 7.23 | `docs: document delegation, the team, and images`       | The runbook and `docs/chat.md`.                                                                                                                                                             |

## Definition of done

- Asking munibot to plan a feature brings in the software architect, visibly, and he reports back in
  his own voice.
- Pasting a diff and asking "is this any good?" gets a real review from the code reviewer.
- A delegated specialist can't reach a tool the human isn't authorized for. This is enforced by the
  test suite, not just intended.
- Many delegations in one turn can't collectively outspend that turn's own budget.
- Showing munibot a drawing produces a specific, useful critique.
- Attaching an image to a persona whose model can't see gets a clear refusal.
- Most questions are still answered by munibot himself.

## Deliberately deferred

- **The hands-on half of the team.** It needs the sandbox. See milestone 08.
- **Automatic delegation.** He delegates when asked, or when a task plainly exceeds conversation.
  Deciding on his own that you'd rather talk to someone else is the routing mistake in a different
  hat.
- **Specialists orchestrating each other.** Chained multi-agent work is the pipeline's job, in
  milestone 10.
- **Audio and video.**
