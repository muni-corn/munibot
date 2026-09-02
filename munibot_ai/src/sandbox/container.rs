//! Container lifecycle: create, start, stop, and remove one sandbox
//! container through bollard, applying every resource and security limit
//! from [`SandboxConfig`].

use std::{collections::HashMap, path::PathBuf};

use bollard::{
    Docker,
    models::{ContainerCreateBody, HostConfig},
    query_parameters::{RemoveContainerOptions, StopContainerOptions},
};
use tokio_util::sync::CancellationToken;

use crate::{
    sandbox::config::{NetworkPolicy, SandboxConfig},
    types::AiError,
};

/// Where the repository is mounted inside every sandbox container - the one
/// writable path on an otherwise read-only root filesystem besides `/tmp`.
/// Matches the `Containerfile`'s own `WORKDIR /workspace`.
pub const WORKSPACE_MOUNT_PATH: &str = "/workspace";

/// Where the per-sandbox rpc socket is mounted inside the container.
pub const SOCKET_MOUNT_DIR: &str = "/run/toolagent";

/// The socket's filename within [`SOCKET_MOUNT_DIR`] (and its host-side
/// mirror, [`Sandbox::socket_host_path`]).
pub const SOCKET_FILENAME: &str = "agent.sock";

/// Where the tool agent binary is bind-mounted inside the container when
/// [`SandboxConfig::tool_agent_path`] is set. The container's entrypoint is
/// overridden to run this path directly, which is what lets an image that
/// has never heard of munibot still host the tool agent - see
/// `docs/plans/ai/milestone-7-projects.md`, commit 204.
pub const TOOL_AGENT_MOUNT_PATH: &str = "/munibot/toolagent";

/// How long `stop` waits for the container to exit on its own before
/// escalating to a kill signal.
const STOP_TIMEOUT_SECS: i32 = 10;

/// One rootless podman container, and everything needed to create, start,
/// stop, and remove it.
///
/// `Drop` triggers a best-effort removal so a panicking test - or a turn
/// that errors out before its own cleanup path runs - can never leak a
/// container indefinitely; see this type's own `impl Drop`.
pub struct Sandbox {
    docker: Docker,
    config: SandboxConfig,
    container_id: Option<String>,
    /// The host directory bind-mounted at [`WORKSPACE_MOUNT_PATH`] - `None`
    /// until [`Self::with_workspace_mount`] sets it, which
    /// [`Self::checkout`] (commit 143) requires before it can do anything.
    workspace_mount: Option<PathBuf>,
    /// The host directory bind-mounted at [`SOCKET_MOUNT_DIR`], holding
    /// just the tool agent's rpc socket. Generated fresh for every
    /// sandbox - unlike the workspace mount, nothing external needs to
    /// choose this path, so there is no `with_socket_mount` builder to
    /// match [`Self::with_workspace_mount`].
    socket_host_dir: PathBuf,
    /// Cancelled when this sandbox tears down normally (`stop`, `remove`,
    /// or `Drop`), so the wall-clock watchdog [`Self::start`] spawns never
    /// outlives the container it watches.
    wall_clock_watchdog: CancellationToken,
}

impl Sandbox {
    /// Connects to rootless podman and builds a sandbox around `config`.
    /// Creates no container yet - see [`Self::create`].
    pub fn new(config: SandboxConfig) -> Result<Self, AiError> {
        let docker = Docker::connect_with_podman_defaults()
            .map_err(|error| AiError::Other(format!("couldn't reach podman :< {error}")))?;
        Ok(Self::with_docker(docker, config))
    }

    /// Builds a sandbox over an already-connected [`Docker`] handle - what
    /// [`Self::new`] uses internally, and what tests use to talk to the
    /// same rootless podman socket without duplicating the connection
    /// logic (or, in a mock provider's case, cannot substitute at all,
    /// since `Docker` itself has no trait to mock behind - see this
    /// module's own tests for why every test here is a real integration
    /// test rather than one against a fake).
    pub(crate) fn with_docker(docker: Docker, config: SandboxConfig) -> Self {
        Self {
            docker,
            config,
            container_id: None,
            workspace_mount: None,
            socket_host_dir: fresh_socket_host_dir(),
            wall_clock_watchdog: CancellationToken::new(),
        }
    }

