# Projects and workspaces

This describes `ai::project`: what a project and a workspace actually are, the three ways an image
gets built, how a project gets registered, and what to check when any of that goes wrong. Read
`docs/ai-operations.md` first if you haven't -- this document assumes you already know how personas,
sandboxes, and pipelines fit together.

## What a project and a workspace are

A **project** is a git repository munibot has cloned. One row in `ai_projects`, one directory on
disk, one container image.

A **workspace** is a `git worktree` of that project, on its own branch. One row in `ai_workspaces`,
one directory on disk. Workspaces are **persistent** -- they survive a run, so a failed pipeline can
be inspected on disk afterwards, and a resumed pipeline finds its own work still there. They are
removed by age (see [collecting unused workspaces](#collecting-unused-workspaces) below), never by
scope exit.

A **sandbox** is an **ephemeral** container over one workspace, built from that project's image. It
exists for the duration of one turn or one pipeline run and no longer -- see
`munibot_ai::sandbox::session::WorkspaceSession` for how that laziness works.

## Filesystem layout

```
<ai.projects.root>/
  <munibot_user_id>/
    cocoa/
      repo.git/                     bare clone, the single source of objects
      worktrees/
        default/                    checked out at registration, on the default branch
        pipeline-142/                worktree on munibot/issue-17
        chat-8891/                   worktree on munibot/chat-8891
```

The bare clone matters: `git clone --bare` gives one object store that every worktree shares, so
three concurrent workspaces cost three checkouts and one copy of history, not three of each.

### The bare-clone refspec gotcha

`git clone --bare` sets **no** `remote.origin.fetch` refspec. Left alone, a later `git fetch`
updates nothing, and every worktree is created from whatever ref existed the moment the clone ran --
forever. `munibot_ai::project::clone_or_fetch_project` sets
`remote.origin.fetch = +refs/heads/*:refs/remotes/origin/*` explicitly right after cloning, then
fetches immediately, so `refs/remotes/origin/*` (the ref worktree creation actually reads) exists
from the very first moment rather than only after some unrelated later caller happens to fetch
first. If you ever see a worktree checked out at a commit that's inexplicably behind upstream,
check that config line first: `git -C repo.git config remote.origin.fetch`.

## Image strategies

A project's image is resolved by `munibot_ai::project::detect_image_strategy` -- a pure function
over a worktree's contents, no git or podman involved -- in this order:

| Order | Strategy        | Condition                                                                                  | Build                                                                                             |
| ----- | --------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| 1     | `Containerfile` | `.munibot/Containerfile` exists in the repository                                          | `podman build -f .munibot/Containerfile -t munibot-project-<slug>:<hash> <worktree>`              |
| 2     | `Devenv`        | `devenv.nix` exists **and** `devenv.yaml` declares both `nix2container` and `mk-shell-bin` | `devenv container build shell`, then `devenv container copy --registry containers-storage: shell` |
| 3     | `Base`          | always applies                                                                             | none -- `[ai.sandbox].image` is used directly, unmodified                                         |

A project opts into the devenv strategy from its own repository, never by munibot mutating anything
on its behalf:

```bash
devenv inputs add nix2container github:nlewo/nix2container --follows nixpkgs
devenv inputs add mk-shell-bin github:rrbutani/nix-mk-shell-bin
```

Every strategy skipped before the one that matched records a real, actionable reason (a
`SkipReason`) -- what the projects page shows verbatim, so "munibot ignored my devenv" always comes
with "here's the exact command to fix it" instead of a shrug. These reasons are never persisted:
`detect_image_strategy` is cheap enough to call fresh every time a project's detail page loads.

Images are tagged `munibot-project-<slug>:<hash>`, where `<hash>` is the first sixteen hex
characters of a SHA-256 over the strategy's own declared inputs (the bytes of
`.munibot/Containerfile` for strategy 1, or `devenv.nix` + `devenv.yaml` + `devenv.lock` for
strategy 2) plus a constant salt bumped whenever munibot's own image contract changes -- see
`munibot_ai::project::compute_source_hash`. A build is skipped entirely when an image with that
exact tag already exists locally (`ImageBuilder::build_if_needed`), which is what stops every
pipeline run and every `coder` turn paying a nix evaluation.

### When an image build fails

A failed build leaves `image_status` as `Failed { reason }` on the project row, and the projects
page shows `reason` verbatim -- read that first. Beyond that:

- The build itself streams its own output through `tracing` as it runs (`build_containerfile_image`
  and `build_devenv_image` both log every line) -- `RUST_LOG=munibot_ai::project=debug` surfaces it.
- For a `Containerfile` build, the failure is whatever `podman build` printed -- a missing base
  image, a failing `RUN` step, the usual.
- For a `Devenv` build, the failure is whatever `devenv` itself printed -- most often a nix
  evaluation error in the project's own flake, or a rootless podman host missing
  `~/.config/containers/storage.conf` with a `graphroot`/`runroot` matching `podman info`.
- The projects page's "rebuild" button (`rebuild_project_image`) force-removes the local image tag
  and tries again from scratch, bypassing the cache-hit check -- useful when the source hash didn't
  change but something about the host (podman itself, available disk) did.

## Registering a project

Projects are never declared in TOML. Every registration -- however it started -- funnels through
`ProjectManager::register`, which clones (or fetches, if the slug already exists), checks out a
`default` workspace, detects the image strategy, and builds an image if one applies, all before the
project row is considered ready. There are three ways to start that:

1. **The operator projects page** (`/projects`, `register_project`). Fill in a slug, display name,
   repository, clone URL, and default branch. Works today, end to end -- this is the only path with
   a currently reachable caller in a running munibot.
2. **A GitHub App installation event.** `munibot_github::webhook::installation::parse_installation_event`
   and `ProjectManager::register_from_installation` exist and are tested, gated by
   `[ai.projects].auto_register_installations` -- but **nothing in the webhook handler
   (`munibot_gui/src/server/webhooks.rs`) calls either of them yet**. An installation event is
   verified and normalized and then has nowhere to go, the same "built but not connected" state
   `docs/ai-operations.md` documents for the pipeline itself.
3. **The `manage_project` AI tool.** `munibot_ai::tools::ManageProjectTool` and the
   `ProjectManagerRegistrar` bridge exist and are tested -- but **no persona's `tools` configuration
   includes it yet**, so nothing can actually call it in a running munibot today.

If you need (2) or (3) to work right now rather than later in this milestone, wiring them in is
small: a webhook handler calling `register_from_installation` with the parsed event, or a
`tools = ["manage_project"]` entry (once persona tool selection supports it) plus constructing the
tool from a `ProjectManagerRegistrar` the same way `service.rs` builds every other tool.

## Binding a conversation to a project

The chat page's project picker (beside a conversation's "view transcript" link) calls
`set_conversation_project` to bind or unbind a conversation to an already-registered project --
owner-gated, not operator-gated, since choosing which project _your own_ conversation talks about is
not a management action the way registering one is. Once bound, a `coder` persona's sandbox for
that conversation provisions over that project's own worktree (a dedicated `munibot/chat-<id>`
branch, not the project's `default` workspace) instead of a fresh scratch directory every time --
see `munibot_ai::service::Ai::resolve_workspace_override`. A delegated persona's own turn (via the
`delegate` tool) does not inherit this yet; it always gets a scratch workspace, regardless of
whether the outer conversation is bound to a project.

