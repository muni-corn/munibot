# Milestone 12 — munibot in Discord

**Status:** outline. The commit list is provisional. Expand it into a full recipe, checked against
the code, before starting. Start after [milestone 11](milestone-11-hardening.md).

**Outcome:** you can talk to munibot in Discord the way you'd talk to a friend in the server. Mention
him, reply to him, or DM him, and he answers in character, streaming his reply. Simple questions get
simple answers. And if you're a developer and ask him to fix something in one of your projects, he
uses his own judgement to start the work, opens a thread for it, and keeps you posted there.

**There are no slash commands.** Discord is a plain chat surface onto the same munibot who lives on
the web: the same personas, tools, limits, care path, and `start_work`.

---

## Starting point

After milestone 11, everything munibot can do exists, and is reachable from the web chat.

From the code as it stands:

- `munibot_discord` has no AI handler, and doesn't depend on `munibot_ai`.
- `munibot/src/bot.rs::start_discord(config)` builds a fixed handler collection. Handlers are
  `Arc<Mutex<…>>` and take `&mut self` (`munibot_discord/src/handler.rs:10`).
- `munibot_ai::Chat` is built in `main.rs` outside the `MUNIBOT_DISABLE_BOTS` guard, and handed only
  to the GUI server.
- `munibot_discord` already depends on `decancer`.
- The pre-strip Discord adapter (`/ask`, `/persona`, `/reset`, and tier-2 bot-data tools) is not
  coming back.

## Design notes

**Triggers.** munibot answers:

- a direct mention;
- a reply to one of his own messages;
- a direct message.

The trigger is checked in memory, before any database or provider call, so ordinary channel traffic
costs nothing. He never answers himself or another bot.

**Who's talking.** A message author is resolved through `linked_accounts` (`provider = "discord"`):

| Author                      | Identity                   | Granted tier                                     | Limits                                                 |
| --------------------------- | -------------------------- | ------------------------------------------------ | ------------------------------------------------------ |
| linked to a munibot account | their `users.id`           | from their permissions, exactly as on the web    | their own per-user limits, **plus** the guild's cap    |
| not linked                  | their Discord user id only | `Safe`: no web tools, no memory, no `start_work` | a per-Discord-user daily cap, **plus** the guild's cap |

**DMs require a linked account.** There's no guild to attribute an unlinked DM's cost to. An unlinked
DM gets a friendly reply explaining how to link an account on the web.

**Conversations are scoped to where they happen.** `ai_conversations` gains `platform` and
`scope_key`, and `owner_user_id` becomes nullable:

- A **DM** is a conversation owned by the linked user. It shows up in their web sidebar, labelled as
  Discord.
- A **guild channel or thread** is an unowned, channel-scoped conversation that everyone in it shares.
  Messages carry their author's display name, so munibot knows who said what.
- A channel conversation **starts fresh after `idle_reset` (default two hours) of silence.** With no
  slash commands, that's the natural replacement for `/reset`.

**Memory never appears in a guild channel.** Memories are private. They're rendered only in DMs. In a
channel, `remember` and `forget` aren't offered, so nothing personal can leak into a public reply.

**The care path runs here too.** An `Acute` message in a channel gets the reviewed template as a
reply, and a gentle offer to continue in DMs.

**Rendering.**

- Send a placeholder, then edit it no more than once a second, to respect the
  five-edits-per-five-seconds channel limit.
- Do a final edit at the end, and split anything over 2,000 characters across follow-up messages at
  paragraph boundaries.
- Tool activity shows as one italic status line above the partial reply, and is replaced by the final
  text.
- Every message munibot sends or edits carries serenity's `CreateAllowedMentions` with nothing
  allowed, so Discord itself refuses to ping anyone, `@everyone` and roles included, whatever the
  text says. That's Discord's own mechanism, and it can't be fooled by a clever spelling the way
  stripping text can. The text is left as written, so `decancer` isn't run on replies either: it
  would mangle legitimate non-Latin writing.

**Turns never block the handler.** Handlers sit behind a mutex, so the AI handler only does the
trigger check, then spawns the turn with `.instrument(span)` and returns at once.

