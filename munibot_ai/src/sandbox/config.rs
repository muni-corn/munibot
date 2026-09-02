//! Container configuration: resource limits, security posture, and network
//! policy for one sandbox.

use std::{path::PathBuf, str::FromStr, time::Duration};

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

use crate::types::AiError;

/// Whether a sandbox's container may reach the network at all.
///
/// Defaults to [`NetworkPolicy::None`] - a persona must explicitly request
/// network access, and even then the only real option today is
/// [`NetworkPolicy::Full`], an explicit, documented weakening of isolation.
/// See `docs/plans/ai/milestone-4-sandbox.md`'s security posture section
/// and the network table in `docs/plans/ai/milestone-7-projects.md`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network access at all - the container has no network namespace
    /// reachable beyond loopback. The default, and the only sensible
    /// choice for untrusted generated code.
    #[default]
    None,
    /// Full, unfiltered network access - podman's plain `bridge` mode. An
    /// explicit per-project opt-in that weakens isolation; never the
    /// default.
    Full,
    /// Access limited to exactly these hosts, proxied rather than granted
    /// directly.
    ///
    /// **Not implemented yet.** No filtering proxy exists to point a
    /// container's only route at, so [`crate::sandbox::Sandbox::create`]
    /// fails loudly naming this variant rather than silently granting
    /// full network the way it used to. See "decisions still open" in
    /// `docs/plans/ai/milestone-7-projects.md`.
    Allowlist(Vec<String>),
}

/// The `[ai.sandbox].network` value as configurable in TOML today - just the
/// two variants an operator can actually pick between.
///
/// [`NetworkPolicy::Allowlist`] has no config surface yet (see its own docs
/// above), so it's deliberately absent here rather than reachable through a
/// string this crate can't actually honor. Serializes and deserializes as
/// its `strum` snake-case string form (`"none"`/`"full"`), matching the
/// convention `munibot_core::Permission` already established.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum SandboxNetworkConfig {
    #[default]
    None,
    Full,
}

impl SandboxNetworkConfig {
    /// Resolves to a real [`NetworkPolicy`].
    pub fn resolve(&self) -> NetworkPolicy {
        match self {
            Self::None => NetworkPolicy::None,
            Self::Full => NetworkPolicy::Full,
        }
    }
}

impl Serialize for SandboxNetworkConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SandboxNetworkConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(|_| {
            serde::de::Error::custom(format!(
                "'{raw}' isn't a known sandbox network policy :< expected \"none\" or \"full\""
            ))
        })
    }
}

/// The TOML-deserializable shape of `[ai.sandbox]`: every field optional in
/// the file, each with a default matching [`SandboxConfig::default`] exactly
/// - the same ergonomic-config-then-resolve shape every other `[ai.*]`
///   sub-section in `crate::persona::config` already uses.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SandboxSettings {
    #[serde(default = "default_sandbox_image")]
    pub image: String,
    #[serde(default = "default_sandbox_cpu_quota")]
    pub cpu_quota: f64,
    /// A byte-size string like `"2 GiB"` or a plain integer number of
    /// bytes.
    #[serde(default = "default_sandbox_memory_limit")]
    pub memory_limit: bytesize::ByteSize,
    #[serde(default = "default_sandbox_pids_limit")]
    pub pids_limit: i64,
    #[serde(default)]
    pub network: SandboxNetworkConfig,
    /// A humantime duration string like `"30m"`.
    #[serde(default = "default_sandbox_wall_clock_limit", with = "humantime_serde")]
    pub wall_clock_limit: Duration,
    /// An explicit override for the tool agent binary's path. Unset
    /// resolves through [`Self::resolve_tool_agent_path`]'s fallback chain
    /// instead: a `munibot_toolagent` sitting next to the running
    /// executable, then `$MUNIBOT_TOOLAGENT_PATH`.
    #[serde(default)]
    pub tool_agent_path: Option<PathBuf>,
}

fn default_sandbox_image() -> String {
    "munibot-sandbox:latest".to_string()
}

fn default_sandbox_cpu_quota() -> f64 {
    2.0
}

