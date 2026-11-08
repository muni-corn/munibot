# Milestone 09 — projects and workspaces

**Status:** outline. The commit list is provisional. Run the pre-flight checks, then expand this into
a full recipe checked against the code, before starting. Start after
[milestone 08](milestone-08-sandbox.md).

**Outcome:** munibot knows what a _project_ is. A developer registers a repository, and munibot
keeps a checkout of it and builds a container image that matches how that project is actually
developed. Then any conversation can be bound to the project, so the coder and the hands-on team work
in a real worktree of a real repository. Afterwards the worktree is still on disk, ready for you to
`git diff`.

## Why this comes before the pipeline

In the pre-strip plan, projects came _after_ the autonomous pipeline. The result was a pipeline with
no production caller and a sandbox whose image nothing built, because the missing noun in both cases
was the project. Building projects first means milestone 10's pipeline is wired to real repositories
from its first commit.

---

## Starting point

After milestone 08:

- The sandbox runs any image with the static tool agent mounted at `/munibot/toolagent`.
- `WorkspaceSession` lazily provisions one container over one host directory.
- Conversations get scratch workspaces.
- `Permission::Developer` grants `Privileged`, but no `Privileged` tool exists yet.

From the code as it stands, none of the project machinery exists: no `ai_projects`, no git plumbing,
no image strategies.

## Architecture

### Projects, workspaces, sandboxes

```
<ai.projects.root>/
  cocoa/
    repo.git/              bare clone, the single object store
    worktrees/
      chat-8891/           worktree on munibot/chat-8891
      pipeline-142/        worktree on munibot/17-fix-parser   (milestone 10)
```

- A **project** is a git repository munibot has cloned. It has one `ai_projects` row, one directory,
  and one container image.
- A **workspace** is a `git worktree` of the project on its own branch. It has one `ai_workspaces`
  row. Workspaces are **persistent**: they survive a turn, and are collected by age.
- A **sandbox** is an **ephemeral** container over one workspace, built from the project's image.

Projects are munibot-wide, not per-user. Any developer can bind a conversation to any registered
project.

> **Gotcha:** `git clone --bare` sets **no** `remote.origin.fetch` refspec, so a later `git fetch`
> silently updates nothing and every worktree is created from a stale ref. Set
> `remote.origin.fetch = +refs/heads/*:refs/remotes/origin/*` right after cloning, and test that a
> second fetch actually advances a remote-tracking ref.

### Image strategy

A pure function over the worktree decides the strategy, in this order:

| Order | Strategy        | Condition                                                                               | Build                                                                                             |
| ----- | --------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| 1     | `Containerfile` | `.munibot/Containerfile` exists                                                         | `podman build -f .munibot/Containerfile -t munibot-project-<slug>:<hash> <worktree>`              |
| 2     | `Devenv`        | `devenv.nix` exists **and** both `nix2container` and `mk-shell-bin` are declared inputs | `devenv container build shell`, then `devenv container --registry containers-storage: copy shell` |
| 3     | `Base`          | always                                                                                  | none: use `[ai.sandbox].image`                                                                    |

A skipped strategy records **why** on the project row, and the projects page shows it as a sentence,
including the exact `devenv inputs add` commands. munibot never edits the project's own repository
to make a strategy work.

Images are tagged with the first sixteen hex characters of a SHA-256 over the strategy's declared
inputs, plus a salt that changes whenever munibot's image contract does. A build is skipped when that
tag already exists, so a pipeline run doesn't pay for a nix evaluation every time.

Image **builds** have network access, and runtime containers keep `[ai.sandbox].network`. Dependency
fetching belongs in the image.

### What's reused, and what isn't

