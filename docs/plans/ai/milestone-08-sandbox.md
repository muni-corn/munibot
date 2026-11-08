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

- Delegation works, and six advisory roles are delegable personas, each a rig `Agent`.
- Tools are rig tools on munibot's `ToolShelf`, each with a `RiskTier`. `RiskTier::Sandbox` exists,
  but no tool uses it.
- The only permission is `Operator`.

From the code as it stands:

- There's no `munibot_toolagent` crate, no `bollard` dependency, and no `Containerfile`.
- Podman isn't installed in `devenv.nix`.
- The NixOS module (`nix/nixos.nix`) has no container options, and `flake.nix` has no `nix2container`
  input.
- CI runs `cargo test` on `ubuntu-latest` with MySQL and Redis services, and no podman.

## What's reused

The pre-strip sandbox hand-wrote everything between the model and the container: a frame codec, an
RPC protocol mirrored by hand on both sides, a socket server and client, a path jail, and six tools.
rig 0.46 covers nearly all of it:

| Need                                               | Reused                                                                                                                                                                              |
| -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `read`, `write`, `edit`, `search`, `list`, `shell` | `rig-tools`: line-numbered reads, atomic writes, exact-match edits with diffs, gitignore-aware search, a shell with timeouts, output limits, spill files, and process-group cleanup |
| Keeping file tools inside the workspace            | `rig-tools`' `base` and `confine` settings                                                                                                                                          |
| Refusing to overwrite a file nobody read           | `rig-tools`' shared `Seen` set between `Read`, `Write`, and `Edit`                                                                                                                  |
| The wire protocol to the container                 | MCP: `rmcp` serves the tools in the container, and `rig-rmcp` turns them into rig `DynamicTool`s on the host                                                                        |
| Talking to rootless podman                         | `bollard`'s `Docker::connect_with_podman_defaults`                                                                                                                                  |
| Parsing `memory_limit = "2 GiB"`                   | `bytesize`, with its `serde` feature                                                                                                                                                |

`munibot_toolagent` shrinks to an MCP adapter of a couple of hundred lines.

**Considered and rejected: implementing rig-tools' `Workspace` for a container.** rig-tools can run
its tools against any `Workspace`, and ships an SSH one. A podman `Workspace` would let the tools run
on the host with no agent in the container at all. But rig's SSH workspace is about 1,700 lines, and
every file operation would become a `podman exec`. Running rig-tools unchanged inside the container,
on its local filesystem, is far less code and keeps the tools on the untrusted side of the boundary.

## What changed from the previous sandbox plan

| Previously                                                      | Now                                                                                                                                        |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| A hand-written RPC protocol, codec, server, client, and tools   | rig-tools served over MCP, consumed through rig-rmcp                                                                                       |
| The tool agent was compiled into the image as its entrypoint    | A static musl binary, **bind-mounted read-only** into any image at `/munibot/toolagent`, with entrypoint and command overridden by bollard |
| `SandboxConfig::wall_clock_limit` was declared and never read   | A watchdog stops the container at the deadline, from the first commit that starts a container                                              |
| `NetworkPolicy::Allowlist` silently granted full bridge network | `None` (default) or `Full` (explicit). An allowlist doesn't exist until a real filtering proxy does                                        |
| `SandboxConfig::default()` was hardcoded at every call site     | `[ai.sandbox]` config from the start                                                                                                       |
| `Optional` provisioned eagerly and failed the whole turn        | Provisioning is lazy, on the first tool call, and a failure is a tool error the model can read                                             |
| An in-container `git clone` needed network                      | No clone here. Milestone 09 clones on the host and mounts a worktree                                                                       |
| Nothing built the image, and integration tests never ran        | devenv scripts build the image and run the gated tests, and a full-chain test exists                                                       |

## Architecture

```
host                                     │ container (rootless podman)
                                         │
munibot_ai::sandbox                      │
  ├── lifecycle (bollard) + watchdog     │   /munibot/toolagent   (read-only bind mount)
  ├── WorkspaceSession (lazy, OnceCell)  │     an MCP server (rmcp) serving rig-tools'
  └── rig-rmcp McpConnection ───────────────>  read, write, edit, search, list, shell,
        over a unix socket in a          │     confined to /workspace
        per-sandbox tmpfs directory      │   /workspace           (read-write bind mount)
```

