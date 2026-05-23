# Tracing in munibot

munibot uses the [`tracing`](https://docs.rs/tracing) ecosystem for structured, contextual logging
and diagnostic instrumentation.

## How it works

At startup, `munibot/src/main.rs` initializes a `tracing-subscriber` registry with two layers:

- **`EnvFilter`** — reads `RUST_LOG` to determine which spans and events to emit (same syntax as the
  old `env_logger`).
- **`fmt`** — formats events as human-readable lines to stdout, including the target module path
  (`with_target(true)`).

The `tracing-log` feature on `tracing-subscriber` is enabled, which bridges any `log`-crate records
from transitive dependencies (serenity, twitch-irc, diesel, reqwest, etc.) into the tracing pipeline
automatically — no extra configuration needed.

## Configuring output with `RUST_LOG`

`RUST_LOG` accepts comma-separated directives. Examples:

```sh
# everything at info and above (default when RUST_LOG is unset)
RUST_LOG=info

# debug output from the whole workspace
RUST_LOG=munibot=debug,munibot_core=debug,munibot_discord=debug,munibot_twitch=debug

# trace-level output from just the Twitch dispatch loop
RUST_LOG=munibot_twitch::bot=trace

# debug output from autodelete, info everywhere else
RUST_LOG=info,munibot_discord::autodelete=debug

# suppress noisy serenity http logs while keeping everything else at debug
RUST_LOG=debug,serenity::http=warn
```

Span-aware directives are also supported:

```sh
# only show events inside a span named "autodelete_loop"
RUST_LOG=[autodelete_loop]=debug
```

## Spans in the codebase

The following span boundaries are instrumented:

| Location                                   | Span name / fields                                 | Purpose                                |
| ------------------------------------------ | -------------------------------------------------- | -------------------------------------- |
| `munibot::main`                            | `discord{}`                                        | Root span for the spawned Discord task |
| `munibot_core::config`                     | `read_or_write_default_from{path}`                 | Config file loading                    |
| `munibot_core::db`                         | `establish_pool{}`                                 | DB pool creation                       |
| `munibot_core::db`                         | `run_pending_migrations{}`                         | Diesel migrations                      |
| `munibot_twitch::bot`                      | `join_channel{channel}`                            | Joining a Twitch IRC channel           |
| `munibot_twitch::bot`                      | per-message `channel`, `kind` fields               | Per-message dispatch log               |
| `munibot_twitch::agent`                    | `ban_user{target_user_id, broadcaster_id, reason}` | Helix ban API call                     |
| `munibot_twitch::agent`                    | `get_channel_info{broadcaster_id}`                 | Helix channel lookup                   |
| `munibot_twitch::agent`                    | `get_user_from_login{login}`                       | Helix user lookup                      |
| `munibot_discord::lib`                     | `discord_event{event}`                             | Per-event dispatch                     |
| `munibot_discord::lib`                     | `discord_handler{handler}`                         | Per-handler dispatch                   |
| `munibot_discord::handlers::ventriloquize` | `ventriloquize{user, guild, channel}`              | Slash command                          |
| `munibot_discord::handlers::ventriloquize` | `ventriloquize_send{channel}`                      | Spawned send task                      |
| `munibot_discord::handlers::logging`       | `handle_discord_event{event}`                      | Audit log handler                      |
| `munibot_discord::autodelete`              | `autodelete_loop{}`                                | Spawned background loop                |
| `munibot_discord::autodelete`              | `fire_due_timers{}`                                | Per-cycle timer dispatch               |
| `munibot_discord::autodelete`              | `get_next_fire{}`                                  | Next-fire calculation                  |
| `munibot_discord::autodelete`              | `clean_now{channel, guild, mode}`                  | Per-channel cleanup                    |
| `munibot_discord::autodelete`              | `check_messages{channel, guild}`                   | Next-clean estimation                  |

## Adding new spans

For a simple async function, use `#[instrument]`:

```rust
use tracing::instrument;

#[instrument(skip_all, fields(user = %user_id))]
async fn do_something(user_id: u64, ctx: &Context) -> Result<()> {
    // all tracing events inside here carry `user` as a field
    tracing::debug!("doing the thing");
    Ok(())
}
```

Key tips:

- **`skip_all`** prevents `tracing` from trying to `Debug`-format every argument. Add only the
  fields you actually want via the `fields(...)` list.
- **`%field`** uses the `Display` impl; **`?field`** uses `Debug`.
- For spawned tasks, propagate span context with `.instrument(span)` (from `tracing::Instrument`)
  instead of `.entered()`, which is `!Send` and cannot be held across `.await`.
- For events, prefer structured fields over format strings:

  ```rust
  // good — filterable and parseable
  tracing::error!(error = %e, channel = %channel_id, "failed to send message");

  // avoid — embeds values into the message string
  tracing::error!("failed to send message to {channel_id}: {e}");
  ```

## The `Passing` trait

`munibot_core::passing::Passing::pass()` swallows a `Result::Err` and emits a `tracing::error!` with
the error as a structured `error` field:

```
ERROR munibot_core::passing: result discarded via Passing error="..."
```

Use this sparingly — it discards error context. Prefer explicit `?` propagation or a named `error!`
event with more context.