    /// Sets the host directory bind-mounted at [`WORKSPACE_MOUNT_PATH`]
    /// once the container is created - the one writable path on an
    /// otherwise read-only root filesystem besides `/tmp`.
    ///
    /// Must be called before [`Self::create`]; the mount is part of the
    /// container's own creation options and cannot be added to one that
    /// already exists.
    pub fn with_workspace_mount(mut self, host_path: impl Into<PathBuf>) -> Self {
        self.workspace_mount = Some(host_path.into());
        self
    }

    /// Creates a container from `config.image`, applying every resource
    /// and security limit from [`SandboxConfig`], with `cmd` as the
    /// entrypoint's arguments (empty uses the image's own default).
    ///
    /// When [`SandboxConfig::tool_agent_path`] is set, the entrypoint is
    /// overridden to [`TOOL_AGENT_MOUNT_PATH`] regardless of what the
    /// image itself declares - see [`Self::build_host_config`] for the
    /// bind mount this pairs with. `cmd` is still whatever the caller
    /// passes (production callers pass [`Self::tool_agent_cmd`]).
    ///
    /// Fails if this sandbox already has a container - each `Sandbox`
    /// value owns at most one container over its lifetime; a caller
    /// wanting a fresh one builds a fresh `Sandbox`.
    pub async fn create(&mut self, cmd: Vec<String>) -> Result<(), AiError> {
        if self.container_id.is_some() {
            return Err(AiError::Other(
                "this sandbox already has a container :< remove it first".to_string(),
            ));
        }

        let entrypoint = self
            .config
            .tool_agent_path
            .is_some()
            .then(|| vec![TOOL_AGENT_MOUNT_PATH.to_string()]);

        let body = ContainerCreateBody {
            image: Some(self.config.image.clone()),
            entrypoint,
            cmd: (!cmd.is_empty()).then_some(cmd),
            host_config: Some(self.build_host_config()?),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(None, body)
            .await
            .map_err(|error| AiError::Other(format!("couldn't create the sandbox :< {error}")))?;

        self.container_id = Some(response.id);
        Ok(())
    }

    /// Every resource and security option [`SandboxConfig`] and the
    /// milestone's own security posture call for: fractional CPUs via
    /// `nano_cpus`, a memory ceiling, a pids limit, every Linux capability
    /// dropped, `no-new-privileges`, a read-only root filesystem with a
    /// writable `/tmp`, and network access exactly as narrow as the
    /// configured [`NetworkPolicy`] - see the network table in
    /// `docs/plans/ai/milestone-7-projects.md`.
    ///
    /// Fails for [`NetworkPolicy::Allowlist`]: no filtering proxy exists
    /// yet to point the container's only route at, so this refuses to
    /// silently grant full network the way it used to.
    fn build_host_config(&self) -> Result<HostConfig, AiError> {
        let nano_cpus = (self.config.cpu_quota * 1_000_000_000.0).round() as i64;

        let network_mode = match &self.config.network {
            NetworkPolicy::None => "none",
            NetworkPolicy::Full => "bridge",
            NetworkPolicy::Allowlist(hosts) => {
                return Err(AiError::Other(format!(
                    "NetworkPolicy::Allowlist({hosts:?}) isn't implemented yet :< no filtering \
                     proxy exists to point the container's route at - use NetworkPolicy::None or \
                     NetworkPolicy::Full instead"
                )));
            }
        };

        let mut binds = vec![format!(
            "{}:{SOCKET_MOUNT_DIR}:rw",
            self.socket_host_dir.display()
        )];
        if let Some(host_path) = &self.workspace_mount {
            binds.push(format!("{}:{WORKSPACE_MOUNT_PATH}:rw", host_path.display()));
        }
        if let Some(tool_agent_path) = &self.config.tool_agent_path {
            binds.push(format!(
                "{}:{TOOL_AGENT_MOUNT_PATH}:ro",
                tool_agent_path.display()
            ));
        }

        Ok(HostConfig {
            nano_cpus: Some(nano_cpus),
            memory: Some(self.config.memory_limit_bytes as i64),
            pids_limit: Some(self.config.pids_limit),
            network_mode: Some(network_mode.to_string()),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges".to_string()]),
            readonly_rootfs: Some(true),
            // the workspace and socket bind mounts are the only other
            // writable paths besides /tmp on this otherwise read-only
            // root filesystem
            binds: Some(binds),
            tmpfs: Some(HashMap::from([("/tmp".to_string(), String::new())])),
            ..Default::default()
        })
    }

    /// This sandbox's configured workspace mount, if
    /// [`Self::with_workspace_mount`] set one.
    pub fn workspace_mount(&self) -> Option<&std::path::Path> {
        self.workspace_mount.as_deref()
    }

    /// The host-side path of the tool agent's rpc socket, once it starts
    /// listening - the file itself does not exist until the container
    /// actually creates it, but the containing directory (bind-mounted at
    /// [`SOCKET_MOUNT_DIR`]) is created up front by [`fresh_socket_host_dir`].
    pub fn socket_host_path(&self) -> PathBuf {
        self.socket_host_dir.join(SOCKET_FILENAME)
    }

    /// The `munibot_toolagent` arguments this sandbox's container should
    /// run with - the socket and repository root paths as seen from
    /// *inside* the container, matching [`SOCKET_MOUNT_DIR`] and
    /// [`WORKSPACE_MOUNT_PATH`].
    pub fn tool_agent_cmd(&self) -> Vec<String> {
        vec![
            "--socket".to_string(),
            format!("{SOCKET_MOUNT_DIR}/{SOCKET_FILENAME}"),
            "--root".to_string(),
            WORKSPACE_MOUNT_PATH.to_string(),
        ]
    }

    /// Runs `cmd` inside the running container via `docker exec`, waiting
    /// for it to finish and failing if it exits non-zero.
    ///
    /// Used by [`Self::checkout`] to run a repository's own dependency
    /// install step - anything a project's install scripts do (an npm
    /// postinstall hook, a `setup.py`, a build-time proc macro) executes
    /// inside the container's own isolation, never on the host.
    pub(crate) async fn exec(&self, cmd: Vec<String>) -> Result<(), AiError> {
        let id = self.require_id()?;

        let exec = self
            .docker
            .create_exec(id, bollard::exec::CreateExecOptions {
                cmd: Some(cmd),
                working_dir: Some(WORKSPACE_MOUNT_PATH.to_string()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            })
            .await
            .map_err(|error| AiError::Other(format!("couldn't prepare a command :< {error}")))?;

        let start_result = self
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(|error| AiError::Other(format!("couldn't run a command :< {error}")))?;

        // the output stream must be drained even though its contents are
        // discarded here - the exec process does not finish (and
        // inspect_exec below never reports an exit code) until this side
        // has read it to completion
        if let bollard::exec::StartExecResults::Attached { mut output, .. } = start_result {
            use futures::StreamExt;
            while output.next().await.is_some() {}
        }

        let inspected = self.docker.inspect_exec(&exec.id).await.map_err(|error| {
            AiError::Other(format!("couldn't check a command's result :< {error}"))
        })?;

        match inspected.exit_code {
            Some(0) => Ok(()),
            other => Err(AiError::Other(format!(
                "command exited with status {other:?}"
            ))),
        }
    }

    /// Starts the created container, and arms a watchdog that force-stops
    /// it after `config.wall_clock_limit` - a ceiling independent of
    /// whatever budget the harness's own turn is running under, so a
    /// wedged container (or a turn that never comes back to tear it down)
    /// can never live forever. [`Self::stop`], [`Self::remove`], and
    /// `Drop` all cancel the watchdog first, so a normal teardown never
    /// races it.
    pub async fn start(&self) -> Result<(), AiError> {
        let id = self.require_id()?;
        self.docker
            .start_container(id, None)
            .await
            .map_err(|error| AiError::Other(format!("couldn't start the sandbox :< {error}")))?;

        self.arm_wall_clock_watchdog(id.to_string());
        Ok(())
    }

    /// Spawns the task that stops this sandbox's container if
    /// `config.wall_clock_limit` elapses before the watchdog is cancelled.
    /// A no-op if no tokio runtime is current (matches [`Drop`]'s own
    /// best-effort posture below) - `start` still succeeds either way,
    /// since a missing watchdog is a lesser failure than refusing to start
    /// the container at all.
    fn arm_wall_clock_watchdog(&self, container_id: String) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                container_id = %container_id,
                "started a sandbox with no tokio runtime available to arm its wall-clock watchdog"
            );
            return;
        };

