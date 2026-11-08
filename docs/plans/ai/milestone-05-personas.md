# Milestone 05 — personas

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 04](milestone-04-affordability.md).

**Outcome:** munibot has a real voice, written down in `companion.md`, and he isn't the only one you
can talk to. A new conversation can be started with a writer or a coder persona from a visible
picker, and a `/personas` page describes each one. Operators define personas in TOML with prompts in
markdown, and can edit the prompts without recompiling.

---

## Starting point

After milestone 04:

- `Chat` runs every conversation with one model (`[ai] model`), one embedded prompt
  (`prompts/conversation.md`), one tool selection (`[ai] tools`), and one budget (`[ai.budget]`).
- Conversations have no persona column, and neither do usage rows.
- Admission control reserves `[ai.budget].max_cost_usd` for every turn.

## Design notes

**The persona is the unit of configuration.** A persona is a model, its parameters, a system prompt,
a tool selection, and a budget. rig-agent's `AgentBuilder` is how a persona becomes runnable, but it
is a builder, not configuration data: it can't be read from TOML, listed in a picker, or recorded on
a conversation. So `Persona` stays munibot's own, and the registry turns each one into a rig `Agent`
at startup. Later milestones add fields as they need them, never before:

| Field       | Added in     |
| ----------- | ------------ |
| `memory`    | milestone 06 |
| `delegable` | milestone 07 |
| `sandbox`   | milestone 08 |
| `handoff`   | milestone 10 |

**Existing config keeps working.** If no `[ai.personas]` table exists, the registry builds a single
persona, `munibot`, from `[ai] model`, `[ai] tools`, `[ai.budget]`, and the embedded `companion.md`.
Deployments that never touch personas see no difference except a better prompt.

```toml
[ai]
default_persona = "munibot"
prompt_dir = "/etc/munibot/prompts" # optional; embedded prompts otherwise

[ai.personas.munibot]
display_name = "munibot"
description = "warm, playful conversation, with research when it helps"
model = "anthropic/claude-opus-5"
prompt = "companion.md"
tools = ["tier0", "tier1"]
temperature = 1.0

[ai.personas.writer]
display_name = "writer"
description = "brainstorming, drafting, and editing"
model = "anthropic/claude-opus-5"
prompt = "writer.md"
tools = ["tier0"]
budget = { max_turns = 4, max_cost_usd = 0.15 }
```

**A conversation picks its persona once, when it starts.** It's a visible choice, never an invisible
switch. Changing persona mid-conversation would make a companion feel like a stranger. Starting a new
conversation is the honest way to talk to someone else, and milestone 07's delegation covers "bring
someone in for a moment".

**The service keeps its name.** `Chat` is already what the API and GUI hold through an `Extension`.
It becomes persona-aware rather than being replaced by a new `Ai` type. Rename it only if it
genuinely outgrows the name.

**No template engine.** rig's `completion::SystemPrompt` is a role plus named `Section`s, rendered
into one system message as `<tag>…</tag>` blocks when the request is sent. A persona's markdown file
is the role. Anything that changes per turn is a section: `person` (the user's display name) and
`today` here, and `memories` and `care` in milestone 06. Sections can't be misspelled the way
`{{variables}}` can, and a hook can read or move one by its tag.

## Commits

| #    | Commit                                                     | Summary                                                                                                                                                                                                                                                                                    |
| ---- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 5.1  | `feat(persona): add persona types`                         | `PersonaId` and `Persona { id, display_name, description, model, temperature, prompt, tools, budget }`, where `model` is a rig catalog reference.                                                                                                                                          |
| 5.2  | `feat(config): add persona configuration`                  | `default_persona`, `prompt_dir`, and `personas: BTreeMap<PersonaId, PersonaConfig>`, using config-friendly shapes such as dollar amounts and humantime strings.                                                                                                                            |
| 5.3  | `feat(persona): add the companion prompt`                  | `prompts/companion.md` replaces `conversation.md`. It covers voice, the instruction hierarchy, untrusted content, advice boundaries, and research guidance. **This file is the product.**                                                                                                  |
| 5.4  | `feat(persona): add the writer and coder prompts`          | `writer.md`, and `coder.md`, which states plainly that he can read pasted code but can't run or change anything yet.                                                                                                                                                                       |
| 5.5  | `feat(persona): add per-turn prompt sections`              | Builds the persona's rig `SystemPrompt`: its markdown as the role, plus `person` and `today` sections.                                                                                                                                                                                     |
| 5.6  | `feat(persona): add the persona registry`                  | Loads personas from config, synthesizing `munibot` when none are declared. Resolves prompts from `prompt_dir` or embedded defaults, connects every model through rig's catalog, and fails startup listing every problem found. Tests hand it `MockCompletionModel`s instead of connecting. |
| 5.7  | `refactor(ai): build one rig agent per persona`            | `Chat` holds the registry's agents instead of a single one. No behaviour change while only `munibot` exists.                                                                                                                                                                               |
| 5.8  | `feat(db): record the persona on conversations and usage`  | `ai_conversations.persona_id` and `ai_usage.persona_id`, with existing rows backfilled to `munibot`.                                                                                                                                                                                       |
| 5.9  | `feat(ai): run turns as the conversation's persona`        | Uses the persona's agent, prompt sections, tools, and budget. Admission reserves the persona's own `max_cost_usd`.                                                                                                                                                                         |
| 5.10 | `feat(chat): add persona wire types`                       | `PersonaSummary`, and the persona on `ConversationSummary`.                                                                                                                                                                                                                                |
| 5.11 | `feat(api): choose a persona when starting a conversation` | `list_personas`, and an optional `persona` on `send_message` that only applies when it creates the conversation.                                                                                                                                                                           |
| 5.12 | `feat(gui): add the persona picker`                        | Shown on a new conversation only, and defaults to `default_persona`.                                                                                                                                                                                                                       |
| 5.13 | `feat(gui): show who you are talking to`                   | The persona's name in the conversation header and the sidebar.                                                                                                                                                                                                                             |
| 5.14 | `feat(gui): add the persona catalogue`                     | `/personas` lists each persona's description, model, and tools. It doubles as user-facing documentation.                                                                                                                                                                                   |
| 5.15 | `docs: document personas`                                  | The runbook gains how to write a persona and how prompt overrides work. `docs/chat.md` gains the picker.                                                                                                                                                                                   |

## Definition of done

- A deployment with no `[ai.personas]` table behaves exactly as before, apart from the new prompt.
- Declaring a writer persona makes it appear in the picker and the catalogue. A conversation started
  with it uses its model, prompt, tools, and budget.
- A persona naming an unconfigured provider, or a prompt file that doesn't exist, fails startup with
  a message listing every problem found.
- Editing a prompt in `prompt_dir` takes effect on restart without rebuilding.
- `/usage` can break spend down by persona.

## Deliberately deferred

- **Automatic persona routing.** It was dropped from the plan for good. An invisible personality
  switch is wrong for a companion, and giving him research tools removes most of the reason for one.
- **Changing persona mid-conversation.** See the design notes. Delegation (milestone 07) is the
  mid-conversation mechanism.
- **Persona-specific memory and care.** Milestone 06 adds the `memory` field and the care path.
