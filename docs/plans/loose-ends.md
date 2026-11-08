# Loose ends — non-AI work

This plan covers fixes that aren't part of the AI plan in `docs/plans/ai/`. It has two parts:

1. **Work the strip commit reverted.** The commit "chore: strip ai chat features down to temporary
   chat only" was meant to remove AI features. It also rewrote many non-AI files back to versions
   older than its own parent, `03107863`, undoing a long run of main-branch commits. Part 1 brings
   that work back.
2. **Outstanding non-AI bugs and gaps.** These are real, documented problems that were tracked in
   the old AI milestone 7 even though none of them involve AI.

Do part 1 first. Several AI milestones assume a working test fixture, and some part 2 items touch
files part 1 restores.

---

## Part 1 — restoring what the strip reverted

### What happened

The strip commit is a single squashed commit built on `03107863`. It's due to be split into smaller
commits, so this plan names it by its subject rather than a hash that will change. When it's split,
the restorations below can go into the split itself rather than landing as fixes afterwards. Between its parent and itself,
several non-AI files went _backwards_. The commit appears to have been produced from a branch cut
from an older main and then laid over current main. It also added genuinely new work that must be
**kept**:

- GitHub and email sign-in, `mailer.rs`, and the `/account` page.
- Account linking and unlinking, `LinkOrSignIn`, and OAuth CSRF `state` for every provider.
- The `email_signin_tokens` migration and its operations, plus the new tests in
  `munibot_core/tests/operations.rs`.
- The CI workflow at `.github/workflows/build-and-test.yml`.
- The temporary chat (`munibot_ai`, `munibot_api/src/chat*`, `munibot_gui/src/{pages,components}/chat*`).
- `enterTest` running `cargo nextest run`, and `cargo-expand` in the devenv packages.

Each check below asks whether a main commit's diff still applies in reverse to `HEAD`. If it does,
the change is present. If its forward diff applies instead, the change was reverted outright:

```bash
# before the split, the strip commit's parent; after it, the commit before its first piece
base=$(git log --format=%H -1 --grep='^chore: strip ai chat features')^
for c in $(git rev-list --no-merges "$base" --max-count=70); do
  s=$(git log -1 --format=%s "$c")
  if git show --format= "$c" | git apply --check -R 2>/dev/null; then p=present
  elif git show --format= "$c" | git apply --check 2>/dev/null; then p=REVERTED
  else p=partial; fi
  echo "$p ${c:0:8} $s"
done
```

"Partial" usually means a later commit touched the same lines, so judge it by reading the file. Read
this way, these are confirmed lost:

