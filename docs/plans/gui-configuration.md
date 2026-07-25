# GUI configuration provider system

Status: planned, not started. Supersedes the earlier key-value draft of this
document.

This plan adds a `ConfigurationProvider` system so munibot's Dioxus GUI can
configure per-Discord-guild settings, with Twitch and GitHub as additive future
scopes. It is written to be self-sufficient: section 3 records every codebase
fact needed to execute it.

---

## 1. Decisions

| Decision              | Choice                                              | Rejected alternatives                                                                                                                                           |
| --------------------- | --------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Schema types location | New wasm-safe leaf crate `munibot_config`           | Server-gated module in `munibot_api` (creates a dependency cycle); feature-gated module in `munibot_core` (puts diesel/tokio one flag away from the wasm build) |
| Schema authoring      | `#[derive(Configurable)]` proc macro                | Fluent builder API (verbose, and schema drifts from storage); declarative macro                                                                                 |
| Field semantics       | Semantic newtypes, attributes for presentation only | Primitive fields plus `#[config(kind = ...)]` attributes on every field                                                                                         |
| Derive host struct    | Dedicated config structs                            | Deriving directly on diesel models (see section 4.2)                                                                                                            |
| Storage               | Typed columns, one table per provider               | Generic key-value rows; one JSON blob per provider                                                                                                              |
| Global TOML config    | Out of scope for v1                                 | Read-only display; migrating it into the database                                                                                                               |
| First providers       | Logging, then autodelete                            | Either alone                                                                                                                                                    |

Two consequences worth stating up front:

- The derive gives provider authors full static types, autocomplete, and
  exhaustiveness. It **cannot** give the wasm client static types — the client
  is a generic renderer by construction. That is the price of having a provider
  trait at all; the alternative is hand-writing a Dioxus form per provider,
  which defeats the purpose.
- Typed columns plus dedicated config structs means a hand-written row-to-config
  mapping per provider. That is fine **provided the mapping uses exhaustive
  struct literals**. See section 2.

---

## 2. Why typed columns plus dedicated config structs is coherent

The config struct and the table are separate types joined by a hand-written
mapping. Drift is prevented at compile time, not at runtime:

| Layer boundary                  | Enforced by                                           |
| ------------------------------- | ----------------------------------------------------- |
| GUI schema from config struct   | the `Configurable` derive                             |
| config struct from database row | exhaustive struct literal in the mapping              |
| database row from table         | diesel `check_for_backend` plus generated `schema.rs` |

Add a field to `LoggingConfig` and the compiler refuses to build until the
mapping handles it.

**Load-bearing convention:** never use `..Default::default()` in a
row-to-config or config-to-row mapping. Doing so silently disables the
compiler's exhaustiveness check and reintroduces drift. This belongs in
`AGENTS.md` once the first provider lands.

---

## 3. Codebase facts

### 3.1 Workspace

Six crates, `resolver = "3"`, edition 2024, MySQL, Dioxus 0.8.0-alpha.0
fullstack.

```
munibot          thin binary; picks an entry point, starts bots, then the gui
munibot_core     config, diesel db layer, errors  (server only)
munibot_discord  poise/serenity integration       (server only, needs nightly)
munibot_twitch   twitch-irc/helix integration     (server only)
munibot_api      rpc boundary: wire dtos, #[server] fns, oauth, session auth
munibot_gui      App, Route, components, layouts, pages, axum server
```

Feature flags cascade `munibot` to `munibot_gui` to `munibot_api`:
`web` (default, wasm) / `server` (axum, diesel, redis, bots) / `desktop`
(scaffolding only, no entry point implemented). This keeps `munibot_core` out
of the wasm build. See `docs/gui.md:23-35`.

`munibot_discord` requires **nightly**: `#![feature(never_type)]`
(`munibot_discord/src/lib.rs:1`), `Duration::from_mins`/`from_hours`
(`autodelete.rs:27-28`), `floor_char_boundary` (`logging.rs:778`).
`devenv.nix:104` sets `channel = "nightly"`.

### 3.2 Entry points and startup order

`munibot/src/main.rs` under `#[tokio::main(flavor = "multi_thread", worker_threads = 8)]`:

1. `dotenvy::dotenv()` (`main.rs:29`)
2. tracing subscriber
3. clap `Args` (`main.rs:11-17`), overridden by `MUNIBOT_CONFIG_FILE` because
   `dx serve` does not forward CLI args (`main.rs:40`)
4. `Config::read_or_write_default_from` (`main.rs:41`)
5. `run_pending_migrations()` (`main.rs:48`)
6. `munibot::bot::start(config).await` unless `MUNIBOT_DISABLE_BOTS` is set
   (`main.rs:53-58`)
7. `munibot_gui::server::run().await` (`main.rs:61`)

`bot::start` must be **awaited, not spawned** — Twitch setup holds a non-`Sync`
handler collection across an `.await` (`munibot/src/bot.rs:99-105`).

Web entry point: `munibot_gui::launch_web()` (`munibot_gui/src/lib.rs:24`).

### 3.3 The GUI as it stands

