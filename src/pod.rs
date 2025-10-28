use anyhow::{Context, Result};
use chrono::Utc;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::collections::HashMap;
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
            ip.clone()
        } else {
            let mut allocator = net::IpAllocator::new();
            allocator.allocate()?
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

        // TODO: Fork and exec container
        // For now, just mark as running (stub)
        container.state = ContainerState::Running;
        container.pid = Some(12345); // Stub PID

        // Emit event
        let evt = events.container_start(&container.name, container.pid.unwrap());
        self.store.append_event(&self.run_id, &evt.to_json()?)?;

        tracing::info!("Container {} started with PID {}", container.name, container.pid.unwrap());

        Ok(())
    }

    /// Tick - process container states and restarts
    pub fn tick(&mut self) -> Result<()> {
        let events = GardenEventBuilder::new(self.run_id.clone(), self.garden.meta.id.clone());

        // Check for exited containers
        for i in 0..self.containers.len() {
            let container = &self.containers[i];

            // Check if container should be restarted
            if container.should_restart(self.restart_policy) {
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

    /// Stop all containers gracefully
    pub fn stop_graceful(&mut self, timeout: Duration) -> Result<()> {
        let events = GardenEventBuilder::new(self.run_id.clone(), self.garden.meta.id.clone());

        tracing::info!("Stopping pod gracefully (timeout: {:?})", timeout);

        self.stop_all(&events)?;

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
