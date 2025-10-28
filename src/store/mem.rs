use super::{RunStatus, SeedRecord, Store};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory store implementation
/// Events are also written to stdout as JSON lines
#[derive(Debug)]
pub struct MemoryStore {
    inner: Arc<Mutex<MemoryStoreInner>>,
}

#[derive(Debug, Default)]
struct MemoryStoreInner {
    seeds: HashMap<String, SeedRecord>,
    runs: HashMap<String, RunRecord>,
}

#[derive(Debug, Clone)]
struct RunRecord {
    run_id: String,
    seed_id: String,
    start_ts: String,
    end_ts: Option<String>,
    status: RunStatus,
    events: Vec<serde_json::Value>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemoryStoreInner::default())),
        }
    }

    /// Get a copy of stored data (for debugging)
    #[allow(dead_code)]
    pub fn dump(&self) -> Result<String> {
        let inner = self.inner.lock().unwrap();
        Ok(serde_json::to_string_pretty(&*inner)?)
    }
}

impl Store for MemoryStore {
    fn upsert_seed(&self, s: SeedRecord) -> Result<()> {
        tracing::debug!("Store: upserting seed {}", s.id);

        let mut inner = self.inner.lock().unwrap();
        inner.seeds.insert(s.id.clone(), s);

        Ok(())
    }

    fn create_run(&self, run_id: &str, seed_id: &str, start_ts: &str) -> Result<()> {
        tracing::debug!("Store: creating run {} for seed {}", run_id, seed_id);

        let mut inner = self.inner.lock().unwrap();

        let record = RunRecord {
            run_id: run_id.to_string(),
            seed_id: seed_id.to_string(),
            start_ts: start_ts.to_string(),
            end_ts: None,
            status: RunStatus::Init,
            events: Vec::new(),
        };

        inner.runs.insert(run_id.to_string(), record);

        Ok(())
    }

    fn append_event(&self, run_id: &str, event: &serde_json::Value) -> Result<()> {
        tracing::debug!("Store: appending event to run {}", run_id);

        // Write to stdout
        println!("{}", serde_json::to_string(event)?);

        let mut inner = self.inner.lock().unwrap();

        if let Some(run) = inner.runs.get_mut(run_id) {
            run.events.push(event.clone());
        } else {
            tracing::warn!("Run {} not found, event not stored", run_id);
        }

        Ok(())
    }

    fn update_run_status(&self, run_id: &str, status: RunStatus, end_ts: Option<&str>) -> Result<()> {
        tracing::debug!("Store: updating run {} status to {:?}", run_id, status);

        let mut inner = self.inner.lock().unwrap();

        if let Some(run) = inner.runs.get_mut(run_id) {
            run.status = status;
            if let Some(ts) = end_ts {
                run.end_ts = Some(ts.to_string());
            }
        } else {
            tracing::warn!("Run {} not found, status not updated", run_id);
        }

        Ok(())
    }
}

impl serde::Serialize for MemoryStoreInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("MemoryStoreInner", 2)?;
        state.serialize_field("seeds", &self.seeds)?;
        state.serialize_field("runs", &self.runs)?;
        state.end()
    }
}

impl serde::Serialize for RunRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("RunRecord", 6)?;
        state.serialize_field("run_id", &self.run_id)?;
        state.serialize_field("seed_id", &self.seed_id)?;
        state.serialize_field("start_ts", &self.start_ts)?;
        state.serialize_field("end_ts", &self.end_ts)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("events", &self.events)?;
        state.end()
    }
}
