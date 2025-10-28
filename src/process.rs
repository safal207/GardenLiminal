use anyhow::{Context, Result};
use chrono::Utc;
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{self, ForkResult, Pid};
use std::ffi::CString;
use std::sync::Arc;
use uuid::Uuid;

use crate::events::{Event, EventBuilder, EventType};
use crate::isolate::{ns, IsolationConfig};
use crate::seed::Seed;
use crate::store::{RunStatus, SeedRecord, Store};

/// Process runner that orchestrates execution
pub struct ProcessRunner {
    seed: Seed,
    store: Arc<dyn Store>,
    run_id: String,
}

impl ProcessRunner {
    pub fn new(seed: Seed, store: Arc<dyn Store>) -> Self {
        let run_id = Uuid::new_v4().to_string();

        Self {
            seed,
            store,
            run_id,
        }
    }

    /// Run the process with full isolation
    pub fn run(self) -> Result<i32> {
        let events = EventBuilder::new(self.run_id.clone(), self.seed.meta.id.clone());

        // Store seed manifest
        self.store_seed_manifest()?;

        // Create run record
        let start_ts = Utc::now().to_rfc3339();
        self.store
            .create_run(&self.run_id, &self.seed.meta.id, &start_ts)?;

        // Emit run created event
        let evt = events.run_created();
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        // Emit seed loaded event
        let evt = events.seed_loaded();
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        // Create isolation config
        let iso_config = IsolationConfig::new(&self.seed, self.run_id.clone());

        // Setup cgroups (parent side)
        iso_config.apply_parent().context("Failed to apply parent isolation")?;

        let evt = events.cgroup_applied();
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        // Create namespaces
        ns::create_namespaces(self.seed.net.enable)
            .context("Failed to create namespaces")?;

        let ns_msg = format!(
            "user, pid, uts, ipc, mnt{}",
            if self.seed.net.enable { ", net" } else { "" }
        );
        let evt = events.ns_created(&ns_msg);
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        // Clone data needed for child process before fork
        let seed_clone = self.seed.clone();
        let run_id_clone = self.run_id.clone();
        let store_clone = Arc::clone(&self.store);

        // Fork: parent will wait, child will exec
        match unsafe { unistd::fork() }.context("Failed to fork")? {
            ForkResult::Parent { child } => {
                // Parent process: wait for child
                self.wait_for_child(child, events)
            }
            ForkResult::Child => {
                // Child process: setup environment and exec
                // If this returns, it's an error
                if let Err(e) = Self::child_exec_static(
                    seed_clone,
                    run_id_clone,
                    store_clone,
                    iso_config,
                    events,
                ) {
                    eprintln!("Child exec failed: {:?}", e);
                    std::process::exit(1);
                }
                unreachable!("exec should not return");
            }
        }
    }

    /// Child process: apply isolation and exec
    fn child_exec_static(
        seed: Seed,
        run_id: String,
        store: Arc<dyn Store>,
        iso_config: IsolationConfig,
        events: EventBuilder,
    ) -> Result<()> {
        // Apply child-side isolation (mounts, uid/gid, caps, seccomp, etc.)
        iso_config
            .apply_child()
            .context("Failed to apply child isolation")?;

        // Emit events for each isolation step
        let evt = events.mount_done("Mounts configured");
        store.append_event(&run_id, &evt.to_json()?)?;

        if seed.user.map_rootless {
            let evt = events.idmap_applied();
            store.append_event(&run_id, &evt.to_json()?)?;
        }

        let evt = events.caps_dropped();
        store.append_event(&run_id, &evt.to_json()?)?;

        if seed.security.seccomp_profile.is_some() {
            let evt = events.seccomp_enabled();
            store.append_event(&run_id, &evt.to_json()?)?;
        }

        // Change to working directory
        std::env::set_current_dir(&seed.entrypoint.cwd)
            .with_context(|| format!("Failed to chdir to {}", seed.entrypoint.cwd))?;

        // Setup environment
        for env_var in &seed.entrypoint.env {
            if let Some(eq_pos) = env_var.find('=') {
                let key = &env_var[..eq_pos];
                let value = &env_var[eq_pos + 1..];
                std::env::set_var(key, value);
            }
        }

        // Emit process start event
        let pid = unistd::getpid().as_raw();
        let evt = events.process_start(pid);
        store.append_event(&run_id, &evt.to_json()?)?;

        // Prepare command
        let program = CString::new(seed.entrypoint.cmd[0].as_str())
            .context("Invalid program path")?;

        let args: Result<Vec<CString>> = seed
            .entrypoint
            .cmd
            .iter()
            .map(|s| CString::new(s.as_str()).context("Invalid argument"))
            .collect();
        let args = args?;

        // Exec (this replaces the current process)
        unistd::execv(&program, &args).context("Failed to exec")?;

        unreachable!("exec should not return");
    }

    /// Parent process: wait for child to exit
    fn wait_for_child(self, child_pid: Pid, events: EventBuilder) -> Result<i32> {
        tracing::info!("Waiting for child process: {}", child_pid);

        // Update run status to Running
        self.store
            .update_run_status(&self.run_id, RunStatus::Running, None)?;

        loop {
            match waitpid(child_pid, None) {
                Ok(WaitStatus::Exited(_, exit_code)) => {
                    tracing::info!("Child exited with code: {}", exit_code);

                    // Emit exit event
                    let evt = events.process_exit(exit_code);
                    self.store.append_event(&self.run_id, &evt.to_json()?)?;

                    // Update run status
                    let end_ts = Utc::now().to_rfc3339();
                    self.store.update_run_status(
                        &self.run_id,
                        RunStatus::Exited(exit_code),
                        Some(&end_ts),
                    )?;

                    // Cleanup
                    self.cleanup()?;

                    return Ok(exit_code);
                }
                Ok(WaitStatus::Signaled(_, signal, _)) => {
                    tracing::warn!("Child killed by signal: {:?}", signal);

                    let evt = events
                        .process_failed(&format!("Killed by signal: {:?}", signal));
                    self.store.append_event(&self.run_id, &evt.to_json()?)?;

                    let end_ts = Utc::now().to_rfc3339();
                    self.store.update_run_status(
                        &self.run_id,
                        RunStatus::Failed(format!("Killed by signal: {:?}", signal)),
                        Some(&end_ts),
                    )?;

                    self.cleanup()?;

                    return Ok(128 + signal as i32);
                }
                Ok(WaitStatus::Stopped(_, _)) => {
                    // Child stopped, continue waiting
                    continue;
                }
                Ok(_) => {
                    // Other status, continue waiting
                    continue;
                }
                Err(nix::errno::Errno::EINTR) => {
                    // Interrupted by signal, retry
                    continue;
                }
                Err(e) => {
                    anyhow::bail!("waitpid failed: {}", e);
                }
            }
        }
    }

    /// Store seed manifest
    fn store_seed_manifest(&self) -> Result<()> {
        let yaml = serde_yaml::to_string(&self.seed)
            .context("Failed to serialize seed to YAML")?;

        let record = SeedRecord {
            id: self.seed.meta.id.clone(),
            name: self.seed.meta.name.clone(),
            manifest_yaml: yaml,
            created_at: Utc::now().to_rfc3339(),
        };

        self.store.upsert_seed(record)?;

        Ok(())
    }

    /// Cleanup resources
    fn cleanup(&self) -> Result<()> {
        // Cleanup cgroups
        if let Err(e) = crate::isolate::cgroups::cleanup_cgroup(&self.seed.meta.id) {
            tracing::warn!("Failed to cleanup cgroups: {}", e);
        }

        Ok(())
    }
}