- App root `App` (`munibot_gui/src/app.rs:10`); `Route` enum
  (`app.rs:35-48`) is the **only** global state. No `AppState`, no
  `use_context`, no signals, no message bus.
- Layouts: `MainLayout` (`app.rs:19`), `HomeLayout`
  (`layouts/home.rs:7`), both rendering `Outlet::<Route>`.
- Total state hooks in the entire codebase: two `use_resource` calls —
  `components.rs:17` and `pages/dashboard.rs:9`. Zero signals.
- **Zero forms.** No `input`, `select`, `textarea`, `form`, `onclick`, or
  `oninput` anywhere. Every interactive element is a `Link` or an `<a href>`.
  The GUI is currently entirely read-only, with no mutating server function.
- `Dashboard` (`pages/dashboard.rs:9-41`) renders a nested `Outlet` at
  `dashboard.rs:20` but `Route` declares **no child routes** under it, so it
  renders nothing. `GuildRow` (`dashboard.rs:58-77`) is styled `cursor-pointer`
  but wraps no `Link`. The scaffolding is shaped for exactly this plan.
- `Dashboard` treats `Some(Err(_))` as "not signed in", swallowing all errors
  including expired tokens and database failures. Config pages must not repeat
  this.

Conventions to respect:

- Dioxus `Link` for router routes; plain `a { href }` for `/auth/...`, because
  those are axum routes needing a real browser navigation
  (`munibot_gui/src/components.rs:9-14`).
- In `#[server]` attributes, reference server-only extractor types by **full
  path inside the attribute**, never via a top-level `use`. `use` items are
  resolved by rustc regardless of the macro, so `use axum::extract::Extension`
  breaks the wasm build (`docs/gui.md:72-95`, and see the comment at
  `munibot_api/src/server_fns/guilds.rs:3-16`).
- `asset!()` resolves relative to the crate invoking it, which is why the
  Tailwind pipeline lives in `munibot_gui` (`docs/gui.md:97-100`).
- Styling is Tailwind v4 plus daisyUI 5, two themes defined inline in
  `munibot_gui/tailwind.css` (`munibot` dark default, `munibot-light`).
- `.wt/gui-old/` is the abandoned Leptos attempt, historical reference only.
  Do not treat it as live code.

### 3.4 Database layer

- `DbPool = bb8::Pool<AsyncDieselConnectionManager<AsyncMysqlConnection>>`
  (`munibot_core/src/db.rs:18`); `establish_pool()` reads `DATABASE_URL`
  (`db.rs:25-33`).
- Three **independent** pools are created: `munibot_discord/src/lib.rs:54`,
  `munibot/src/bot.rs:61` (twitch), `munibot_gui/src/server.rs:27` (gui).
  Nothing is shared between them.
- `diesel.toml` prints schema to `munibot_core/src/db/schema.rs`; migrations
  live in `/migrations` at the workspace root.
- MySQL has no `RETURNING`; `last_insert_id()` is declared at
  `munibot_core/src/db/operations.rs:21`.
- Existing upserts use `REPLACE INTO` (`operations.rs:31`, `operations.rs:78`),
  which **deletes then inserts**, nulling unspecified columns. The doc comment
  at `operations.rs:26-28` acknowledges this. It must not be used for the new
  multi-column config tables.

### 3.5 There is no GUI-to-bot channel

The GUI and the bots are independent subsystems in one process communicating
**only through MySQL**. There is no shared memory, channel, or message bus, and
no cache invalidation signal. This works today only because the bots re-read
rows per use, for example `get_logging_channel_for_guild`
(`munibot_discord/src/handlers/logging.rs:724-735`) which queries fresh on
every event.

`AutoDeleteHandler` is the exception: it loads every timer into a `HashMap` at
boot (`munibot_discord/src/autodelete.rs:31-53`) and never re-reads. A raw
database write from the GUI would be invisible to it. This is what the
`on_changed` hook exists to solve.

### 3.6 Handler registration

The single wiring point is `munibot/src/bot.rs:28-52`, which builds two vectors
by hand:

```rust
let discord_handlers: DiscordMessageHandlerCollection = vec![ /* Arc<Mutex<dyn ...>> */ ];
let discord_command_providers: DiscordCommandProviderCollection = vec![ /* Box<dyn ...> */ ];
```

Relevant traits:

- `DiscordEventHandler` (`munibot_discord/src/handler.rs:9-18`), collection
  alias at `state.rs:16`, dispatched for every event at `lib.rs:131-158`.
- `DiscordCommandProvider` (`munibot_discord/src/commands.rs:10-12`),
  collection alias at `commands.rs:33`.
- `TwitchMessageHandler` (`munibot_twitch/src/handler.rs:12-36`).
- `AdminCommandProvider` is always appended regardless of the provider list
  (`munibot_discord/src/lib.rs:63-64`).

`GlobalAccess { db, http: Arc<Http>, cache: Arc<Cache> }`
(`munibot_discord/src/state.rs:18-45`) is the data-access handle, constructed
in `on_ready` (`lib.rs:106-129`) and stored in `DiscordState`
(`state.rs:47-54`).

### 3.7 Authentication

