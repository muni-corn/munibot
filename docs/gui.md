# munibot's gui

munibot's binary is a [Dioxus](https://dioxuslabs.com) 0.8 fullstack web app that runs the discord
and twitch bots alongside it, replacing an earlier, abandoned Leptos-based attempt (still visible in
the `.wt/gui-old` worktree, kept only as historical reference).

## Architecture

The gui is split across three crates, each compiling to two very different targets selected by cargo
feature:

- **`munibot_api`** -- the rpc boundary between the gui and its data: wire dtos (`UserData`,
  `GuildSummary`, and the `chat`/`settings`/`pipeline` dto modules), dioxus server functions, and
  (server-only) the oauth2 clients, the plain axum oauth routes, the session/auth glue (`User`,
  `AuthSession`), and the authorization gates (`require_operator`, `require_guild_admin`). Depends
  on `munibot_core` under its `server` feature to translate `munibot_core`'s db models into these
  wire types.
- **`munibot_gui`** -- `App`/`Route`/`components`/`layouts`/`pages`, plus the two entry points:
  `launch_web()` (wasm client) and (server-only) `server::run()`, which builds and serves the whole
  axum app (the dioxus fullstack app, the oauth routes from `munibot_api`, the attachment and github
  webhook routes, the redis-backed session layers, and the `Extension`s carrying the db pool,
  `Option<Arc<Ai>>`, webhook config, and pipeline registry).
- **`munibot`** -- the thin binary. `main.rs` just picks an entry point: `munibot_gui::launch_web()`
  for the wasm client, or (server-only) tracing setup, config, migrations, operator permission sync,
  `ai::build`, `bot::start` (discord/twitch), then `munibot_gui::server::run()`.

`Arc<Ai>` is built in `main.rs` **unconditionally and outside the `MUNIBOT_DISABLE_BOTS` guard**, and
handed to both the bots and the gui server. That ordering is deliberate: the chat page needs the ai
service too, and disabling the bots is the documented local gui workflow (below), so gating the ai
service on the same guard would leave the chat page with nothing behind it.

Each of the three defines the same two features:

- **`web`** (default) -- the wasm client, hydrated into server-rendered HTML by dioxus. Only
  cross-platform-safe dependencies (dioxus itself, serde, thiserror) are available here.