        let docker = self.docker.clone();
        let limit = self.config.wall_clock_limit;
        let watchdog = self.wall_clock_watchdog.clone();
        handle.spawn(async move {
            tokio::select! {
                () = watchdog.cancelled() => {}
                () = tokio::time::sleep(limit) => {
                    tracing::warn!(
                        container_id = %container_id,
                        wall_clock_limit_secs = limit.as_secs(),
                        "sandbox container exceeded its wall-clock limit, stopping it"
                    );
                    let options = StopContainerOptions {
                        t: Some(STOP_TIMEOUT_SECS),
                        ..Default::default()
                    };
                    if let Err(error) = docker.stop_container(&container_id, Some(options)).await {
                        tracing::warn!(
                            %error,
                            container_id = %container_id,
                            "wall-clock watchdog couldn't stop an overrunning sandbox container"
                        );
                    }
                }
            }
        });
    }

    /// Stops the container, giving it [`STOP_TIMEOUT_SECS`] to exit on its
    /// own before podman escalates to `SIGKILL`.
    pub async fn stop(&self) -> Result<(), AiError> {
        let id = self.require_id()?;
        self.wall_clock_watchdog.cancel();
        let options = StopContainerOptions {
            t: Some(STOP_TIMEOUT_SECS),
            ..Default::default()
        };
        self.docker
            .stop_container(id, Some(options))
            .await
            .map_err(|error| AiError::Other(format!("couldn't stop the sandbox :< {error}")))
    }

    /// Force-removes the container, whether or not it has already stopped.
    pub async fn remove(&mut self) -> Result<(), AiError> {
        self.wall_clock_watchdog.cancel();
        let id = self.container_id.take().ok_or_else(|| {
            AiError::Other("this sandbox has no container to remove :<".to_string())
        })?;
        remove_container_best_effort(&self.docker, &id)
            .await
            .map_err(|error| AiError::Other(format!("couldn't remove the sandbox :< {error}")))
    }

    /// This sandbox's container id, once [`Self::create`] has run.
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    fn require_id(&self) -> Result<&str, AiError> {
        self.container_id
            .as_deref()
            .ok_or_else(|| AiError::Other("this sandbox has no container yet :<".to_string()))
    }
}