| Lost change                                                                         | Original commit(s)                                                                 | Evidence at `HEAD`                                                                             |
| ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `upsert_guild_config` uses `ON DUPLICATE KEY UPDATE`                                | (pre-`40bab589`)                                                                   | `munibot_core/src/db/operations.rs:31` is `replace_into` again                                 |
| `/admin stop-logging` clears one column instead of deleting the row                 | the logging-column fix                                                             | `munibot_discord/src/admin.rs` calls `delete_guild_config`                                     |
| `push_safer` markdown escaping for greeted names                                    | v0.3.3                                                                             | `push_safer.rs` exists but `lib.rs` no longer declares it; `vc_greeter.rs` uses `push_safe`    |
| the 22% to "21.99%" magical special case                                            | v0.3.3                                                                             | `munibot_core/src/magical.rs`                                                                  |
| the 0.3.3 release (version and changelog entry)                                     | `47345357`                                                                         | `Cargo.toml` says `0.3.2`, `CHANGELOG.md` has no v0.3.3                                        |
| `TestDb` connecting only as `munibot_test`                                          | `762c25c6`                                                                         | `munibot_core/tests/common/mod.rs` defaults to `root` again, which devenv no longer creates    |
| `dx-serve` reading `DATABASE_URL` from devenv's dynamic port, and `RUST_LOG=info,…` | `c0e9f7ab`, `aa7b6306`                                                             | `devenv.nix` process block and `env`                                                           |
| the dioxus-cli build-dir v2 patch being applied                                     | `f7585902`, `16858855`                                                             | `nix/patches/dioxus-cli-build-dir-v2.patch` exists, `nix/dioxus-overlay.nix` no longer uses it |
| dioxus `0.8.0-alpha.3` and `moka` in the workspace requirements                     | `a90deb31`, `40bab589`, `fdf35d3e`                                                 | root `Cargo.toml`                                                                              |
| `update_linked_account_tokens`                                                      | `4dae414e`                                                                         | absent from `operations.rs`                                                                    |
| Discord token refresh, shared credentials and client                                | `817f6b3f`, `1b457e8f`, `f0b1a49d`, `e6154019`                                     | `oauth/discord/{credentials,client,token}.rs` are tracked but never declared as modules        |
| Discord 429 retry, `RateLimited` errors, JSON deserialization errors                | `0647654c`, `d9b33f07`, `8bc93d5a`, `f611b4b2`                                     | `oauth/discord/rate_limit.rs` undeclared; error variants gone                                  |
| per-user guild cache, and invalidation at sign-in and sign-out                      | `dcbe2087`, `1ebc2067`, `76764911`                                                 | `oauth/discord/guild_cache.rs` undeclared; `routes.rs` no longer invalidates                   |
| the single-request logging page                                                     | `fc2c98d0`, `710a616c`, `f7fc72c1`, `5c78d4c7`, `f6320888`, `563b3857`, `abbab503` | `logging_page.rs` files undeclared; `server_fns/settings/channels.rs` is back                  |
| settings tracing and import cleanups                                                | `7088b725`, `d82fc7d9`, `72770bcd`                                                 | reverted per the check above                                                                   |
| AGENTS.md import guidance and the MySQL troubleshooting tip                         | `9a2874ec`, plus the MySQL docs consolidation                                      | `AGENTS.md`                                                                                    |
| the dx fat-link and symbol-mangling notes                                           | `16858855`, `38d73b40`                                                             | `docs/notes/dioxus-nix-build-gotchas.md`                                                       |

Because of these files, two docs currently describe code that doesn't run:
`docs/notes/discord-rate-limits.md` describes `guild_cache.rs` and `rate_limit.rs` as live, and
`docs/notes/gui-configuration-research.md` says the guild-config storage bugs "no longer apply". The
restoration below makes the first note true again; commit 1.20 corrects the second.

### How to restore

For each commit below, start from the file as it was at `03107863`
(`git show 03107863:<path>` or `git checkout 03107863 -- <path>`). Then re-apply whatever the strip
intentionally added to that file. Never blanket-checkout a file the strip also extended:
`oauth/routes.rs`, `auth.rs`, `operations.rs`, `models.rs`, `schema.rs`, and
`tests/operations.rs` all need a hand merge.

Build, `cargo clippy --all-features -- -D warnings`, and `devenv test` after every commit.

### 1.1 Restore the whole-row-safe guild config upsert

`munibot_core/src/db/operations.rs`: change `upsert_guild_config` back to
`insert_into(...).on_conflict(DuplicatedKeys).do_update().set(&config)`, along with its original doc
comment explaining why. Add an integration test that upserts twice and asserts the row survives.

```gitcommit
fix(operations): upsert guild configs without replacing the row

restores insert ... on duplicate key update in upsert_guild_config. a
recent squash reverted it to replace into, which deletes and reinserts
the row and nulls every column the caller didn't populate. guild_configs
only holds the logging channel today, but the next guild setting would
be silently erased on every logging save.
```

### 1.2 Restore clearing only the logging column

`munibot_discord/src/admin.rs`: `stop_logging` goes back to upserting `logging_channel: None` instead
of calling `delete_guild_config`.

```gitcommit
fix(admin): clear the logging channel instead of deleting the config row

restores the earlier fix that a squash reverted. deleting the whole guild_configs
row to turn logging off would also wipe any other per-guild setting
stored beside it.
```

### 1.3 Restore safer markdown escaping in greetings

Declare `mod push_safer;` in `munibot_discord/src/lib.rs` again, and switch `vc_greeter.rs` back to
`push_safer(name)`.

```gitcommit
fix(discord): restore safer markdown escaping for greeted names

push_safer.rs was still tracked but no longer declared, so the voice
greeter fell back to serenity's push_safe, which doesn't escape links.
display names are user-controlled, so this is the escaping that matters.
```

