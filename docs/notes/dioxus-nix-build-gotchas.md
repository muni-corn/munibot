# Notes: dioxus + nix build gotchas

Things that weren't obvious while wiring up the gui (see `docs/gui.md`) and the `dx bundle` nix build
(`nix/build.nix`). Leaving these here since they're the kind of thing that's fast to re-discover the
hard way.

## `RUSTFLAGS` from `mold`/rustflags breaks the wasm build

devenv's `languages.rust.mold.enable` sets `RUSTFLAGS=-C link-arg=-fuse-ld=mold` as a plain
environment variable, and a custom `languages.rust.rustflags` string does the same via
`.cargo/config.toml`'s `build.rustflags`. Both apply **globally, regardless of `--target`** --
including `wasm32-unknown-unknown`, which has no cc-style linker driver (`rustc` invokes `rust-lld`
directly) and doesn't understand `-fuse-ld` or `-Wl,-rpath` at all. The wasm build fails outright
with a `rust-lld` link error if either is set.

If a project needs both a native mold-linked binary and a wasm target, don't reach for
`languages.rust.mold.enable` or a blanket `rustflags` string -- there isn't a clean per-target way to
scope `RUSTFLAGS`/`build.rustflags` from within devenv's rust module. We just dropped both.

**If you're debugging a "why is my wasm build failing" issue in a long-running shell session**,
also check whether `RUSTFLAGS`/`RUSTDOCFLAGS` got exported into your _current shell_ from an earlier
`devenv shell` invocation, before the fix. Fixing `devenv.nix` doesn't retroactively unset an
already-exported variable in a shell that was entered before the fix -- `unset RUSTFLAGS
RUSTDOCFLAGS` (or `env -u RUSTFLAGS -u RUSTDOCFLAGS <cmd>`) before re-testing.

## `dx`'s per-target feature selection

`dx serve`/`dx bundle --fullstack` correctly builds the wasm client with the `web` feature only
(no `server`) and the native binary with `server` -- confirmed empirically, since munibot's `server`
feature pulls in diesel/tokio/mysql client bindings that don't support wasm32 at all, and the wasm
build succeeds. You don't need to pass `--features` explicitly for normal dev/prod use.

## `dx bundle --release` output layout differs from debug

In `--release` mode, the wasm file lives at `public/assets/<name>_bg-dxh<hash>.wasm` (hashed, for
manganis). In **debug** mode (`dx serve`'s default), it's `public/wasm/<name>_bg.wasm` -- no hash, no
`assets/` prefix. If you're writing a script that finds the wasm file, make sure you're checking a
release build's layout, not debug's.

## `wasm-opt` and the nix sandbox

`dx bundle --release` runs `wasm-opt` (from `binaryen`) on the compiled wasm. Under the nix build
sandbox, binaryen's thread pool spawning gets blocked by the seccomp profile and SIGABRTs. The
workaround (borrowed from musicaloft-web, see `nix/build.nix`): put a passthrough shell script named
`wasm-opt` on `$PATH` ahead of the real one before running `dx bundle` (so dx's own internal call
succeeds as a no-op), then invoke the _real_ `wasm-opt` **by absolute path** afterward for the actual
optimization pass -- a bare `wasm-opt` call after `dx bundle` would still hit the passthrough stub,
since the `$PATH` override from before `dx bundle` is still in effect for the rest of the script.

Also: `export NO_DOWNLOADS=1` before running `dx bundle`, or it tries to download its own copy of
`wasm-opt` instead of using `$PATH`'s (and fails, no network in the sandbox).

## Cargo workspaces share one `target/`, even across member directories

If your `dx`-managed crate lives in a subdirectory of a cargo workspace (like `munibot/` here, with
`munibot_core`/`munibot_discord`/`munibot_twitch` as siblings), running `dx bundle` from inside that
subdirectory still writes to `<workspace-root>/target/dx/...`, not `<subdir>/target/dx/...`. Any
script that `cd`s into the crate directory before running `dx bundle` needs to `cd` back out (or use
`../target/...`) before looking for build output.

This also means `embed_migrations!("../migrations")` (in `munibot_core`) resolves correctly against
the _real_ sibling `migrations/` directory when the whole workspace is present as one source tree --
no symlink workaround needed, unlike the old crate2nix build, which unpacked each crate in isolation.

## `libmysqlclient`'s `.so` lives one directory too deep for `autoPatchelfHook`

nixpkgs' `libmysqlclient` (really `mariadb-connector-c`) puts `libmariadb.so.3` at
`lib/mariadb/libmariadb.so.3`, not `lib/libmariadb.so.3`. `autoPatchelfHook` only scans the top-level
`lib/` of each `buildInputs`/`runtimeDependencies` package by default, so it reports
`libmariadb.so.3 -> not found!` even though the package is right there in the search list. Fix:
register the extra path explicitly in `preFixup`:

```nix
preFixup = ''
  addAutoPatchelfSearchPath ${pkgs.libmysqlclient}/lib/mariadb
'';
```
