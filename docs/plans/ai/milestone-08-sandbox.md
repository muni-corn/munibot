# Milestone 08 — sandbox

**Status:** outline. The commit list is provisional. Run the pre-flight checks, then expand this into
a full recipe checked against the code, before starting. Start after
[milestone 07](milestone-07-specialists.md).

**Outcome:** a developer can ask munibot to write and run code, and he does it inside a rootless
container that can't touch the host. Each conversation gets its own scratch workspace. Files written
in one turn are still there in the next, and the hands-on half of the engineering team can actually
build and test things.

This is the milestone with the largest security surface in the plan: it executes model-authored
shell commands. Treat every generated command as hostile. Most aren't, but the one that is is the one
that matters.

---

## Starting point

After milestone 07:

- Delegation works.
- Six advisory roles are delegable.
- Personas have `memory` and `delegable` fields.
- `RiskTier::Sandbox` exists, but no tool uses it.
- The only permission is `Operator`.

From the code at `9b4a38e1`:

- There's no `munibot_toolagent` crate, no `bollard` dependency, and no `Containerfile`.
- Podman isn't installed in `devenv.nix`.
- The NixOS module (`nix/nixos.nix`) has no container options.
- `flake.nix` has no `nix2container` input.
- CI runs `cargo test` on `ubuntu-latest` with MySQL and Redis services, with no podman.

## What changed from the previous sandbox plan

The pre-strip implementation shipped a sandbox, then needed a whole later phase of corrections. This
plan starts from the corrected design instead:

| Previously                                                      | Now                                                                                                                                        |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| The tool agent was compiled into the image as its entrypoint    | A static musl binary, **bind-mounted read-only** into any image at `/munibot/toolagent`, with entrypoint and command overridden by bollard |
| `SandboxConfig::wall_clock_limit` was declared and never read   | A watchdog stops the container at the deadline from the first commit that starts a container                                               |
| `NetworkPolicy::Allowlist` silently granted full bridge network | `None` (default) or `Full` (explicit). An allowlist doesn't exist until a real filtering proxy does                                        |
| `SandboxConfig::default()` was hardcoded at every call site     | `[ai.sandbox]` config from the start                                                                                                       |
| `Optional` provisioned eagerly and failed the whole turn        | Provisioning is lazy, on first tool call, and a failure is a tool error the model can read                                                 |
| An in-container `git clone` needed network                      | No clone here. Milestone 09 clones on the host and mounts a worktree                                                                       |
| Nothing built the image, and integration tests never ran        | devenv scripts build the image and run the gated tests, and a full-chain test exists                                                       |

## Architecture

```
host                                     │ container (rootless podman)
                                         │
munibot_ai::sandbox                      │
  ├── lifecycle (bollard) + watchdog     │
  ├── WorkspaceSession (lazy, OnceCell)  │   /munibot/toolagent  (read-only bind mount)
  ├── sandbox tools: read, write, edit,  │     ├── rpc server on a unix socket
  │   bash, grep, glob                   │     ├── path jail at /workspace
  └── RpcClient ─────────────────────────────>  └── filesystem and shell execution
        length-prefixed json frames      │   /workspace           (read-write bind mount)
```

**`munibot_toolagent` depends on no munibot crate at all.** It ships into an untrusted container, so
it must stay tiny regardless of what `munibot_ai` pulls in (rig, diesel, bollard). It defines its own
`ToolRequest` and `ToolResponse` types, about a dozen lines, mirrored by hand on the host. That bit of
duplication buys a hard isolation boundary for the one binary attacker-influenced code can reach.

**Why a socket rather than `podman exec`:** exec pays process startup on every `read`. A persistent
agent gives correlated request ids, real cancellation, and per-call timeouts.

**Scratch workspaces.** Until milestone 09 adds projects, a conversation's workspace is a host
directory at `<ai.sandbox.scratch_root>/<conversation_id>`, mounted at `/workspace`. It survives
between turns, so work accumulates, and it's collected by age.

### Security posture

- Rootless podman, mapped into a user namespace.
- No network unless the persona's policy says `Full`.
- A read-only root filesystem. Only `/workspace` and `/tmp` are writable.
- Dropped capabilities, `no-new-privileges`, and the default seccomp profile.
- CPU, memory, pids, and disk quotas.
- Every path is resolved and checked against `/workspace` **after** following symlinks.
- A wall-clock ceiling on the container itself, independent of the turn budget.

### Who can use it

Sandbox tools are tier `Sandbox`. This milestone adds `Permission::Developer`, and the granted tier
derives from permissions:

