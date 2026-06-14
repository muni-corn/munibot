# PluralKit compatibility

munibot's message logging integrates with the [PluralKit](https://pluralkit.me) API to reduce noise
and add context for servers where members use PluralKit for proxying.

## What PluralKit does

When a user sends a message that matches their proxy tags, PluralKit:

1. Creates a webhook message with the member's name and avatar (the "proxy").
2. Immediately deletes the original user message (the "trigger").

Without compatibility support, step 2 causes munibot to emit a "message deleted" log entry for every
proxied message — even though no one actually deleted anything.

## How munibot handles it

On every `MessageDelete` event for a message younger than ~30 minutes, munibot queries
`GET https://api.pluralkit.me/v2/messages/{id}` to check whether the deleted message was a proxy
trigger. There are three outcomes:

| Deleted message is...                                                                | Action                                                                                       |
| ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| The proxy **trigger** (`original` in the PK response)                                | Log is **suppressed** entirely.                                                              |
| The proxy **webhook** (`id` in the PK response) — e.g. deleted via `pk;delete` or ❌ | Log is **enriched** with the proxying member name, system name, and original sender mention. |
| Not known to PluralKit (404)                                                         | Logged normally, no change.                                                                  |

For `MessageUpdate` events, munibot checks whether the edited message is a webhook message. If it
is, it queries PluralKit and, on a match, replaces the generic webhook author attribution with the
member's display name and appends the same "proxied by" enrichment field.

## Caveats

- **30-minute lookup window:** The PluralKit API only supports looking up a message by its original
  (trigger) ID for approximately 30 minutes after it was sent. Older deletions are never queried and
  are always logged normally. Proxy webhook message IDs have no time restriction.

- **Timing:** PluralKit creates the proxy before deleting the trigger, so the API should already
  know about the proxy by the time munibot receives the delete event. munibot retries once after ~1
  second on a 404 for delete-trigger lookups to handle any rare race between Discord's event
  delivery and PluralKit's indexing.

- **Rate limits:** The PluralKit API allows roughly 2 requests per second. munibot queries the API
  only for messages younger than 30 minutes (delete path) or webhook messages (edit path), keeping
  volume low. On a 429 or any unexpected error, munibot **fails open** — the log is emitted normally
  rather than silently dropped.

- **Mutex hold time:** The PluralKit lookup is awaited while the `LoggingHandler` mutex is held.
  This means delete log entries for recent messages may appear with up to ~1 second of added latency
  (only on the first 404 retry path). Other handlers are unaffected since they hold separate
  mutexes.

- **Always-on:** PluralKit support is always active. There is no per-guild toggle. On servers that
  do not use PluralKit, the only cost is the occasional API call (which will 404 quickly) for recent
  non-PK message deletions.

## Module layout

```
munibot_discord/src/
  pluralkit.rs              -- module declaration and re-exports
  pluralkit/
    api.rs                  -- PkClient and PkLookup enum
    models.rs               -- PkMessage, PkMember, PkSystem (Deserialize)
```

The `LoggingHandler` in `handlers/logging.rs` owns a `PkClient` instance and uses it in both the
`MessageDelete` and `MessageUpdate` event arms.
