use super::{RunStatus, SeedRecord, Store};
use anyhow::Result;

/// Liminal-DB store adapter (stub implementation)
///
/// TODO: This is a placeholder for future Liminal-DB integration.
/// When ready, this will connect to the Liminal-DB and use its API
/// to persist seeds, runs, and events.
///
/// For now, it mirrors events to stdout like the memory store.
#[derive(Debug)]
pub struct LiminalStore {
    // Future: connection pool, client config, etc.
}

impl LiminalStore {
    pub fn new() -> Result<Self> {
        tracing::info!("Initializing Liminal-DB store (stub mode)");
        tracing::warn!("Liminal-DB integration not yet implemented - events will only go to stdout");

        // TODO: Initialize actual connection to Liminal-DB
        // For now, we just create a stub

        Ok(Self {})
    }
}

impl Store for LiminalStore {
    fn upsert_seed(&self, s: SeedRecord) -> Result<()> {
        tracing::debug!("LiminalStore: upserting seed {} (stub)", s.id);

        // TODO: Call Liminal-DB API to store seed record
        // Example (pseudo-code):
        // self.client.put_seed(s)?;

        tracing::info!("Seed {} upserted (stub - not persisted)", s.id);
        Ok(())
    }

    fn create_run(&self, run_id: &str, seed_id: &str, start_ts: &str) -> Result<()> {
        tracing::debug!("LiminalStore: creating run {} for seed {} (stub)", run_id, seed_id);

        // TODO: Call Liminal-DB API to create run record
        // Example (pseudo-code):
        // self.client.create_run(run_id, seed_id, start_ts)?;

        tracing::info!("Run {} created (stub - not persisted)", run_id);
        Ok(())
    }

    fn append_event(&self, run_id: &str, event: &serde_json::Value) -> Result<()> {
        tracing::debug!("LiminalStore: appending event to run {} (stub)", run_id);

        // Write to stdout for now
        println!("{}", serde_json::to_string(event)?);

        // TODO: Call Liminal-DB API to append event
        // Example (pseudo-code):
        // self.client.append_event(run_id, event)?;

        Ok(())
    }

    fn update_run_status(&self, run_id: &str, status: RunStatus, _end_ts: Option<&str>) -> Result<()> {
        tracing::debug!("LiminalStore: updating run {} status (stub)", run_id);

        // TODO: Call Liminal-DB API to update run status
        // Example (pseudo-code):
        // self.client.update_run_status(run_id, status, end_ts)?;

        match status {
            RunStatus::Init => {
                tracing::info!("Run {} status updated to Init (stub - not persisted)", run_id);
            }
            RunStatus::Running => {
                tracing::info!("Run {} status updated to Running (stub - not persisted)", run_id);
            }
            RunStatus::Exited(code) => {
                tracing::info!("Run {} exited with code {} (stub - not persisted)", run_id, code);
            }
            RunStatus::Failed(ref err) => {
                tracing::error!("Run {} failed: {} (stub - not persisted)", run_id, err);
            }
        }

        Ok(())
    }
}

impl Default for LiminalStore {
    fn default() -> Self {
        Self::new().expect("Failed to create LiminalStore")
    }
}
