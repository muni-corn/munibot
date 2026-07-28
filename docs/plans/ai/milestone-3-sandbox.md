# Milestone 3 — sandbox

**Outcome:** munibot can read a repository, edit files, run a build, and run a test suite, all inside
a rootless container that cannot touch the host.

This is the milestone that turns munibot from something that talks about code into something that
writes it. It is also the milestone with the largest security surface in the whole plan, because it
executes model-authored shell commands.

**Phase 14, commits 99 through 114.**

---

## Architecture

Two crates, split along the container boundary:

```
host                                    │ container
                                        │
munibot_ai_harness                      │
  └── munibot_ai_sandbox                │
        ├── podman lifecycle (bollard)  │
        ├── Tool impls (read, write,    │   munibot_ai_toolagent
        │   edit, bash, grep, glob)     │     ├── RPC server
        └── RpcClient ─────────────────────> ├── filesystem + shell execution
              Unix socket, length-       │     └── path jail at the repo root
              prefixed JSON frames      │
```

The host-side `Tool` implementations hold no logic beyond argument marshalling. All execution happens
in `munibot_ai_toolagent`, which is a separate binary baked into the image. This is why
`munibot_ai_toolagent` depends only on `munibot_ai_types` — it must stay small and must never pull in
diesel, poise, or serenity.

### Why a socket rather than `podman exec`

An exec per tool call costs process startup on every `read`, and there are hundreds per pipeline run.
A persistent agent over a socket also gives correlated request identifiers, real cancellation, and
per-call timeouts, none of which exec provides cleanly.

### Security posture

- Rootless podman. The container runs as an unprivileged user mapped into a user namespace.
- No network by default. A persona must explicitly request it, and even then only through a proxy
  allowlist.
- Read-only root filesystem with a writable mount only at the repository path and `/tmp`.
- Dropped capabilities, `no-new-privileges`, and a seccomp profile.
- Hard CPU, memory, process, and disk quotas.
- Every path resolved and verified to sit under the repository root, checked after symlink
  resolution rather than before.
- A wall-clock ceiling on the container itself, independent of the harness budget, so a wedged
  container cannot live forever.

Treat generated shell commands as hostile. They frequently are not, but the one time they are is the
one that matters.

---

## Phase 14 — `munibot_ai_sandbox` and `munibot_ai_toolagent`

