# munibot's gui

munibot's binary is a [Dioxus](https://dioxuslabs.com) 0.7 fullstack web app that runs the discord
and twitch bots alongside it, replacing an earlier, abandoned Leptos-based attempt (still visible in
the `.wt/gui-old` worktree, kept only as historical reference).

## Architecture

The `munibot` crate (not `munibot_core`/`munibot_discord`/`munibot_twitch` -- just the top-level
binary crate) compiles to two very different targets, selected by cargo feature:

- **`web`** (default) -- the wasm client, hydrated into server-rendered HTML by dioxus. Only
  cross-platform-safe dependencies (dioxus itself, serde, thiserror) are available here.
- **`server`** -- an axum server that renders the gui, serves dioxus server functions, and spawns
  the discord/twitch bots (`munibot::bot`). Everything native-only -- `munibot_core`,
  `munibot_discord`, `munibot_twitch`, diesel, tokio, axum, the session stack -- is gated behind this
  feature so the wasm build never sees it.

`dx serve` (dev) and `dx bundle --fullstack` (release) both build the client and server halves as
separate cargo invocations, picking the right feature set for each automatically based on which
features enable `dioxus/web` vs `dioxus/server`.

### Module layout

```
munibot/src/
  main.rs           dual entry point: dioxus::launch(App) for web, axum server for server
  lib.rs            module declarations
  bot.rs            (server only) discord/twitch startup, moved here from the old single-purpose main.rs
  app.rs            Route enum, App root, MainLayout
  components.rs     route-aware glue (AccountStatus)
  pages/            route target components (home, dashboard)
  api.rs
  api/
    auth.rs           shared types: UserData, AuthError/AuthResult
    auth/server.rs    (server only) Authentication/HasPermission impls, session User type
    guilds.rs         shared type: GuildSummary
    oauth.rs          (server only) module declaration
    oauth/discord.rs  (server only) discord oauth2 client (token exchange, identity, guilds)
    oauth/routes.rs   (server only) plain axum routes for the oauth dance + logout
    server_fns.rs
    server_fns/auth.rs    get_authenticated_user
    server_fns/guilds.rs  get_guilds
```

`munibot_core` gained a `users` + `linked_accounts` migration and matching models/operations (see
`munibot_core/src/db/{schema,models,operations}.rs`) -- a `users` row is a munibot account; each
`linked_accounts` row links one external provider account (discord for now) to a user. A user can
have multiple linked accounts; a provider account belongs to exactly one user. This shape is meant to
support twitch/github linking later without a schema change.

### A pattern worth knowing: server-only extractor types in shared code

Dioxus 0.7's `#[server(name: Type)]`/`#[post("/path", name: Type)]` macros hoist extractor arguments
(session state, `axum::extract::Extension<T>`, etc.) out of the function signature entirely for the
generated client stub -- the client never sees or calls that part of the function. This means the
_type path_ in the attribute doesn't need to resolve for the client build, even when it references a
crate (like `axum`) or module (like `crate::api::auth::server`) that's entirely absent from the wasm
compilation.

Concretely, this compiles fine on both `web` and `server`:

```rust
#[server(
    auth: crate::api::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guilds() -> AuthResult<Vec<GuildSummary>> { /* server-only body */ }
```

The key is referencing the type by its **full path directly in the attribute**, not via a top-level
`use`. A plain `use crate::api::auth::server::AuthSession;` at the top of the file _would_ fail to
resolve on the wasm client, since `use` items are resolved by rustc regardless of what any macro does
with the name afterward -- unlike the attribute's tokens, which the server-only parts of the macro
expansion simply never look at for the client target.

## Discord OAuth sign-in flow

1. The home page and account status component link to `/auth/discord/authorize` (a plain `<a>` tag,
   not a dioxus `Link` -- this needs a real browser navigation, not a client-side route change).
2. `GET /auth/discord/authorize` (`api/oauth/routes.rs`) redirects to discord's consent screen, with
   `identify guilds` scopes and a `redirect_uri` built from `MUNIBOT_BASE_URL`.
3. Discord redirects back to `GET /auth/discord/callback?code=...`. That handler:
   - exchanges the code for an access/refresh token (`api/oauth/discord.rs::exchange_code`)
   - fetches the discord identity (`get_current_user`)
   - calls `munibot_core::db::operations::get_or_create_user_from_linked_account`, which finds the
     user by `(provider, provider_user_id)` or creates one, refreshing the stored username, tokens,
     display name, and avatar either way (so a repeat sign-in stays in sync with discord)
   - logs the session in (`auth.login_user(user_id)`) and redirects to `/dashboard`
4. `GET /auth/logout` clears the session and redirects home.

Sessions are backed by redis (`axum_session` + `axum_session_auth`, keyed by an opaque cookie), with
`munibot_core`'s diesel pool injected as an `axum::Extension` for anything that needs direct db
access (loading the current user, fetching a linked account's stored token).

The dashboard (`pages/dashboard.rs`) calls the `get_guilds` server function, which loads the caller's
stored discord access token, calls `GET /users/@me/guilds`, and filters to guilds the user owns or
has `MANAGE_GUILD` on (`DiscordGuild::is_administered_by_user`).

### Known gaps (by design, for a minimum product)

- **No token refresh.** Discord access tokens are used as-is until they expire (~7 days); there's no
  refresh-token rotation yet. When a token expires, `get_guilds` will error and the dashboard falls
  back to a sign-in prompt -- signing in again fixes it.
  Tracking: bump `linked_accounts.access_token`/`refresh_token` using `token_expires_at` and the
  stored `refresh_token` when it's close to expiring.
- **No CSRF `state` parameter** on the oauth authorize/callback round trip.
- **No permission system.** `HasPermission` in `api/auth/server.rs` always returns `false`; the old
  gui had a `BotAdmin` concept that's worth revisiting once the gui needs any admin-gated views.
- **Twitch/github sign-in aren't implemented yet**, though the `linked_accounts` schema and the
  `oauth` module structure are meant to make adding them mostly additive: a new `oauth/<provider>.rs`
  client, new routes in a `oauth/routes.rs`-shaped module (or added to the existing one), and reusing
  `get_or_create_user_from_linked_account` with `provider = "twitch"`/`"github"`.

## Local dev workflow

`devenv up` starts mysql, redis, a `tailwind` process (rebuilds `munibot/assets/tailwind.css` on any
source change), and `dx-serve` (`dx serve` from `munibot/`, after mysql+redis are up).

To iterate on the gui without connecting the live discord/twitch bots (so hot-reloads don't
reconnect discord every time), set `MUNIBOT_DISABLE_BOTS=1`. This still runs migrations and the
session/db plumbing -- only `munibot::bot::start` is skipped.

`MUNIBOT_CONFIG_FILE` overrides the `--config-file` clap default, since `dx serve` doesn't forward
CLI args to the server binary it launches.

Required env vars beyond what the bots already need (see `secretspec.toml`): `REDIS_URL` and
`MUNIBOT_BASE_URL` (used to build the oauth `redirect_uri` -- must match what's registered with the
discord application; `http://localhost:8080` in dev).

## Production build

`nix/build.nix` (wired as `outputs.default` in `devenv.nix`) runs `dx bundle --release --fullstack`
across the whole workspace (not just `munibot/` -- see the comments there for why) and installs the
resulting server binary + prebuilt web assets. `nix/nixos.nix`'s `services.munibot` module runs it,
with `createDatabase`/`createRedis` options to auto-provision local mysql/redis (mirroring each
other), and a required `baseUrl` option for `MUNIBOT_BASE_URL`.
