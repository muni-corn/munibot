# munibot workspace refactoring plan

## Target structure

```
munibot/
  Cargo.toml              (workspace root)
  munibot_core/
    Cargo.toml
    src/
      lib.rs            (re-exports)
      config.rs         (Config, DiscordConfig, TwitchConfig)
      db.rs             (pool, migrations)
      db/
        schema.rs       (diesel auto-generated)
        models.rs       (all ORM structs)
        operations.rs   (all CRUD functions)
      error.rs          (MuniBotError, core error types)
      passing.rs        (Passing trait)
      greeting.rs       (shared greeting regex logic)
      magical.rs        (shared magicalness calculation)
  munibot_discord/
    Cargo.toml
    src/
      lib.rs            (start_discord_integration, re-exports)
      handler.rs        (DiscordEventHandler trait)
      commands.rs       (DiscordCommandProvider trait)
      state.rs          (DiscordState, GlobalAccess)
      admin.rs
      autodelete.rs
      utils.rs
      vc_greeter.rs
      simple.rs
      handlers/
        bot_affection.rs
        dice.rs
        economy.rs
        economy/wallet.rs
        economy/payout.rs
        eight_ball.rs
        greeting.rs     (thin adapter using munibot_core::greeting)
        logging.rs
        magical.rs      (thin adapter using munibot_core::magical)
        temperature.rs
        ventriloquize.rs
  munibot_twitch/
    Cargo.toml
    src/
      lib.rs            (re-exports)
      bot.rs            (TwitchBot, launch)
      handler.rs        (TwitchMessageHandler trait)
      tokens.rs         (TwitchTokenStorage, TwitchAuth)
      agent.rs          (TwitchAgent, Helix wrapper)
      handlers/
        affection.rs
        autoban.rs
        bonk.rs
        content_warning.rs
        greeting.rs     (thin adapter using munibot_core::greeting)
        lift.rs
        lurk.rs
        magical.rs      (thin adapter using munibot_core::magical)
        quotes.rs
        shoutout.rs
        socials.rs
  munibot/
    Cargo.toml
    src/
      main.rs           (CLI, wires everything together)
  diesel.toml             (updated schema path)
  migrations/             (stays at workspace root)
```

## Dependency graph

```
munibot (binary)
  ├── munibot_core
  ├── munibot_discord ──> munibot_core
  └── munibot_twitch ──> munibot_core
```

`munibot_core` stays **platform-agnostic** — no poise, serenity, or twitch-irc dependencies.

## Phased commit plan

### Phase 1: Cleanup — remove SurrealDB migration

| #   | Commit                                          | Description                                                                                                                              |
| --- | ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `refactor(db): remove surrealdb migration code` | Delete `src/db/migration.rs`, `tests/migration.rs`, `tests/export.surql`. Remove SurrealDB call from `src/discord.rs` startup.           |
| 2   | `chore: remove surrealdb dependency`            | Remove `surrealdb` from `[dependencies]` and `[dev-dependencies]` in `Cargo.toml`. Remove `tokio-tungstenite` if only used by SurrealDB. |

### Phase 2: Create workspace and `munibot_core`

| #   | Commit                                                  | Description                                                                                                                                                                                                                                                                                                   |
| --- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 3   | `build: convert project to cargo workspace`             | Create `crates/` dir. Convert root `Cargo.toml` to a workspace manifest. Create `crates/munibot/` with the existing binary package. Move `src/` to `crates/munibot/src/`. Update `diesel.toml` schema path. Everything still compiles as a single crate at this point.                                        |
| 4   | `build(munibot_core): create core crate skeleton`       | Create `crates/munibot_core/` with `Cargo.toml` and empty `src/lib.rs`. Add to workspace members.                                                                                                                                                                                                             |
| 5   | `refactor(munibot_core): move error types to core`      | Move `MuniBotError` and core error types to `munibot_core::error`. Keep platform-specific variants (like `DiscordCommand`, `SerenityError`) in the binary or respective crates. The core error only has `ParseError`, `RequestError`, `DbError`, `LoadConfig`, `DurationParseError`, `MissingToken`, `Other`. |
| 6   | `refactor(munibot_core): move passing trait to core`    | Move `passing.rs` to `munibot_core::passing`.                                                                                                                                                                                                                                                                 |
| 7   | `refactor(munibot_core): move config module to core`    | Move `config.rs` to `munibot_core::config`.                                                                                                                                                                                                                                                                   |
| 8   | `refactor(munibot_core): move database layer to core`   | Move `db.rs`, `db/schema.rs`, `db/models.rs`, `db/operations.rs` to `munibot_core::db`. Update `diesel.toml` to point to `crates/munibot_core/src/db/schema.rs`.                                                                                                                                              |
| 9   | `refactor(munibot_core): extract shared greeting logic` | Extract the greeting regex pattern and matching logic from `handlers/greeting.rs` into `munibot_core::greeting` as a pure function.                                                                                                                                                                           |
| 10  | `refactor(munibot_core): extract shared magical logic`  | Extract the magicalness hash calculation from `handlers/magical.rs` into `munibot_core::magical` as a pure function.                                                                                                                                                                                          |

### Phase 3: Create `munibot_discord`