| #   | Commit                                                       | Description                                                                                                                                                                                                                                                                                                                         |
| --- | ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 99  | `build(devenv): add podman for sandboxed execution`          | Add `podman` to `devenv.nix` packages, set `DOCKER_HOST` to the rootless socket at `$XDG_RUNTIME_DIR/podman/podman.sock`, and document the one-time `podman system service` setup in the plan. Add `bollard` to `[workspace.dependencies]`.                                                                                         |
| 100 | `feat(ai_types): add tool rpc protocol types`                | `ToolRequest { id, tool, input }` and `ToolResponse { id, result }` in `munibot_ai_types`, shared by both sides of the socket so host and container cannot drift. Serde roundtrip tests.                                                                                                                                            |
| 101 | `build(ai_toolagent): create tool agent crate skeleton`      | Create `munibot_ai_toolagent/` as a binary depending only on `munibot_ai_types`, `tokio`, `serde_json`, `clap`, and `tracing`. A `--socket` argument, and nothing else. Keep this dependency list short on purpose.                                                                                                                 |
| 102 | `feat(ai_toolagent): add length prefixed frame codec`        | A four-byte big-endian length prefix followed by a JSON payload, with a maximum frame size that rejects oversized frames rather than allocating for them. Encode and decode tested against truncated and oversized input.                                                                                                           |
| 103 | `feat(ai_toolagent): add rpc server with request dispatch`   | Listen on the Unix socket, accept concurrently, decode frames, dispatch by tool name, and write responses correlated by identifier. Graceful shutdown on `SIGTERM`, draining in-flight requests.                                                                                                                                    |
| 104 | `feat(ai_toolagent): add path jail resolution`               | `resolve_in_jail(root, path) -> Result<PathBuf>` canonicalising the path and rejecting anything that escapes `root`, evaluated **after** symlink resolution. Tests cover `..` traversal, absolute paths, symlinks pointing outward, and symlinks created during the operation. **The single most security-critical function here.** |
| 105 | `feat(ai_toolagent): add read and glob execution`            | `read` returning `<line>: <content>` prefixed output with `offset` and `limit`, truncating lines beyond a maximum width. `glob` over the `ignore` crate, respecting `.gitignore`, returning paths sorted by modification time descending so recently touched files surface first.                                                   |
| 106 | `feat(ai_toolagent): add write and edit execution`           | `write` creating parent directories. `edit` performing exact string replacement, erroring when the target string is absent or appears more than once unless `replace_all` is set. Ambiguity must be an error; a silent wrong-match edit is far worse than a failed call.                                                            |
| 107 | `feat(ai_toolagent): add grep execution`                     | `grep` over the `grep` crate that ripgrep is built from, with an include glob filter, returning file paths with line numbers and matching lines, capped at a maximum match count.                                                                                                                                                   |
| 108 | `feat(ai_toolagent): add bash execution with output capture` | Run through a shell with an optional working directory relative to the repository root, a timeout, and separate stdout and stderr capture. Oversized output is truncated with a byte count and written to a paged file the model can read back, so a chatty build cannot blow the context window.                                   |
| 109 | `build(ai_sandbox): create sandbox crate skeleton`           | Create `munibot_ai_sandbox/` depending on `munibot_ai_types`, `munibot_ai_tools`, `bollard`, `tokio`, and `tracing`. Add a `Containerfile` building a Debian base with git and common toolchains, and the tool agent binary baked in.                                                                                               |
| 110 | `feat(ai_sandbox): add container configuration types`        | `SandboxConfig { image, cpu_quota, memory_limit, pids_limit, disk_limit, network, wall_clock_limit }` and `NetworkPolicy` (`None`, `Allowlist(Vec<String>)`). Conservative defaults with no network. Types before the lifecycle code that consumes them.                                                                            |
| 111 | `feat(ai_sandbox): add container lifecycle management`       | `Sandbox::create`, `start`, `stop`, and `remove` through bollard, applying every limit and security option from the posture section. `impl Drop` triggers best-effort cleanup so a panicking test cannot leak containers.                                                                                                           |
| 112 | `feat(ai_sandbox): add repository checkout`                  | `Sandbox::checkout(repo_url, base_branch, token)` cloning into the workspace mount with the token supplied through a credential helper on stdin rather than the URL, so it never lands in shell history or process arguments. Detect and run the obvious dependency install for the project type.                                   |
| 113 | `feat(ai_sandbox): add rpc client and tool agent startup`    | Mount a per-sandbox socket from a host `tmpfs` directory, launch the agent, wait for the socket with a bounded timeout, and return a connected `RpcClient` with per-call timeouts and cancellation. `stop` sends `SIGTERM` then `SIGKILL` after a grace period.                                                                     |
| 114 | `feat(ai_sandbox): add sandboxed tool implementations`       | Six `Tool` impls at tier `Sandbox` — `read`, `write`, `edit`, `bash`, `grep`, `glob` — each marshalling to `RpcClient`. `write` refuses a file the session has not read, tracked in harness state, which stops the classic failure of a model overwriting a file it never looked at. `bash` is marked serial.                       |

---

## Wiring into personas

`SandboxPolicy::Required` on a persona means the facade provisions a sandbox before the turn and tears
it down afterwards. `Optional` provisions lazily on the first sandbox tool call. `Forbidden` is the
default and filters those tools out of the schema list entirely, so a chat persona never even learns
they exist.

Update `coder.md` in this milestone: it can now run and verify code, and the prompt should say so and
should instruct it to actually run tests rather than assert correctness.

---

## Definition of done

- A coding persona clones a repository, greps for a symbol, reads the file, makes an edit, runs the
  test suite, and reports the result.
- A container cannot reach the network unless the persona's policy allows it.
- Path traversal attempts fail, including via symlinks.
- Exceeding the memory or CPU quota kills the container without affecting the host.
- Killing munibot mid-run leaves no orphaned containers.
- Sandbox tests are gated behind a feature flag so `devenv test` passes without podman.

## Deployment note

`nix/nixos.nix` needs `virtualisation.podman` enabled with the rootless socket, and the munibot
service user needs `subuid` and `subgid` ranges. This is a real infrastructure change and should be
verified on the deployment target before phase 14 starts, not after it finishes.
