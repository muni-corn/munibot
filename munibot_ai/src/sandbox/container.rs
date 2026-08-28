//! Container lifecycle: create, start, stop, and remove one sandbox
//! container through bollard, applying every resource and security limit
//! from [`SandboxConfig`].

use std::collections::HashMap;

use bollard::{
    Docker,
    models::{ContainerCreateBody, HostConfig},
    query_parameters::{RemoveContainerOptions, StopContainerOptions},
};

use crate::{
    sandbox::config::{NetworkPolicy, SandboxConfig},
    types::AiError,
};

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
        }
    }

    /// Creates a container from `config.image`, applying every resource
    /// and security limit from [`SandboxConfig`], with `cmd` as the
    /// entrypoint's arguments (empty uses the image's own default).
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

        let body = ContainerCreateBody {
            image: Some(self.config.image.clone()),
            cmd: (!cmd.is_empty()).then_some(cmd),
            host_config: Some(self.build_host_config()),
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
    /// writable `/tmp`, and no network unless the policy says otherwise.
    fn build_host_config(&self) -> HostConfig {
        let nano_cpus = (self.config.cpu_quota * 1_000_000_000.0).round() as i64;

        // proxy-allowlisted network access is a real feature this commit
        // does not build: for now, anything other than `None` gets a real
        // network namespace, which is strictly more permissive than the
        // allowlist a persona actually asked for. Tracked as a gap to close
        // before any persona's policy allows network, rather than silently
        // pretending an allowlist filters anything yet.
        let network_mode = match &self.config.network {
            NetworkPolicy::None => "none",
            NetworkPolicy::Allowlist(_) => "bridge",
        };

        HostConfig {
            nano_cpus: Some(nano_cpus),
            memory: Some(self.config.memory_limit_bytes as i64),
            pids_limit: Some(self.config.pids_limit),
            network_mode: Some(network_mode.to_string()),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges".to_string()]),
            readonly_rootfs: Some(true),
            tmpfs: Some(HashMap::from([("/tmp".to_string(), String::new())])),
            ..Default::default()
        }
    }

    /// Starts the created container.
    pub async fn start(&self) -> Result<(), AiError> {
        let id = self.require_id()?;
        self.docker
            .start_container(id, None)
            .await
            .map_err(|error| AiError::Other(format!("couldn't start the sandbox :< {error}")))
    }

    /// Stops the container, giving it [`STOP_TIMEOUT_SECS`] to exit on its
    /// own before podman escalates to `SIGKILL`.
    pub async fn stop(&self) -> Result<(), AiError> {
        let id = self.require_id()?;
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
        }
    }

    fn docker() -> Docker {
        Docker::connect_with_podman_defaults().expect("podman should be reachable for this test")
    }

    #[test]
    fn test_build_host_config_applies_the_security_posture() {
        let sandbox = Sandbox::with_docker(docker(), test_config("alpine:latest"));
        let host_config = sandbox.build_host_config();

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
        let host_config = sandbox.build_host_config();
        assert_eq!(host_config.nano_cpus, Some(500_000_000));
    }

    #[test]
    fn test_build_host_config_carries_the_memory_and_pids_limits() {
        let config = test_config("alpine:latest");
        let sandbox = Sandbox::with_docker(docker(), config.clone());
        let host_config = sandbox.build_host_config();
        assert_eq!(host_config.memory, Some(config.memory_limit_bytes as i64));
        assert_eq!(host_config.pids_limit, Some(config.pids_limit));
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