### 1.4 Restore the 21.99 percent magic

`munibot_core/src/magical.rs`: put back the `magic_amount_display` match and its comment.

```gitcommit
feat(magical): restore the 21.99 percent special case

reinstates the v0.3.3 easter egg that a recent squash reverted.
```

### 1.5 Restore the 0.3.3 release metadata

Set the workspace version back to `0.3.3`, and restore the v0.3.3 section of `CHANGELOG.md`
verbatim from `03107863`.

```gitcommit
chore(release): restore the 0.3.3 version and changelog entry

0.3.3 was released before a squash rolled the workspace version and the
changelog back to 0.3.2. without this the next release would reuse an
already-published version number.
```

### 1.6 Restore the root-free test fixture

`munibot_core/tests/common/mod.rs`: go back to `03107863`'s version, which manages per-test databases
as `munibot_test` alone through its wildcard grant on `munibot_test_%`. Keep a single
`MUNIBOT_TEST_DB_BASE_URL` override for machines where devenv hands MySQL a different port. Check
that the CI workflow's provisioning step still grants that wildcard.

```gitcommit
test(core): stop using a root db user for test setup again

devenv stopped creating a root mysql user in 31ac54bf, but a squash
brought back a fixture that defaults to connecting as root, so every
integration test fails locally with a connection error. munibot_test's
wildcard grant already covers creating and dropping its own databases.
```

### 1.7 Restore devenv's database url and log level

`devenv.nix`: put back `env.DATABASE_URL` on the `dx-serve` process (built from
`config.processes.mysql.ports.main.value`), and restore `RUST_LOG` to start with `info,`.

```gitcommit
chore(devenv): restore database url from devenv's dynamic port

devenv may move mysql off 3306 when the port is taken; c0e9f7ab pointed
dx serve at whichever port it actually allocated, and a squash undid
it. also restores the info default log level so dependency warnings
aren't hidden.
```

### 1.8 Reapply the dioxus-cli patch

`nix/dioxus-overlay.nix`: restore `old:` and
`patches = (old.patches or [ ]) ++ [ ./patches/dioxus-cli-build-dir-v2.patch ];`, along with the
comment explaining when to drop it.

```gitcommit
fix(nix): reapply the dioxus-cli build-dir v2 patch

the patch file survived a squash but the overlay stopped applying it,
so restarting dx serve without code changes fails with "missing linker
args for fat link" again.
```

### 1.9 Restore the dependency requirements

Root `Cargo.toml`: set `dioxus`, `dioxus-desktop`, and `dioxus-html` back to `0.8.0-alpha.3`, and add
`moka = { version = "0.12", features = ["future"] }`. Add `moka` and `tokio` to `munibot_api`'s
`server` feature with `cargo add`. The lockfile should only change for `moka`.

```gitcommit
build(deps): restore moka and the dioxus alpha.3 requirement

the guild cache needs moka, and the workspace was already built and
locked against dioxus 0.8.0-alpha.3 before a squash lowered the stated
minimum back to alpha.0.
```

### 1.10 Restore the linked account token update operation

`update_linked_account_tokens(pool, linked_account_id, access_token, refresh_token, token_expires_at)`
in `operations.rs`, plus its integration test, merged alongside the strip's `link_account_to_user`.

```gitcommit
feat(operations): restore updating a linked account's tokens

needed by discord token refresh. a squash dropped it while adding the
account linking operations around it.
```

### 1.11 Restore the shared discord client and credentials

Declare `client` and `credentials` in `oauth/discord.rs`. `routes.rs` reads Discord's id and secret
through `Credentials::from_env`, and every Discord call shares the one `reqwest::Client`.

```gitcommit
refactor(oauth): restore the shared client and credentials modules

restores 817f6b3f and e6154019, which a squash left as undeclared files:
one reqwest client reused across discord calls, and one place that
reads the oauth client credentials.
```

### 1.12 Restore Discord's refresh token grant

`refresh_access_token` in `oauth/discord.rs`, and the `Deserialize(serde_json::Error)` variant on
`DiscordOAuthError`.

```gitcommit
feat(discord): restore discord's refresh token grant

restores 1b457e8f and 0647654c. a squash removed the refresh grant and
the json error variant while keeping the files that call them.
```