- `AuthSession` (`munibot_api/src/auth/server.rs:12`), session `User`
  (`server.rs:15-21`), client-safe `UserData` (`munibot_api/src/auth.rs:9-13`).
  The internal `i64` user id and all OAuth tokens never cross to the client.
- Sessions live in Redis, table name `munibot_sessions`
  (`munibot_gui/src/server.rs:37`), keyed by an opaque cookie.
- OAuth routes: `/auth/discord/authorize`, `/auth/discord/callback`,
  `/auth/logout` (`munibot_api/src/oauth/routes.rs:22-27`).
- **Scopes are `identify guilds`** (`oauth/discord.rs:11`). This does **not**
  permit `GET /guilds/{id}/channels`.
- `MANAGE_GUILD = 0x20` (`oauth/discord.rs:144`);
  `DiscordGuild::is_administered_by_user` (`oauth/discord.rs:158-174`).
  Currently used only as a display filter in `get_guilds`
  (`munibot_api/src/server_fns/guilds.rs:36`).
- `HasPermission::has` always returns `false`
  (`munibot_api/src/auth/server.rs:63-69`). There is no permission system.
- Known gaps (`docs/gui.md:125-139`): no token refresh (tokens expire after
  roughly seven days; `refresh_token` and `token_expires_at` are stored but
  never used), no CSRF `state` parameter, no Twitch or GitHub sign-in.
- Type mismatch at the boundary: `GuildSummary.id` is a `String`
  (`munibot_api/src/guilds.rs:7`) while `guild_id` columns are `i64`.

### 3.8 Existing settings surfaces

| Scope                 | Storage             | Read                                                                   | Write                                      |
| --------------------- | ------------------- | ---------------------------------------------------------------------- | ------------------------------------------ |
| Guild logging channel | `guild_configs`     | `logging.rs:724-735`                                                   | `admin.rs:65-70`, delete at `admin.rs:108` |
| Channel autodelete    | `autodelete_timers` | boot load `autodelete.rs:31-53`                                        | `autodelete.rs:55-107`, `:111-135`         |
| Global bot config     | TOML file           | `Config::read_or_write_default_from` (`munibot_core/src/config.rs:37`) | none; only writes the default              |

Models: `GuildConfig` (`models.rs:15-18`), `AutoDeleteTimerRow`
(`models.rs:26-33`). `AutoDeleteMode` (`munibot_discord/src/autodelete.rs:226-253`)
derives `EnumString` plus `poise::ChoiceParameter` and hand-serializes via
`to_db_str`/`from_db_str`; unknown strings fall back to `AfterSilence`.

All guild settings today are Discord-slash-command only, gated on
`required_permissions = "MANAGE_GUILD"` (`admin.rs:28`). That gate is the
precedent the GUI must mirror.

The global TOML config has **no save path**, and `nix/nixos.nix:103` generates
it read-only in production. Hence global scope is out of scope for v1.

There is **no GitHub integration** anywhere in the Rust code. `linked_accounts`
uses `provider VARCHAR(32)` so adding providers is additive.

### 3.9 Environment and development

`devenv up` starts mysql, redis, a `tailwind` watcher, and `dx-serve`
(port 8080) under `secretspec run`.

```bash
devenv shell <command>                    # run inside the dev environment
SECRETSPEC_REASON="running tests" devenv shell cargo test
```

Required env: `DATABASE_URL`, `REDIS_URL`, `MUNIBOT_BASE_URL`,
`DISCORD_APPLICATION_ID`, `DISCORD_CLIENT_SECRET`, plus `DISCORD_TOKEN`,
`TWITCH_TOKEN`, `TWITCH_CLIENT_ID` for the bots. `MUNIBOT_DISABLE_BOTS`
skips bot startup, which is useful for GUI work.

Database test harness: `munibot_core/tests/common/mod.rs` — `TestDb`
(`:32-35`) creates a random `munibot_test_<12 chars>` database, runs embedded
migrations, and drops it on `Drop` (`:84-96`). Uses `127.0.0.1` rather than
`localhost` to force TCP. Existing guild-config tests are at
`munibot_core/tests/operations.rs:16-95` and must be updated when
`guild_configs` is dropped.

### 3.10 File-size guideline

`AGENTS.md` caps files at 500 lines. Already over: `logging.rs` (1261),
`autodelete.rs` (631), `operations.rs` (562). **Put all new code in new files.**
Suggested layout:

```
munibot_core/src/config.rs              module declarations
munibot_core/src/config/provider.rs     ConfigurationProvider
munibot_core/src/config/dyn_provider.rs DynConfigProvider plus blanket impl
munibot_core/src/config/registry.rs     ConfigRegistry
munibot_core/src/config/context.rs      ConfigContext, ConfigError
munibot_core/src/config/collection.rs   collection_diff helper
munibot_core/src/db/operations/         split new per-provider operations here
munibot_discord/src/config.rs           module declarations
munibot_discord/src/config/logging.rs   LoggingConfig plus provider
munibot_discord/src/config/autodelete.rs
munibot_discord/src/config/refs.rs      channel and role option lookup
```

---

## 4. Architecture

### 4.1 Crate graph