fn default_sandbox_memory_limit() -> bytesize::ByteSize {
    bytesize::ByteSize::gib(2)
}

fn default_sandbox_pids_limit() -> i64 {
    256
}

fn default_sandbox_wall_clock_limit() -> Duration {
    Duration::from_secs(30 * 60)
}

impl Default for SandboxSettings {
    /// Not `#[derive(Default)]`: none of these fields' sensible defaults
    /// are their own zero value, and a derived `Default` would silently
    /// disagree with what `#[serde(default = "...")]` fills in for a
    /// config file that mentions `[ai.sandbox]` but leaves a field unset.
    /// This mirrors every `#[serde(default = "...")]` above exactly,
    /// matching the same trap `AiConfig::default` documents for
    /// `max_delegation_depth`.
    fn default() -> Self {
        Self {
            image: default_sandbox_image(),
            cpu_quota: default_sandbox_cpu_quota(),
            memory_limit: default_sandbox_memory_limit(),
            pids_limit: default_sandbox_pids_limit(),
            network: SandboxNetworkConfig::default(),
            wall_clock_limit: default_sandbox_wall_clock_limit(),
            tool_agent_path: None,
        }
    }
}

impl SandboxSettings {
    /// Resolves to a real [`SandboxConfig`], matching
    /// [`SandboxConfig::default`] exactly when every field here is left
    /// unset.
    pub fn resolve(&self) -> SandboxConfig {
        SandboxConfig {
            image: self.image.clone(),
            cpu_quota: self.cpu_quota,
            memory_limit_bytes: self.memory_limit.as_u64(),
            pids_limit: self.pids_limit,
            disk_limit_bytes: None,
            network: self.network.resolve(),
            wall_clock_limit: self.wall_clock_limit,
        }
    }