| Need                              | Choice                                                                                                                                                                            |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Clone, fetch, worktrees, refspecs | **The `git` command line.** `git2` needs libgit2, openssl, and libssh2 under nix, and `gix` can't yet remove or prune worktrees or write config back, which the refspec fix needs |
| Slugs for branch and image names  | `slug`, which transliterates non-ASCII titles. The length cap and collision suffix stay munibot's                                                                                 |
| Building images                   | `podman build` and `devenv container`, run as commands                                                                                                                            |
| Image tags from a source hash     | `sha2`, already a workspace dependency                                                                                                                                            |

Projects themselves are munibot's own concept. Nothing in rig, octocrab, or bollard describes a
repository munibot keeps a checkout of, its worktrees, and how its image is built.

## Schema

```
ai_projects   (id, slug UNIQUE, display_name, clone_url, default_branch,
               image_strategy, image_tag NULL, image_source_hash NULL, image_built_at NULL,
               image_error NULL, skip_reasons JSON, network_policy,
               registered_by_user_id NULL -> users ON DELETE SET NULL, created_at, updated_at)

ai_workspaces (id, project_id -> ai_projects ON DELETE CASCADE, name, branch, path,
               conversation_id NULL -> ai_conversations ON DELETE SET NULL,
               created_at, last_used_at, UNIQUE (project_id, name))

ai_conversations gains project_id NULL -> ai_projects ON DELETE SET NULL
```

`image_strategy` and `network_policy` are stored as their strum snake-case strings, matching
`Permission`.

```toml
[ai.projects]
root = "/var/lib/munibot/projects"
workspace_ttl = "7d"
max_concurrent_image_builds = 1
```

## Pre-flight checks

Record the results in `docs/notes/projects-preflight.md` before writing code:

| Check                                                                                                                                  | Why it matters                                                                               |
| -------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `devenv container build shell`, then `copy shell` to `containers-storage:`, lands an image that rootless podman lists and bollard runs | The devenv strategy depends on it. If not, fall back to `docker-archive:` plus `podman load` |
| `podman build` works with a **git worktree** as its context                                                                            | A worktree has a `.git` _file_, not a directory                                              |
| The static tool agent runs inside a devenv-built image                                                                                 | Confirms milestone 08's mount strategy holds for the image type munibot itself would produce |

## Commits

