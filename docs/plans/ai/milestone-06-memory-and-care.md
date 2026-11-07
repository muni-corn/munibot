# Milestone 06 — memory and care

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 05](milestone-05-personas.md).

**Outcome:** munibot remembers you, if you ask him to, and you can see and erase everything he
remembers. And when someone tells him they're not safe, he answers with care that a human reviewed,
never with improvisation.

These two belong together. Memory is what makes him someone people confide in, and the care path is
what that trust obliges. This milestone is the last gate before `/chat` can be opened to the public:
see [before going public](overview.md#before-going-public).

---

## Starting point

After milestone 05:

- Personas exist, defined in config. `companion.md` is the default prompt.
- Templates render `{{display_name}}` and `{{today}}`.
- Conversations are saved per user. Nothing carries over between conversations.
- There's a utility model for cheap single calls.
- Nothing in the chat path recognizes distress.

## Design notes

**Memory is opt-in, per user, and fully auditable.** It's off by default. When it's off, `remember`
refuses and tells the model to point the person at the memory panel. Nothing is ever stored about
someone who didn't agree. An opt-in you can't inspect isn't consent, so the panel ships with the
tools, not after them.

**The gate lives in the store, not the callers.** A `GatedMemoryStore` decorator checks opt-in for
`list`, `record`, and `forget`. Any future caller, or store implementation, gets the check by being
wrapped. `wipe` is exempt: it's never offered to a model, and someone who opted out still has every
right to delete what's left.

**Retrieval is the host's job.** There's no recall tool. Before a turn, the host loads the person's
memories and renders them into `{{memories}}`, wrapped as user-provided data. That's predictable,
costs one query, and needs no model round trip. Only personas with `memory = "user"` get them.

**Memories are keyed by `users.id`.** They survive linking a second sign-in method. Remember the
trap documented in `docs/notes/gui-configuration-research.md`: `guild_wallets.user_id` is a raw
Discord snowflake, while `linked_accounts.user_id` is the internal id. Every table here uses the
internal one.

**The care path never lets a model improvise a crisis response.** Every inbound chat message gets a
small classifier call on the utility model, or on `[ai.care].classifier_model`, which returns a
severity:

| Severity  | Response                                                                                                                                                                                           |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `None`    | A normal turn.                                                                                                                                                                                     |
| `Concern` | A normal turn, with a care addendum in the system prompt: don't diagnose, don't counsel, gently mention support.                                                                                   |
| `Acute`   | **No model turn.** The reply is a reviewed template, `prompts/care/crisis.md`, plus the configured resources. It's stored as an assistant message flagged `care`, and shown with distinct styling. |

The classifier is tuned to over-trigger, because the costs of a miss and a false alarm are nowhere
near symmetric. If the classifier itself fails, the turn proceeds at `Concern`, and the failure is
logged.

```toml
[ai.care]
classifier_model = "anthropic:claude-haiku-5" # optional; utility model otherwise

[[ai.care.resources]]
name = "988 Suicide and Crisis Lifeline"
contact = "call or text 988 (US)"

[[ai.care.resources]]
name = "Samaritans"
contact = "116 123 (UK and Ireland)"
```

Resources are real, reviewed, and configured by the operator. Nothing in this path is ever generated.

**Classification costs a small call per message.** That's an accepted cost of being a companion,
and it is charged to the turn like any other usage.

## Commits

| #    | Commit                                            | Summary                                                                                                                                             |
| ---- | ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 6.1  | `feat(db): add memory and user settings tables`   | `ai_memories (user_id, key, value, …, UNIQUE (user_id, key))` and `ai_user_settings (user_id PK, memory_opt_in)`, both cascading from `users`.      |
| 6.2  | `feat(db): add memory operations`                 | List, upsert with `on_conflict`, forget, wipe, a per-user count, and opt-in get and set.                                                            |
| 6.3  | `feat(persona): add the memory policy`            | `MemoryPolicy` is `None` or `User`. It defaults to `None`, while the synthesized `munibot` persona gets `User`.                                     |
| 6.4  | `feat(memory): add the memory store`              | A `MemoryStore` trait with in-memory and diesel implementations, and a per-user cap.                                                                |
| 6.5  | `feat(memory): gate memory on opt-in`             | The `GatedMemoryStore` decorator. `wipe` is exempt, and the reason is documented on it.                                                             |
| 6.6  | `feat(tools): add remember and forget tools`      | Tier `Safe`. They refuse without opt-in, telling the model to point at the memory panel. Labels: "remembered {key}" and "forgot {key}".             |
| 6.7  | `feat(ai): render memories into prompts`          | Fills `{{memories}}` for `memory = "user"` personas, through the untrusted wrapper.                                                                 |
| 6.8  | `feat(persona): teach the companion about memory` | What he remembers, that remembering is the person's choice, and to say so plainly.                                                                  |
| 6.9  | `feat(api): add memory server functions`          | Get and set opt-in, list, edit, forget, and wipe. All are the caller's own.                                                                         |
| 6.10 | `feat(gui): add the memory panel`                 | `/memory`: the opt-in toggle, every memory editable and deletable, and a confirmed wipe.                                                            |
| 6.11 | `feat(config): add care settings`                 | `[ai.care]`: the classifier model and resources. Startup warns if no resources are configured.                                                      |
| 6.12 | `feat(ai): add the care classifier`               | `CareSeverity`, and a classifier call that over-triggers and fails toward `Concern`.                                                                |
| 6.13 | `feat(ai): add the reviewed crisis response`      | `Acute` bypasses the model and answers from `prompts/care/crisis.md` plus the resources. `Concern` adds the care addendum. Both are recorded.       |
| 6.14 | `feat(chat): show care responses distinctly`      | A wire flag, plus styling that makes the resources easy to read and copy.                                                                           |
| 6.15 | `test(ai): add the care path suite`               | Fixture messages at each severity. Asserts that an `Acute` reply never comes from a provider call, and that a classifier failure still yields care. |
| 6.16 | `docs: document memory and the care path`         | The runbook covers how to review and change resources, and what is and isn't stored. `docs/chat.md` covers memory and its controls.                 |

## Definition of done

- After opting in and telling munibot something about yourself, he uses it in a new conversation a
  week later, and the memory panel shows exactly what he kept.
- Opting out stops new memories. Wiping removes everything, verifiably, from the same panel.
- `remember` without opt-in refuses, and munibot points you at the panel.
- A simulated crisis message gets the reviewed template and real resources, never generated text.
  The suite proves no provider call was made.
- A classifier outage doesn't silence munibot or skip care.

## Deliberately deferred

- **Vector or semantic memory.** Key-value facts work well and need no embeddings. rig's vector store
  support is there if semantic recall ever earns its keep.
- **Memory in Discord channels.** Memories are private. Milestone 12 loads them only in direct
  messages, never in a guild channel where others can read the reply.
- **Moderation endpoints.** Provider moderation is milestone 11. The care path isn't moderation; it's
  a duty of care.
