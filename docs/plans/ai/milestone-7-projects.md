# Milestone 7 — projects, workspaces, and closing the loop

**Outcome:** munibot knows what a _project_ is, keeps a real checkout of one, builds a container
image that matches how that project is actually developed, and drives an autonomous pipeline from a
GitHub issue to a pull request inside it — end to end, on your machine, with you watching.

Milestones 1 through 6 built every part of that sentence except the nouns. The pipeline is a
complete, fully tested library with **no production caller at all**. The sandbox is a complete,
fully tested container runtime whose image **nothing ever builds**. Both are one seam away from
working, and the shape of that seam is the thing this milestone names: a project is a git
repository, a workspace is a git worktree inside it, and a sandbox is an ephemeral container over a
workspace, from an image the project itself describes.

**Phases 24 through 28, commits 200 through 255.**

---

## What this milestone actually fixes

Every item below is a real, verified gap in the code as it stands, not a hypothetical. They are
listed here once, with evidence, so that the phase tables below can just say what to do.

| #   | Gap                                                                                                                 | Evidence                                          |
| --- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| 1   | Nothing constructs an `Executor`, a `HarnessDispatcher`, or a `DieselPipelineStore` in any binary                   | zero non-test call sites for all three            |
| 2   | No real `SandboxLifecycle` implementation exists — only `NoSandbox` and test fakes                                  | `pipeline/executor.rs:88-105`                     |
| 3   | `resume_all` is implemented and exported but never called at boot                                                   | `pipeline/resume.rs:47`                           |
| 4   | `WebhookConfig` is hardcoded to `triggers: Vec::new()` / `dispatch: None`, and no config source for triggers exists | `munibot_gui/src/server.rs:71-72`                 |
| 5   | Nothing builds `munibot-sandbox:latest` — no nix, no script, no CI, only prose in a note                            | `docs/notes/sandbox-verification-gaps.md:54-63`   |
| 6   | `SandboxConfig::wall_clock_limit` is declared, defaulted, and **never read**, yet promised in the security posture  | `sandbox/config.rs:52` vs `container.rs:130-167`  |
| 7   | `NetworkPolicy::Allowlist` silently maps to podman's plain `bridge` — full, unfiltered network                      | `sandbox/container.rs:139-142`                    |
| 8   | The default `NetworkPolicy::None` makes `checkout`'s in-container dependency install structurally impossible        | `sandbox/checkout.rs:47`                          |
| 9   | `SandboxPolicy::Optional` provisions as eagerly as `Required`, and a failure kills the whole turn                   | `sandbox/provision.rs:73-75`, `service.rs:1035`   |
| 10  | `Sandbox::checkout` is unreachable from production code — every provisioned workspace is empty                      | `sandbox/provision.rs:62-67`                      |
| 11  | Pipeline turns record no usage and no tool calls, and fabricate a colliding `ConversationId(pipeline_id.0)`         | `pipeline/executor.rs:394`, `dispatch.rs:190`     |
| 12  | `SandboxConfig` is not operator-configurable — both production call sites hardcode `::default()`                    | `service.rs:1032`, `service.rs:1195`              |
| 13  | The full sandbox chain (real image → real container → real tool agent → real tool call) has never run once          | `docs/notes/sandbox-verification-gaps.md:25-40`   |
| 14  | `GitHubForge`'s trait method bodies have never run against anything, real or mocked                                 | `docs/notes/github-forge-verification-gaps.md`    |
| 15  | There is no CI at all, and `devenv test` runs bare `cargo test`, so no `sandbox-integration` test ever runs         | `devenv.nix:43-45`, `.github/` holds only funding |
| 16  | The autodelete cache is loaded once at boot and never invalidated                                                   | `munibot_discord/src/autodelete.rs:31-53`         |
| 17  | Editing an autodelete timer's duration resets its sweep cursor to the epoch, forcing a full re-scan                 | `operations.rs:142` (`replace_into`)              |
| 18  | `quotes.created_at` stores local time while every other table stores UTC                                            | `munibot_twitch/src/handlers/quotes.rs:46`        |
| 19  | Discord OAuth access tokens are never refreshed; the dashboard just breaks after ~7 days                            | `docs/gui.md:132-136`                             |
| 20  | Operator permissions are grant-only; removing an entry from `[[operators]]` revokes nothing                         | `docs/notes/permission-system.md:26-46`           |
| 21  | `GITHUB_BOT_LOGIN` is read at runtime but undeclared in `secretspec.toml`                                           | `munibot_gui/src/server.rs:67`                    |
| 22  | `docs/gui.md` and `docs/tracing.md` carry stale version, layout, and attribution claims                             | `docs/gui.md:3,39-66`, `docs/tracing.md:53`       |

Three items previously recorded as gaps are **already fixed** and their notes are stale:
`upsert_guild_config` now uses `on_conflict` (`operations.rs:47`), `/admin stop-logging` clears just
the logging column (`admin.rs:107-114`), and `HasPermission::has` checks a real permission set.
Commit 245 corrects the notes.

---

## Pre-flight checks

Four unknowns, each of which would invalidate a design decision below if it turns out false. Run
them **before writing any code**, exactly as `docs/notes/ai-preflight-findings.md` was produced for
milestone 1, and record the results in a new note. A pre-flight that fails is not a blocker; it is a
design correction made cheaply instead of expensively.