```
munibot_config_derive  (proc macro)
        ↑
munibot_config   (serde, serde_json, thiserror -- wasm safe)
        ↑                          ↑
munibot_core                  munibot_api  (re-exports wire types for the gui)
        ↑
munibot_discord / munibot_twitch  (implement providers)
```

The schema types cannot live in `munibot_api`: its `server` feature depends on
`munibot_core`, so `munibot_core` depending back on it is a cycle. Nor can the
trait live in `munibot_api`, because the registry would force
`munibot_api` to depend on `munibot_discord` while `munibot_discord` depends on
`munibot_api`.

`munibot_config_derive` is re-exported from `munibot_config` behind a `derive`
feature, serde-style. `syn` and `quote` are already in the tree transitively via
diesel, poise, and serde derive, so compile-time cost is marginal.

### 4.2 Why not derive on diesel models

The models make the case themselves:

- `AutoDeleteTimerRow` (`models.rs:26-33`) has six fields; **two are settings**.
  `channel_id` and `guild_id` are the scope key; `last_cleaned` and
  `last_message_id_cleaned` are sweep bookkeeping. Four of six would need
  `#[config(skip)]`.
- `GuildConfig` (`models.rs:15-18`) has two fields, one setting.
- Database types carry almost no useful semantics: `logging_channel: Option<i64>`
  gives no hint whether it is a channel, role, user, or plain number; `mode:
String` gives no hint whether it is free text or a choice. Every field would
  need an attribute anyway, so the saving is small.
- The model would serve two masters, table shape and form shape, which diverge
  as soon as a setting is grouped or computed.

---

## 5. The `munibot_config` crate

### 5.1 Scope and descriptor

```rust
pub enum ConfigScope {
    DiscordGuild { guild_id: String },     // text, matching GuildSummary.id
}

pub struct ProviderDescriptor {
    pub id: String,                         // "logging", "autodelete"
    pub label: String,
    pub description: Option<String>,
    pub icon: Option<String>,               // phosphor class name
    pub scopes: Vec<ScopeKind>,
}
```

v1 ships `DiscordGuild` only. `TwitchChannel` and `GitHubRepo` are additive
because each provider declares its supported scopes and each provider owns its
own tables.

### 5.2 Paths, values, and patches

```rust
pub struct ConfigPath(Vec<PathSegment>);        // Display/FromStr: "timers[6437].lifetime"
pub enum PathSegment { Field(String), Key(String) }

pub enum ConfigValue {
    Unset, Bool(bool), Int(i64), Float(f64), Text(String),
    Duration(u64),                              // seconds
    Choice(String),                             // option id
    MultiChoice(Vec<String>),
    Ref(String),                                // snowflake as text
    List(Vec<ConfigValue>),
}

pub type ConfigValues = BTreeMap<ConfigPath, ConfigValue>;

pub struct ConfigChange { pub path: ConfigPath, pub value: ConfigValue }
pub struct ConfigPatch  { pub changes: Vec<ConfigChange> }
pub struct FieldError   { pub path: ConfigPath, pub message: String }
```

Values travel as a **flat map**, never as a nested object. This makes saves
patches, which buys partial writes, per-field error reporting, and trivial
dirty tracking in the GUI.

### 5.3 Schema tree

```rust
pub enum ConfigNode {
    Section { id: String, label: String, description: Option<String>,
              toggle: Option<Field>, children: Vec<ConfigNode> },
    Collection { id: String, label: String, key: KeyKind,
                 add_label: String, item: Vec<ConfigNode> },
    Field(Field),
    Note { markdown: String },
}

pub struct Field {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: FieldKind,
    pub default: ConfigValue,
    pub required: bool,
    pub visible_when: Option<Condition>,
}

pub enum FieldKind {
    Bool,
    Int { min: Option<i64>, max: Option<i64>, step: i64, unit: Option<String> },
    Text { multiline: bool, max_len: Option<usize>, placeholder: Option<String> },
    Duration { min: Option<u64>, max: Option<u64> },
    Choice { options: Vec<ChoiceOption> },
    MultiChoice { options: Vec<ChoiceOption> },
    Ref { kind: RefKind },
    List { item: Box<FieldKind>, max_items: Option<usize> },
    Color, Url,
}

pub enum RefKind { DiscordChannel, DiscordRole, DiscordUser }
pub enum KeyKind { DiscordChannel, DiscordRole }
```

`Collection` is the key trick: autodelete has one row per channel, but the
schema stays static and the value map carries the keys
(`timers[6437].lifetime`). No async schema generation, and no separate channel
scope in the URL.

`Ref` options are deliberately **not** in the schema — a guild's channel list is
dynamic. They are fetched separately (section 7).

### 5.4 The `ConfigField` trait: semantics in the type

```rust
pub trait ConfigField: Sized {
    fn field_kind() -> FieldKind;
    fn to_value(&self) -> ConfigValue;
    fn from_value(v: &ConfigValue) -> Result<Self, ValueError>;
}
```

Implemented for `bool`, `i64`, `f64`, `String`, `Option<T: ConfigField>`,
`Vec<T: ConfigField>`, plus semantic newtypes:

