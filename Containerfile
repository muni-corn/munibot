# The ai sandbox image: a Debian base with git and common toolchains, and
# the munibot_toolagent binary baked in. Built and run by ai::sandbox
# (munibot_ai/src/sandbox.rs) through rootless podman - never built or run
# manually as part of the ordinary devenv workflow.
#
# munibot_toolagent takes no munibot dependency at all (see its own
# Cargo.toml), but it is still a member of this workspace, so the builder
# stage needs the whole workspace as build context to resolve Cargo.lock.
#
# NOTE: pinned to a nightly toolchain close to, but not guaranteed identical
# to, the exact nightly devenv.nix pins (languages.rust.channel = "nightly").
# munibot_toolagent's own dependency tree is small and has stayed stable
# across nightlies so far; re-verify this image still builds whenever the
# devenv nightly pin moves.
FROM rustlang/rust:nightly AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked -p munibot_toolagent

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/munibot_toolagent /usr/local/bin/munibot_toolagent

# the repository is checked out here by Sandbox::checkout (commit 143),
# mounted as the one writable path besides /tmp on an otherwise read-only
# root filesystem
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/munibot_toolagent"]
