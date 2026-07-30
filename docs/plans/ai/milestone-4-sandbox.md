# Milestone 4 — sandbox

**Outcome:** munibot can read a repository, edit files, run a build, and run a test suite, all inside
a rootless container that cannot touch the host.

This is the milestone that turns munibot from something that talks about code into something that
writes it. It is also the milestone with the largest security surface in the whole plan, because it
executes model-authored shell commands.

**Phases 18 through 19, commits 130 through 151.**

---

## Architecture

Split along the container boundary, between a module of `munibot_ai` and a fully independent crate:

```
host                                    │ container
                                        │
munibot_ai                              │
  └── ai::sandbox                       │
        ├── podman lifecycle (bollard)  │
        ├── Tool impls (read, write,    │   munibot_toolagent
        │   edit, bash, grep, glob)     │     ├── RPC server
        └── RpcClient ─────────────────────> ├── filesystem + shell execution
              Unix socket, length-       │     └── path jail at the repo root
              prefixed JSON frames      │
```

The host-side `Tool` implementations hold no logic beyond argument marshalling. All execution happens
in `munibot_toolagent`, which is a separate binary baked into the image and takes **no munibot
dependency at all** — not even `munibot_ai`. Before milestone 1's consolidation refactor this crate
could depend on `munibot_ai_types` for the RPC wire types at zero cost, because that crate had nothing
heavier than `serde` in it. Now that those types live inside `munibot_ai` alongside `rig-core` and
(from this phase on) `bollard`, depending on the crate at all — for any single module — would pull
that entire dependency tree into the container image. `munibot_toolagent` instead defines its own tiny
copy of the wire protocol types (`ToolRequest`, `ToolResponse` — a dozen lines), mirrored by hand in
`ai::sandbox` on the host side. A little duplication buys a hard isolation boundary for the one binary
an attacker's generated shell command can reach.

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

## Phase 18 — `ai::sandbox` module and `munibot_toolagent` crate

| #   | Commit                                                              | Description                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 130 | `build(devenv): add podman for sandboxed execution`                 | Add `podman` to `devenv.nix` packages, set `DOCKER_HOST` to the rootless socket at `$XDG_RUNTIME_DIR/podman/podman.sock`, and document the one-time `podman system service` setup. Add `bollard` to `[workspace.dependencies]`. **podman is not currently installed**, so this commit is a prerequisite, not a formality — see `docs/notes/ai-preflight-findings.md`. Re-run the container pre-flight check here before writing commit 112. |
| 131 | `build(toolagent): create tool agent crate skeleton`                | Create `munibot_toolagent/` as its own workspace member: a binary depending only on `tokio`, `serde`, `serde_json`, `clap`, and `tracing` — deliberately **no** path dependency on `munibot_ai` or any other munibot crate. A `--socket` argument, and nothing else.                                                                                                                                                                        |
| 132 | `feat(toolagent): add tool rpc protocol types`                      | `ToolRequest { id, tool, input }` and `ToolResponse { id, result }`, defined here and nowhere else in this crate's dependency graph. Serde roundtrip tests. These are mirrored by hand in `ai::sandbox` (commit 111) rather than shared, which is the one deliberate duplication this phase accepts — see the architecture note above for why.                                                                                              |
| 133 | `feat(toolagent): add length prefixed frame codec`                  | A four-byte big-endian length prefix followed by a JSON payload, with a maximum frame size that rejects oversized frames rather than allocating for them. Encode and decode tested against truncated and oversized input.                                                                                                                                                                                                                   |
| 134 | `feat(toolagent): add rpc server with request dispatch`             | Listen on the Unix socket, accept concurrently, decode frames, dispatch by tool name, and write responses correlated by identifier. Graceful shutdown on `SIGTERM`, draining in-flight requests.                                                                                                                                                                                                                                            |
| 135 | `feat(toolagent): add path jail resolution`                         | `resolve_in_jail(root, path) -> Result<PathBuf>` canonicalising the path and rejecting anything that escapes `root`, evaluated **after** symlink resolution. Tests cover `..` traversal, absolute paths, symlinks pointing outward, and symlinks created during the operation. **The single most security-critical function here.**                                                                                                         |
| 136 | `feat(toolagent): add read and glob execution`                      | `read` returning `<line>: <content>` prefixed output with `offset` and `limit`, truncating lines beyond a maximum width. `glob` over the `ignore` crate, respecting `.gitignore`, returning paths sorted by modification time descending so recently touched files surface first.                                                                                                                                                           |
| 137 | `feat(toolagent): add write and edit execution`                     | `write` creating parent directories. `edit` performing exact string replacement, erroring when the target string is absent or appears more than once unless `replace_all` is set. Ambiguity must be an error; a silent wrong-match edit is far worse than a failed call.                                                                                                                                                                    |
| 138 | `feat(toolagent): add grep execution`                               | `grep` over the `grep` crate that ripgrep is built from, with an include glob filter, returning file paths with line numbers and matching lines, capped at a maximum match count.                                                                                                                                                                                                                                                           |
| 139 | `feat(toolagent): add bash execution with output capture`           | Run through a shell with an optional working directory relative to the repository root, a timeout, and separate stdout and stderr capture. Oversized output is truncated with a byte count and written to a paged file the model can read back, so a chatty build cannot blow the context window.                                                                                                                                           |
| 140 | `build(sandbox): add ai sandbox module skeleton`                    | Add `munibot_ai/src/sandbox.rs` and its `sandbox/` submodule directory. Add `bollard` to `munibot_ai/Cargo.toml`. Add a `Containerfile` building a Debian base with git and common toolchains, and the `munibot_toolagent` binary baked in.                                                                                                                                                                                                 |
| 141 | `feat(sandbox): add container configuration and rpc protocol types` | `SandboxConfig { image, cpu_quota, memory_limit, pids_limit, disk_limit, network, wall_clock_limit }` and `NetworkPolicy` (`None`, `Allowlist(Vec<String>)`). Conservative defaults with no network. Also mirrors `munibot_toolagent`'s `ToolRequest`/`ToolResponse` types by hand for the host side of the wire — see the architecture note on why these are duplicated rather than shared.                                                |
| 142 | `feat(sandbox): add container lifecycle management`                 | `Sandbox::create`, `start`, `stop`, and `remove` through bollard, applying every limit and security option from the posture section. `impl Drop` triggers best-effort cleanup so a panicking test cannot leak containers.                                                                                                                                                                                                                   |
| 143 | `feat(sandbox): add repository checkout`                            | `Sandbox::checkout(repo_url, base_branch, token)` cloning into the workspace mount with the token supplied through a credential helper on stdin rather than the URL, so it never lands in shell history or process arguments. Detect and run the obvious dependency install for the project type.                                                                                                                                           |
| 144 | `feat(sandbox): add rpc client and tool agent startup`              | Mount a per-sandbox socket from a host `tmpfs` directory, launch `munibot_toolagent`, wait for the socket with a bounded timeout, and return a connected `RpcClient` with per-call timeouts and cancellation. `stop` sends `SIGTERM` then `SIGKILL` after a grace period.                                                                                                                                                                   |
| 145 | `feat(sandbox): add sandboxed tool implementations`                 | Six `Tool` impls at tier `Sandbox` — `read`, `write`, `edit`, `bash`, `grep`, `glob` — each marshalling to `RpcClient`. `write` refuses a file the session has not read, tracked in harness state, which stops the classic failure of a model overwriting a file it never looked at. `bash` is marked serial.                                                                                                                               |