```rust
pub struct ChannelRef(u64);      // -> FieldKind::Ref { DiscordChannel }
pub struct RoleRef(u64);
pub struct UserRef(u64);
pub struct SettingDuration(Duration);
pub struct Toggled<T> { pub enabled: bool, #[serde(flatten)] pub settings: T }
```

Plus `ConfigChoice` (derived on enums, supplying `ChoiceOption`s) and
`ConfigKey` (for collection map keys).

This is the crux: **semantics live in the type, presentation lives in the
attribute**. Attributes carry only labels, descriptions, and constraints. It
composes the way `schemars`, `clap`, and `validator` do.

### 5.5 The `Configurable` trait

```rust
pub trait Configurable: Serialize + DeserializeOwned + Default + Sized {
    fn config_schema() -> ConfigSchema;                    // derived
    fn validate(&self) -> Result<(), Vec<FieldError>>;     // derived from constraints

    // provided, written once
    fn to_values(&self) -> ConfigValues;
    fn with_patch(&self, patch: &ConfigPatch) -> Result<Self, Vec<FieldError>>;
}
```

`to_values` and `with_patch` move data through `serde_json`:

```
ConfigValues --unflatten--> serde_json::Value --from_value--> Self
Self --to_value--> serde_json::Value --flatten--> ConfigValues
```

So the macro only generates the schema and the constraint checks; serde does all
the data movement. Use `serde_path_to_error` (pure Rust, wasm safe) to recover
per-field paths from deserialization failures.

---

## 6. The derive macros

### 6.1 Type-driven mappings

| Rust type                                 | Generated node                             |
| ----------------------------------------- | ------------------------------------------ |
| `bool`, `i64`, `String`                   | `Field` with the corresponding `FieldKind` |
| `ChannelRef`, `RoleRef`, `UserRef`        | `Field { kind: Ref { .. } }`               |
| `SettingDuration`                         | `Field { kind: Duration { .. } }`          |
| enum with `#[derive(ConfigChoice)]`       | `Field { kind: Choice { .. } }`            |
| `Option<T>`                               | the `T` field, clearable                   |
| `Vec<T>`                                  | `Field { kind: List { .. } }`              |
| nested struct with `Configurable`         | `Section`                                  |
| `Toggled<T>`                              | `Section` with `toggle` set                |
| `BTreeMap<K: ConfigKey, V: Configurable>` | `Collection`                               |

`Toggled<T>` removes the stringly-typed `visible_when` for the common
enable-or-disable case. Where an explicit condition is still needed,
`#[config(visible_when = "sibling_field")]` should be validated by the macro to
name a real sibling at compile time.

### 6.2 Attributes

Container: `id`, `label`, `description`, `icon`, `scope`.
Field: `label`, `description`, `min`, `max`, `step`, `unit`, `default`,
`placeholder`, `multiline`, `max_len`, `add_label` (collections),
`visible_when`, `skip`.

### 6.3 Config structs for v1

```rust
#[derive(Configurable, Serialize, Deserialize, Default)]
#[config(id = "logging", label = "server logging", scope = DiscordGuild)]
pub struct LoggingConfig {
    #[config(label = "enable logging",
             description = "post an audit trail of server events to a channel.")]
    pub logging: Toggled<LoggingSettings>,
}

#[derive(Configurable, Serialize, Deserialize, Default)]
pub struct LoggingSettings {
    #[config(label = "log channel")]
    pub channel: Option<ChannelRef>,
}

#[derive(Configurable, Serialize, Deserialize, Default)]
#[config(id = "autodelete", label = "autodelete", scope = DiscordGuild)]
pub struct AutoDeleteConfig {
    #[config(label = "autodelete timers", add_label = "add a channel")]
    pub timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
}

#[derive(Configurable, Serialize, Deserialize)]
pub struct ChannelAutoDelete {
    #[config(label = "message lifetime", min = "1h", default = "24h")]
    pub lifetime: SettingDuration,
    #[config(label = "cleanup mode")]
    pub mode: AutoDeleteMode,
}
```

`AutoDeleteMode` gains `#[derive(ConfigChoice)]` alongside its existing
`EnumString` and `poise::ChoiceParameter` derives.

Note that logging gains an explicit `enabled` toggle. Today the off switch is
"clear the channel", which loses the setting; an explicit toggle is better UX
and costs one boolean column.

`min = "1h"` must equal `AutoDeleteHandler::MINIMUM_TIMER_DURATION`
(`autodelete.rs:28`). Keep them in sync or reference the constant.

---

## 7. Provider traits

`ConfigurationProvider` carries an associated type, so it is **not object
safe**. Providers implement the typed trait; the registry stores an erased
facade; a blanket impl bridges them.

