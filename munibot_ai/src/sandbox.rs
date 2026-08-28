//! The container sandbox: rootless podman lifecycle, repository checkout,
//! and the six filesystem/shell `Tool` implementations that talk to
//! `munibot_toolagent` inside it.
//!
//! Split along the container boundary from `munibot_toolagent`, which is a
//! fully independent crate rather than a module of this one:
//!
//! ```text
//! host                                    │ container
//!                                         │
//! munibot_ai                              │
//!   └── ai::sandbox                       │
//!         ├── podman lifecycle (bollard)  │
//!         ├── Tool impls (read, write,    │   munibot_toolagent
//!         │   edit, bash, grep, glob)     │     ├── RPC server
//!         └── RpcClient ─────────────────────> ├── filesystem + shell execution
//!               Unix socket, length-       │     └── path jail at the repo root
//!               prefixed JSON frames      │
//! ```
//!
//! The host-side `Tool` implementations hold no logic beyond argument
//! marshalling; all execution happens in `munibot_toolagent`. This module
//! mirrors that crate's wire protocol types by hand rather than sharing a
//! dependency on it - the one deliberate duplication this phase accepts, so
//! that a container an attacker's generated shell command can reach never
//! needs to pull in `rig-core`, `diesel-async`, or anything else this crate
//! depends on. See `docs/plans/ai/milestone-4-sandbox.md`'s architecture
//! note for the full rationale.
//!
//! ## Security posture
//!
//! - Rootless podman: the container runs as an unprivileged user mapped into a
//!   user namespace.
//! - No network by default; a persona must explicitly request it.
//! - Read-only root filesystem, writable only at the repository path and
//!   `/tmp`.
//! - Dropped capabilities, `no-new-privileges`, and a seccomp profile.
//! - A wall-clock ceiling on the container itself, independent of the harness's
//!   own turn budget, so a wedged container cannot live forever.
//!
//! Treat every model-authored shell command as hostile. It usually is not,
//! but the one time it is is the one that matters.

pub mod checkout;
pub mod config;
pub mod container;
pub mod protocol;
