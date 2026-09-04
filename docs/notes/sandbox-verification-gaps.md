# Sandbox verification gaps

What milestone 4's implementation actually verified live, and the one
combination it didn't - written down so it isn't discovered by surprise
during rollout.

**Closed as of commit 240.** The unrun end-to-end chain this note originally
asked for is `test_the_full_chain_a_real_tool_call_round_trips_through_a_real_container`
(`munibot_ai/src/sandbox/tools.rs`) - a real `munibot-sandbox:latest` image
(`devenv shell -- build-sandbox-image`), a real compiled static
`munibot_toolagent` (`devenv shell -- build-toolagent`), a real container, a
real `RpcClient` connecting across the container boundary, and a real
`write` then `read` round trip through all of it. It passes. Eager
`Optional` provisioning was closed earlier, by commits 206 and 218. Note
that milestone 7 also changed the shape of the manual `podman build` at the
bottom of this note: from commit 204 the tool agent is **mounted** into an
image rather than baked into it, so the root `Containerfile` is a
single-stage build now, and a project's own image can host the agent
without knowing munibot exists.

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

## What's verified against a real container now (commit 240)

The six `Tool` implementations (`munibot_ai/src/sandbox/tools.rs`) are
still mostly tested against a scripted fake agent speaking the real wire
protocol - fast, and sufficient for marshalling and the read-before-write
guard - but one test now runs the real thing: a real
`munibot-sandbox:latest` image, a real compiled static
`munibot_toolagent` bind-mounted in, a real container, a real `RpcClient`
connecting across the container boundary, and a real `write` then `read`
round trip through all of it. Requires
`devenv shell -- build-sandbox-image` and `devenv shell -- build-toolagent`
to have already run; gated behind `sandbox-integration` like every other
podman-touching test here.

## `coder`'s `Optional` policy is lazy now (commits 206, 218)

`coder` (`munibot_ai/src/persona/registry.rs`) is configured with
`sandbox: SandboxPolicy::Optional`. `provision_if_needed` no longer treats
that the same as `Required`: constructing a `WorkspaceSession` only
reserves a scratch directory, and no container is created until the model
actually calls a sandbox tool - see `docs/plans/ai/milestone-7-projects.md`
commit 218's own doc comment for why this resolved the gap more completely
than an eager-vs-degrade split alone could. Plain chat-coding help
("what does this regex do?") never touches podman at all any more.
