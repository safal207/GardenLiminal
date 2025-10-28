pub mod mem;
pub mod liminal;
pub mod cas;
pub mod pacts;
pub mod oci;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Seed record stored in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedRecord {
    pub id: String,
    pub name: String,
    pub manifest_yaml: String,
    pub created_at: String,
}

/// Run status enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunStatus {
    Init,
    Running,
    Exited(i32),
    Failed(String),
}

/// Store trait for persistence
pub trait Store: Send + Sync {
    /// Insert or update a seed record
    fn upsert_seed(&self, s: SeedRecord) -> Result<()>;

    /// Create a new run record
    fn create_run(&self, run_id: &str, seed_id: &str, start_ts: &str) -> Result<()>;

    /// Append an event to a run
    fn append_event(&self, run_id: &str, event: &serde_json::Value) -> Result<()>;

    /// Update run status
    fn update_run_status(&self, run_id: &str, status: RunStatus, end_ts: Option<&str>) -> Result<()>;

    /// Append metrics (Iteration 2)
    fn append_metrics(&self, _run_id: &str, _container: &str, _metrics: &serde_json::Value) -> Result<()> {
        // Default implementation: no-op
        Ok(())
    }
}

/// Store kind selector
#[derive(Debug, Clone)]
pub enum StoreKind {
    Memory,
    Liminal,
}

impl StoreKind {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "mem" | "memory" => Ok(StoreKind::Memory),
            "liminal" => Ok(StoreKind::Liminal),
            _ => anyhow::bail!("Unknown store kind: {}", s),
        }
    }

    pub fn create(&self) -> Result<Arc<dyn Store>> {
        match self {
            StoreKind::Memory => Ok(Arc::new(mem::MemoryStore::new())),
            StoreKind::Liminal => Ok(Arc::new(liminal::LiminalStore::new()?)),
        }
    }
}
