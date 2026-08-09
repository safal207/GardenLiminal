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

    /// Apply all isolation settings (host supervisor side).
    pub fn apply_parent(&self) -> Result<()> {
        if self.should_apply_cgroups() {
            cgroups::setup_cgroups(self)?;
        }

        Ok(())
    }

    /// Apply isolation inside the PID-1 workload child.
    ///
    /// Rootless UID/GID map files are configured by the namespace bootstrap
    /// immediately after user-namespace creation. This function only enters
    /// that already-pinned mapped identity after mount setup is complete.
    pub fn apply_child(&self) -> Result<()> {
        if let Some(ref hostname) = self.seed.security.hostname {
            ns::set_hostname(hostname)?;
        }

        mount::setup_mounts(self)?;

        if self.seed.user.map_rootless {
            idmap::enter_mapped_identity(&self.seed.user)?;
        }

        // Required before unprivileged seccomp installation and blocks later
        // privilege gain through exec transitions.
        ns::set_no_new_privs()?;

        caps::drop_capabilities(&self.seed.security.drop_caps)?;

        if let Some(ref profile) = self.seed.security.seccomp_profile {
            seccomp::apply_seccomp(profile)?;
        }

        Ok(())
    }

    fn should_apply_cgroups(&self) -> bool {
        self.seed.limits.cpu.shares.is_some()
            || self.seed.limits.memory.max.is_some()
            || self.seed.limits.pids.max.is_some()
    }
}
