use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Event types for process lifecycle
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
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

    /// Seed ID
    pub seed: String,

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
    /// Create a new event
    pub fn new(run_id: String, seed_id: String, event: EventType) -> Self {
        Self {
            ts: Utc::now(),
            level: LogLevel::Info,
            run: run_id,
            seed: seed_id,
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