| Check                                                                                                                                                                                                                   | Why it matters                                                                                                                                                                                                |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `devenv container build shell` followed by `devenv container --registry containers-storage: copy shell` lands an image that `podman images` lists and `bollard` can run                                                 | The whole devenv image strategy (commit 216) depends on skopeo's `containers-storage:` transport writing somewhere rootless podman actually reads. If not, fall back to `docker-archive:` plus `podman load`. |
| A `x86_64-unknown-linux-musl` static `munibot_toolagent` runs unmodified inside a nix2container devenv image, `debian:bookworm-slim`, and `alpine:latest`                                                               | Mounting one binary into arbitrary images (commit 204) is the mechanism that makes bring-your-own-image possible at all. If the nix image lacks something it needs, the whole strategy ordering changes.      |
| Bind-mounting a host file into a container with `readonly_rootfs: true`, at a path that does not exist in the image, succeeds — and bollard's `Config::entrypoint` override actually beats the image's own `ENTRYPOINT` | Both are assumed by commit 204. Podman normally creates bind-mount targets before sealing the rootfs, but "normally" is not "verified". Test with `/munibot/toolagent`.                                       |
| `podman build` works with a **git worktree** as its context directory                                                                                                                                                   | A worktree has a `.git` _file_ pointing elsewhere, not a `.git` directory. Anything that walks up from the context root may be surprised. Affects commit 215.                                                 |

→ `docs(ai): record projects and container preflight findings`

---

## Architecture

### Projects and workspaces

```
<ai.projects.root>/
  cocoa/
    repo.git/                     bare clone, the single source of objects
    worktrees/
      pipeline-142/               worktree on munibot/issue-17
      chat-8891/                  worktree on munibot/chat-8891
      scratch-a3f1/               worktree on the default branch
```

A **project** is a git repository munibot has cloned. One row in `ai_projects`, one directory on
disk, one container image.

A **workspace** is a `git worktree` of that project, on its own branch. One row in `ai_workspaces`,
one directory on disk. Workspaces are **persistent**: they survive a run, so a failed pipeline can be
inspected on disk and a resumed pipeline finds its own work still there. They are collected by age
(commit 221), not by scope exit.

A **sandbox** is an **ephemeral** container over one workspace, from that project's image. It exists
for the duration of a session and no longer.

The bare clone matters: `git clone --bare` gives one object store that every worktree shares, so
three concurrent workspaces cost three checkouts and one copy of history rather than three of each.

> **Gotcha, and it will bite you:** `git clone --bare` sets **no** `remote.origin.fetch` refspec, so
> a later `git fetch` updates nothing and every worktree is created from a stale ref. Commit 212 must
> set `remote.origin.fetch = +refs/heads/*:refs/remotes/origin/*` explicitly right after cloning, and
> a test must assert a second fetch actually advances a remote-tracking ref.

### Image strategy

A project's image is resolved by a **pure function** over its worktree contents, in this order:

| Order | Strategy        | Condition                                                                               | Build                                                                                             |
| ----- | --------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| 1     | `Containerfile` | `.munibot/Containerfile` exists                                                         | `podman build -f .munibot/Containerfile -t munibot-project-<slug>:<hash> <worktree>`              |
| 2     | `Devenv`        | `devenv.nix` exists **and** both `nix2container` and `mk-shell-bin` are declared inputs | `devenv container build shell`, then `devenv container --registry containers-storage: copy shell` |
| 3     | `Base`          | always                                                                                  | none — use `[ai.sandbox].image` directly                                                          |

A skipped strategy records **why** on the project row, and the projects page shows it. This is the
difference between "munibot ignored my devenv" and "munibot told me my devenv is missing two
inputs and exactly how to add them". Never mutate the project's own repository to make a strategy
work; a project that wants the devenv path opts in by running, in its own repo:

```bash
devenv inputs add nix2container github:nlewo/nix2container --follows nixpkgs
devenv inputs add mk-shell-bin github:rrbutani/nix-mk-shell-bin
```

Images are tagged `munibot-project-<slug>:<hash>`, where `<hash>` is the first sixteen hex
characters of a SHA-256 over the strategy's declared inputs — the bytes of `.munibot/Containerfile`
for strategy 1, or of `devenv.nix` + `devenv.yaml` + `devenv.lock` for strategy 2 — plus a constant
salt bumped whenever munibot's own image contract changes. A build is skipped when an image with
that exact tag already exists locally. This is what stops every pipeline run paying a nix
evaluation.

### The tool agent is mounted, not baked

Today `munibot_toolagent` is compiled into the image by the root `Containerfile` and made its
`ENTRYPOINT`. That cannot work for an image the project built, which has never heard of munibot.

From commit 204 onward, the tool agent is a **static musl binary bind-mounted read-only** into any
image at `/munibot/toolagent`, with bollard overriding both entrypoint and command:

```rust
Config {
    entrypoint: Some(vec!["/munibot/toolagent".to_string()]),
    cmd: Some(self.tool_agent_cmd()),
    ..
}
```

This is the single change that makes all three image strategies work through one mechanism. It also
shrinks the root `Containerfile` from a two-stage build into a plain toolchain base image, and means
a tool agent bug is fixed by rebuilding one small binary rather than every project's image.

Static musl is realistic here: `munibot_toolagent` depends only on `tokio`, `serde`, `serde_json`,
`clap`, `tracing`, `ignore`, and `grep`, all pure Rust with no C dependencies.