```rust
// munibot_core/src/config/provider.rs
#[async_trait]
pub trait ConfigurationProvider: Send + Sync + 'static {
    /// The struct describing this provider's settings.
    type Config: Configurable + Send + Sync;

    fn descriptor(&self) -> ProviderDescriptor;

    /// Loads persisted settings for `ctx.scope`, or defaults if unset.
    async fn read(&self, ctx: &ConfigContext) -> Result<Self::Config, ConfigError>;

    /// Persists `desired`. `previous` is supplied so collection-backed providers
    /// can issue a minimal set of inserts, updates, and deletes.
    async fn write(&self, ctx: &ConfigContext, previous: &Self::Config,
                   desired: &Self::Config) -> Result<(), ConfigError>;

    /// Runs after a successful write. Override to invalidate in-memory caches.
    async fn on_changed(&self, _ctx: &ConfigContext, _config: &Self::Config)
        -> Result<(), ConfigError> { Ok(()) }

    /// Candidate values for `Ref` fields (channel and role pickers).
    async fn ref_options(&self, _ctx: &ConfigContext, _kind: RefKind)
        -> Result<Vec<RefOption>, ConfigError> { Ok(Vec::new()) }
}
```

```rust
// munibot_core/src/config/dyn_provider.rs
#[async_trait]
pub trait DynConfigProvider: Send + Sync + 'static {
    fn descriptor(&self) -> ProviderDescriptor;
    fn schema(&self) -> ConfigSchema;
    async fn load_values(&self, ctx: &ConfigContext) -> Result<ConfigValues, ConfigError>;
    async fn apply_patch(&self, ctx: &ConfigContext, patch: &ConfigPatch)
        -> Result<ConfigValues, ConfigError>;
    async fn ref_options(&self, ctx: &ConfigContext, kind: RefKind)
        -> Result<Vec<RefOption>, ConfigError>;
}

#[async_trait]
impl<P: ConfigurationProvider> DynConfigProvider for P {
    fn descriptor(&self) -> ProviderDescriptor { ConfigurationProvider::descriptor(self) }
    fn schema(&self) -> ConfigSchema { P::Config::config_schema() }

    async fn load_values(&self, ctx: &ConfigContext) -> Result<ConfigValues, ConfigError> {
        Ok(self.read(ctx).await?.to_values())
    }

    async fn apply_patch(&self, ctx: &ConfigContext, patch: &ConfigPatch)
        -> Result<ConfigValues, ConfigError> {
        let previous = self.read(ctx).await?;
        let desired  = previous.with_patch(patch)?;   // merge, deserialize, validate
        self.write(ctx, &previous, &desired).await?;
        self.on_changed(ctx, &desired).await?;
        Ok(desired.to_values())
    }
    // ...
}
```

Supporting types:

```rust
pub struct ConfigContext { pub db: DbPool, pub scope: ConfigScope, pub actor: Option<i64> }
pub struct ConfigRegistry { providers: Vec<Arc<dyn DynConfigProvider>> }
pub struct RefOption { pub id: String, pub label: String, pub group: Option<String> }
```

`ConfigRegistry` is built in `munibot/src/bot.rs` next to the existing handler
vectors, then injected into the axum router as an `Extension`, exactly like
`DbPool`.

Providers therefore write only: `descriptor`, `read`, `write`, and optionally
`on_changed` and `ref_options`. Schema generation, patch merging, validation,
and error paths are all derived or blanket.

### 7.1 Collections need a three-way diff

`AutoDeleteConfig::timers` is a map, so `write` must compute added, changed, and
removed rows. That is why `write` receives `previous`. A
`collection_diff(previous, desired) -> CollectionDelta<K, V>` helper in
`munibot_core/src/config/collection.rs` keeps each provider to a few lines.

---

## 8. Storage

One table per provider. Settings are separated from runtime state.

```sql
-- replaces guild_configs
CREATE TABLE `logging_configs` (
  `guild_id`   BIGINT  NOT NULL PRIMARY KEY,
  `enabled`    BOOLEAN NOT NULL DEFAULT TRUE,
  `channel_id` BIGINT  NULL
);

-- user settings
CREATE TABLE `autodelete_configs` (
  `channel_id`    BIGINT      NOT NULL PRIMARY KEY,
  `guild_id`      BIGINT      NOT NULL,
  `lifetime_secs` BIGINT      NOT NULL,
  `mode`          VARCHAR(32) NOT NULL,
  KEY `idx_guild` (`guild_id`)
);

-- sweep bookkeeping, never user facing
CREATE TABLE `autodelete_state` (
  `channel_id`              BIGINT   NOT NULL PRIMARY KEY,
  `last_cleaned`            DATETIME NOT NULL,
  `last_message_id_cleaned` BIGINT   NOT NULL
);
```

`idx_guild` supports the provider's `read` (`WHERE guild_id = ?`).

Upserts must use `ON DUPLICATE KEY UPDATE`, not `REPLACE INTO`:

```rust
diesel::insert_into(logging_configs::table)
    .values(&row)
    .on_conflict(diesel::dsl::DuplicatedKeys)
    .do_update()
    .set(&row)
    .execute(&mut conn)
    .await?;
```

Verified available in diesel 2.3
(`diesel/src/upsert/on_conflict_extension.rs:281`,
`diesel/src/mysql/query_builder/query_fragment_impls.rs:132`).

Each migration must ship both `up.sql` and `down.sql`, and `schema.rs` must be
regenerated. Existing tests at `munibot_core/tests/operations.rs:16-95` cover
`guild_configs` and must be replaced.

### 8.1 Bug this fixes

