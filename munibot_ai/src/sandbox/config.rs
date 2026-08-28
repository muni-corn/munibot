//! Container configuration: resource limits, security posture, and network
//! policy for one sandbox.

use std::time::Duration;

/// Whether a sandbox's container may reach the network at all.
///
/// Defaults to [`NetworkPolicy::None`] - a persona must explicitly request
/// network access, and even then only through an allowlist, never open
/// access. See `docs/plans/ai/milestone-4-sandbox.md`'s security posture
/// section.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network access at all - the container has no network namespace
    /// reachable beyond loopback.
    #[default]
    None,
    /// Access limited to exactly these hosts, proxied rather than granted
    /// directly.
    Allowlist(Vec<String>),
}

/// Everything a [`crate::sandbox::Sandbox`] needs to know to create and
/// bound one container.
///
/// Every field has a conservative default (see [`Default`] below) - a
/// caller opts into more resources or network access explicitly, rather
/// than a sandbox ever being more permissive than it has to be by omission.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxConfig {
    /// The container image to run - see the repository's own
    /// `Containerfile`.
    pub image: String,
    /// Fractional CPUs the container may use, e.g. `2.0` for two full
    /// cores.
    pub cpu_quota: f64,
    /// Maximum resident memory, in bytes, before the container is killed
    /// by the kernel's own OOM handling.
    pub memory_limit_bytes: u64,
    /// Maximum number of processes/threads the container's cgroup may
    /// create - bounds a fork bomb from a runaway build script.
    pub pids_limit: i64,
    /// Maximum writable disk usage, in bytes, at the repository mount.
    /// `None` leaves it unbounded, since not every storage driver podman
    /// can run with actually enforces a disk quota.
    pub disk_limit_bytes: Option<u64>,
    pub network: NetworkPolicy,
    /// A hard ceiling on the container's own lifetime, independent of
    /// whatever budget the harness's own turn is running under - so a
    /// wedged container can never live forever even if the turn that
    /// started it never comes back to tear it down.
    pub wall_clock_limit: Duration,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: "munibot-sandbox:latest".to_string(),
            cpu_quota: 2.0,
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
            pids_limit: 256,
            disk_limit_bytes: None,
            network: NetworkPolicy::default(),
            wall_clock_limit: Duration::from_secs(30 * 60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_network_policy_is_none() {
        assert_eq!(NetworkPolicy::default(), NetworkPolicy::None);
    }

    #[test]
    fn test_default_config_has_no_network_access() {
        assert_eq!(SandboxConfig::default().network, NetworkPolicy::None);
    }

    #[test]
    fn test_default_config_has_a_positive_cpu_quota() {
        assert!(SandboxConfig::default().cpu_quota > 0.0);
    }

    #[test]
    fn test_default_config_has_a_positive_memory_limit() {
        assert!(SandboxConfig::default().memory_limit_bytes > 0);
    }

    #[test]
    fn test_default_config_has_a_positive_pids_limit() {
        assert!(SandboxConfig::default().pids_limit > 0);
    }

    #[test]
    fn test_default_config_has_no_disk_limit() {
        // not every storage driver podman can run with enforces one
        assert_eq!(SandboxConfig::default().disk_limit_bytes, None);
    }

    #[test]
    fn test_default_config_has_a_bounded_wall_clock_limit() {
        let limit = SandboxConfig::default().wall_clock_limit;
        assert!(limit > Duration::ZERO);
        assert!(
            limit < Duration::from_secs(24 * 60 * 60),
            "a default sandbox should never be allowed to live for a full day"
        );
    }

    #[test]
    fn test_allowlist_network_policy_carries_its_hosts() {
        let policy = NetworkPolicy::Allowlist(vec!["api.example.com".to_string()]);
        match policy {
            NetworkPolicy::Allowlist(hosts) => {
                assert_eq!(hosts, vec!["api.example.com".to_string()]);
            }
            NetworkPolicy::None => panic!("expected an allowlist"),
        }
    }
}