### Network

The runtime sandbox stays closed by default. Per-project opt-in replaces the current situation where
`Allowlist` quietly grants everything:

| `NetworkPolicy` | podman `network_mode` | Availability                                                            |
| --------------- | --------------------- | ----------------------------------------------------------------------- |
| `None`          | `"none"`              | default, and the only sensible one for untrusted generated code         |
| `Full`          | `"bridge"`            | **new**, explicit per-project opt-in, documented as weakening isolation |
| `Allowlist(_)`  | —                     | returns an error naming itself unimplemented, until a real proxy exists |

Image **builds** have network, because `podman build` and `nix` both need it. That is the right
place for dependency fetching to happen, and it is why gap 8 stops mattering: `npm install` belongs
in the image, not in a network-less runtime container.

### Lazy provisioning falls out of the workspace session

`WorkspaceSession` (commit 218) holds a `tokio::sync::OnceCell<ProvisionedSandbox>`. The six sandbox
tools hold an `Arc<WorkspaceSession>` and call `session.sandbox().await?` inside `Tool::call`.

Two consequences worth stating plainly, because they resolve gap 9 more completely than a special
case would have:

- The tools are always **present in the schema** when the policy is not `Forbidden`, so the model's
  view of its own capabilities never depends on provisioning timing. Nothing is created until a tool
  is actually called.
- A provisioning failure surfaces as a **tool error on the first call**, not a turn failure. The
  model can read "podman isn't available" and adapt, which is strictly better than the turn dying at
  `service.rs:1035` before the model has said a word.

---

## Database schema

```
ai_projects    (id, slug UNIQUE, display_name, forge, repo_owner, repo_name, clone_url,
                default_branch, local_path, image_strategy, image_tag NULL,
                image_source_hash NULL, image_built_at NULL, image_error NULL,
                network_policy, trigger_mode, trigger_label NULL,
                created_at, updated_at)

ai_workspaces  (id, project_id FK -> ai_projects ON DELETE CASCADE,
                name, branch, path, pipeline_id NULL, conversation_id NULL,
                created_at, last_used_at,
                UNIQUE (project_id, name))
```

Plus, on existing tables:

| Table              | Column                 | Why                                                   |
| ------------------ | ---------------------- | ----------------------------------------------------- |
| `ai_pipelines`     | `project_id` NULL, FK  | which project a run belongs to                        |
| `ai_conversations` | `project_id` NULL, FK  | the chat project picker                               |
| `ai_usage`         | `pipeline_id` NULL, FK | pipeline spend visible on `/usage`                    |
| `ai_tool_calls`    | `pipeline_id` NULL, FK | a pipeline's tool-call audit trail, addressing gap 11 |

`ai_tool_calls.conversation_id` and `ai_usage.conversation_id` are already nullable, and
`ToolCallRecord.conversation_id` / `UsageRecord.conversation_id` are already `Option`. The only
non-optional one in the chain is `ToolCtx.conversation_id` (`tools/context.rs:94`), which commit 230
makes `Option<ConversationId>` alongside a new `pipeline_id: Option<PipelineId>`. That is what lets
the executor stop fabricating `ConversationId(pipeline_id.0)` at `executor.rs:394` — an identifier
that would collide with a real conversation the moment anything recorded under it.

`image_strategy` and `network_policy` are stored as their `strum` snake-case string forms, matching
the convention `munibot_core::Permission` already established.

---

## Configuration

```toml
[ai.sandbox]
# every field optional; these are the defaults
image = "munibot-sandbox:latest"
cpu_quota = 2.0
memory_limit = "2 GiB"
pids_limit = 256
network = "none"          # "none" | "full"
wall_clock_limit = "30m"
# defaults to a `munibot_toolagent` sitting next to the running executable,
# then to $MUNIBOT_TOOLAGENT_PATH
tool_agent_path = "/var/lib/munibot/bin/munibot_toolagent"

[ai.projects]
root = "/var/lib/munibot/projects"
# a repository the github app is installed into becomes a project on first use
auto_register_installations = true
# workspaces untouched for this long are collected
workspace_ttl = "7d"
# how many project images may be built concurrently
max_concurrent_image_builds = 1
```

Projects themselves are **not** declared in TOML. They arrive by GitHub App installation, by the
operator projects page, or by munibot's own `manage_project` tool — three paths that all funnel
through one `ProjectManager::register`. Trigger configuration lives on the project row, which is why
commit 234 can delete `WebhookConfig::triggers: Vec<RepoTriggerConfig>` entirely rather than
inventing a TOML section for it.

---

## Phase 24 — sandbox primitives

Seven independent fixes to the container runtime. None of them depends on projects, and every one of
them is a bug in code that already shipped. Do this phase first: it is the smallest, it is the most
self-contained, and everything after it assumes the primitives are honest.

