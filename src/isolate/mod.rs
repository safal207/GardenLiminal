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
use std::path::PathBuf;

/// Isolation configuration aggregator.
pub struct IsolationConfig<'a> {
    pub seed: &'a Seed,
    pub run_id: String,
}

impl<'a> IsolationConfig<'a> {
    pub fn new(seed: &'a Seed, run_id: String) -> Self {
        Self { seed, run_id }
    }

    /// Prepare host-side resources without moving the supervisor into the
    /// workload's namespaces or cgroup.
    ///
    /// Returns the prepared cgroup path only when the Seed requested native
    /// CPU/memory/PID limits. The caller moves the isolated workload PID into
    /// that cgroup before releasing it.
    pub fn apply_parent(&self) -> Result<Option<PathBuf>> {
        if self.cgroups_requested() {
            Ok(Some(cgroups::setup_cgroups(self)?))
        } else {
            Ok(None)
        }
    }

    /// Apply isolation that must happen inside the already-created workload
    /// task. User-namespace UID/GID maps are installed by the host supervisor
    /// before this task is released.
    pub fn apply_child(&self) -> Result<()> {
        if self.seed.user.map_rootless {
            idmap::verify_current_identity(&self.seed.user)?;
        }

        if let Some(ref hostname) = self.seed.security.hostname {
            ns::set_hostname(hostname)?;
        }

        mount::setup_mounts(self)?;

        // no_new_privs must precede capability enforcement and seccomp.
        ns::set_no_new_privs()?;

        caps::drop_capabilities(&self.seed.security.drop_caps)?;

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
