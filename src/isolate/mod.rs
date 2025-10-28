pub mod ns;
pub mod mount;
pub mod idmap;
pub mod cgroups;
pub mod caps;
pub mod seccomp;

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

    /// Apply all isolation settings (parent process side)
    pub fn apply_parent(&self) -> Result<()> {
        // Create cgroups and add process to them
        if self.should_apply_cgroups() {
            cgroups::setup_cgroups(self)?;
        }

        Ok(())
    }

    /// Apply all isolation settings (child process side)
    pub fn apply_child(&self) -> Result<()> {
        // Set hostname
        if let Some(ref hostname) = self.seed.security.hostname {
            ns::set_hostname(hostname)?;
        }

        // Setup mounts
        mount::setup_mounts(self)?;

        // Setup UID/GID mapping if rootless
        if self.seed.user.map_rootless {
            idmap::apply_uid_gid_mapping(&self.seed.user)?;
        }

        // Drop capabilities
        caps::drop_capabilities(&self.seed.security.drop_caps)?;

        // Apply seccomp
        if let Some(ref profile) = self.seed.security.seccomp_profile {
            seccomp::apply_seccomp(profile)?;
        }

        // Set no_new_privs
        ns::set_no_new_privs()?;

        Ok(())
    }

    fn should_apply_cgroups(&self) -> bool {
        self.seed.limits.cpu.shares.is_some()
            || self.seed.limits.memory.max.is_some()
            || self.seed.limits.pids.max.is_some()
    }
}