    /// Resolves the tool agent binary's path: an explicit
    /// [`Self::tool_agent_path`] first, then a `munibot_toolagent` sitting
    /// next to the currently running executable, then
    /// `$MUNIBOT_TOOLAGENT_PATH` - failing loudly and naming all three
    /// when none of them pan out, rather than starting a sandbox that can
    /// never actually run a tool.
    pub fn resolve_tool_agent_path(&self) -> Result<PathBuf, AiError> {
        if let Some(path) = &self.tool_agent_path {
            return Ok(path.clone());
        }

        if let Ok(exe) = std::env::current_exe()
            && let Some(sibling) = exe.parent().map(|dir| dir.join("munibot_toolagent"))
            && sibling.exists()
        {
            return Ok(sibling);
        }

        if let Ok(path) = std::env::var("MUNIBOT_TOOLAGENT_PATH") {
            return Ok(PathBuf::from(path));
        }

        Err(AiError::Config(
            "couldn't find munibot_toolagent :< set [ai.sandbox].tool_agent_path, put a \
             munibot_toolagent binary next to the running executable, or set \
             $MUNIBOT_TOOLAGENT_PATH"
                .to_string(),
        ))
    }
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
            NetworkPolicy::None | NetworkPolicy::Full => panic!("expected an allowlist"),
        }
    }

    #[test]
    fn test_sandbox_network_config_round_trips_through_its_string_form() {
        assert_eq!(
            SandboxNetworkConfig::None.to_string(),
            SandboxNetworkConfig::from_str("none").unwrap().to_string()
        );
        assert_eq!(
            SandboxNetworkConfig::Full.to_string(),
            SandboxNetworkConfig::from_str("full").unwrap().to_string()
        );
    }

    #[test]
    fn test_sandbox_network_config_resolves_to_the_matching_network_policy() {
        assert_eq!(SandboxNetworkConfig::None.resolve(), NetworkPolicy::None);
        assert_eq!(SandboxNetworkConfig::Full.resolve(), NetworkPolicy::Full);
    }

    #[test]
    fn test_sandbox_network_config_serializes_as_a_snake_case_string() {
        assert_eq!(
            serde_json::to_string(&SandboxNetworkConfig::None).unwrap(),
            "\"none\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxNetworkConfig::Full).unwrap(),
            "\"full\""
        );
    }

    #[test]
    fn test_sandbox_network_config_rejects_an_unknown_string() {
        let error = toml::from_str::<SandboxNetworkConfig>("\"allowlist\"").unwrap_err();
        assert!(error.to_string().contains("allowlist"));
    }

    #[test]
    fn test_default_sandbox_settings_matches_default_sandbox_config_exactly() {
        let resolved = SandboxSettings::default().resolve();
        let expected = SandboxConfig::default();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_sandbox_settings_toml_round_trip_with_every_field_set() {
        let toml_str = r#"
            image = "my-project:abc123"
            cpu_quota = 1.5
            memory_limit = "4 GiB"
            pids_limit = 128
            network = "full"
            wall_clock_limit = "10m"
            tool_agent_path = "/opt/munibot/toolagent"
        "#;
        let settings: SandboxSettings = toml::from_str(toml_str).unwrap();

        assert_eq!(settings.image, "my-project:abc123");
        assert_eq!(settings.cpu_quota, 1.5);
        assert_eq!(settings.memory_limit, bytesize::ByteSize::gib(4));
        assert_eq!(settings.pids_limit, 128);
        assert_eq!(settings.network, SandboxNetworkConfig::Full);
        assert_eq!(settings.wall_clock_limit, Duration::from_secs(10 * 60));
        assert_eq!(
            settings.tool_agent_path,
            Some(PathBuf::from("/opt/munibot/toolagent"))
        );

        let resolved = settings.resolve();
        assert_eq!(resolved.image, "my-project:abc123");
        assert_eq!(resolved.cpu_quota, 1.5);
        assert_eq!(
            resolved.memory_limit_bytes,
            bytesize::ByteSize::gib(4).as_u64()
        );
        assert_eq!(resolved.pids_limit, 128);
        assert_eq!(resolved.network, NetworkPolicy::Full);
        assert_eq!(resolved.wall_clock_limit, Duration::from_secs(10 * 60));
    }

    #[test]
    fn test_sandbox_settings_defaults_every_field_from_an_empty_toml_table() {
        let settings: SandboxSettings = toml::from_str("").unwrap();
        assert_eq!(settings, SandboxSettings::default());
    }

    #[test]
    fn test_resolve_tool_agent_path_prefers_an_explicit_override() {
        let settings = SandboxSettings {
            tool_agent_path: Some(PathBuf::from("/explicit/path/to/munibot_toolagent")),
            ..SandboxSettings::default()
        };
        assert_eq!(
            settings.resolve_tool_agent_path().unwrap(),
            PathBuf::from("/explicit/path/to/munibot_toolagent")
        );
    }

    // both cases below read $MUNIBOT_TOOLAGENT_PATH, so they share one test
    // function - no other test in this suite touches that variable, but two
    // separate #[test] functions both mutating it would race across threads
    // the way `cargo test`'s default runner schedules them.
    #[test]
    fn test_resolve_tool_agent_path_env_var_fallback_and_failure() {
        let settings = SandboxSettings::default();

        // SAFETY: single-threaded with respect to this variable within this
        // test process; no other test reads or writes MUNIBOT_TOOLAGENT_PATH
        unsafe { std::env::remove_var("MUNIBOT_TOOLAGENT_PATH") };
        // the running test binary's own sibling directory won't contain a
        // munibot_toolagent, so this exercises the final failure branch
        let error = settings.resolve_tool_agent_path().unwrap_err();
        assert!(error.to_string().contains("tool_agent_path"));
        assert!(error.to_string().contains("MUNIBOT_TOOLAGENT_PATH"));

        // SAFETY: see above
        unsafe { std::env::set_var("MUNIBOT_TOOLAGENT_PATH", "/env/path/munibot_toolagent") };
        let resolved = settings.resolve_tool_agent_path();
        // SAFETY: see above
        unsafe { std::env::remove_var("MUNIBOT_TOOLAGENT_PATH") };
        assert_eq!(
            resolved.unwrap(),
            PathBuf::from("/env/path/munibot_toolagent")
        );
    }
}