| #   | Commit                                                       | Description                                                                                                                                                                                                                                                        |
| --- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 11  | `build(munibot_discord): create discord crate skeleton`      | Create `crates/munibot_discord/` with `Cargo.toml` (depends on `munibot_core`, `poise`, `async-trait`, etc.) and empty `src/lib.rs`. Add to workspace members.                                                                                                     |
| 12  | `refactor(munibot_discord): move discord integration module` | Move `discord.rs`, `discord/handler.rs`, `discord/commands.rs`, `discord/state.rs`, `discord/utils.rs` to `munibot_discord`. Move Discord-specific error variants (`DiscordCommand`, `SerenityError`) into this crate.                                             |
| 13  | `refactor(munibot_discord): move admin and autodelete`       | Move `discord/admin.rs` and `discord/autodelete.rs` to `munibot_discord`.                                                                                                                                                                                          |
| 14  | `refactor(munibot_discord): move discord-only handlers`      | Move `handlers/bot_affection.rs`, `handlers/dice.rs`, `handlers/economy/`, `handlers/eight_ball.rs`, `handlers/logging.rs`, `handlers/simple.rs`, `handlers/temperature.rs`, `handlers/ventriloquize.rs`, `handlers/vc_greeter.rs` to `munibot_discord::handlers`. |
| 15  | `refactor(munibot_discord): add discord greeting adapter`    | Create thin `munibot_discord::handlers::greeting` that implements `DiscordEventHandler` using `munibot_core::greeting` for the matching logic.                                                                                                                     |
| 16  | `refactor(munibot_discord): add discord magical adapter`     | Create thin `munibot_discord::handlers::magical` that implements `DiscordCommandProvider` using `munibot_core::magical` for the calculation.                                                                                                                       |

### Phase 4: Create `munibot_twitch`

| #   | Commit                                                     | Description                                                                                                                                                                                                                                        |
| --- | ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 17  | `build(munibot_twitch): create twitch crate skeleton`      | Create `crates/munibot_twitch/` with `Cargo.toml` (depends on `munibot_core`, `twitch-irc`, `twitch_api`, etc.) and empty `src/lib.rs`. Add to workspace members.                                                                                  |
| 18  | `refactor(munibot_twitch): move twitch integration module` | Move `twitch.rs`, `twitch/bot.rs`, `twitch/handler.rs`, `twitch/tokens.rs`, `twitch/agent.rs` to `munibot_twitch`.                                                                                                                                 |
| 19  | `refactor(munibot_twitch): move twitch-only handlers`      | Move `handlers/affection.rs`, `handlers/autoban.rs`, `handlers/bonk.rs`, `handlers/content_warning.rs`, `handlers/lift.rs`, `handlers/lurk.rs`, `handlers/quotes.rs`, `handlers/shoutout.rs`, `handlers/socials.rs` to `munibot_twitch::handlers`. |
| 20  | `refactor(munibot_twitch): add twitch greeting adapter`    | Create thin `munibot_twitch::handlers::greeting` that implements `TwitchMessageHandler` using `munibot_core::greeting`.                                                                                                                            |
| 21  | `refactor(munibot_twitch): add twitch magical adapter`     | Create thin `munibot_twitch::handlers::magical` that implements `TwitchMessageHandler` using `munibot_core::magical`.                                                                                                                              |

### Phase 5: Slim down the binary

| #   | Commit                                          | Description                                                                                                                                                                                                                                          |
| --- | ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 22  | `refactor(munibot): slim binary to wiring-only` | Remove the now-moved handler collection type aliases from the binary. `main.rs` just parses CLI, loads config, runs migrations, and spawns `munibot_discord::start()` and `munibot_twitch::start()`. Clean up the old `src/handlers.rs` module root. |

### Phase 6: Build and config updates

| #   | Commit                                                      | Description                                                                                                                                                                                                                                                                     |
| --- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 23  | `build(nix): update devenv and flake for workspace`         | Update `devenv.nix` crate overrides for the new workspace structure. Update `RUST_LOG` filter to include new crate names. Verify `nix develop` still works.                                                                                                                     |
| 24  | `refactor(munibot_discord): remove never_type feature flag` | The `#![feature(never_type)]` is only used for autodelete's return type. Replace with `-> !` using `std::convert::Infallible` or restructure the loop to avoid needing the never type, so we can remove the nightly feature flag. _(Optional — only if you want to drop this.)_ |

## Key considerations

1. **`#![feature(never_type)]`**: Currently in `lib.rs`, only used by `autodelete.rs`. After the
   split, it moves to `munibot_discord`'s `lib.rs`. Alternatively, we could refactor it away (commit
   24).

2. **Handler collection types** (`TwitchHandlerCollection`, `DiscordMessageHandlerCollection`,
   etc.): These currently live in `src/handlers.rs` and reference traits from both platforms. After
   the split, each platform crate defines its own collection type.

3. **`diesel.toml`**: The `print_schema.file` path needs updating to
   `crates/munibot_core/src/db/schema.rs` and `migrations_directory.dir` stays at the workspace root
   (`migrations/`).

4. **`Rocket.toml`**: Used for Twitch OAuth redirect. Stays at workspace root or moves to
   `munibot_twitch/`.

5. **Nix build**: The `crate2nix`-based build in `devenv.nix` should handle workspaces, but the
   `crateOverrides` for the `munibot` package name may need adjusting.

6. **`logging.rs` is 1163 lines**: Might be worth flagging for a follow-up refactor, but out of
   scope for this workspace split.