- **`server`** -- pulls in `munibot_core`, diesel, tokio, axum, the session stack, and (in
  `munibot`'s case) `munibot_discord`/`munibot_twitch`. Everything native-only is gated behind this
  feature so the wasm build never sees it.

`web`/`server` forward down the dependency chain (`munibot/web` enables `munibot_gui/web` enables
`munibot_api/web`, same for `server`), so `dx serve` (dev) and `dx bundle --fullstack` (release) only
need to inspect the binary crate's features -- cargo's feature unification does the rest. `munibot_core`
stays native-only (no dioxus dependency at all); its db models are translated into `munibot_api`'s wire
dtos rather than shared directly, since core can't compile to wasm.

### Module layout

```
munibot/src/                (binary)
  main.rs           dual entry point: picks munibot_gui::launch_web() or ::server::run()
  lib.rs            module declarations
  bot.rs            (server only) discord/twitch startup, and the `discord` root tracing span
  ai.rs             (server only) ai::build -- the single place every optional ai capability
                     (tools, memory, delegation, usage, auditing, rate limits, spend caps,
                     abuse detection, moderation, crisis classifier) is opted into
  permissions.rs    (server only) sync_operators, granting Operator from [[operators]] at startup

munibot_gui/src/
  lib.rs            module declarations, launch_web()
  app.rs            Route enum, App root
  layouts.rs        + layouts/home.rs -- shared route layouts
  components.rs     route-aware glue (AccountStatus, nav)
  components/chat/  composer, message list, markdown, persona picker, tool activity,
                     delegation, turn failure
  components/settings.rs
  pages/            route targets: home, dashboard, account, chat (+ sidebar, conversation),
                     memory, personas, usage, transcript, pipelines,
                     guild_settings (+ logging, ai)
  server.rs         (server only) builds/serves the axum app: dioxus fullstack + oauth routes +
                     attachments + github webhooks + redis sessions + extensions
  server/attachments.rs  (server only) attachment fetch route
  server/webhooks.rs     (server only) POST /webhooks/github: signature verification,
                          normalization, trigger matching, dispatch

munibot_api/src/
  lib.rs            module declarations
  auth.rs           shared types: UserData, AuthError/AuthResult
  auth/server.rs    (server only) Authentication/HasPermission impls, session User type -- see
                     docs/notes/permission-system.md for how HasPermission is actually resolved
  auth/operator.rs  (server only) require_operator, the gate every operator-only page uses
  auth/guild.rs     (server only) require_guild_admin, the per-guild counterpart -- a different
                     authority to require_operator, and neither substitutes for the other
  auth/linked_account.rs (server only) linking/unlinking, refusing to remove the last provider
  guilds.rs         shared type: GuildSummary
  chat.rs           + chat/ -- shared chat dtos: conversation, message, event, persona, memory,
                     attachment, transcript, usage (+ usage/breakdown)
  settings.rs       + settings/ -- shared settings dtos: logging, ai, channel, error
  pipeline.rs       shared pipeline monitor dtos
  mailer.rs         (server only) smtp, behind email sign-in
  oauth.rs          (server only) module declaration
  oauth/discord.rs  (server only) discord oauth2 client (token exchange, identity, guilds)
  oauth/discord/bot.rs (server only) bot-token REST calls -- channel listing, which the user's
                     own `identify guilds` scope cannot do
  oauth/github.rs   (server only) github oauth2 client (sign-in, not the github App)
  oauth/email.rs    (server only) email sign-in tokens
  oauth/routes.rs   (server only) plain axum routes for every oauth dance + logout
  server_fns.rs     + server_fns/ -- auth, guilds, chat/*, settings/*, pipeline
```

`munibot_core` gained a `users` + `linked_accounts` migration and matching models/operations (see
`munibot_core/src/db/{schema,models,operations}.rs`) -- a `users` row is a munibot account; each
`linked_accounts` row links one external provider account to a user. A user can have multiple linked
accounts; a provider account belongs to exactly one user. That shape is what let discord, github, and
email sign-in all land without a schema change, and it is why every ai table keys on the internal
`users.id` rather than a platform's own identifier -- see
`docs/notes/gui-configuration-research.md`'s "per-user settings" section for the trap that avoids.

### A pattern worth knowing: server-only extractor types in shared code

Dioxus's `#[server(name: Type)]`/`#[post("/path", name: Type)]` macros hoist extractor arguments
(session state, `axum::extract::Extension<T>`, etc.) out of the function signature entirely for the
generated client stub -- the client never sees or calls that part of the function. This means the
_type path_ in the attribute doesn't need to resolve for the client build, even when it references a
crate (like `axum`) or module (like `crate::auth::server`) that's entirely absent from the wasm
compilation.

Concretely, this compiles fine on both `web` and `server`, inside `munibot_api/src/server_fns/guilds.rs`:

```rust
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guilds() -> AuthResult<Vec<GuildSummary>> { /* server-only body */ }
```

The key is referencing the type by its **full path directly in the attribute**, not via a top-level
`use`. A plain `use crate::auth::server::AuthSession;` at the top of the file _would_ fail to
resolve on the wasm client, since `use` items are resolved by rustc regardless of what any macro does
with the name afterward -- unlike the attribute's tokens, which the server-only parts of the macro
expansion simply never look at for the client target.

This same rule is why the asset pipeline is crate-scoped, too: `munibot_gui/src/app.rs`'s
`asset!("/assets/tailwind.css")` (manganis) resolves relative to whichever crate the macro is
invoked in, so the tailwind input, its `@source` scan, and the generated output all live in
`munibot_gui/`, not the `munibot` binary crate that actually gets served.

## Sign-in

Three providers reach the same internal `users.id`: **discord** and **github** through oauth, and
**email** through a signed token link (`munibot_api/src/oauth/email.rs`, disabled entirely when
`SMTP_HOST` is unset). All three funnel through `get_or_create_user_from_linked_account` on the
`(provider, provider_user_id)` unique index, so which door someone comes through never changes who
they are. `/account` lists a signed-in user's linked providers and can unlink one, refusing to remove
the last remaining sign-in method (`munibot_api/src/auth/linked_account.rs`).

The discord flow below is the worked example; github follows the same shape against
`oauth/github.rs`.

### Discord OAuth flow

1. The home page and account status component link to `/auth/discord/authorize` (a plain `<a>` tag,
   not a dioxus `Link` -- this needs a real browser navigation, not a client-side route change).
2. `GET /auth/discord/authorize` (`munibot_api/src/oauth/routes.rs`) generates a CSRF `state` token,
   stashes it in the (already-cookied, pre-login) session, and redirects to discord's consent screen
   with it, `identify guilds` scopes, and a `redirect_uri` built from `MUNIBOT_BASE_URL`.
3. Discord redirects back to `GET /auth/discord/callback?code=...&state=...`. That handler:
   - verifies `state` against the session's stashed value first, refusing (and clearing it either
     way) before touching discord's api at all if it's missing or doesn't match
   - exchanges the code for an access/refresh token (`munibot_api/src/oauth/discord.rs::exchange_code`)
   - fetches the discord identity (`get_current_user`)
   - calls `munibot_core::db::operations::get_or_create_user_from_linked_account`, which finds the
     user by `(provider, provider_user_id)` or creates one, refreshing the stored username, tokens,
     display name, and avatar either way (so a repeat sign-in stays in sync with discord)
   - logs the session in (`auth.login_user(user_id)`) and redirects to `/dashboard`