**Starting work from Discord.** `start_work` is the same tool, with the same developer gate and the
same judgement rules as in [milestone 10](milestone-10-autonomous.md#starting-work-from-conversation).
When he starts work from a Discord message, munibot:

1. replies with the issue link;
2. opens a thread on that message;
3. binds the run to a `DiscordThreadAdapter`, so the run's questions are posted in the thread;
4. resumes the run when you reply there, relayed through `answer_work_question` just as on the web;
5. posts the pull request link in the thread when the run finishes.

**Guild settings live on the web.** A guild admin turns munibot's chat on for their server and picks
channels on a dashboard page. That uses a separate `guild_ai_settings` table rather than new columns
on `guild_configs`, so there's no chance of tripping over `upsert_guild_config`'s whole-row
semantics (see [loose ends](../loose-ends.md) commit 1.1).

```
guild_ai_settings (guild_id PK, enabled BOOL, channel_mode VARCHAR(16), persona_id NULL,
                   daily_usd_micros NULL, updated_at)
guild_ai_channels (guild_id, channel_id, PRIMARY KEY (guild_id, channel_id))
ai_usage          gains guild_id NULL and discord_user_id NULL
ai_conversations  gains platform, scope_key NULL, UNIQUE (platform, scope_key);
                  owner_user_id becomes NULL-able
```

`channel_mode` is either `allowlist` (the default: only listed channels) or `all`. Chat is **off**
in every guild until an admin turns it on.

```toml
[ai.discord]
idle_reset = "2h"
unlinked_user_daily_usd = 0.05
default_guild_daily_usd = 2.00
```

## Commits

| #     | Commit                                                  | Summary                                                                                                                                |
| ----- | ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| 12.1  | `feat(db): scope conversations to a platform`           | `platform` and `scope_key`, and `owner_user_id` becomes nullable. Existing rows become `web`.                                          |
| 12.2  | `feat(db): attribute usage to guilds and discord users` | `ai_usage.guild_id` and `discord_user_id`.                                                                                             |
| 12.3  | `feat(db): add guild ai settings`                       | `guild_ai_settings` and `guild_ai_channels`, plus their operations.                                                                    |
| 12.4  | `feat(memory): add scoped conversations`                | `ConversationScope` and `load_or_create_scoped(platform, scope_key)`, with idle-reset semantics.                                       |
| 12.5  | `feat(config): add discord chat settings`               | `[ai.discord]` as above.                                                                                                               |
| 12.6  | `feat(ai): add per-guild and unlinked-user limits`      | New admission scopes, with refusals worded for Discord.                                                                                |
| 12.7  | `feat(ai): run turns for scoped conversations`          | `Chat::turn` accepts a scope and an author. Memory and memory tools only in owned conversations. Author names prefixed in shared ones. |
| 12.8  | `feat(discord): send replies without pings`             | Every send and edit sets an empty `CreateAllowedMentions`, through one helper the renderer uses. Tested on the builder it produces.    |
| 12.9  | `feat(discord): add the streaming renderer`             | Placeholder, throttled edits, a final edit, paragraph-boundary splitting, and the status line.                                         |
| 12.10 | `feat(discord): add the ai chat handler`                | Trigger checks in memory, author resolution, guild and channel gating, the unlinked-DM reply, and a spawned turn.                      |
| 12.11 | `feat(discord): open threads for started work`          | Opens the thread when a turn's `start_work` succeeds, and binds the run to it.                                                         |
| 12.12 | `feat(discord): ask pipeline questions in threads`      | `DiscordThreadAdapter`, implementing `InteractionAdapter`. Thread replies are routed through the scoped conversation for that thread.  |
| 12.13 | `feat(munibot): register the ai handler`                | `start_discord(config, chat)` passes the same `Arc<Chat>` the GUI gets. Skipped entirely when chat is disabled.                        |
| 12.14 | `feat(api): add guild ai settings server functions`     | Get and set, gated by `require_guild_admin`.                                                                                           |
| 12.15 | `feat(gui): add the guild ai settings page`             | An enable toggle, channel mode and list, default persona, and daily cap, under the existing guild settings layout.                     |
| 12.16 | `feat(gui): label discord conversations in the sidebar` | DMs appear in your web sidebar, read-only on the web, since they continue in Discord.                                                  |
| 12.17 | `test(discord): cover triggers, identity, and gating`   | Self and bot messages ignored, unlinked DMs refused, channel gating, and memory never offered in a guild.                              |
| 12.18 | `docs: document munibot in discord`                     | `docs/chat.md` and the runbook: enabling a guild, linking accounts, and what unlinked users can do.                                    |

## Definition of done

- In an enabled channel, mentioning munibot gets a streamed, in-character reply. In a channel that
  isn't enabled, nothing happens and nothing is spent.
- Replying to munibot continues the same conversation. After two quiet hours, a new mention starts
  fresh.
- An unlinked member gets answers without web tools, within the per-user cap. An unlinked DM gets
  instructions for linking an account.
- A linked developer asking munibot to fix a bug in a registered project gets an issue link and a
  thread. The run's questions arrive in that thread, and replies resume it. The pull request link
  is posted there at the end.
- A linked non-developer making the same request gets an explanation, and no run starts.
- Nothing he says can ping `@everyone`, `@here`, or a role.
- Memory is never used, or offered, in a guild channel.
- A guild admin can turn munibot's chat on and off for their server without affecting logging.

## Deliberately deferred

- **Slash commands.** Deliberately absent. Discord is a conversation, not a control panel.
- **Bot-data tools.** Tier `BotData` stays reserved. Reading someone's balance or quotes might be
  fun, but it's a privacy decision worth making on purpose, not by default.
- **Twitch.** It can't edit messages, so it would need a completely different renderer, and it isn't
  where the companion gets shown off.