**`munibot_toolagent` depends on no munibot crate.** It ships into an untrusted container, so it must
not drag in `munibot_ai`'s dependency tree (diesel, bollard, rig-agent). It depends only on
`rig-core`, `rig-tools`, `rmcp`, `tokio`, and `clap`. MCP is the shared protocol, so there are no
wire types to keep in sync by hand.

**The model sees the tools before anything is provisioned.** The host shelves six proxy tools whose
definitions come from rig-tools' own `Tool::parameters()`, so the schema the model sees is exactly
what the agent serves. A proxy's first call provisions the session's container, connects through
rig-rmcp, and forwards the call. A provisioning failure comes back as a tool error, not a failed
turn.

**Scratch workspaces.** Until milestone 09 adds projects, a conversation's workspace is a host
directory at `<ai.sandbox.scratch_root>/<conversation_id>`, mounted at `/workspace`. It survives
between turns, so work accumulates, and it's collected by age.

### Security posture

- Rootless podman, mapped into a user namespace.
- No network unless the persona's policy says `Full`.
- A read-only root filesystem. Only `/workspace` and `/tmp` are writable.
- Dropped capabilities, `no-new-privileges`, and the default seccomp profile.
- CPU, memory, pids, and disk quotas.
- File tools are confined to `/workspace` by rig-tools. rig-tools checks paths by resolving them
  first, which a process racing symlinks could slip past, so that check only keeps tools tidy. The
  container is the boundary that actually protects the host.
- A wall-clock ceiling on the container itself, independent of the turn budget.

### Who can use it

Sandbox tools are tier `Sandbox`. This milestone adds `Permission::Developer`, and the granted tier
derives from permissions:

| Caller                    | Granted tier  |
| ------------------------- | ------------- |
| signed in                 | `NetworkRead` |
| `developer` or `operator` | `Privileged`  |

`Privileged` has no tools until milestones 09 and 10. A delegated turn still inherits the human's
`Invoker`, so marking the builder delegable doesn't let a non-developer reach a shell.

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

| Check                                                                                                                                                                             | Why it matters                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| A static `x86_64-unknown-linux-musl` binary depending on `rig-tools` and `rmcp` builds, and runs unmodified in `debian:bookworm-slim`, `alpine:latest`, and a nix2container image | Mounting one binary into arbitrary images is what makes milestone 09's bring-your-own-image possible |
| rig-rmcp can talk to an rmcp server over a unix socket created inside a container and bind-mounted out                                                                            | The whole transport depends on it                                                                    |
| A read-only bind mount to a path that doesn't exist in the image works with `readonly_rootfs: true`, and bollard's `entrypoint` override beats the image's own `ENTRYPOINT`       | Both are assumed by the lifecycle commit                                                             |
| bollard talks to rootless podman's socket on the development machine and on the deployment target                                                                                 | Nothing else in this milestone can be verified otherwise                                             |

## Commits

### Infrastructure

| #   | Commit                                                   | Summary                                                                                                                            |
| --- | -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 8.1 | `docs(notes): record sandbox preflight findings`         | The results of the checks above.                                                                                                   |
| 8.2 | `build(devenv): add podman for sandboxed execution`      | Adds podman, `DOCKER_HOST` pointing at the rootless socket, one-time setup notes, and the musl target in `languages.rust.targets`. |
| 8.3 | `build(nix): enable rootless podman in the nixos module` | An opt-in `enableAiSandbox` option: `virtualisation.podman`, the socket, and subuid and subgid ranges for the service user.        |

### Tool agent

| #   | Commit                                                | Summary                                                                                                                                                                                                             |
| --- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.4 | `build(toolagent): create the tool agent crate`       | A binary depending on `rig-core`, `rig-tools`, `rmcp` (server), `tokio`, and `clap`. It takes `--socket` and `--workspace`.                                                                                         |
| 8.5 | `feat(toolagent): serve rig-tools over mcp`           | An rmcp server handler listing rig-tools' `Read`, `Write`, `Edit`, `Search`, `List`, and `Shell`, configured with `base = /workspace` and `confine = true`, and calling each through its rig `Tool` implementation. |
| 8.6 | `feat(toolagent): listen on a unix socket`            | Serves MCP on the socket, and shuts down cleanly on `SIGTERM`.                                                                                                                                                      |
| 8.7 | `build(nix): build the tool agent as a static binary` | A `nix/build.nix` package plus a devenv `build-toolagent` script that asserts `ldd` reports a static executable.                                                                                                    |

### Host side

