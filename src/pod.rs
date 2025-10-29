use anyhow::{Context, Result};
use chrono::Utc;
use nix::sys::prctl;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{self, ForkResult, Pid};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::process::CommandExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::events::{EventType, GardenEventBuilder};
use crate::isolate::{cgroups, net, overlay};
use crate::metrics::{MetricsCollectorThread, ContainerMetrics};
use crate::seed::{Container, Garden, RestartPolicy};
use crate::store::{RunStatus, SeedRecord, Store};

/// Container state machine
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    Init,
    Starting,
    Running,
    Exited(i32),
    Failed(String),
    Backoff(Duration),
}

/// Container handle with runtime state
#[derive(Debug)]
pub struct ContainerHandle {
    pub name: String,
    pub spec: Container,
    pub state: ContainerState,
    pub pid: Option<i32>,
    pub cgroup_path: String,
    pub restart_count: u32,
    pub last_start: Option<Instant>,
    pub backoff_duration: Duration,
}

impl ContainerHandle {
    pub fn new(name: String, spec: Container, garden_id: &str) -> Self {
        let cgroup_path = format!("/sys/fs/cgroup/garden/{}/{}", garden_id, name);

        Self {
            name,
            spec,
            state: ContainerState::Init,
            pid: None,
            cgroup_path,
            restart_count: 0,
            last_start: None,
            backoff_duration: Duration::from_secs(1), // Initial backoff
        }
    }

    /// Check if container should be restarted
    pub fn should_restart(&self, policy: RestartPolicy) -> bool {
        match (policy, &self.state) {
            (RestartPolicy::Never, _) => false,
            (RestartPolicy::OnFailure, ContainerState::Exited(0)) => false,
            (RestartPolicy::OnFailure, ContainerState::Exited(_)) => true,
            (RestartPolicy::OnFailure, ContainerState::Failed(_)) => true,
            (RestartPolicy::Always, ContainerState::Exited(_)) => true,
            (RestartPolicy::Always, ContainerState::Failed(_)) => true,
            _ => false,
        }
    }

    /// Calculate next backoff duration (exponential with cap)
    pub fn next_backoff(&mut self) -> Duration {
        const MAX_BACKOFF: Duration = Duration::from_secs(30);
        const BACKOFF_FACTOR: u32 = 2;

        let next = self.backoff_duration * BACKOFF_FACTOR;
        self.backoff_duration = next.min(MAX_BACKOFF);

        self.backoff_duration
    }

    /// Reset backoff if container ran successfully for a while
    pub fn reset_backoff_if_stable(&mut self) {
        const STABLE_DURATION: Duration = Duration::from_secs(10);

        if let Some(start) = self.last_start {
            if start.elapsed() > STABLE_DURATION {
                self.backoff_duration = Duration::from_secs(1);
                self.restart_count = 0;
                tracing::debug!("Reset backoff for container {}", self.name);
            }
        }
    }
}

/// Backoff configuration
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    pub base: Duration,
    pub factor: u32,
    pub max: Duration,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            factor: 2,
            max: Duration::from_secs(30),
        }
    }
}

/// Pod Supervisor - manages multiple containers
pub struct PodSupervisor {
    pub run_id: String,
    pub garden: Garden,
    pub store: Arc<dyn Store>,
    pub containers: Vec<ContainerHandle>,
    pub restart_policy: RestartPolicy,
    pub backoff: BackoffConfig,
    pub netns_name: String,
    pub pod_ip: Option<String>,
    pub max_restarts_per_10m: u32,
    pub restart_times: Vec<Instant>,
}

impl PodSupervisor {
    pub fn new(garden: Garden, store: Arc<dyn Store>) -> Result<Self> {
        let run_id = Uuid::new_v4().to_string();
        let restart_policy = garden.get_restart_policy()?;

        let netns_name = format!("glns-{}", &garden.meta.id);

        let containers: Vec<ContainerHandle> = garden
            .containers
            .iter()
            .map(|c| ContainerHandle::new(c.name.clone(), c.clone(), &garden.meta.id))
            .collect();

        Ok(Self {
            run_id,
            garden,
            store,
            containers,
            restart_policy,
            backoff: BackoffConfig::default(),
            netns_name,
            pod_ip: None,
            max_restarts_per_10m: 20,
            restart_times: Vec::new(),
        })
    }