`set_autodelete` (`munibot_discord/src/autodelete.rs:62-73`) constructs a full
`AutoDeleteTimerRow` with `last_cleaned: epoch` and
`last_message_id_cleaned: 1`, then `upsert_autodelete_timer`
(`munibot_core/src/db/operations.rs:78`) issues `REPLACE INTO`. So **editing an
existing channel's duration or mode resets the sweep cursor to the beginning of
time**, forcing a full re-scan. Splitting settings from state fixes this
structurally rather than by patching the upsert.

---

## 9. Serenity access from outside the bot task

**A channel picker cannot be built with what is in the repo today.** The user's
OAuth token has scope `identify guilds` (`oauth/discord.rs:11`), which does not
permit `GET /guilds/{id}/channels`. Listing channels needs the bot token, and
the axum server has no handle to serenity: `GlobalAccess`
(`munibot_discord/src/state.rs:18-45`) is constructed inside `on_ready`
(`munibot_discord/src/lib.rs:106-129`).

Fix: `munibot/src/bot.rs` creates an `Arc<OnceCell<GlobalAccess>>`, hands one
clone to `start_discord_integration` (filled in `on_ready`) and one to the
Discord config providers. Requests arriving before the bot connects return a
friendly "i'm still connecting to discord, try again in a moment".

Putting `ref_options` on the trait keeps Discord knowledge inside
`munibot_discord` and gives Twitch and GitHub the same extension point later.

**This is a hard prerequisite for the logging provider, not a nice-to-have.**

---

## 10. Server functions and authorization

```rust
list_config_providers(scope)                -> Vec<ProviderSummary>   // + configured badge
get_config_page(scope, provider)            -> ConfigPage { schema, values }
apply_config_patch(scope, provider, patch)  -> ConfigOutcome { values, errors }
get_ref_options(scope, provider, kind)      -> Vec<RefOption>
```

Extractors follow the `get_guilds` pattern, referenced by full path inside the
attribute:

```rust
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    registry: axum::extract::Extension<std::sync::Arc<munibot_core::config::ConfigRegistry>>,
)]
```

Every function, read or write, must call `require_guild_admin(pool, user_id,
guild_id)` first: resolve the linked account, call
`discord::get_current_user_guilds`, and check
`DiscordGuild::is_administered_by_user`. That is an HTTP round trip per request,
so cache the administered-guild set in Redis with a short TTL (roughly 60
seconds). Also verify the bot is actually in the guild, and surface
`DiscordConfig::invite_link` as an "invite munibot" call to action when it is
not.

Writes re-validate server-side via the derived `Configurable::validate`, so the
client can never bypass constraints.

---

## 11. GUI

```
/dashboard                                  guild picker (exists)
/dashboard/:guild_id                        provider cards
/dashboard/:guild_id/:provider_id           provider page
```

Nest these under `Dashboard` so the existing `Outlet` at
`pages/dashboard.rs:20` fills, and turn `GuildRow` (`dashboard.rs:58-77`) into
a `Link`.

Components:

- `ConfigForm` walks `Vec<ConfigNode>` and dispatches per `FieldKind`, holding
  `dirty: Signal<ConfigValues>` and submitting only changed paths.
- One small component per `FieldKind`: `BoolField`, `IntField`, `TextField`,
  `DurationField`, `ChoiceField`, `RefPicker`, `ListField`.
- `RefPicker` is a searchable combobox fed by `get_ref_options`.
- `CollectionEditor` renders keyed items with add and remove controls.
- Field errors from `ConfigOutcome.errors` render inline, keyed by path.

This is the app's first form work. There is currently not one `input` or event
handler in the GUI, so budget time for basic Dioxus form patterns.

Do not copy `Dashboard`'s error handling: it collapses every error into "sign
in" (`pages/dashboard.rs:31-38`). Config pages must distinguish unauthenticated,
unauthorized, bot-not-in-guild, and genuine failure.

---

## 12. Commit plan

Ordered so the first user-visible slice lands as early as possible. Autodelete
is deliberately last: logging alone proves the whole pipeline, then autodelete
validates collections and cache invalidation.

You are trusted to make commits on your own without pausing for user confirmation. Never amend commits.

### Phase 1 — `munibot_config` types

1. `build(config): add wasm-safe munibot_config crate`
2. `feat(config): add ConfigScope and ProviderDescriptor types`
3. `feat(config): add ConfigPath with display and parse impls`
4. `feat(config): add ConfigValue and flat ConfigValues map`
5. `feat(config): add FieldKind, Field, and constraint types`
6. `feat(config): add ConfigNode tree with sections and collections`
7. `feat(config): add ConfigField trait with primitive impls`
8. `feat(config): add discord reference newtypes`
9. `feat(config): add SettingDuration and Toggled wrappers`
10. `feat(config): add ConfigChoice and ConfigKey traits`
11. `feat(config): add ConfigValues flatten and unflatten`
12. `feat(config): add ConfigPatch and FieldError types`
13. `feat(config): add Configurable trait with to_values and with_patch`
14. `test(config): cover flatten roundtrips, patching, and coercion`

### Phase 2 — the derives