| #   | Commit                                                        | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 200 | `fix(sandbox): enforce the container wall clock limit`        | `SandboxConfig::wall_clock_limit` is read by nothing (`config.rs:52` vs `container.rs:130-167`), while `sandbox.rs:38-39` promises "a wall-clock ceiling on the container itself". Arm a watchdog task in `Sandbox::start` that calls `stop` at the deadline, cancelled by a token the `Sandbox` holds so a normal teardown does not leave it running. Test with a limit of a few hundred milliseconds against a sleeping container, gated on `sandbox-integration`. |
| 201 | `fix(sandbox): stop granting full network for an allowlist`   | `container.rs:139-142` maps `Allowlist(_)` to `"bridge"` — unfiltered access, under a name that promises the opposite. Add `NetworkPolicy::Full` as the explicit opt-in, map `Allowlist(_)` to an `AiError` naming it unimplemented, and update the `sandbox.rs` posture text so it no longer describes a proxy that does not exist. Harmless today only because nothing sets it; that is not a reason to leave it.                                                  |
| 202 | `build(toolagent): build a static musl binary`                | Add `x86_64-unknown-linux-musl` to `languages.rust.targets` in `devenv.nix`, a `build-toolagent` script wrapping `cargo build --release --target x86_64-unknown-linux-musl -p munibot_toolagent`, and a package in `nix/build.nix`. No behaviour change yet — this exists so commit 204 has something to mount. Assert in the script that the result is actually static (`ldd` reports "not a dynamic executable").                                                  |
| 203 | `feat(ai): add the sandbox configuration section`             | `[ai.sandbox]` as shown above: `SandboxSettings` with serde defaults matching today's `SandboxConfig::default()` exactly, humantime durations, a byte-size parser for `memory_limit`, and `tool_agent_path` resolution (explicit → sibling of `current_exe()` → `$MUNIBOT_TOOLAGENT_PATH` → error naming all three). Types, parsing, and defaults only; nothing reads it yet. Round-trip and default-equivalence tests.                                              |
| 204 | `feat(sandbox): mount the tool agent instead of baking it`    | `SandboxConfig` gains `tool_agent_path`. `build_host_config` adds a read-only bind mount to `/munibot/toolagent`; `create` sets `entrypoint` to it and `cmd` to `tool_agent_cmd()`. Shrink the root `Containerfile` to a single-stage toolchain base with no builder, no `COPY --from`, and no `ENTRYPOINT`. **The keystone commit of the milestone** — it is what lets any image at all host the tool agent. Do not start it before the pre-flight checks pass.     |
| 205 | `feat(ai): thread operator sandbox config through the turn`   | Replace `SandboxConfig::default()` at `service.rs:1032` and `service.rs:1195` with the configured value, threaded from `AiConfig` through `munibot/src/ai.rs::build` onto `Ai`. Closes gap 12; the operator can now change the image, the limits, and the network without a recompile.                                                                                                                                                                               |
| 206 | `fix(ai): let an optional sandbox degrade instead of failing` | `service.rs:1035` and `:1198` both use `?`, so `SandboxPolicy::Optional` fails a turn outright when podman or the image is missing — behaving as `Required` in the failure direction as well as the eager one. On `Optional`, log a warning naming the cause and continue with the base registry. Tested by pointing at a nonexistent image and asserting the turn still completes.                                                                                  |

---

## Phase 25 — projects and workspaces

The new subsystem. Twenty-two commits, and the largest single body of new work since milestone 2.
Order is deliberate and follows `AGENTS.md`: types first, then configuration, then functionality,
then the surfaces that expose it.

Everything lives in a new `munibot_ai::project` module (`munibot_ai/src/project.rs` plus
`project/`), which depends on `ai::sandbox` and `munibot_vcs` and is depended on by `ai::pipeline`.
Keep it below `ai::pipeline` in the dependency graph in `docs/plans/ai/overview.md`.

### Types, storage, and configuration (207–211)

| #   | Commit                                                     | Description                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 207 | `feat(project): add project and workspace domain types`    | `ProjectId`, `WorkspaceId`, `Project`, `Workspace`, `ImageStrategy` (`Containerfile`/`Devenv`/`Base`), `ImageStatus` (`Unbuilt`/`Built { tag, hash, at }`/`Failed { reason }`), `TriggerMode`, and `ProjectError`. Pure types: `serde`, `schemars`, `strum`, `thiserror`, nothing else. Serde round-trip tests, and a test that every `ImageStrategy` and `TriggerMode` string form round-trips through `Display`/`FromStr`.                  |
| 208 | `feat(db): add ai projects and workspaces tables`          | The migration in the schema section above, plus `schema.rs` and the diesel models. Include `project_id` on `ai_pipelines` and `ai_conversations` in the same migration — they are the same logical change (projects now exist and things belong to them), and splitting them would leave a migration nothing references. Every foreign key gets `ON DELETE CASCADE` for workspaces and `ON DELETE SET NULL` for the nullable back-references. |
| 209 | `feat(db): add project and workspace operations`           | CRUD in `munibot_core/src/db/operations/ai.rs`, following the existing shape there. `upsert_project` uses `on_conflict(...).do_update()`, never `replace_into` — the trap documented at `operations.rs:31` and re-learned the hard way in `docs/notes/gui-configuration-research.md:31-53`. Tested against the `TestDb` fixture.                                                                                                              |
| 210 | `feat(ai): add the projects configuration section`         | `[ai.projects]` as shown above. Parsing, defaults, and a `root` that is created on first use rather than required to pre-exist. Types only.                                                                                                                                                                                                                                                                                                   |
| 211 | `feat(project): add a project store trait and diesel impl` | `ProjectStore` mirroring `PipelineStore`'s shape (`pipeline/store.rs`), with an in-memory fake for the tests every later commit in this phase needs. This is to this phase what `MockProvider` is to the whole plan: build it before the things that consume it.                                                                                                                                                                              |

### Git plumbing (212–213)

