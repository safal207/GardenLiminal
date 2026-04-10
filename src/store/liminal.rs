use super::{RunStatus, SeedRecord, Store};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::net::TcpStream;
use std::sync::Mutex;
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};

/// Default LiminalDB WebSocket address (liminal-cli default port)
const DEFAULT_LIMINAL_URL: &str = "ws://127.0.0.1:8787";

/// LiminalDB store adapter
///
/// Connects to a running LiminalDB instance via WebSocket and sends
/// container lifecycle events as impulses using the LiminalDB protocol:
///   {"command": "impulse", "data": { ...event... }}
pub struct LiminalStore {
    /// WebSocket connection (None if offline — falls back to stdout)
    socket: Mutex<Option<WebSocket<MaybeTlsStream<TcpStream>>>>,
    /// Target URL
    url: String,
}

impl std::fmt::Debug for LiminalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiminalStore")
            .field("url", &self.url)
            .finish()
    }
}

impl LiminalStore {
    pub fn new() -> Result<Self> {
        Self::with_url(DEFAULT_LIMINAL_URL)
    }

    pub fn with_url(url: &str) -> Result<Self> {
        let socket = match connect(url) {
            Ok((ws, _response)) => {
                tracing::info!(url = url, "Connected to LiminalDB");
                Some(ws)
            }
            Err(err) => {
                tracing::warn!(
                    url = url,
                    error = %err,
                    "LiminalDB not reachable — events will fall back to stdout"
                );
                None
            }
        };

        Ok(Self {
            socket: Mutex::new(socket),
            url: url.to_string(),
        })
    }

    /// Send an impulse to LiminalDB.
    /// Format: {"command": "impulse", "data": { ...payload... }}
    fn send_impulse(&self, data: Value) {
        // LiminalDB WebSocket protocol: {"cmd": "impulse", "data": {...}}
        let msg = json!({
            "cmd": "impulse",
            "data": data
        });

        let text = match serde_json::to_string(&msg) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize LiminalDB impulse");
                return;
            }
        };

        let mut guard = self.socket.lock().unwrap();
        if let Some(ws) = guard.as_mut() {
            if let Err(err) = ws.send(Message::Text(text.clone())) {
                tracing::warn!(error = %err, "LiminalDB send failed — falling back to stdout");
                // Drop broken connection so next attempt tries reconnect
                *guard = None;
                println!("{}", text);
            }
        } else {
            // Offline mode: try to reconnect once
            drop(guard);
            if let Ok((ws, _)) = connect(&self.url) {
                tracing::info!(url = self.url, "Reconnected to LiminalDB");
                let mut guard = self.socket.lock().unwrap();
                *guard = Some(ws);
                if let Some(ws) = guard.as_mut() {
                    let _ = ws.send(Message::Text(text.clone()));
                    return;
                }
            }
            // Still offline — stdout fallback
            println!("{}", text);
        }
    }
}

impl Store for LiminalStore {
    fn upsert_seed(&self, s: SeedRecord) -> Result<()> {
        self.send_impulse(json!({
            "type": "SEED_UPSERT",
            "seed_id": s.id,
            "seed_name": s.name,
        }));
        Ok(())
    }

    fn create_run(&self, run_id: &str, seed_id: &str, start_ts: &str) -> Result<()> {
        self.send_impulse(json!({
            "type": "RUN_CREATED",
            "run_id": run_id,
            "seed_id": seed_id,
            "start_ts": start_ts,
        }));
        Ok(())
    }

    fn append_event(&self, run_id: &str, event: &Value) -> Result<()> {
        self.send_impulse(json!({
            "type": "EVENT",
            "run_id": run_id,
            "event": event,
        }));
        Ok(())
    }

    fn update_run_status(&self, run_id: &str, status: RunStatus, end_ts: Option<&str>) -> Result<()> {
        let status_str = match &status {
            RunStatus::Init => "init",
            RunStatus::Running => "running",
            RunStatus::Exited(_) => "exited",
            RunStatus::Failed(_) => "failed",
        };

        let mut payload = json!({
            "type": "RUN_STATUS",
            "run_id": run_id,
            "status": status_str,
        });

        if let Some(ts) = end_ts {
            payload["end_ts"] = json!(ts);
        }

        match &status {
            RunStatus::Exited(code) => payload["exit_code"] = json!(code),
            RunStatus::Failed(err) => payload["error"] = json!(err),
            _ => {}
        }

        self.send_impulse(payload);
        Ok(())
    }
}

impl Default for LiminalStore {
    fn default() -> Self {
        Self::new().expect("Failed to create LiminalStore")
    }
}

/// Build a LiminalStore from an optional URL env var.
/// Falls back to DEFAULT_LIMINAL_URL if not set.
pub fn liminal_store_from_env() -> Result<LiminalStore> {
    let url = std::env::var("LIMINAL_URL")
        .unwrap_or_else(|_| DEFAULT_LIMINAL_URL.to_string());
    LiminalStore::with_url(&url).context("Failed to initialise LiminalStore")
}
