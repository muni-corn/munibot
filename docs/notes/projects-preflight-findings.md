# Projects and container pre-flight findings

Results of the pre-flight checks listed in `docs/plans/ai/milestone-7-projects.md`, run before any
implementation work started.

**Verified on:** `rustc 1.99.0-nightly (da86f4d07 2026-07-24)`, edition 2024, resolver 3, `podman
5.8.4` (rootless, systemd-managed `podman.socket`), `devenv 2.2.2`, `nix 2.35.1`.

**Headline:** all four checks pass, but two of them only pass after a host-level fix that has
nothing to do with munibot's own code. Both fixes are recorded here so commit 216 and
`docs/ai-operations.md` can tell an operator what to do rather than let them rediscover this by
staring at a `mkdir /run/containers: permission denied` error.

## Summary

| Check                                                                             | Result                                                                                                                                                 |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `podman build` with a git worktree as context                                     | Pass, unmodified                                                                                                                                       |
| Bind-mount to a nonexistent path under `readonly_rootfs: true`                    | Pass, unmodified                                                                                                                                       |
| `bollard`/podman entrypoint override beats the image's own `ENTRYPOINT`           | Pass, unmodified                                                                                                                                       |
| Static musl `munibot_toolagent` runs in alpine, debian, and a nix2container image | Pass, **but only at `opt-level` 0 or 1** — `opt-level` 2 and 3 ICE the pinned nightly on this target                                                   |
| `devenv container build shell` + `containers-storage:` copy                       | Pass, **but only after writing a rootless `~/.config/containers/storage.conf`** — otherwise skopeo tries to `mkdir /run/containers` as if it were root |

## 1. `podman build` accepts a git worktree as context, no caveats

A worktree's `.git` is a file (`gitdir: /path/to/repo.git/worktrees/name`), not a directory. Nothing
in `podman build`'s context-tar walk cares: it built a two-line `Containerfile` against a worktree
exactly as it would against a normal clone. Commit 215 needs no worktree-specific handling.

## 2. Bind-mount-to-nonexistent-path and entrypoint override both work exactly as commit 204 assumes

```
podman run --rm --read-only \
  --entrypoint /munibot/toolagent \
  -v /host/fake-toolagent:/munibot/toolagent:ro,Z \
  alpine:latest --some-cmd-arg
# => runs the mounted binary, /munibot/toolagent did not exist in the image
```

Repeated against an image with its own `ENTRYPOINT ["echo", "this should never print"]`: the
override wins, nothing from the image's entrypoint runs. Podman creates bind-mount target paths
before sealing the read-only rootfs, confirming the "normally" in the plan's own phrasing. `bollard`
issues the identical libpod call `podman run` does, so this transfers directly to commit 204's
`Config { entrypoint, cmd, .. }`.

## 3. The static musl tool agent runs unmodified in alpine, debian, and a nix2container image — with one build caveat

`cargo build --release --target x86_64-unknown-linux-musl -p munibot_toolagent`, with the target
added to `languages.rust.targets` in `devenv.nix`, produces a `static-pie linked` binary (`ldd`:
"statically linked") that ran unmodified — same `--help` output, same behavior — under:

- `alpine:latest`
- `docker.io/library/debian:bookworm-slim`
- the nix2container image built for check 4 below

**Caveat, and it is a real one, not a style nit:** the pinned nightly (`da86f4d07`, 2026-07-24)
**internally-compilation-errors while codegen'ing `tokio` for this target at `opt-level` 2 or 3**:

```
error: internal compiler error: .../rustc_codegen_ssa/src/mir/operand.rs:291:18:
  not immediate: OperandRef(Uninit @ TyAndLayout {
    ty: unsafe_cell::UnsafeCell<MaybeUninit<runtime::task::Notified<Arc<multi_thread::handle::Handle>>>>,
    ...
  })
```

`opt-level = 0` and `opt-level = 1` both build cleanly and produce a working static binary; only
`2` and `3` (cargo's `--release` default) trigger the ICE. Commit 202's `build-toolagent` script
must set `RUSTFLAGS="-C opt-level=1"` (not `opt-level=0` — the tool agent should still run
reasonably fast inside a wall-clock-limited container) alongside `--release`, and its `ldd` assertion
should stay as planned. This is worth re-checking against a newer nightly someday; it reads like an
LLVM codegen bug narrow to this exact snapshot rather than anything musl-specific to our code, and
the workaround costs nothing but a flag.

## 4. `devenv container build shell` + `containers-storage:` copy works, but needs a rootless storage config the host doesn't have by default

Tested against a throwaway fixture (`devenv.yaml` declaring `nix2container` and `mk-shell-bin`,
per the milestone's own `devenv inputs add` instructions), never against munibot's own
`devenv.yaml` — matching the plan's rule that a project's own repository is never mutated to make a
strategy work.

`devenv container build shell` succeeded on the first try and produced a nix2container image
manifest. `devenv container copy --registry containers-storage: shell` (see below for the corrected
argument order) initially failed:

```
level=fatal msg="Invalid destination name containers-storage:shell:latest: mkdir /run/containers: permission denied"
```

`skopeo` (invoked internally by `devenv container copy`) reads `/etc/containers/storage.conf`
literally rather than applying podman's own rootless auto-detection (swapping `graphroot`/`runroot`
to per-user paths when `geteuid() != 0`). The system-wide file on this host points `runroot` at
`/run/containers/storage`, a root-owned path. **Fix:** a rootless override at
`~/.config/containers/storage.conf`:

```toml
[storage]
driver = "overlay"
graphroot = "/home/<user>/.local/share/containers/storage"
runroot = "/run/user/<uid>/containers"
```

(Match `graphroot`/`runroot` to whatever `podman info --format json | jq .store` already reports for
your rootless setup — podman itself gets these right without any config file present; skopeo does
not.) After adding this file, the copy succeeded, `podman images` listed `shell:latest`, and
`podman run --rm docker.io/library/shell:latest hello` (the fixture's own test package) printed
`Hello, world!`. Commit 216 must document this file as an operator prerequisite for the devenv image
strategy — a fresh rootless-podman host will hit exactly this error the first time it builds a
project's devenv image — and `docs/ai-projects.md` (commit 228) and `docs/ai-operations.md` should
both mention it.

**Also corrected:** the plan's own CLI syntax, `devenv container --registry containers-storage: copy
shell`, does not match `devenv` 2.2.2. The registry flag belongs on the `copy` subcommand itself:
`devenv container copy --registry containers-storage: shell`.

## Net effect on the plan

No design decision changes. Both fixes are additive:

- Commit 202's build script pins `RUSTFLAGS="-C opt-level=1"` for the musl target instead of
  trusting cargo's release default.
- Commit 216 (and its documentation) states the rootless `storage.conf` prerequisite explicitly,
  rather than the devenv image strategy failing on a fresh host with an error that names `skopeo`
  and `/run/containers` but nothing about rootless podman.
