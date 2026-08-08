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

## Local mysql `root` password can drift from what `devenv.nix` declares

devenv's `services.mysql.ensureUsers` sets a user's password **at creation time only** -- if the
mysql data directory already existed (e.g. from before a password was added to `devenv.nix`, or from
some other prior state), it won't retroactively enforce the declared password. If some user's access
suddenly starts failing with the password from `devenv.nix` but an empty password works, that's why;
`ALTER USER '<user>'@'localhost' IDENTIFIED BY '<password>';` against the (passwordless) local
instance brings it back in line with the declared config.

`munibot_core`'s `TestDb` used to hit exactly this for `root` specifically, because it used to
connect as `root` (with a password `devenv.nix` never actually declared for that user) just to
create/drop each test's database. That dependency turned out to be unnecessary and has been removed:
mysql's wildcard database-level grants (`GRANT ALL PRIVILEGES ON `` `munibot_test\_%` ``.* TO
'munibot_test'@'localhost'`, already declared in `devenv.nix`) cover `CREATE DATABASE`/
`DROP DATABASE` for any name matching the pattern, not just operations on tables within a database
that already exists. `TestDb` now does everything as `munibot_test`, with no root/admin user
involved at all -- see `munibot_core/tests/common/mod.rs`.

Similar drift can still leave `munibot`/`munibot_test` themselves missing entirely from an old data
directory, since `ensureUsers` never ran against it in the first place. `CREATE USER IF NOT EXISTS`/
`GRANT` by hand, matching `devenv.nix`'s declared username/password/privilege scope, brings an old
data dir back in line.

**A blank-username anonymous account at the same host silently shadows a real, differently-hosted
user of the same name.** MySQL/MariaDB sorts `mysql.user` rows by host specificity first (a literal
host like `localhost` beats a wildcard host like `%`), and only _among rows with the same host_ does
it prefer a non-blank username over a blank one. So if `''@'localhost'` (an anonymous account --
common on distro-default installs, not something this project's devenv config creates) exists, and
the intended account is only registered at `'<user>'@'%'` with no `'<user>'@'localhost'` row, a
client connecting from the local machine matches the anonymous row _before_ it ever considers the
wildcard-host row for the real username -- and authentication is silently checked against the
anonymous account's own credentials instead. The error message still names the client's actual
username and resolved host (e.g. `Access denied for user 'munibot_test'@'localhost'`), which reads
like a simple password mismatch and hides the real cause. Fix: `DROP USER ''@'localhost';` (and any
other anonymous host entries), then `FLUSH PRIVILEGES;`.