| #   | Commit                                       | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| --- | -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 212 | `feat(project): clone a project repository`  | `git clone --bare <url> <root>/<slug>/repo.git`, **then** `git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'` — without which every subsequent fetch is a silent no-op and every worktree is created from a stale ref. Idempotent: an existing clone is fetched, not re-cloned. The token goes through `git credential approve` on stdin, reusing the approach already proven at `sandbox/checkout.rs:70-118`, never in the URL and never in argv. Tested against a local bare repository with zero network, per this crate's testing rule. |
| 213 | `feat(project): add git worktree management` | `create_worktree(project, name, branch)`, `list_worktrees`, `remove_worktree`, and a `prune` that reconciles `ai_workspaces` against what is actually on disk after a crash. Branch naming goes through `pipeline::branch::resolve_branch_name`, which is already built, already tested, and currently has **zero callers** — this is where it finally earns its place. Tested against a local bare repository, no network.                                                                                                                                    |

### Images (214–217)

| #   | Commit                                               | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 214 | `feat(project): detect a project's image strategy`   | A **pure function** from a worktree path to `(ImageStrategy, Vec<SkipReason>)`, following the ordering table above. The devenv check parses `devenv.yaml` (and `devenv.lock` when present) for both `nix2container` and `mk-shell-bin`. A skip reason is a real, actionable sentence naming the two `devenv inputs add` commands — this string is user-facing on the projects page, so write it like an error message, per `AGENTS.md`. Table-driven tests over fixture directories for every branch, including "devenv.nix but no inputs". |
| 215 | `feat(project): build an image from a containerfile` | `podman build -f .munibot/Containerfile -t munibot-project-<slug>:<hash> <worktree>`, with output streamed into a `tracing` span so a slow build is visible rather than silent. A build failure records `image_error` on the project and returns a `ProjectError` naming the tag; it never panics and never leaves a half-built tag recorded as `Built`.                                                                                                                                                                                    |
| 216 | `feat(project): build an image from devenv`          | `devenv container build shell` in the worktree, then `devenv container --registry containers-storage: copy shell`, then retag to `munibot-project-<slug>:<hash>`. Depends directly on pre-flight check 1; if `containers-storage:` turns out not to be readable by rootless podman, this commit is where the `docker-archive:` + `podman load` fallback goes instead. Nix evaluation is slow — the span here matters more than anywhere else in the milestone.                                                                              |
| 217 | `feat(project): cache project images by source hash` | The SHA-256-over-declared-inputs scheme from the architecture section, plus the constant salt. Skip the build when the tag already exists locally; persist `image_tag`, `image_source_hash`, `image_built_at`, and `image_error` on the project. Serialise builds behind `max_concurrent_image_builds`, because two nix evaluations at once on one machine is how you discover what swap is for. Tested by asserting an unchanged input builds once and a changed one builds twice.                                                         |

### Sessions and the manager (218–221)

| #   | Commit                                                         | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| --- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 218 | `feat(sandbox): provision a workspace sandbox lazily`          | `WorkspaceSession` over a `tokio::sync::OnceCell<ProvisionedSandbox>`, as described in the architecture section. The six sandbox tools take an `Arc<WorkspaceSession>` and provision on first call. Replaces the eager path at `provision.rs:73-75` and closes the second half of gap 9 — the one commit 206 could not reach. Tests: the tools appear in the schema with nothing provisioned; exactly one container is created across many concurrent first calls; drop tears down. |
| 219 | `refactor(ai): provision sandboxes through workspace sessions` | Rewire `Ai::prepare` (`service.rs:1030-1039`) and `Delegator::delegate` (`service.rs:1193-1202`) onto sessions. A conversation with no project bound gets a scratch workspace, preserving today's behaviour exactly. Pure refactor: no new capability, and the existing sandbox tests should pass unchanged.                                                                                                                                                                        |
| 220 | `feat(project): add the project manager service`               | The facade that ties store, clone, worktree, strategy detection, image build, and session together behind two calls: `register(spec)` and `workspace_for(project, branch, owner)`. Everything above this commit is a component; this is the only thing anything outside `ai::project` should need to hold. Keep the file small — it is a coordinator, and the work belongs in the modules it calls.                                                                                 |
| 221 | `feat(project): collect workspaces that have gone unused`      | The price of choosing persistent workspaces. A periodic sweep removing worktrees whose `last_used_at` is older than `workspace_ttl` and which no running pipeline holds, checked against `PipelineRegistry::is_running`. Removes the worktree, prunes git's own administrative record, and deletes the row — in that order, so a crash mid-sweep leaves something `prune` (commit 213) can reconcile rather than an orphan.                                                         |

### Registration and surfaces (222–228)