| Caller                    | Granted tier  |
| ------------------------- | ------------- |
| signed in                 | `NetworkRead` |
| `developer` or `operator` | `Privileged`  |

`Privileged` has no tools until milestones 09 and 10. A delegated turn still inherits the human's
tier, so marking the builder delegable doesn't let a non-developer reach a shell.

```toml
[ai.sandbox]
image = "munibot-sandbox:latest"
cpu_quota = 2.0
memory_limit = "2 GiB"
pids_limit = 256
network = "none"            # "none" | "full"
wall_clock_limit = "30m"
scratch_root = "/var/lib/munibot/scratch"
scratch_ttl = "7d"
# defaults to a munibot_toolagent next to the running executable, then $MUNIBOT_TOOLAGENT_PATH
tool_agent_path = "/var/lib/munibot/bin/munibot_toolagent"
```

## Pre-flight checks

Run these before writing any code, and record the results in `docs/notes/sandbox-preflight.md`. A
failed check is a design correction made cheaply, not a blocker.

| Check                                                                                                                                                                       | Why it matters                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| A static `x86_64-unknown-linux-musl` `munibot_toolagent` runs unmodified in `debian:bookworm-slim`, `alpine:latest`, and a nix2container image                              | Mounting one binary into arbitrary images is what makes milestone 09's bring-your-own-image possible |
| A read-only bind mount to a path that doesn't exist in the image works with `readonly_rootfs: true`, and bollard's `entrypoint` override beats the image's own `ENTRYPOINT` | Both are assumed by the lifecycle commit                                                             |
| bollard talks to rootless podman's socket on the development machine and on the deployment target                                                                           | Nothing else in this milestone can be verified otherwise                                             |

## Commits

### Infrastructure

| #   | Commit                                                   | Summary                                                                                                                                          |
| --- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| 8.1 | `docs(notes): record sandbox preflight findings`         | The results of the checks above.                                                                                                                 |
| 8.2 | `build(devenv): add podman for sandboxed execution`      | Adds podman and `DOCKER_HOST` pointing at the rootless socket, plus one-time setup notes. Also adds the musl target to `languages.rust.targets`. |
| 8.3 | `build(nix): enable rootless podman in the nixos module` | An opt-in `enableAiSandbox` option, `virtualisation.podman`, the socket, and subuid and subgid ranges for the service user.                      |

### Tool agent

| #    | Commit                                                | Summary                                                                                                                                                                                     |
| ---- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.4  | `build(toolagent): create the tool agent crate`       | A binary depending only on tokio, serde, serde_json, clap, and tracing. It takes a `--socket` argument.                                                                                     |
| 8.5  | `feat(toolagent): add rpc protocol types`             | `ToolRequest { id, tool, input }` and `ToolResponse { id, result }`.                                                                                                                        |
| 8.6  | `feat(toolagent): add the frame codec`                | A four-byte big-endian length prefix and a maximum frame size. Oversized and truncated frames are rejected without allocating for them.                                                     |
| 8.7  | `feat(toolagent): add the rpc server`                 | Concurrent connections, dispatch by tool name, correlated responses, and graceful shutdown on `SIGTERM`.                                                                                    |
| 8.8  | `feat(toolagent): add the path jail`                  | `resolve_in_jail` checks after symlink resolution. Tested against `..`, absolute paths, outward symlinks, and symlinks swapped mid-operation. **The most security-critical function here.** |
| 8.9  | `feat(toolagent): add read and glob`                  | Line-numbered reads with offset and limit. Glob through `ignore`, respecting `.gitignore`, newest first.                                                                                    |
| 8.10 | `feat(toolagent): add write and edit`                 | Exact-match edit. Ambiguity is an error unless `replace_all` is set.                                                                                                                        |
| 8.11 | `feat(toolagent): add grep`                           | Through the `grep` crate, with an include filter and a match cap.                                                                                                                           |
| 8.12 | `feat(toolagent): add bash`                           | A timeout and separate stdout and stderr. Oversized output is truncated and spilled to a file the model can read back.                                                                      |
| 8.13 | `build(nix): build the tool agent as a static binary` | A `nix/build.nix` package plus a devenv `build-toolagent` script, which asserts that `ldd` reports a static executable.                                                                     |

### Host side