---

## Wiring into personas

`SandboxPolicy::Required` on a persona means the `Ai` service handle provisions a sandbox before the
turn and tears it down afterwards. `Optional` provisions lazily on the first sandbox tool call.
`Forbidden` is the default and filters those tools out of the schema list entirely, so a chat persona
never even learns they exist.

Update `coder.md` in this milestone: it can now run and verify code, and the prompt should say so and
should instruct it to actually run tests rather than assert correctness.

---

## Phase 19 — the hands-on engineering team

Milestone 3 phase 16 shipped the six `municode` roles that need nothing but text. These are the other
six: they were held back because a builder that cannot write a file, or a test engineer that cannot run
a test, is a role-play exercise rather than an engineer. Now that a sandbox exists, they get hands.

Same porting rule as phase 16: role, standards, and judgement stay in the markdown; the machine-readable
output contract lives in `Persona.handoff`, left `None` for chat delegation and attached by the pipeline
in milestone 5. All six are `delegable`, so you can ask munibot to bring in the builder directly rather
than only reaching it through an autonomous run.

| #   | Commit                                                              | Description                                                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 146 | `feat(persona): add the codebase researcher persona`                | `codebase-researcher.md` exploring a checked-out repository with `read`, `grep`, and `glob`, producing the structured summary the software architect from milestone 3 was already written to consume. The two halves of the team meet here.                                      |
| 147 | `feat(persona): add the builder persona`                            | `builder.md` implementing exactly one subtask with `write` and `edit`. Tier `Sandbox`, so it is unreachable unless the invoking human is authorized for it — the delegation tier rule from milestone 3 phase 15 is what makes shipping this safe.                                |
| 148 | `feat(persona): add the test engineer persona`                      | `test-engineer.md` writing tests _before_ the implementation exists, and actually running them to confirm they fail for the right reason. Test-driven development is only meaningful once the tests can be executed, which is exactly why this waited for the sandbox.           |
| 149 | `feat(persona): add the final code reviewer persona`                | `final-code-reviewer.md` reviewing every change across every subtask holistically, against the original plan. Needs the complete diff, so it needs a filesystem.                                                                                                                 |
| 150 | `feat(persona): add the commit crafter and pr author personas`      | `commit-crafter.md` producing atomic commits — pointed at this repository's own Conventional Commit rules and the `git-commits` skill, since munibot committing to munibot should follow munibot's conventions — and `pr-author.md` writing a title and body from the real diff. |
| 151 | `feat(persona): add the hands on team to the default configuration` | Mark all six `delegable` with descriptions, sandbox policies, and budgets sized to their scope. With this commit the full twelve-role team is reachable from a chat message, one milestone before the pipeline that automates them.                                              |

---

## Definition of done

- A coding persona clones a repository, greps for a symbol, reads the file, makes an edit, runs the
  test suite, and reports the result.
- Asking munibot to fix something in a repository brings in the builder, which edits real files and
  reports what it changed.
- The codebase researcher's summary is directly consumable by the software architect from milestone 3,
  with no glue in between.
- A container cannot reach the network unless the persona's policy allows it.
- Path traversal attempts fail, including via symlinks.
- Exceeding the memory or CPU quota kills the container without affecting the host.
- Killing munibot mid-run leaves no orphaned containers.
- Sandbox tests are gated behind a feature flag so `devenv test` passes without podman.

## Deployment note

`nix/nixos.nix` needs `virtualisation.podman` enabled with the rootless socket, and the munibot
service user needs `subuid` and `subgid` ranges. This is a real infrastructure change and should be
verified on the deployment target before phase 18 starts, not after it finishes.