### 1.13 Restore refreshing tokens before use

Declare `token` and route both `get_guilds` and `require_guild_admin` through
`token::access_token_for`.

```gitcommit
feat(api): restore refreshing expired discord tokens before use

discord access tokens last about a week. without this the dashboard
quietly turns into a sign-in prompt once a token expires, which looks
like a bug rather than an expired session.
```

### 1.14 Restore the discord rate limit retry

Declare `rate_limit`. Restore `DiscordOAuthError::RateLimited`, `SettingsError::RateLimited`, the
`AuthError` mapping, and the `From` impls from `03107863`.

```gitcommit
fix(discord): restore retrying discord requests that hit a rate limit

restores 8bc93d5a, d9b33f07, and f611b4b2. with them gone, a 429 from
discord surfaces as an opaque request failure rather than being retried
or explained.
```

### 1.15 Restore the guild cache

Declare `guild_cache` and serve `require_guild_admin` and `get_guilds` from it. Invalidate the cache
in **every** sign-in callback (Discord, GitHub, and email, since the strip added the latter two) and
in `logout`, not just the Discord path `03107863` knew about.

```gitcommit
feat(api): restore the per-user discord guild cache

restores dcbe2087, 1ebc2067, and 76764911. every settings call was back
to a live discord round trip, and discord rate limits guild listing
aggressively. invalidation now also runs on github and email sign-in,
which didn't exist when the cache was first written.
```

### 1.16 Restore channel sorting and the combined logging wire type

`settings::sort_text_channels`, and `settings/logging_page.rs` declared and exported as
`GuildLoggingPage`.

```gitcommit
refactor(api): restore channel sorting and the logging page wire type

restores fc2c98d0 and f7fc72c1 ahead of the combined server function.
```

### 1.17 Restore the combined logging server function

Declare `server_fns/settings/logging_page.rs`, and delete `server_fns/settings/channels.rs` along with
any server function it superseded.

```gitcommit
refactor(api): restore the single combined logging settings function

restores 710a616c, f6320888, and 563b3857. the logging page needs one
request rather than three, each of which repeated the guild admin
check against discord.
```

### 1.18 Restore the single-request logging page

`munibot_gui/src/pages/guild_settings/logging.rs` goes back to `03107863`'s single-resource version,
along with the doc comment on `GuildSettings`.

```gitcommit
refactor(gui): restore loading the logging page with a single request

restores 5c78d4c7 and abbab503 on top of the combined server function.
```

### 1.19 Restore settings tracing and import cleanups

Restore `7088b725`, `d82fc7d9`, and `72770bcd`: `#[instrument]` on the settings server functions,
the shorter import paths, and Discord token validation living in the `bot` module.

```gitcommit
refactor(api): restore settings tracing and import cleanups

these three refactors were reverted by a squash with no behavioural
reason. restoring them keeps settings server functions traceable per
docs/tracing.md.
```

### 1.20 Restore the reverted guidelines and notes

- `AGENTS.md`: the inline-import guidance and the `devenv tasks run devenv:mysql:configure` tip.
- `docs/notes/dioxus-nix-build-gotchas.md`: the fat-link section and the note on blanket
  `-Csymbol-mangling-version`.
- `docs/notes/gui-configuration-research.md`: undo the claim that the guild-config storage bugs "no
  longer apply", since commits 1.1 and 1.2 fix them for real.
- `docs/gui.md`: delete the "No token refresh" known gap, which commit 1.13 closes.

```gitcommit
docs: restore guidelines and notes a squash reverted

brings back the agents.md import and mysql guidance and the dioxus
build notes, and corrects gui-configuration-research.md now that the
guild config storage fixes are restored rather than merely moot. drops
the token refresh known gap from gui.md, since refresh works again.
```

---

## Part 2 — outstanding non-AI work

These are independent, small, and good warm-ups. Order is only a suggestion.

### 2.1 Cascade linked accounts when a user is deleted

A migration recreating the `linked_accounts.user_id` foreign key with `ON DELETE CASCADE`. AI
milestones 03 and 06 promise that deleting a user removes their conversations and memories. That
promise means nothing while this older foreign key blocks deleting the user in the first place.