4. `GET /auth/logout` clears the session and redirects home.

Sessions are backed by redis (`axum_session` + `axum_session_auth`, keyed by an opaque cookie), with
`munibot_core`'s diesel pool injected as an `axum::Extension` for anything that needs direct db
access (loading the current user, fetching a linked account's stored token).

The dashboard (`munibot_gui/src/pages/dashboard.rs`) calls the `get_guilds` server function, which loads the caller's
stored discord access token, calls `GET /users/@me/guilds`, and filters to guilds the user owns or
has `MANAGE_GUILD` on (`DiscordGuild::is_administered_by_user`).

### Known gaps

- **No token refresh.** Discord access tokens are used as-is until they expire (~7 days); there's no
  refresh-token rotation yet. When a token expires, `get_guilds` will error and the dashboard falls
  back to a sign-in prompt -- signing in again fixes it.
  Tracking: bump `linked_accounts.access_token`/`refresh_token` using `token_expires_at` and the
  stored `refresh_token` when it's close to expiring. Planned as commit 248 in
  `docs/plans/ai/milestone-7-projects.md`, and worth pulling forward before any long stretch of
  testing through the gui.
- **Twitch sign-in isn't implemented yet** (discord, github, and email all are), though the
  `linked_accounts` schema and the `oauth` module structure are meant to make adding it mostly
  additive: a new `oauth/twitch.rs` client, new routes added to `oauth/routes.rs`, and reusing
  `get_or_create_user_from_linked_account` with `provider = "twitch"`.
- **`linked_accounts.user_id` has no `ON DELETE CASCADE`.** Deleting a `users` row would orphan its
  links. Unlinking the last provider is already refused, so this is not reachable today, but the
  constraint is still missing -- see `docs/notes/gui-configuration-research.md`.

## Local dev workflow

`devenv up` starts mysql, redis, a `tailwind` process (rebuilds `munibot_gui/assets/tailwind.css` on
any change under `munibot_gui/`), and `dx-serve` (`dx serve` from `munibot/`, after mysql+redis are
up). `dx serve` still runs from `munibot/` -- it's the binary crate dx builds and serves -- even
though the components/routing/assets it renders now live in `munibot_gui/`.

To iterate on the gui without connecting the live discord/twitch bots (so hot-reloads don't
reconnect discord every time), set `MUNIBOT_DISABLE_BOTS=1`. This still runs migrations and the
session/db plumbing -- only `munibot::bot::start` is skipped.

`MUNIBOT_CONFIG_FILE` overrides the `--config-file` clap default, since `dx serve` doesn't forward
CLI args to the server binary it launches.

Required env vars beyond what the bots already need (see `secretspec.toml`): `REDIS_URL` and
`MUNIBOT_BASE_URL` (used to build the oauth `redirect_uri` -- must match what's registered with the
discord application; `http://localhost:8080` in dev).

## Production build

`nix/build.nix` (wired as `outputs.default` in `devenv.nix`) pre-generates `munibot_gui/assets/tailwind.css`
(manganis validates the `asset!()` reference at compile time), then runs `dx bundle --release
--fullstack` from `munibot/` across the whole workspace (not just `munibot/` -- see the comments
there for why, now that `munibot` path-depends on `munibot_gui`/`munibot_api` too) and installs the
resulting server binary + prebuilt web assets. `nix/nixos.nix`'s `services.munibot` module runs it,
with `createDatabase`/`createRedis` options to auto-provision local mysql/redis (mirroring each
other), and a required `baseUrl` option for `MUNIBOT_BASE_URL`.
