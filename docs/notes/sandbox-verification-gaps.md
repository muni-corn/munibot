# Sandbox verification gaps

What milestone 4's implementation actually verified live, and the one
combination it didn't - written down so it isn't discovered by surprise
during rollout.

## What's verified live, with real podman

- Container lifecycle (`create`/`start`/`stop`/`remove`), the full security
  posture (dropped capabilities, `no-new-privileges`, read-only root,
  network `none`, a memory limit that actually OOM-kills an over-budget
  process), `Drop` cleanup, and `docker exec` against a running container -
  all against real rootless podman, gated behind the `sandbox-integration`
  feature. See `munibot_ai/src/sandbox/container.rs`'s own tests.
- `Sandbox::checkout`'s git plumbing - against a local bare repository, zero
  network. The dependency-install exec path - against a real running
  container. See `munibot_ai/src/sandbox/checkout.rs`.
- The wire protocol and framing between the host and a **real, compiled**
  `munibot_toolagent` binary, spawned directly via `cargo run -p` (not
  containerized) - see `test_talks_to_the_real_tool_agent_binary` in
  `munibot_ai/src/sandbox/rpc.rs`. This is the one place the two
  independently hand-mirrored protocol implementations are checked against
  each other rather than each just testing itself.

## What's still only verified against fakes

The six `Tool` implementations (`munibot_ai/src/sandbox/tools.rs`) are
tested against a scripted fake agent speaking the real wire protocol, not
against a real `munibot_toolagent` process - and never against one running
**inside a real container** with the real `Containerfile` image.

Nothing in this milestone actually built the `Containerfile` into an image
and ran the full chain - `Sandbox` creates a container, mounts the
workspace and socket, the real `munibot_toolagent` binary starts inside it,
`RpcClient` connects across the container boundary, and a `Tool` call
round-trips through all of it - end to end. Building that image means
compiling `munibot_toolagent` inside a `rustlang/rust:nightly` builder
stage, which is a meaningfully slower, more infrastructure-heavy check than
anything else in this milestone's test suite, and needs to happen at least
once before this ships anywhere real.

**Do this before milestone 4 is considered done, not milestone 5**:

```bash
podman build -t munibot-sandbox:latest -f Containerfile .
```

Then wire a `SandboxConfig { image: "munibot-sandbox:latest".to_string(), .. }`
through `Sandbox::create` with `tool_agent_cmd()`, `with_workspace_mount`,
and `connect_tool_agent`, and run one real `Tool` call against it. If this
works, every piece already verified separately is confirmed to actually
compose.