```gitcommit
fix(db): cascade linked accounts when their user is deleted

linked_accounts.user_id is the only foreign key on users without
on delete cascade, so a user can't be deleted while any sign-in method
is linked. every later per-user table cascades, so this one should too.
```

### 2.2 Invalidate the autodelete cache on write

`AutoDeleteHandler` loads every timer once at boot (`munibot_discord/src/autodelete.rs:30-53`) and
never re-reads. Add a reload hook that the write path calls, so any future GUI or direct database
write takes effect without a restart.

```gitcommit
fix(autodelete): reload the timer cache after a write

timers are read into memory once at boot, so any write that doesn't go
through this handler is invisible until restart. this is a
prerequisite for ever managing autodelete from the web.
```

### 2.3 Stop resetting the autodelete sweep cursor

`set_autodelete` always writes `last_cleaned: epoch` and `last_message_id_cleaned: 1`, and
`upsert_autodelete_timer` uses `replace_into` (`operations.rs:78`). Editing a timer's duration
therefore restarts its sweep from the beginning of time. Upsert only the settings columns, and leave
the sweep state alone on a settings-only change.

```gitcommit
fix(db): keep the autodelete sweep cursor when a timer changes

editing a timer's duration rewrote its sweep state to the epoch, so the
next clean re-scanned the whole channel history. settings writes now
only touch settings columns.
```

### 2.4 Store quote timestamps in utc

`munibot_twitch/src/handlers/quotes.rs:46` writes `Local::now().naive_local()`, while every other
table stores `Utc::now().naive_utc()`. Decide in the commit body whether existing rows are backfilled.

```gitcommit
fix(quotes): store quote timestamps in utc

every other table stores naive utc; quotes alone stored server-local
time, so the same instant read back differently depending on where the
bot ran. existing rows are left as they are because <decide and say>.
```

### 2.5 Explain an already-linked account on `/account`

The callback redirects to `/account?error=already_linked`, but `munibot_gui/src/pages/account.rs`
never reads the query. Show a friendly banner explaining that the account belongs to a different
munibot user. Delete `AuthError::AlreadyLinkedElsewhere` if nothing ends up constructing it.

```gitcommit
feat(account): explain when a sign-in is already linked elsewhere

linking an account that belongs to another munibot user redirected
here with an error nobody read, so the click appeared to do nothing.
```

### 2.6 Offer every sign-in provider when chat needs a sign-in

`munibot_gui/src/components/chat/turn_failure.rs` hard-codes `/auth/discord/authorize`. Render
`SignInLinks` instead.

```gitcommit
fix(chat): offer every sign-in provider when the session has expired

the banner only offered discord, stranding anyone who signs in with
github or email.
```

### 2.7 Throttle email sign-in requests

`POST /auth/email/request` sends mail to any address with no limit, which makes munibot a free
spamming relay. Limit per address and per client IP in redis, keeping the same response either way so
the endpoint still reveals nothing about which addresses exist.

```gitcommit
fix(oauth): throttle email sign-in requests

anyone could make munibot send unlimited sign-in emails to any
address. the response stays identical whether or not a request was
throttled, so this adds no enumeration signal.
```

### 2.8 Document every environment variable

`.env.example` is missing `GITHUB_OAUTH_CLIENT_ID`, `GITHUB_OAUTH_CLIENT_SECRET`, and every `SMTP_*`
variable. The `environmentFile` description in `nix/nixos.nix` lists only Discord and Twitch.
`secretspec.toml` is the source of truth; mirror it.

```gitcommit
docs: list every supported environment variable

.env.example and the nixos module description had fallen behind
secretspec.toml, so github sign-in, email sign-in, and the ai keys were
undiscoverable without reading code.
```

### 2.9 Correct `docs/gui.md`

`auth/linked_account.rs` is described as server-only, but it is an always-compiled DTO. `web` is
described as the default feature, but both crates have `default = []`.

```gitcommit
docs(gui): correct the module layout and feature notes

fixes two small inaccuracies that send readers looking for things in
the wrong place.
```

## Deliberately deferred

- **Twitch sign-in.** Still a known gap in `docs/gui.md`. It is additive whenever it is wanted.
- **Operator permissions.** There is no permission system yet. AI milestone 04 introduces one,
  including revocation, because affordability is the first thing that needs it.