| #    | Commit                                                          | Summary                                                                                                                                                                                                           |
| ---- | --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 9.1  | `docs(notes): record projects preflight findings`               | The results of the checks above.                                                                                                                                                                                  |
| 9.2  | `feat(project): add project and workspace types`                | `ProjectId`, `WorkspaceId`, `Project`, `Workspace`, `ImageStrategy`, `ImageStatus`, `SkipReason`, and `ProjectError`. Pure types.                                                                                 |
| 9.3  | `feat(db): add projects and workspaces tables`                  | Includes `ai_conversations.project_id`.                                                                                                                                                                           |
| 9.4  | `feat(db): add project and workspace operations`                | Upserts use `on_conflict`, never `replace_into`.                                                                                                                                                                  |
| 9.5  | `feat(config): add the projects section`                        | `[ai.projects]`. The root is created on first use.                                                                                                                                                                |
| 9.6  | `feat(project): add the project store`                          | A trait, diesel and in-memory implementations. Every later test in this milestone uses the in-memory one.                                                                                                         |
| 9.7  | `feat(project): clone a project repository`                     | Bare clone, then the refspec fix. Idempotent: an existing clone is fetched instead. A `CloneCredentials` seam with only a no-credential implementation for now. Tested against a local bare repo with no network. |
| 9.8  | `feat(project): name branches`                                  | `munibot/<scope>-<slug>`, with the slug from the `slug` crate, capped at 60 characters, and an attempt suffix on collision. A pure function.                                                                      |
| 9.9  | `feat(project): manage worktrees`                               | Create, list, remove, and a `prune` that reconciles rows against disk after a crash.                                                                                                                              |
| 9.10 | `feat(project): detect a project's image strategy`              | The ordering table above, with skip reasons written as user-facing sentences. Table tests over fixture directories.                                                                                               |
| 9.11 | `feat(project): build images from a containerfile`              | Output streams into a tracing span. A failure records `image_error` and never leaves a tag marked built.                                                                                                          |
| 9.12 | `feat(project): build images from devenv`                       | Follows pre-flight check 1's outcome.                                                                                                                                                                             |
| 9.13 | `feat(project): cache images by source hash`                    | The hash scheme and salt, a skip when the tag exists, and builds serialized behind `max_concurrent_image_builds`.                                                                                                 |
| 9.14 | `refactor(sandbox): open sessions over any workspace and image` | A session takes a path and an image, and scratch workspaces become one caller of that. No behaviour change.                                                                                                       |
| 9.15 | `feat(project): add the project manager`                        | `register(spec)` and `workspace_for(project, branch, purpose)`. A coordinator only: split it the moment it grows past a couple of hundred lines.                                                                  |
| 9.16 | `feat(project): collect unused workspaces`                      | Removes the worktree, then git's record of it, then the row, in that order, so `prune` can reconcile a crash midway.                                                                                              |
| 9.17 | `feat(api): add project server functions`                       | List, get, register, remove, and rebuild, gated on a new `require_developer`. Removal refuses while a workspace is in use.                                                                                        |
| 9.18 | `feat(gui): add the projects page`                              | `/projects` shows each project's strategy **with skip reasons as prose**, image status and age, workspace count, and network policy, plus rebuild and register actions.                                           |
| 9.19 | `feat(tools): add the project management tool`                  | `manage_project`, a rig tool shelved at `Privileged`: munibot can register and inspect projects when a developer asks. Goes through `ProjectManager::register` like everything else.                              |
| 9.20 | `feat(api): bind a conversation to a project`                   | `set_conversation_project`, owner-checked, and only for developers.                                                                                                                                               |
| 9.21 | `feat(ai): work in a project worktree when bound`               | A bound conversation's sandbox runs over a worktree on `munibot/chat-<id>`, from the project's image. Unbound conversations keep scratch workspaces.                                                              |
| 9.22 | `feat(gui): add the project picker`                             | Beside the persona name: the bound project and its image status.                                                                                                                                                  |
| 9.23 | `test(project): add image build integration tests`              | All three strategies, the devenv-without-inputs fallthrough and its reason, and a cache hit. Gated on `sandbox-integration`.                                                                                      |
| 9.24 | `docs: document projects and workspaces`                        | `docs/ai-projects.md`, cross-linked from the runbook.                                                                                                                                                             |

## Definition of done

- Registering a repository with a `.munibot/Containerfile` builds that image without anyone running
  a build command, and `podman images` shows it.
- A project whose `devenv.nix` lacks the container inputs is told so in a sentence on the projects
  page, with the exact commands to fix it.
- A coder conversation bound to a project reads and edits real files. Afterwards, the worktree is
  still on disk on its own branch.
- An unchanged project isn't rebuilt. Changing its `Containerfile` triggers exactly one rebuild.
- Killing munibot mid-clone or mid-worktree leaves nothing that `prune` can't reconcile.

## Risks

1. **This subsystem mutates the filesystem.** Clones, worktrees, and builds can all be interrupted.
   Reconciliation is the difference between a crash costing a restart and costing manual cleanup.
2. **Nix evaluation is slow.** A cold devenv build can take minutes. The hash cache and build limit
   keep that survivable, and the spans keep it diagnosable.
3. **This is the first time generated code touches a real repository.** Milestone 08's posture stops
   being theoretical here.
4. **`ProjectManager` will want to become a god object.** Keep the work in the modules it
   coordinates.

## Deliberately deferred

- **Private repositories.** Only public clone URLs work until milestone 10 adds GitHub App
  installation tokens through `CloneCredentials`.
- **Registering from a GitHub App installation.** Milestone 10.
- **Whether the base image should exist at all.** If every real project ends up resolving to
  strategy 1 or 2, the base image becomes a test fixture rather than a product.
