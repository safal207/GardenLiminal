use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Event types for process lifecycle
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    // Iteration 1 - Seed events
    RunCreated,
    SeedLoaded,
    NsCreated,
    MountDone,
    CgroupApplied,
    IdmapApplied,
    CapsDropped,
    SeccompEnabled,
    ProcessStart,
    ProcessOutput,
    ProcessExit,
    ProcessFailed,

    // Iteration 2 - Garden (Pod) events
    PodNetReady,
    ContainerStart,
    ContainerExit,
    PodHealth,
    PodExit,
    Metric,
}

/// Structured event for logging and storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Timestamp in RFC3339 format
    pub ts: DateTime<Utc>,

    /// Log level
    pub level: LogLevel,

    /// Run ID (UUID)
    pub run: String,

    /// Seed ID (for Iteration 1) or Garden ID (for Iteration 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,

    /// Garden ID (Iteration 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub garden: Option<String>,

    /// Container name (Iteration 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,

    /// Event type
    pub event: EventType,

    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,

    /// Optional exit code (for PROCESS_EXIT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,

    /// Optional error details (for PROCESS_FAILED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Optional additional data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl Event {
    /// Create a new event (Iteration 1 - Seed)
    pub fn new(run_id: String, seed_id: String, event: EventType) -> Self {
        Self {
            ts: Utc::now(),
            level: LogLevel::Info,
            run: run_id,
            seed: Some(seed_id),
            garden: None,
            container: None,
            event,
            msg: None,
            code: None,
            error: None,
            data: None,
        }
    }

    /// Create a new garden event (Iteration 2)
    pub fn new_garden(run_id: String, garden_id: String, event: EventType) -> Self {
        Self {
            ts: Utc::now(),
            level: LogLevel::Info,
            run: run_id,
            seed: None,
            garden: Some(garden_id),
            container: None,
            event,
            msg: None,
            code: None,
            error: None,
            data: None,
        }
    }

    /// Create a new container event (Iteration 2)
    pub fn new_container(run_id: String, garden_id: String, container_name: String, event: EventType) -> Self {
        Self {
            ts: Utc::now(),
            level: LogLevel::Info,
            run: run_id,
            seed: None,
            garden: Some(garden_id),
            container: Some(container_name),
            event,
            msg: None,
            code: None,
            error: None,
            data: None,
        }
    }

    /// Set log level
    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    /// Set message
    pub fn with_msg(mut self, msg: impl Into<String>) -> Self {
        self.msg = Some(msg.into());
        self
    }

    /// Set exit code
    pub fn with_code(mut self, code: i32) -> Self {
        self.code = Some(code);
        self
    }

    /// Set error
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Set additional data
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Emit event to stdout as JSON line
    pub fn emit_stdout(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            println!("{}", json);
        }
    }

    /// Convert to JSON Value
    pub fn to_json(&self) -> serde_json::Result<Value> {
        serde_json::to_value(self)
    }
}

/// Event builder for convenience
pub struct EventBuilder {
    run_id: String,
    seed_id: String,
}

impl EventBuilder {
    pub fn new(run_id: String, seed_id: String) -> Self {
        Self { run_id, seed_id }
    }

    pub fn build(&self, event_type: EventType) -> Event {
        Event::new(self.run_id.clone(), self.seed_id.clone(), event_type)
    }

    pub fn run_created(&self) -> Event {
        self.build(EventType::RunCreated)
    }

    pub fn seed_loaded(&self) -> Event {
        self.build(EventType::SeedLoaded)
    }

    pub fn ns_created(&self, msg: &str) -> Event {
        self.build(EventType::NsCreated).with_msg(msg)
    }

    pub fn mount_done(&self, msg: &str) -> Event {
        self.build(EventType::MountDone).with_msg(msg)
    }

    pub fn cgroup_applied(&self) -> Event {
        self.build(EventType::CgroupApplied)
    }

    pub fn idmap_applied(&self) -> Event {
        self.build(EventType::IdmapApplied)
    }

    pub fn caps_dropped(&self) -> Event {
        self.build(EventType::CapsDropped)
    }

    pub fn seccomp_enabled(&self) -> Event {
        self.build(EventType::SeccompEnabled)
    }

    pub fn process_start(&self, pid: i32) -> Event {
        self.build(EventType::ProcessStart)
            .with_msg(format!("Process started with PID {}", pid))
    }

    pub fn process_exit(&self, code: i32) -> Event {
        self.build(EventType::ProcessExit)
            .with_code(code)
            .with_msg(format!("Process exited with code {}", code))
    }

    pub fn process_failed(&self, error: &str) -> Event {
        self.build(EventType::ProcessFailed)
            .with_level(LogLevel::Error)
            .with_error(error)
    }
}

/// Garden event builder (Iteration 2)
pub struct GardenEventBuilder {
    run_id: String,
    garden_id: String,
}

impl GardenEventBuilder {
    pub fn new(run_id: String, garden_id: String) -> Self {
        Self { run_id, garden_id }
    }

    /// Build garden-level event
    pub fn build(&self, event_type: EventType) -> Event {
        Event::new_garden(self.run_id.clone(), self.garden_id.clone(), event_type)
    }

    /// Build container-level event
    pub fn build_container(&self, container_name: String, event_type: EventType) -> Event {
        Event::new_container(
            self.run_id.clone(),
            self.garden_id.clone(),
            container_name,
            event_type,
        )
    }

    pub fn pod_net_ready(&self, bridge: &str, ip: &str) -> Event {
        self.build(EventType::PodNetReady)
            .with_msg(format!("Network ready: bridge={}, ip={}", bridge, ip))
            .with_data(serde_json::json!({
                "bridge": bridge,
                "ip": ip,
            }))
    }

    pub fn container_start(&self, container_name: &str, pid: i32) -> Event {
        self.build_container(container_name.to_string(), EventType::ContainerStart)
            .with_msg(format!("Container {} started with PID {}", container_name, pid))
            .with_data(serde_json::json!({"pid": pid}))
    }

    pub fn container_exit(&self, container_name: &str, code: i32) -> Event {
        self.build_container(container_name.to_string(), EventType::ContainerExit)
            .with_code(code)
            .with_msg(format!("Container {} exited with code {}", container_name, code))
    }

    pub fn pod_health(&self, status: &str) -> Event {
        self.build(EventType::PodHealth)
            .with_msg(format!("Pod health: {}", status))
            .with_data(serde_json::json!({"status": status}))
    }

    pub fn pod_exit(&self, status: &str) -> Event {
        self.build(EventType::PodExit)
            .with_msg(format!("Pod exited: {}", status))
            .with_data(serde_json::json!({"status": status}))
    }

    pub fn metric(&self, container_name: &str, metrics: &crate::metrics::ContainerMetrics) -> Event {
        self.build_container(container_name.to_string(), EventType::Metric)
            .with_data(crate::metrics::metrics_to_json(metrics))
    }
}