15. `build(config): add munibot_config_derive proc-macro crate`
16. `feat(config-derive): parse container and field attributes`
17. `feat(config-derive): generate schema for scalar fields`
18. `feat(config-derive): generate sections for nested and toggled structs`
19. `feat(config-derive): generate collections for keyed maps`
20. `feat(config-derive): add ConfigChoice derive for enums`
21. `feat(config-derive): generate constraint validation`
22. `feat(config): re-export derives behind a derive feature`
23. `test(config): cover derived schemas end to end`

### Phase 3 — trait, registry, helpers

24. `feat(core): add ConfigContext and ConfigError types`
25. `feat(core): add typed ConfigurationProvider trait`
26. `feat(core): add DynConfigProvider facade with blanket impl`
27. `feat(core): add ConfigRegistry for provider registration`
28. `feat(core): add collection_diff helper for keyed settings`
29. `test(core): cover collection_diff and the dyn facade`

### Phase 4 — serenity access from outside the bot task

30. `feat(discord): publish GlobalAccess through a shared OnceCell`
31. `feat(discord): add discord channel and role option lookup`

### Phase 5 — logging, end to end on the server

32. `feat(migrations): add logging_configs table and backfill from guild_configs`
33. `feat(core): add logging_configs model and operations`
34. `refactor(discord): read the logging channel from logging_configs`
35. `refactor(discord): write the logging channel through logging_configs`
36. `feat(migrations): drop guild_configs`
37. `feat(discord): add LoggingConfig with a derived schema`
38. `feat(discord): add logging configuration provider`

### Phase 6 — api layer

39. `feat(api): re-export config wire types from munibot_config`
40. `feat(api): add guild admin authorization helper with redis caching`
41. `feat(api): add list_config_providers server function`
42. `feat(api): add get_config_page server function`
43. `feat(api): add apply_config_patch server function`
44. `feat(api): add get_ref_options server function`

### Phase 7 — wiring

45. `refactor(gui): accept a config registry in server::run`
46. `feat(munibot): build and register the configuration provider registry`

### Phase 8 — GUI

47. `feat(gui): add guild settings routes`
48. `feat(gui): link guild rows to their settings page`
49. `feat(gui): add provider list navigation`
50. `feat(gui): add scalar field components`
51. `feat(gui): add discord reference picker component`
52. `feat(gui): add ConfigForm renderer with dirty tracking`
53. `feat(gui): wire form submission to apply_config_patch`

At this point logging is configurable in the GUI. Ship, then continue.

### Phase 9 — autodelete

54. `feat(migrations): add autodelete_configs and autodelete_state tables`
55. `feat(core): add autodelete config and state models and operations`
56. `refactor(discord): load autodelete timers from the split tables`
57. `fix(discord): stop resetting the sweep cursor when a timer is edited`
58. `feat(migrations): drop the old autodelete_timers table`
59. `feat(discord): add AutoDeleteConfig with a derived schema`
60. `feat(discord): add autodelete configuration provider with cache invalidation`
61. `feat(gui): add collection editor for keyed settings`
62. `docs: document the configuration provider system`

---

## 13. Risks and gotchas

- **Slash commands and the GUI must share one write path.** Dropping
  `guild_configs` forces `munibot_discord/src/admin.rs` to change in the same
  commits. Ideally `admin.rs` calls the provider's `write` so there is literally
  one writer, and `on_changed` fires for slash commands too.
- **Commit 30 gates commit 38.** The logging provider cannot render a channel
  picker without shared `GlobalAccess`. Do not reorder.
- **No `..Default::default()` in config mappings.** It disables the compiler's
  exhaustiveness check, which is the entire justification for typed columns.
- **A migration per new setting** is the accepted cost. The loop is tight:
  migration, column, struct field, mapping line — and the compiler walks you
  through the last two.
- **No token refresh** (`docs/gui.md:125-131`). Config pages must distinguish an
  expired token from a real failure, or admins will see nonsense.
- **`REPLACE INTO` must not be used** for the new multi-column config tables.
- **Proc-macro debuggability.** A derive that generates a nested tree is the
  hardest thing here to debug. Use `cargo expand`, and land commits 17 through
  19 each with tests.
- **File-size guideline.** Put new code in new files; `logging.rs`,
  `autodelete.rs`, and `operations.rs` are already over 500 lines.
- **`AutoDeleteMode::from_db_str`** silently falls back to `AfterSilence` on
  unknown input (`autodelete.rs:247`). The derived `ConfigChoice` should reject
  unknown option ids instead of falling back, so bad patches surface as field
  errors.

## 14. Verified technical details

- Diesel 2.3 MySQL upsert: `.on_conflict(diesel::dsl::DuplicatedKeys)
.do_update().set(...)` — `diesel/src/upsert/on_conflict_extension.rs:281`,
  `diesel/src/mysql/query_builder/query_fragment_impls.rs:132`.
- Diesel does support MySQL `JSON` mapped to `serde_json::Value` behind the
  `serde_json` feature (`diesel/src/mysql/types/json.rs:6-16`). Not needed under
  the typed-column design.
- `serde_path_to_error` is pure Rust and wasm safe, suitable for
  `munibot_config`.
- `strum` is already a workspace dependency, so `VariantNames` is available for
  the `ConfigChoice` derive if useful.