| #    | Commit                                                | Summary                                                                                                                                                                                             |
| ---- | ----------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.8  | `build: add the sandbox base image`                   | A single-stage `Containerfile` with a Debian toolchain base. No tool agent inside and no `ENTRYPOINT`. A devenv `build-sandbox-image` script.                                                       |
| 8.9  | `feat(core): add the developer permission`            | `Permission::Developer`, plus a `[[developers]]` config list synced exactly like operators.                                                                                                         |
| 8.10 | `feat(sandbox): add sandbox and network policy types` | `SandboxPolicy` (`Forbidden`, `Optional`, `Required`), `NetworkPolicy` (`None`, `Full`), and the `sandbox` field on personas.                                                                       |
| 8.11 | `feat(config): add the sandbox section`               | `[ai.sandbox]` as above, with `bytesize` for `memory_limit`. The tool agent path resolves from explicit config, then the executable's sibling, then the environment, and the error names all three. |
| 8.12 | `feat(sandbox): add container lifecycle`              | Create, start, stop, and remove through bollard, with every limit applied, the tool agent mounted, the wall-clock watchdog armed at start, and best-effort cleanup on drop.                         |
| 8.13 | `feat(sandbox): connect to the tool agent`            | Waits for the socket with a timeout, then opens a rig-rmcp `McpConnection` and returns its tools as rig `DynamicTool`s.                                                                             |
| 8.14 | `feat(sandbox): add workspace sessions`               | A `OnceCell` over a provisioned sandbox and its connection for one workspace directory. Exactly one container per session, however many concurrent first calls arrive.                              |
| 8.15 | `feat(sandbox): add sandbox proxy tools`              | Six proxies whose definitions come from rig-tools, each forwarding through its session's connection, shelved at `RiskTier::Sandbox`.                                                                |
| 8.16 | `feat(ai): derive granted tiers from permissions`     | The table above, used by every chat entry point.                                                                                                                                                    |
| 8.17 | `feat(ai): give conversations scratch workspaces`     | Personas with a sandbox policy get the proxies, over a session for `<scratch_root>/<conversation_id>`. A provisioning failure is a tool error, never a turn failure.                                |
| 8.18 | `feat(sandbox): collect stale scratch workspaces`     | A periodic sweep by `scratch_ttl`.                                                                                                                                                                  |

### The hands-on team

| #    | Commit                                                                 | Summary                                                                                                                                    |
| ---- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| 8.19 | `feat(persona): let the coder run code`                                | `coder.md` can now run and verify code, and is told to actually run tests rather than assert that they pass.                               |
| 8.20 | `feat(persona): add the codebase researcher`                           | Produces the summary the software architect from milestone 07 was written to consume.                                                      |
| 8.21 | `feat(persona): add the builder and test engineer`                     | The builder implements exactly one subtask. The test engineer writes tests first, and runs them to confirm they fail for the right reason. |
| 8.22 | `feat(persona): add the final reviewer, commit crafter, and pr author` | The commit crafter follows the Conventional Commit rules this repository uses.                                                             |
| 8.23 | `feat(persona): make the hands-on team delegable by default`           | Descriptions, sandbox policies, and budgets sized to scope. The full twelve-role team is now reachable from chat.                          |

### Verification

| #    | Commit                                             | Summary                                                                                                                                                                             |
| ---- | -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8.24 | `chore(devenv): add a sandbox test script`         | `test-sandbox` runs `cargo test --features sandbox-integration`.                                                                                                                    |
| 8.25 | `test(sandbox): add a full chain integration test` | Real base image, real mounted tool agent, a real rig-rmcp connection across the boundary, and one real tool call. Also tests the watchdog, and that `none` network really has none. |
| 8.26 | `ci: state why sandbox tests are excluded`         | A comment in the workflow saying that `sandbox-integration` needs rootless podman, so its absence is a decision rather than an oversight.                                           |
| 8.27 | `docs: document the sandbox`                       | The runbook covers setup, the image, permissions, limits, and what to check when provisioning fails.                                                                                |

## Definition of done

- A developer asks munibot to write a small script and run its tests. He does, in a container, and
  reports the real output.
- A conversation's files persist between turns and vanish after `scratch_ttl`.
- A non-developer can't reach sandbox tools, directly or through delegation.
- Writing to a path outside `/workspace` fails.
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
- **rig-tools' `delete` and `move`.** Six tools are enough to build and test. Add the others when a
  role actually needs them.