| #   | Commit                                                             | Description                                                                                                                                                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 222 | `feat(project): register a project from a github app installation` | The first of the three registration paths, and the one the pipeline needs. Resolve an installation's repository to a slug, clone it, detect its strategy. Gated on `auto_register_installations`; when off, an unregistered repository's webhook is ignored with a logged reason rather than silently dropped.                                                                   |
| 223 | `feat(api): add project server functions`                          | `list_projects`, `get_project`, `register_project`, `remove_project`, `rebuild_project_image`. Operator-gated via `auth::operator::require_operator`, following the existing server-function slices exactly. `remove_project` refuses while a pipeline is running against it.                                                                                                    |
| 224 | `feat(gui): add a projects page`                                   | `/projects`, operator-gated. One row per project: slug, repository, image strategy with its **skip reasons rendered as prose**, image status and age, workspace count, network policy, trigger mode. A rebuild button and a register form. The skip reason is the whole point of this page — it is where "why didn't munibot use my devenv" gets answered without reading a log. |
| 225 | `feat(tools): add a project management tool`                       | The third registration path: a `RiskTier::Privileged` tool letting munibot register and clone a project when asked. Tier 4 means it is unreachable from public chat by construction (`overview.md`'s tier table), and the operator grant is what makes it reachable at all. Nothing here bypasses `ProjectManager::register`.                                                    |
| 226 | `feat(api): bind a conversation to a project`                      | `set_conversation_project(conversation_id, project_id)`, owner-gated, writing `ai_conversations.project_id`. A bound conversation's `coder` turns get a real worktree from `workspace_for`; an unbound one keeps the scratch workspace.                                                                                                                                          |
| 227 | `feat(gui): add a project picker to the chat page`                 | A selector beside the persona picker, showing the bound project and its image status. This is the commit that makes chat `coder` genuinely useful rather than a persona that can only reason about code it was pasted.                                                                                                                                                           |
| 228 | `docs(ai): document projects and workspaces`                       | `docs/ai-projects.md`: what a project and a workspace are, the filesystem layout, the three image strategies and how to opt into each, the bare-clone refspec gotcha, how to register a project three ways, how workspaces are collected, and what to check when an image build fails. Cross-link from `docs/ai-operations.md`.                                                  |

---

## Phase 26 — pipeline wiring

The seam. Ten commits that turn "a fully tested pipeline library" into "a pipeline that runs". Every
piece it needs already exists and is tested; this phase is almost entirely composition.

| #   | Commit                                                            | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --- | ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 229 | `feat(db): add pipeline id to ai usage and tool calls`            | Nullable `pipeline_id` columns, foreign-keyed, plus `schema.rs` and models. `ToolCallRecord` and `UsageRecord` gain the field; both already carry `conversation_id` as an `Option`, so this is symmetric rather than novel.                                                                                                                                                                                                                                                  |
| 230 | `feat(pipeline): record usage and tool calls for pipeline turns`  | `ToolCtx.conversation_id` becomes `Option<ConversationId>` and gains `pipeline_id: Option<PipelineId>`. `HarnessDispatcher` gains `with_auditor` and `with_usage_recorder`, matching what the chat path already does at `service.rs:511,640,1206`. **Delete the fabricated `ConversationId(pipeline_id.0)` at `executor.rs:394`** — an identifier that collides with real conversation ids the moment anything records under it. Pipeline spend becomes visible on `/usage`. |
| 231 | `feat(pipeline): add a project backed sandbox lifecycle`          | The real `SandboxLifecycle` (gap 2): resolve the project, take a workspace on the run's branch via `ProjectManager::workspace_for`, provision a session from the project's image, hand back the layered registry. `teardown` drops the session, leaving the **worktree** intact for inspection. Tested against a fake `ProjectManager` for the lifecycle mechanics, and once for real in commit 243.                                                                         |
| 232 | `feat(pipeline): add a pipeline launcher`                         | The factory that does not currently exist anywhere (gap 1): `GitHubForge` + `DieselPipelineStore` + `HarnessDispatcher` + the commit 231 lifecycle + `registry.try_start` + a spawned `Executor::run_with_interaction` + `registry.finish`. Every one of those types is built and tested and has zero production callers today. Spawn with `.instrument(span)`, never `.entered()`, per `docs/tracing.md`.                                                                   |
| 233 | `feat(pipeline): implement pipeline dispatch for forge events`    | `impl PipelineDispatch` (the trait at `webhooks.rs:30-32`, which has no implementation outside its own tests) over the launcher: resolve or auto-register the project, create the `ai_pipelines` row, launch. With this commit, a webhook delivery can start a run.                                                                                                                                                                                                          |
| 234 | `refactor(gui): source webhook triggers from registered projects` | Replace `WebhookConfig::triggers: Vec<RepoTriggerConfig>` (hardcoded empty at `server.rs:71`) with a trigger source backed by the project store, read per delivery. Trigger mode and label live on the project row, so there is no TOML section to invent and the projects page can edit them.                                                                                                                                                                               |
| 235 | `feat(munibot): wire the pipeline launcher into the server`       | Construct the launcher in `munibot/src/ai.rs::build` alongside every other optional capability, and inject it as `WebhookConfig::dispatch`. `Option`, like `ai` itself: no `GITHUB_APP_ID`, no launcher, no startup failure — the convention `server.rs:60-63` already documents.                                                                                                                                                                                            |
| 236 | `feat(munibot): resume non terminal pipelines at boot`            | Call `resume_all` (`resume.rs:47`, zero callers today) with the same launcher, before the server starts accepting webhooks. This is what makes `docs/ai-operations.md:171-174`'s "restarting the process is a valid way to regain control" true rather than aspirational.                                                                                                                                                                                                    |
| 237 | `fix(gui): correct the stale pipeline wiring comments`            | `webhooks.rs:24-25` says the pipeline "doesn't exist yet"; `server.rs:69-73` and `:76-78` describe wiring that now exists. Small, but a comment that lies is worse than no comment.                                                                                                                                                                                                                                                                                          |
| 238 | `docs(ai): document the pipeline launch path in the runbook`      | Add to `docs/ai-operations.md`: how a webhook becomes a run, what `auto_register_installations` does, what happens to a workspace when a run ends, and how to start a run by hand for testing.                                                                                                                                                                                                                                                                               |

---

## Phase 27 — verification

Everything above is still only proven against fakes without this phase. Milestone 4 shipped without
ever building its own image; milestone 5 shipped without ever running its own pipeline. This is where
that stops.

| #   | Commit                                                      | Description                                                                                                                                                                                                                                                                                                                                                                                                         |
| --- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 239 | `chore(devenv): add sandbox image and integration scripts`  | `build-sandbox-image` (the base image, `podman build -f Containerfile .`) and `test-sandbox` (`cargo test --features sandbox-integration`). Nine `sandbox-integration` tests exist and **nothing has ever run them automatically** (`devenv.nix:43-45`). Closes gap 5's local half.                                                                                                                                 |
| 240 | `test(sandbox): add a full chain integration test`          | The test `docs/notes/sandbox-verification-gaps.md:25-40` has been asking for since milestone 4: real base image, real mounted tool agent, real container, real `RpcClient` across the container boundary, one real `Tool` call round-tripping through all of it. Gated on `sandbox-integration`. If this passes, every separately verified piece is confirmed to actually compose.                                  |
| 241 | `test(project): add image build integration tests`          | All three strategies against fixture repositories, including the `devenv.nix`-without-inputs fallthrough and its skip reason, and the cache-hit path from commit 217. Gated on `sandbox-integration`.                                                                                                                                                                                                               |
| 242 | `test(github): add a wiremock backed forge suite`           | Gap 14. `GitHubForge`'s trait bodies have never run against anything. Cover `create_branch`'s idempotent-reuse branch, `push`'s **failure** path (a rejected push, not just a successful one), and `open_pull_request` end to end. `octocrab::GitHubError` is `#[non_exhaustive]` with no public constructor, which is exactly why a mocked HTTP server is the right tool rather than a constructed error.          |
| 243 | `test(pipeline): add a pipeline launch integration test`    | A signed webhook payload → project resolution → executor start → abort, against `MockProvider` so no model is called and no money is spent. The first test in the workspace that exercises the seam this milestone exists to build.                                                                                                                                                                                 |
| 244 | `ci: add a build and test workflow`                         | Gap 15: there is no CI at all. Build, `cargo clippy --all-features -- -D warnings`, `treefmt --fail-on-change`, and `cargo test` on a MySQL and Redis service. Leave `sandbox-integration` out of CI (it needs a rootless podman socket) but state that plainly in the workflow, so its absence is a decision rather than an oversight.                                                                             |
| 245 | `docs(ai): refresh the stale verification notes`            | Rewrite `sandbox-verification-gaps.md` and `pipeline-sandbox-wiring-gap.md` for what is now true, update `github-forge-verification-gaps.md` with commit 242's results, and **delete the two already-fixed storage bugs** from `gui-configuration-research.md:31-53` — `upsert_guild_config` and `/admin stop-logging` were both fixed and the note still claims otherwise. A stale note costs more than no note.   |
| 246 | `fix(nix): provision the sandbox image in the nixos module` | `enableAiSandbox` defaults to `true` and installs podman (`nix/nixos.nix:76-120`) but never supplies an image, so a fresh deploy fails at `create_container` with nothing explaining why. Build the base image as part of the module, or fail activation with a message naming `build-sandbox-image`. Also either use or remove the `nix2container` flake input at `flake.nix:26-29`, which is declared and unused. |

---

## Phase 28 — outstanding loose ends

Real, documented bugs, none of them AI-specific, all of them small and independent. They are last in
the document but they are **not last in priority** — every one is a good warm-up, and two should be
pulled forward.

| Item                             | Pull forward to                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------ |
| `GITHUB_BOT_LOGIN` in secretspec | before phase 26, since webhook testing is the first thing that will notice it missing            |
| OAuth token refresh              | before you spend a week testing through the GUI and get logged out on day seven with no idea why |

| #   | Commit                                                       | Description                                                                                                                                                                                                                                                                                                                                                                                                               |
| --- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 247 | `build(secretspec): declare the github bot login`            | `GITHUB_BOT_LOGIN` is read at `munibot_gui/src/server.rs:67` and declared nowhere. `required = false`, defaulting to `munibot[bot]` as the code already does. Add it to `docs/ai-operations.md`'s GitHub table in the same commit.                                                                                                                                                                                        |
| 248 | `feat(auth): refresh expiring oauth access tokens`           | `docs/gui.md:132-136`. Discord access tokens live about seven days and are never refreshed, so `get_guilds` starts failing and the dashboard silently degrades into a sign-in prompt. Rotate using the stored `token_expires_at` and `refresh_token` before a call rather than after a failure. Remove the "known gaps" entry in the same commit.                                                                         |
| 249 | `fix(discord): invalidate the autodelete cache on a write`   | `AutoDeleteHandler` loads every timer into a `HashMap` once at boot (`autodelete.rs:31-53`) and never re-reads, so any write that is not its own is invisible until restart. Add an invalidation hook the write path calls. Prerequisite for ever managing autodelete from the GUI, and a live bug today for anything that writes the table directly.                                                                     |
| 250 | `fix(db): stop resetting the autodelete sweep cursor`        | `set_autodelete` always writes `last_cleaned: epoch` and `last_message_id_cleaned: 1` alongside the duration, and `upsert_autodelete_timer` uses `replace_into` (`operations.rs:142`), so **editing a timer's duration resets its sweep cursor to the beginning of time** and forces a full re-scan. Split settings columns from sweep-state columns, or at minimum stop rewriting sweep state on a settings-only change. |
| 251 | `fix(twitch): store quote timestamps in utc`                 | `munibot_twitch/src/handlers/quotes.rs:46` stores `Local::now().naive_local()` while every other table stores `Utc::now().naive_utc()`. Fix the write, and decide in the commit body whether existing rows are backfilled or left — either is defensible, silence is not.                                                                                                                                                 |
| 252 | `feat(core): add permission revocation`                      | `revoke_permission` and `list_users_with_permission`, mirroring `grant_permission`. The design work is spelled out at `docs/notes/permission-system.md:36-46`; the hard part is that "no longer matches" must consider **all** of a user's linked accounts, not just the one that was removed.                                                                                                                            |
| 253 | `feat(munibot): revoke operator from unconfigured users`     | Make `sync_operators` a real sync rather than a grant-only pass. Removing someone from `[[operators]]` should actually remove their operator permission. Includes the decision `permission-system.md:44-46` defers: what "no longer matches" means for the `MunibotUser { munibot_user_id }` config variant, which has no linked-account identity to fall out of sync with.                                               |
| 254 | `docs(gui): refresh the stale layout and version references` | `docs/gui.md:3` claims Dioxus 0.7 against a pinned `0.8.0-alpha.0`; the module diagram at `:39-66` predates `munibot/src/ai.rs`, `permissions.rs`, `server/attachments.rs`, and `server/webhooks.rs`; and the Twitch sign-in gap note is out of date now that GitHub and email sign-in exist.                                                                                                                             |
| 255 | `docs(tracing): correct the discord span attribution`        | `docs/tracing.md:53` attributes the `discord{}` root span to `munibot::main`; it lives at `munibot/src/bot.rs:55`. While there, note the two limitations already stated inline — `turn_streamed` covering setup only, and `Passing::pass()` discarding context — in the document's own structure rather than only in passing.                                                                                             |

---

## Definition of done

- `podman images` shows a project image munibot built itself, from that project's own `devenv.nix`
  or `.munibot/Containerfile`, without you running a build command.
- Opening a GitHub issue on a registered repository starts a pipeline you can watch on `/pipelines`,
  abort from that page, and read the full event log of afterwards.
- That pipeline's spend appears on `/usage`, broken down by persona and model, alongside chat spend.
- Killing munibot mid-run and restarting it resumes the run from its event log.
- A `coder` conversation bound to a project can read and edit real files in a real worktree, and the
  worktree is still there afterwards for you to `git diff`.
- A `coder` conversation on a machine with no podman still works, degraded, with a warning — it does
  not fail the turn.
- `devenv shell test-sandbox` passes against real rootless podman.
- A project whose `devenv.nix` is missing the container inputs is told so, in a sentence, on the
  projects page.
- No container outlives its wall-clock limit, and no network policy grants more than it says.

## Risks

1. **This milestone builds a filesystem-mutating subsystem.** Clones, worktrees, and image builds all
   touch real disk outside the process, and every one of them can be interrupted. Reconciliation
   (commit 213's `prune`, commit 221's ordering) is not polish; it is the difference between a crash
   costing a restart and a crash costing manual cleanup.
2. **Nix evaluation is slow and munibot is not.** The devenv image strategy can take minutes on a cold
   cache while a pipeline sits waiting. `max_concurrent_image_builds` and the source-hash cache are
   what keep that survivable; the spans in commits 215 and 216 are what keep it diagnosable.
3. **Commit 204 is load-bearing for everything after it.** If mounting the tool agent into an
   arbitrary image does not work, all three image strategies collapse back to "munibot's image or
   nothing". Run the pre-flight checks first, and do not start phase 25 until 204 is green.
4. **This is the first time model-authored code runs against a real repository.** Everything before
   this ran in an empty workspace. The security posture stops being theoretical here, which is why
   phase 24 fixes the wall-clock and network holes before phase 25 gives the sandbox anything worth
   attacking.
5. **`ProjectManager` will want to become a god object.** It coordinates six modules. Keep it a
   coordinator, keep the work in the modules, and split it the moment it grows past a couple of
   hundred lines — per `AGENTS.md`'s own file-size rule.

## Decisions still open

1. **`NetworkPolicy::Allowlist` needs a real proxy** to mean anything. Commit 201 makes it fail
   loudly rather than lie; building it properly (a filtering proxy the container's only route points
   at) is its own piece of work with no concrete demand behind it yet.
2. **Per-workspace warm containers.** Persistent workspaces with ephemeral containers is the right
   default, but a long coding session pays container startup on every turn. Pooling is the obvious
   next lever and the most state to leak; wait for a real complaint.
3. **Non-GitHub forges.** `munibot_vcs` exists precisely so Forgejo or GitLab is one crate
   implementing two traits. Nothing in this milestone should assume GitHub outside `munibot_github`
   and the installation-registration path in commit 222.
4. **Whether the base image should exist at all** once every project resolves a strategy. It is the
   third fallback and the only one munibot itself maintains; if in practice every real project uses
   one of the first two, it becomes a test fixture rather than a product.