    /// Start the pod and all containers
    pub fn start(&mut self) -> Result<()> {
        let events = GardenEventBuilder::new(self.run_id.clone(), self.garden.meta.id.clone());

        // Make this process a subreaper to collect zombie processes
        unsafe {
            if nix::libc::prctl(nix::libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0 {
                tracing::warn!("Failed to set child subreaper: {}", std::io::Error::last_os_error());
            } else {
                tracing::debug!("Set process as child subreaper");
            }
        }

        // Store garden manifest
        self.store_garden_manifest()?;

        // Create run record
        let start_ts = Utc::now().to_rfc3339();
        self.store
            .create_run(&self.run_id, &self.garden.meta.id, &start_ts)?;

        // Setup network for pod
        self.setup_pod_network(&events)?;

        // Start all containers
        for i in 0..self.containers.len() {
            self.start_container(i, &events)?;
        }

        Ok(())
    }

    /// Setup pod-level network
    fn setup_pod_network(&mut self, events: &GardenEventBuilder) -> Result<()> {
        // Ensure bridge exists
        net::ensure_bridge().context("Failed to ensure bridge")?;

        // Create network namespace for pod
        net::create_netns(&self.netns_name).context("Failed to create netns")?;

        // Allocate or use specified IP
        let ip = if let Some(ref ip) = self.garden.net.ip {
            // User specified IP (strip CIDR if present)
            ip.split('/').next().unwrap_or(ip).to_string()
        } else {
            // Allocate from IPAM
            let mut allocator = net::IpAllocator::new();
            allocator.allocate(&self.garden.meta.id)?
        };

        self.pod_ip = Some(ip.clone());

        // Setup veth pair
        let (veth_host, _veth_pod) = net::setup_veth_pair(&self.garden.meta.name, &self.netns_name)?;

        tracing::info!("Pod network ready: bridge=gl0, ip={}", ip);

        // Emit event
        let evt = events.pod_net_ready(net::BRIDGE_NAME, &ip);
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        Ok(())
    }

    /// Start a specific container
    fn start_container(&mut self, idx: usize, events: &GardenEventBuilder) -> Result<()> {
        let container = &mut self.containers[idx];

        tracing::info!("Starting container: {}", container.name);

        container.state = ContainerState::Starting;
        container.last_start = Some(Instant::now());

        // Prepare rootfs (OverlayFS or direct path)
        let rootfs = overlay::prepare_rootfs(&container.name, &container.spec.rootfs)?;
        tracing::debug!("Rootfs prepared at: {}", rootfs.display());

        // Setup cgroups for container
        cgroups::setup_cgroup_for_container(
            &self.garden.meta.id,
            &container.name,
            &container.spec.limits,
        )?;

        // Clone data for child process
        let container_spec = container.spec.clone();
        let container_name = container.name.clone();
        let garden_id = self.garden.meta.id.clone();
        let run_id = self.run_id.clone();
        let store = Arc::clone(&self.store);
        let events_clone = events.clone();
        let netns_name = self.netns_name.clone();

        // Fork: parent waits, child execs
        match unsafe { unistd::fork() }.context("Failed to fork container process")? {
            ForkResult::Parent { child } => {
                // Parent: store PID and emit event
                let pid = child.as_raw();
                container.pid = Some(pid);
                container.state = ContainerState::Running;

                // Emit container forked event
                let evt = events.container_forked(&container.name, pid);
                self.store.append_event(&self.run_id, &evt.to_json()?)?;

                tracing::info!("Container {} forked with PID {}", container.name, pid);

                Ok(())
            }
            ForkResult::Child => {
                // Child: setup and exec
                // Clone again for error handling
                let container_name_err = container_name.clone();
                let run_id_err = run_id.clone();
                let store_err = Arc::clone(&store);
                let events_err = events_clone.clone();

                if let Err(e) = Self::container_child_exec(
                    container_spec,
                    container_name,
                    garden_id,
                    run_id,
                    store,
                    events_clone,
                    rootfs,
                    netns_name,
                ) {
                    eprintln!("Container exec failed: {:?}", e);

                    // Try to emit exec_failed event
                    let errno = format!("{:?}", e);
                    let evt = events_err.exec_failed(&container_name_err, &errno);
                    let _ = store_err.append_event(&run_id_err, &evt.to_json().unwrap_or_default());

                    std::process::exit(127);
                }
                unreachable!("exec should not return");
            }
        }
    }

    /// Child process: apply isolation and exec container
    fn container_child_exec(
        spec: Container,
        container_name: String,
        garden_id: String,
        run_id: String,
        store: Arc<dyn Store>,
        events: GardenEventBuilder,
        rootfs: std::path::PathBuf,
        netns_name: String,
    ) -> Result<()> {
        use std::os::unix::fs::MetadataExt;

        // Set PR_SET_PDEATHSIG: child gets SIGKILL if parent dies
        unsafe {
            prctl::set_pdeathsig(Signal::SIGKILL)
                .context("Failed to set PR_SET_PDEATHSIG")?;
        }

        // Create new session
        unistd::setsid().context("Failed to setsid")?;

        // Enter network namespace if pod has one
        let netns_path = format!("/var/run/netns/{}", netns_name);
        if std::path::Path::new(&netns_path).exists() {
            crate::isolate::ns::setns_net(&netns_path)
                .context("Failed to enter pod network namespace")?;
        }

        // Mount rootfs (chroot for MVP, pivot_root later)
        crate::isolate::mount::do_chroot(&rootfs)
            .context("Failed to chroot to rootfs")?;

        // Mount proc if specified
        for mount in &spec.mounts {
            if mount.mount_type == "proc" {
                crate::isolate::mount::mount_proc(&mount.target)
                    .context("Failed to mount proc")?;
            }
        }

        // Move process into container cgroup
        let cgroup_path = format!("/sys/fs/cgroup/garden/{}/{}", garden_id, container_name);
        cgroups::move_pid_to_cgroup(&cgroup_path, unistd::getpid().as_raw())?;

        // Change to working directory
        let cwd = &spec.entrypoint.cwd;
        std::env::set_current_dir(cwd)
            .with_context(|| format!("Failed to chdir to {}", cwd))?;

        // Setup environment variables
        for env_var in &spec.entrypoint.env {
            if let Some(eq_pos) = env_var.find('=') {
                let key = &env_var[..eq_pos];
                let value = &env_var[eq_pos + 1..];
                std::env::set_var(key, value);
            }
        }

        // Apply no_new_privs
        unsafe {
            if nix::libc::prctl(nix::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                anyhow::bail!("Failed to set no_new_privs");
            }
        }

        // Apply seccomp (stub for now)
        // TODO: Apply actual seccomp profile from garden.security

        // Prepare execve arguments
        let program = CString::new(spec.entrypoint.cmd[0].as_str())
            .context("Invalid program path")?;

        let args: Result<Vec<CString>> = spec
            .entrypoint
            .cmd
            .iter()
            .map(|s| CString::new(s.as_str()).context("Invalid argument"))
            .collect();
        let args = args?;

        // Emit container start event
        let pid = unistd::getpid().as_raw();
        let evt = events.container_start(&container_name, pid);
        store.append_event(&run_id, &evt.to_json()?)?;

        // Exec (this replaces the current process)
        unistd::execv(&program, &args).context("Failed to execv")?;

        unreachable!("exec should not return");
    }

    /// Tick - process container states and restarts
    pub fn tick(&mut self) -> Result<()> {
        let events = GardenEventBuilder::new(self.run_id.clone(), self.garden.meta.id.clone());

        // Check for exited processes using non-blocking waitpid
        self.reap_exited_processes(&events)?;

        // Check for containers that need restart
        for i in 0..self.containers.len() {
            // Reset backoff if container has been stable
            if self.containers[i].state == ContainerState::Running {
                self.containers[i].reset_backoff_if_stable();
            }

            // Check if container should be restarted
            let should_restart = self.containers[i].should_restart(self.restart_policy);

            if should_restart {
                // Check crash loop protection
                if self.is_crash_looping()? {
                    tracing::error!("Crash loop detected for pod {}", self.garden.meta.id);
                    self.stop_all(&events)?;
                    return Ok(());
                }

                // Schedule restart with backoff
                self.schedule_restart(i, &events)?;
            }
        }

        Ok(())
    }

    /// Reap exited processes and update container states
    fn reap_exited_processes(&mut self, events: &GardenEventBuilder) -> Result<()> {
        use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
        use nix::unistd::Pid;

        loop {
            // Non-blocking wait for any child process
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, exit_code)) => {
                    tracing::info!("Process {} exited with code {}", pid, exit_code);

                    // Find container with this PID and update its state
                    if let Some(container) = self.containers.iter_mut().find(|c| c.pid == Some(pid.as_raw())) {
                        container.state = ContainerState::Exited(exit_code);
                        container.pid = None;

                        // Emit container exit event
                        let evt = events.container_exit(&container.name, exit_code);
                        self.store.append_event(&self.run_id, &evt.to_json()?)?;

                        tracing::info!("Container {} exited with code {}", container.name, exit_code);
                    }
                }
                Ok(WaitStatus::Signaled(pid, signal, _)) => {
                    tracing::warn!("Process {} killed by signal {:?}", pid, signal);

                    // Find container with this PID and update its state
                    if let Some(container) = self.containers.iter_mut().find(|c| c.pid == Some(pid.as_raw())) {
                        let exit_code = 128 + signal as i32;
                        container.state = ContainerState::Exited(exit_code);
                        container.pid = None;

                        // Emit container exit event
                        let evt = events.container_exit(&container.name, exit_code);
                        self.store.append_event(&self.run_id, &evt.to_json()?)?;

                        tracing::warn!("Container {} killed by signal {:?}", container.name, signal);
                    }
                }
                Ok(WaitStatus::StillAlive) => {
                    // No more children to reap
                    break;
                }
                Ok(_) => {
                    // Other status (stopped, continued, etc.) - ignore for now
                    continue;
                }
                Err(nix::errno::Errno::ECHILD) => {
                    // No children exist
                    break;
                }
                Err(nix::errno::Errno::EINTR) => {
                    // Interrupted by signal, retry
                    continue;
                }
                Err(e) => {
                    tracing::warn!("waitpid error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Check if pod is crash looping
    fn is_crash_looping(&mut self) -> Result<bool> {
        let now = Instant::now();
        let ten_min_ago = now - Duration::from_secs(600);

        // Remove old restart times
        self.restart_times.retain(|&t| t > ten_min_ago);

        Ok(self.restart_times.len() >= self.max_restarts_per_10m as usize)
    }

    /// Schedule container restart
    fn schedule_restart(&mut self, idx: usize, events: &GardenEventBuilder) -> Result<()> {
        let container = &mut self.containers[idx];

        let backoff = container.next_backoff();

        tracing::info!(
            "Scheduling restart for container {} in {:?}",
            container.name,
            backoff
        );

        container.state = ContainerState::Backoff(backoff);
        container.restart_count += 1;
        self.restart_times.push(Instant::now());

        // TODO: Actually schedule restart (sleep + start_container)

        Ok(())
    }

    /// Stop all containers gracefully with timeout
    pub fn stop_graceful(&mut self, timeout: Duration) -> Result<()> {
        let events = GardenEventBuilder::new(self.run_id.clone(), self.garden.meta.id.clone());

        tracing::info!("Stopping pod gracefully (timeout: {:?})", timeout);

        // Emit pod stop requested event
        let evt = events.pod_stop_requested();
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        // Send SIGTERM to all running containers
        for container in &self.containers {
            if let Some(pid) = container.pid {
                tracing::info!("Sending SIGTERM to container {} (PID {})", container.name, pid);

                // Emit signal forward event
                let evt = events.signal_forward("SIGTERM", &container.name);
                self.store.append_event(&self.run_id, &evt.to_json()?)?;

                // Send SIGTERM
                if let Err(e) = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM,
                ) {
                    tracing::warn!("Failed to send SIGTERM to PID {}: {}", pid, e);
                }
            }
        }

        // Wait for containers to exit gracefully
        let start = Instant::now();
        while start.elapsed() < timeout {
            // Reap any exited processes
            self.reap_exited_processes(&events)?;

            // Check if all containers have exited
            let all_exited = self.containers.iter().all(|c| c.pid.is_none());
            if all_exited {
                tracing::info!("All containers exited gracefully");
                self.cleanup_after_stop(&events)?;
                return Ok(());
            }

            // Sleep briefly before checking again
            std::thread::sleep(Duration::from_millis(100));
        }

        // Timeout expired, send SIGKILL to remaining containers
        tracing::warn!("Graceful timeout expired, sending SIGKILL to remaining containers");

        // Emit timeout event
        let timeout_ms = timeout.as_millis() as u64;
        let evt = events.pod_timeout(timeout_ms);
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        for container in &self.containers {
            if let Some(pid) = container.pid {
                tracing::warn!("Sending SIGKILL to container {} (PID {})", container.name, pid);

                // Emit signal forward event
                let evt = events.signal_forward("SIGKILL", &container.name);
                self.store.append_event(&self.run_id, &evt.to_json()?)?;

                // Send SIGKILL
                if let Err(e) = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                ) {
                    tracing::warn!("Failed to send SIGKILL to PID {}: {}", pid, e);
                }
            }
        }

        // Wait for SIGKILL to take effect
        for _ in 0..10 {
            self.reap_exited_processes(&events)?;
            let all_exited = self.containers.iter().all(|c| c.pid.is_none());
            if all_exited {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        self.cleanup_after_stop(&events)?;

        Ok(())
    }

    /// Cleanup after all containers have stopped
    fn cleanup_after_stop(&mut self, events: &GardenEventBuilder) -> Result<()> {
        // Cleanup network
        self.cleanup_network()?;

        // Emit pod exit
        let evt = events.pod_exit("Stopped");
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        let end_ts = Utc::now().to_rfc3339();
        self.store
            .update_run_status(&self.run_id, RunStatus::Exited(0), Some(&end_ts))?;

        Ok(())
    }

    /// Stop all containers
    fn stop_all(&mut self, events: &GardenEventBuilder) -> Result<()> {
        for container in &mut self.containers {
            if let Some(pid) = container.pid {
                tracing::info!("Stopping container {}", container.name);

                // TODO: Send SIGTERM, wait, then SIGKILL

                container.state = ContainerState::Exited(0);
                container.pid = None;

                // Emit exit event
                let evt = events.container_exit(&container.name, 0);
                self.store.append_event(&self.run_id, &evt.to_json()?)?;
            }
        }

        // Cleanup network
        self.cleanup_network()?;

        // Emit pod exit
        let evt = events.pod_exit("Stopped");
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        let end_ts = Utc::now().to_rfc3339();
        self.store
            .update_run_status(&self.run_id, RunStatus::Exited(0), Some(&end_ts))?;

        Ok(())
    }

    /// Cleanup pod network
    fn cleanup_network(&self) -> Result<()> {
        // Delete netns
        if let Err(e) = net::delete_netns(&self.netns_name) {
            tracing::warn!("Failed to delete netns {}: {}", self.netns_name, e);
        }

        Ok(())
    }

    /// Store garden manifest
    fn store_garden_manifest(&self) -> Result<()> {
        let yaml = serde_yaml::to_string(&self.garden)
            .context("Failed to serialize garden to YAML")?;

        let record = SeedRecord {
            id: self.garden.meta.id.clone(),
            name: self.garden.meta.name.clone(),
            manifest_yaml: yaml,
            created_at: Utc::now().to_rfc3339(),
        };

        self.store.upsert_seed(record)?;

        Ok(())
    }

    /// Get primary container (first in list)
    pub fn primary_container(&self) -> Option<&ContainerHandle> {
        self.containers.first()
    }

    /// Get pod exit code (from primary container)
    pub fn get_exit_code(&self) -> i32 {
        if let Some(primary) = self.primary_container() {
            match &primary.state {
                ContainerState::Exited(code) => *code,
                ContainerState::Failed(_) => 1,
                _ => 0,
            }
        } else {
            0
        }
    }
}