## Collecting unused workspaces

`munibot_ai::project::collect_unused_workspaces` removes every workspace under a project that has
sat unused past `[ai.projects].workspace_ttl`, skipping any a still-running pipeline holds. It is
built and tested, but **nothing schedules it** -- there is no periodic sweep anywhere in any binary
yet. Left alone, a project's `worktrees/` directory only ever grows. Until a sweep exists, run it by
hand (or via a one-off script) if disk usage becomes a concern.

## Configuration

```toml
[ai.projects]
root = "/var/lib/munibot/projects"
# a repository the github app is installed into becomes a project on first use
# (once the webhook handler actually calls register_from_installation - see above)
auto_register_installations = true
# workspaces untouched for this long are eligible for collection
workspace_ttl = "7d"
# how many project images may build concurrently - nix evaluation is slow,
# and this is what keeps two cold-cache builds from thrashing the same host
max_concurrent_image_builds = 1
```

Every field is optional; a config file that never mentions `[ai.projects]` at all still gets a
working, if generic, setup (`munibot_ai::project::ProjectsSettings::default`).

## Further reading

- `docs/plans/ai/milestone-7-projects.md` -- the plan this document was written to satisfy (commit
  228), with the full architecture, database schema, and per-commit rationale behind everything
  above.
- `docs/ai-operations.md` -- personas, providers, safety systems, and the pipeline's own
  not-yet-wired-up status.
