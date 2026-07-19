pub mod ns;
pub mod mount;
pub mod idmap;
pub mod cgroups;
pub mod caps;
pub mod seccomp;
pub mod overlay;
pub mod net;
pub mod dns;

use anyhow::Result;
use crate::seed::Seed;

/// Isolation configuration aggregator
pub struct IsolationConfig<'a> {
    pub seed: &'a Seed,
    pub run_id: String,
}

impl<'a> IsolationConfig<'a> {
    pub fn new(seed: &'a Seed, run_id: String) -> Self {
        Self { seed, run_id }
    }

    /// Apply all isolation settings that must happen in the parent process.
    ///
    /// Returns true only when a native Garden cgroup was actually requested
    /// and configured. Callers use this to avoid emitting false
    /// CGROUP_APPLIED lifecycle evidence for an empty limits block.
    pub fn apply_parent(&self) -> Result<bool> {
        let requested = self.cgroups_requested();
        if requested {
            cgroups::setup_cgroups(self)?;
        }

        Ok(requested)
    }

    /// Apply all isolation settings that happen in the child process.
    pub fn apply_child(&self) -> Result<()> {
        // A rootless user namespace starts without a usable host-filesystem
        // identity. Establish the UID/GID map before hostname or rootfs work.
        if self.seed.user.map_rootless {
            idmap::apply_uid_gid_mapping(&self.seed.user)?;
        }

        // Set hostname
        if let Some(ref hostname) = self.seed.security.hostname {
            ns::set_hostname(hostname)?;
        }

        // Setup mounts
        mount::setup_mounts(self)?;

        // Set no_new_privs first — required before applying seccomp without CAP_SYS_ADMIN
        ns::set_no_new_privs()?;

        // Drop capabilities
        caps::drop_capabilities(&self.seed.security.drop_caps)?;

        // Apply seccomp (must come after no_new_privs)
        if let Some(ref profile) = self.seed.security.seccomp_profile {
            seccomp::apply_seccomp(profile)?;
        }

        Ok(())
    }

    pub fn cgroups_requested(&self) -> bool {
        self.seed.limits.cpu.shares.is_some()
            || self.seed.limits.memory.max.is_some()
            || self.seed.limits.pids.max.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(extra: &str) -> Seed {
        serde_yaml::from_str(&format!(
            r#"
apiVersion: v0
kind: Seed
meta:
  name: unit
  id: unit-001
rootfs:
  path: /tmp
entrypoint:
  cmd: ["/bin/true"]
{extra}
"#
        ))
        .expect("seed parses")
    }

    #[test]
    fn empty_limits_do_not_request_native_cgroups() {
        let seed = seed("");
        let config = IsolationConfig::new(&seed, "run".to_string());
        assert!(!config.cgroups_requested());
    }

    #[test]
    fn each_limit_requests_native_cgroups() {
        for extra in [
            "limits:\n  cpu:\n    shares: 128",
            "limits:\n  memory:\n    max: 128Mi",
            "limits:\n  pids:\n    max: 32",
        ] {
            let seed = seed(extra);
            let config = IsolationConfig::new(&seed, "run".to_string());
            assert!(config.cgroups_requested(), "missing request for {extra}");
        }
    }
}
