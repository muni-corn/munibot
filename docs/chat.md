# Chat

The gui's `/chat` page is a plain, temporary conversation with a model. Nothing about it is saved.

## What it does

- Signed-in users type a message and munibot's reply streams back as it is generated.
- The conversation lives in the browser, in the chat page component's own state. Every message sends
  the **whole** conversation to the server, which forwards it to the model and streams the reply.
- Leaving or reloading the page ends the conversation. "New conversation" clears it on demand.

## What it deliberately doesn't do

There are no saved conversations, personas, memory, tools, projects, attachments, usage tracking, or
rate limits. The server stores nothing, so there is no database table behind the chat.

Because the server is stateless and the browser sends the history, the server bounds what it will
forward (see `validate_conversation` in `munibot_api/src/chat/validate.rs`), and every request needs
a signed-in session. There are **no per-user rate limits or spend caps**, so anyone who can sign in
can spend the configured provider key's budget.

## Configuration

Chat is off unless the `[ai]` section of the config file turns it on:

```toml
[ai]
enabled = true
model = "anthropic/claude-opus-5"
```

`model` is a [rig](https://github.com/0xPlaygrounds/rig) catalog reference, written `vendor/model`.
rig decides whether it names a real model when munibot starts, and reads the vendor's api key from
the environment: `ANTHROPIC_API_KEY` for `anthropic/`, `OPENAI_API_KEY` for `openai/`,
`OPENROUTER_API_KEY` for `openrouter/`, and so on. `ollama/` needs no key, but always talks to
`http://localhost:11434`. Startup fails with a message naming what's missing if chat is enabled
but the model, or its key, isn't set.

Earlier versions wrote `model` as `provider:model` (and accepted `default_model`). Neither form is
read anymore: change the first `:` to `/`.

## Layout

- `munibot_ai` -- `Chat`, which streams a reply to a history through rig's type-erased model, plus
  the `[ai]` config. rig itself is re-exported as `munibot_ai::rig`, and every model, message,
  stream, and error type in the chat is rig's own (see `docs/notes/rig-findings.md`).
- `munibot_api` -- the `chat_stream` server function (`POST /api/ai/chat/stream`, answered as
  server-sent events) and its wire types. The wire types are deliberately narrower than rig's: the
  browser may only send plain user and assistant text. A failed reply is shown as a friendly
  sentence chosen from rig's classification of the error, and the provider's own text goes to the
  server log.
- `munibot_gui` -- the chat page and its components.