/// Creates a fresh, unique host directory to bind-mount the tool agent's
/// rpc socket into a container from.
///
/// Prefers `$XDG_RUNTIME_DIR` (already tmpfs-backed on every system this
/// runs on - it's where rootless podman's own socket already lives, per
/// `devenv.nix`) and falls back to `/dev/shm`, so the socket file never
/// touches a real disk.
fn fresh_socket_host_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/dev/shm"));

    let dir = base.join(format!(
        "munibot-sandbox-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    // best-effort: if this somehow fails, the later bind-mount attempt
    // fails loudly instead when the container is actually created, which
    // is a clearer error than one from here ever could be
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Removes a container by id, force-removing so a still-running container
/// (one that never got a chance to `stop` cleanly) is torn down anyway.
async fn remove_container_best_effort(
    docker: &Docker,
    id: &str,
) -> Result<(), bollard::errors::Error> {
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    docker.remove_container(id, Some(options)).await
}

impl Drop for Sandbox {
    /// Best-effort cleanup: a panicking test, or a caller that errors out
    /// before reaching its own `remove` call, must never leak a container
    /// indefinitely. `Drop` cannot be `async`, so this spawns a detached
    /// task on whatever Tokio runtime is current - and, since a runtime is
    /// not guaranteed to still be around by the time this actually runs
    /// (process shutdown, a runtime already torn down), logs rather than
    /// panicking when there isn't one, since a container link left for
    /// podman's own garbage collection is still far better than a panic in
    /// a destructor.
    fn drop(&mut self) {
        // cancel the wall-clock watchdog before anything else - a
        // container this `Drop` is about to remove itself must never race
        // a spawned watchdog task also trying to stop it
        self.wall_clock_watchdog.cancel();

        // the socket directory is a plain host directory, cleaned up
        // synchronously regardless of whether a container ever existed -
        // unlike the container itself, there is no async daemon call
        // needed to remove it
        std::fs::remove_dir_all(&self.socket_host_dir).ok();

        let Some(id) = self.container_id.take() else {
            return;
        };

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                container_id = %id,
                "dropped a sandbox with no tokio runtime available to clean it up"
            );
            return;
        };

        let docker = self.docker.clone();
        handle.spawn(async move {
            if let Err(error) = remove_container_best_effort(&docker, &id).await {
                tracing::warn!(
                    %error,
                    container_id = %id,
                    "best-effort cleanup of a dropped sandbox container failed"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test in this module talks to a **real** rootless podman
    /// socket - `Docker` has no trait to substitute a fake behind, so
    /// there is no meaningful way to unit test container lifecycle without
    /// a real daemon. Gated behind the `sandbox-integration` feature so
    /// `devenv test`/`cargo test` stay green on a machine where podman was
    /// never set up; see `docs/notes/ai-preflight-findings.md`.
    fn test_config(image: &str) -> SandboxConfig {
        SandboxConfig {
            image: image.to_string(),
            cpu_quota: 0.5,
            memory_limit_bytes: 64 * 1024 * 1024,
            pids_limit: 32,
            disk_limit_bytes: None,
            network: NetworkPolicy::None,
            wall_clock_limit: std::time::Duration::from_secs(30),
            tool_agent_path: None,
        }
    }

    fn docker() -> Docker {
        Docker::connect_with_podman_defaults().expect("podman should be reachable for this test")
    }

    #[test]
    fn test_build_host_config_applies_the_security_posture() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let host_config = sandbox.build_host_config().expect("should build");

        assert_eq!(host_config.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(
            host_config.security_opt,
            Some(vec!["no-new-privileges".to_string()])
        );
        assert_eq!(host_config.readonly_rootfs, Some(true));
        assert_eq!(host_config.network_mode, Some("none".to_string()));
        assert!(host_config.tmpfs.unwrap().contains_key("/tmp"));
    }

    #[test]
    fn test_build_host_config_converts_cpu_quota_to_nano_cpus() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let host_config = sandbox.build_host_config().expect("should build");
        assert_eq!(host_config.nano_cpus, Some(500_000_000));
    }

    #[test]
    fn test_build_host_config_carries_the_memory_and_pids_limits() {
        let config = test_config("alpine:latest");
        let sandbox = Sandbox::with_docker(docker(), config.clone());
        let host_config = sandbox.build_host_config().expect("should build");
        assert_eq!(host_config.memory, Some(config.memory_limit_bytes as i64));
        assert_eq!(host_config.pids_limit, Some(config.pids_limit));
    }

    #[test]
    fn test_with_workspace_mount_adds_a_bind_mount() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"))
            .with_workspace_mount("/host/repo");
        let host_config = sandbox.build_host_config().expect("should build");
        let binds = host_config.binds.expect("should have binds");
        assert!(binds.contains(&format!("/host/repo:{WORKSPACE_MOUNT_PATH}:rw")));
    }

    #[test]
    fn test_no_workspace_mount_means_only_the_socket_bind() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let binds = sandbox
            .build_host_config()
            .expect("should build")
            .binds
            .expect("the socket mount is always present");
        assert_eq!(binds.len(), 1);
        assert!(binds[0].ends_with(&format!(":{SOCKET_MOUNT_DIR}:rw")));
    }

    #[test]
    fn test_full_network_policy_maps_to_bridge() {
        let mut config = test_config("alpine:latest");
        config.network = NetworkPolicy::Full;
        let sandbox = Sandbox::with_docker(docker(), config);
        let host_config = sandbox.build_host_config().expect("should build");
        assert_eq!(host_config.network_mode, Some("bridge".to_string()));
    }

    #[test]
    fn test_no_tool_agent_path_means_no_extra_bind_and_only_the_socket_mount() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let binds = sandbox
            .build_host_config()
            .expect("should build")
            .binds
            .expect("the socket mount is always present");
        assert_eq!(binds.len(), 1);
    }

    #[test]
    fn test_tool_agent_path_adds_a_read_only_bind_mount() {
        let mut config = test_config("alpine:latest");
        config.tool_agent_path = Some(PathBuf::from("/host/munibot_toolagent"));
        let sandbox = Sandbox::with_docker(docker(), config);
        let binds = sandbox
            .build_host_config()
            .expect("should build")
            .binds
            .unwrap();
        assert!(binds.contains(&format!(
            "/host/munibot_toolagent:{TOOL_AGENT_MOUNT_PATH}:ro"
        )));
    }

    #[test]
    fn test_allowlist_network_policy_fails_loudly_instead_of_granting_full_network() {
        let mut config = test_config("alpine:latest");
        config.network = NetworkPolicy::Allowlist(vec!["api.example.com".to_string()]);
        let sandbox = Sandbox::with_docker(docker(), config);
        let error = sandbox
            .build_host_config()
            .expect_err("an allowlist should be refused, not silently widened to full network");
        assert!(error.to_string().contains("Allowlist"));
    }

    #[test]
    fn test_socket_host_path_lives_under_the_generated_socket_directory() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let path = sandbox.socket_host_path();
        assert_eq!(path.file_name().unwrap(), SOCKET_FILENAME);
        assert!(path.to_string_lossy().contains("munibot-sandbox-"));
    }

    #[test]
    fn test_tool_agent_cmd_points_at_the_in_container_mount_paths() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let cmd = sandbox.tool_agent_cmd();
        assert_eq!(cmd, vec![
            "--socket".to_string(),
            format!("{SOCKET_MOUNT_DIR}/{SOCKET_FILENAME}"),
            "--root".to_string(),
            WORKSPACE_MOUNT_PATH.to_string(),
        ]);
    }

    #[test]
    fn test_each_sandbox_gets_a_distinct_socket_host_directory() {
        let first = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let second = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        assert_ne!(first.socket_host_path(), second.socket_host_path());
    }

    #[test]
    fn test_workspace_mount_accessor_reflects_what_was_set() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        assert!(sandbox.workspace_mount().is_none());

        let sandbox = sandbox.with_workspace_mount("/host/repo");
        assert_eq!(
            sandbox.workspace_mount(),
            Some(std::path::Path::new("/host/repo"))
        );
    }

    #[tokio::test]
    async fn test_calling_create_or_remove_before_a_container_exists_is_an_error() {
        let mut sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        assert!(sandbox.start().await.is_err());
        assert!(sandbox.stop().await.is_err());
        assert!(sandbox.remove().await.is_err());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_full_lifecycle_create_start_stop_remove() {
        let mut sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));

        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");
        assert!(sandbox.container_id().is_some());

        sandbox.start().await.expect("should start");
        sandbox.stop().await.expect("should stop");
        sandbox.remove().await.expect("should remove");
        assert!(sandbox.container_id().is_none());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_create_fails_when_a_container_already_exists_on_this_sandbox() {
        let mut sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");

        let error = sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect_err("should refuse a second container on the same sandbox");
        assert!(error.to_string().contains("already has a container"));

        sandbox.remove().await.expect("cleanup should succeed");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_dropping_a_sandbox_removes_its_container() {
        let docker_handle = docker();
        let mut sandbox = Sandbox::with_docker(docker_handle.clone(), test_config("alpine:latest"));
        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");
        let id = sandbox.container_id().unwrap().to_string();

        drop(sandbox);
        // Drop only spawns the cleanup task; give it a moment to actually run
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let inspected = docker_handle
            .inspect_container(
                &id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await;
        assert!(
            inspected.is_err(),
            "the container should no longer exist after being dropped"
        );
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_the_workspace_mount_is_visible_inside_the_running_container() {
        let host_dir = std::env::temp_dir().join(format!(
            "munibot_ai_sandbox_workspace_mount_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&host_dir).unwrap();
        std::fs::write(host_dir.join("marker.txt"), "hello from the host").unwrap();

        let mut sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"))
            .with_workspace_mount(&host_dir);
        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");

        sandbox
            .exec(vec!["cat".to_string(), "marker.txt".to_string()])
            .await
            .expect("cat should succeed against a file the host already wrote");

        let error = sandbox
            .exec(vec!["cat".to_string(), "does_not_exist.txt".to_string()])
            .await
            .expect_err("a nonzero exit should be an error");
        assert!(error.to_string().contains("exited with status"));

        sandbox.remove().await.expect("cleanup should succeed");
        std::fs::remove_dir_all(&host_dir).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_tool_agent_path_overrides_the_image_entrypoint_and_runs_the_mounted_binary() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let host_workspace =
            std::env::temp_dir().join(format!("munibot_ai_tool_agent_mount_workspace_{nanos}"));
        std::fs::create_dir_all(&host_workspace).unwrap();

        // stands in for a real munibot_toolagent binary: a script that
        // proves it ran (not the image's own entrypoint) by writing its
        // arguments to the workspace mount, where the host can read them
        // back after the container exits
        let host_script_dir =
            std::env::temp_dir().join(format!("munibot_ai_tool_agent_mount_script_{nanos}"));
        std::fs::create_dir_all(&host_script_dir).unwrap();
        let script_path = host_script_dir.join("fake-toolagent");
        std::fs::write(
            &script_path,
            "#!/bin/sh\necho \"ran with args: $*\" > /workspace/output.txt\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        // an image with its own ENTRYPOINT, to prove the override actually
        // beats it rather than merely never having one to beat in the
        // first place
        let mut config = test_config("alpine:latest");
        config.tool_agent_path = Some(script_path);
        let mut sandbox =
            Sandbox::with_docker(docker(), config).with_workspace_mount(&host_workspace);

        sandbox
            .create(vec!["--socket".to_string(), "sock".to_string()])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");

        // give the script a moment to run and exit on its own
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let output = std::fs::read_to_string(host_workspace.join("output.txt"))
            .expect("the mounted script should have run and written its output");
        assert_eq!(output.trim(), "ran with args: --socket sock");

        sandbox.remove().await.expect("cleanup should succeed");
        std::fs::remove_dir_all(&host_workspace).ok();
        std::fs::remove_dir_all(&host_script_dir).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_container_exceeding_its_wall_clock_limit_is_stopped() {
        let mut config = test_config("alpine:latest");
        config.wall_clock_limit = std::time::Duration::from_millis(300);
        let mut sandbox = Sandbox::with_docker(docker(), config);

        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");

        // give the watchdog time to fire well past its 300ms deadline, but
        // well short of the container's own 30s sleep finishing on its own
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let inspected = sandbox
            .docker
            .inspect_container(
                sandbox.container_id().unwrap(),
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .expect("should inspect");
        let running = inspected
            .state
            .as_ref()
            .and_then(|state| state.running)
            .unwrap_or(false);
        assert!(
            !running,
            "a container that overran its wall-clock limit should have been stopped"
        );

        sandbox.remove().await.expect("cleanup should succeed");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_normal_stop_cancels_the_watchdog_instead_of_racing_it() {
        let mut config = test_config("alpine:latest");
        config.wall_clock_limit = std::time::Duration::from_secs(30 * 60);
        let mut sandbox = Sandbox::with_docker(docker(), config);

        sandbox
            .create(vec!["sleep".to_string(), "5".to_string()])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");
        sandbox.stop().await.expect("a normal stop should succeed");
        assert!(
            sandbox.wall_clock_watchdog.is_cancelled(),
            "stop should cancel the watchdog rather than leave it armed"
        );

        sandbox.remove().await.expect("cleanup should succeed");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_container_exceeding_its_memory_limit_is_killed_without_affecting_the_host() {
        let mut config = test_config("alpine:latest");
        config.memory_limit_bytes = 16 * 1024 * 1024;
        let mut sandbox = Sandbox::with_docker(docker(), config);

        // dd runs as the container's own pid 1 here (no wrapping shell) so
        // that the kernel oom-killing it ends the whole container - a
        // wrapping `sh -c "dd ...; sleep 5"` would survive dd's own death
        // and keep the container running for the rest of the script,
        // proving nothing about the memory limit actually being enforced
        sandbox
            .create(vec![
                "dd".to_string(),
                "if=/dev/zero".to_string(),
                "of=/tmp/x".to_string(),
                "bs=1M".to_string(),
                "count=200".to_string(),
            ])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let inspected = sandbox
            .docker
            .inspect_container(
                sandbox.container_id().unwrap(),
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .expect("should inspect");
        let running = inspected
            .state
            .as_ref()
            .and_then(|state| state.running)
            .unwrap_or(false);
        assert!(
            !running,
            "a container that overran its memory limit should have been killed"
        );

        sandbox.remove().await.expect("cleanup should succeed");
    }
}
