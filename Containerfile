# The ai sandbox base image: a Debian base with git and common toolchains.
# Built and run by ai::sandbox (munibot_ai/src/sandbox.rs) through rootless
# podman - never built or run manually as part of the ordinary devenv
# workflow.
#
# No munibot_toolagent here anymore: from
# docs/plans/ai/milestone-7-projects.md commit 204 onward, the tool agent is
# a static musl binary (nix/toolagent.nix, `devenv shell -- build-toolagent`)
# bind-mounted read-only into whichever image a sandbox actually runs, with
# its container entrypoint overridden to run it directly
# (munibot_ai/src/sandbox/container.rs). That is what lets a project's own
# image - one that has never heard of munibot - host the tool agent just as
# well as this one does, through the same mechanism rather than a
# munibot-specific build step.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# the repository is checked out here by Sandbox::checkout, mounted as the
# one writable path besides /tmp on an otherwise read-only root filesystem
WORKDIR /workspace