| #    | Commit                                                | Summary                                                                                                                                                                                            |
| ---- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.14 | `build: add the sandbox base image`                   | A single-stage `Containerfile` with a Debian toolchain base. No tool agent inside, no `ENTRYPOINT`. A devenv `build-sandbox-image` script.                                                         |
| 8.15 | `feat(core): add the developer permission`            | `Permission::Developer`, plus a `[[developers]]` config list synced exactly like operators.                                                                                                        |
| 8.16 | `feat(sandbox): add sandbox and network policy types` | `SandboxPolicy` (`Forbidden`, `Optional`, `Required`), `NetworkPolicy` (`None`, `Full`), and the `sandbox` field on personas.                                                                      |
| 8.17 | `feat(config): add the sandbox section`               | `[ai.sandbox]` as above. The tool agent path resolves from explicit config, then the executable's sibling, then the environment, and the error names all three.                                    |
| 8.18 | `feat(sandbox): add rpc protocol types`               | The host-side mirror of `ToolRequest` and `ToolResponse`.                                                                                                                                          |
| 8.19 | `feat(sandbox): add container lifecycle`              | Create, start, stop, and remove through bollard, with every limit applied, the tool agent mounted, the wall-clock watchdog armed at start, and best-effort cleanup on drop.                        |
| 8.20 | `feat(sandbox): add the rpc client`                   | Waits for the socket with a timeout, with per-call timeouts and cancellation.                                                                                                                      |
| 8.21 | `feat(sandbox): add workspace sessions`               | A `OnceCell` over a provisioned sandbox for one workspace directory. Exactly one container per session, however many concurrent first calls arrive.                                                |
| 8.22 | `feat(sandbox): add sandboxed tools`                  | `read`, `write`, `edit`, `bash`, `grep`, and `glob`, each holding an `Arc<WorkspaceSession>` and provisioning on first call. `write` refuses a file the session hasn't read, and `bash` is serial. |
| 8.23 | `feat(ai): derive granted tiers from permissions`     | The table above, used by every chat entry point.                                                                                                                                                   |
| 8.24 | `feat(ai): give conversations scratch workspaces`     | Personas with a sandbox policy get the tools, over a session for `<scratch_root>/<conversation_id>`. A provisioning failure is a tool error, never a turn failure.                                 |
| 8.25 | `feat(sandbox): collect stale scratch workspaces`     | A periodic sweep by `scratch_ttl`.                                                                                                                                                                 |

### The hands-on team

| #    | Commit                                                                 | Summary                                                                                                                                    |
| ---- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| 8.26 | `feat(persona): let the coder run code`                                | `coder.md` can now run and verify code, and is told to actually run tests rather than assert that they pass.                               |
| 8.27 | `feat(persona): add the codebase researcher`                           | Produces the summary the software architect from milestone 07 was written to consume.                                                      |
| 8.28 | `feat(persona): add the builder and test engineer`                     | The builder implements exactly one subtask. The test engineer writes tests first, and runs them to confirm they fail for the right reason. |
| 8.29 | `feat(persona): add the final reviewer, commit crafter, and pr author` | The commit crafter follows the Conventional Commit rules this repository uses.                                                             |
| 8.30 | `feat(persona): make the hands-on team delegable by default`           | Descriptions, sandbox policies, and budgets sized to scope. The full twelve-role team is now reachable from chat.                          |

### Verification

| #    | Commit                                             | Summary                                                                                                                                                                     |
| ---- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.31 | `chore(devenv): add a sandbox test script`         | `test-sandbox` runs `cargo test --features sandbox-integration`.                                                                                                            |
| 8.32 | `test(sandbox): add a full chain integration test` | Real base image, real mounted tool agent, a real `RpcClient` across the boundary, and one real tool call. Also tests the watchdog, and that `none` network really has none. |
| 8.33 | `ci: state why sandbox tests are excluded`         | A comment in the workflow saying that `sandbox-integration` needs rootless podman, so its absence is a decision rather than an oversight.                                   |
| 8.34 | `docs: document the sandbox`                       | The runbook covers setup, the image, permissions, limits, and what to check when provisioning fails.                                                                        |

## Definition of done

- A developer asks munibot to write a small script and run its tests. He does, in a container, and
  reports the real output.
- A conversation's files persist between turns and vanish after `scratch_ttl`.
- A non-developer can't reach sandbox tools, directly or through delegation.
- Path traversal fails, including through symlinks.
- With `network = "none"`, `curl` from inside the container fails.
- A container that outlives `wall_clock_limit` is stopped. Exceeding the memory quota kills the
  container, not the host.
- Killing munibot mid-turn leaves no orphaned containers.
- On a machine with no podman, the coder still answers, and the first sandbox tool call explains
  that podman isn't available.
- `devenv shell test-sandbox` passes against real rootless podman.

## Deliberately deferred

- **Real repositories.** Cloning, worktrees, and per-project images are milestone 09.
- **A network allowlist.** Only meaningful with a filtering proxy that the container's only route
  points at. Wait for real demand.
- **Warm container pools.** Each session pays container startup once. Pool only after a real
  complaint.
