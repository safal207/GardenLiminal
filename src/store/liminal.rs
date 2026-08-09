use super::{RunStatus, SeedRecord, Store};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::Mutex;
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};

/// Default LiminalDB WebSocket address (liminal-cli default port)
const DEFAULT_LIMINAL_URL: &str = "ws://127.0.0.1:8787";
const MAX_PENDING_IMPULSES: usize = 1024;

type LiminalSocket = WebSocket<MaybeTlsStream<TcpStream>>;

struct LiminalState {
    socket: Option<LiminalSocket>,
    /// FIFO outbox. A payload is removed only after tungstenite reports a
    /// successful WebSocket write. Failed/offline payloads remain here for the
    /// next Store operation to reconnect and replay in order.
    pending: VecDeque<String>,
}

/// LiminalDB store adapter.
///
/// GardenLiminal lifecycle records are wrapped in the documented raw LiminalDB
/// WebSocket command shape:
///
/// ```text
/// {"cmd":"impulse","data":{...}}
/// ```
///
/// Transport semantics are deliberately bounded: WebSocket `send()` success is
/// evidence that the frame was accepted by the local transport, not a durable
/// LiminalDB commit acknowledgement. The current LiminalDB impulse protocol has
/// no per-impulse durable ACK, so this adapter does not claim one.
pub struct LiminalStore {
    state: Mutex<LiminalState>,
    url: String,
}

impl std::fmt::Debug for LiminalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pending = self
            .state
            .lock()
            .map(|state| state.pending.len())
            .unwrap_or(usize::MAX);
        f.debug_struct("LiminalStore")
            .field("url", &self.url)
            .field("pending", &pending)
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
                    "LiminalDB not reachable — impulses will be queued for reconnect"
                );
                None
            }
        };

        Ok(Self {
            state: Mutex::new(LiminalState {
                socket,
                pending: VecDeque::new(),
            }),
            url: url.to_string(),
        })
    }

    /// Queue an impulse first, then attempt an ordered flush.
    ///
    /// No send error may silently discard the current payload: the FIFO front
    /// is popped only after a successful WebSocket write. If the socket breaks,
    /// it is discarded and the still-pending payload is retried on a later Store
    /// call after reconnect.
    fn send_impulse(&self, data: Value) -> Result<()> {
        let text = serde_json::to_string(&json!({
            "cmd": "impulse",
            "data": data
        }))
        .context("Failed to serialize LiminalDB impulse")?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("LiminalDB store state lock poisoned"))?;

        if state.pending.len() >= MAX_PENDING_IMPULSES {
            anyhow::bail!(
                "LiminalDB pending outbox is full ({} impulses); refusing to drop audit evidence",
                MAX_PENDING_IMPULSES
            );
        }
        state.pending.push_back(text);
        self.flush_pending(&mut state)
    }

    fn flush_pending(&self, state: &mut LiminalState) -> Result<()> {
        if state.pending.is_empty() {
            return Ok(());
        }

        if state.socket.is_none() {
            match connect(&self.url) {
                Ok((ws, _response)) => {
                    tracing::info!(
                        url = self.url,
                        pending = state.pending.len(),
                        "Reconnected to LiminalDB; replaying pending impulses"
                    );
                    state.socket = Some(ws);
                }
                Err(err) => {
                    tracing::warn!(
                        url = self.url,
                        pending = state.pending.len(),
                        error = %err,
                        "LiminalDB still offline; retaining impulses in bounded outbox"
                    );
                    return Ok(());
                }
            }
        }

        while let Some(text) = state.pending.front().cloned() {
            let send_result = state
                .socket
                .as_mut()
                .expect("socket established before flush")
                .send(Message::Text(text));

            match send_result {
                Ok(()) => {
                    state.pending.pop_front();
                }
                Err(err) => {
                    tracing::warn!(
                        url = self.url,
                        pending = state.pending.len(),
                        error = %err,
                        "LiminalDB send failed; retaining unsent impulse for reconnect"
                    );
                    state.socket = None;
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }
}

impl Store for LiminalStore {
    fn upsert_seed(&self, s: SeedRecord) -> Result<()> {
        self.send_impulse(json!({
            "type": "SEED_UPSERT",
            "seed_id": s.id,
            "seed_name": s.name,
        }))
    }

    fn create_run(&self, run_id: &str, seed_id: &str, start_ts: &str) -> Result<()> {
        self.send_impulse(json!({
            "type": "RUN_CREATED",
            "run_id": run_id,
            "seed_id": seed_id,
            "start_ts": start_ts,
        }))
    }

    fn append_event(&self, run_id: &str, event: &Value) -> Result<()> {
        self.send_impulse(json!({
            "type": "EVENT",
            "run_id": run_id,
            "event": event,
        }))
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

        self.send_impulse(payload)
    }
}

impl Default for LiminalStore {
    fn default() -> Self {
        Self::new().expect("Failed to create LiminalStore")
    }
}

/// Build a LiminalStore from an optional URL env var.
pub fn liminal_store_from_env() -> Result<LiminalStore> {
    let url = std::env::var("LIMINAL_URL")
        .unwrap_or_else(|_| DEFAULT_LIMINAL_URL.to_string());
    LiminalStore::with_url(&url).context("Failed to initialise LiminalStore")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use tungstenite::accept;

    fn reserve_address() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    }

    #[test]
    fn offline_impulse_is_replayed_in_order_after_reconnect() {
        let addr = reserve_address();
        let url = format!("ws://{}", addr);
        let store = LiminalStore::with_url(&url).expect("create offline store");

        store
            .append_event("run-1", &json!({"seq": 1}))
            .expect("queue first offline event");
        assert_eq!(store.pending_len(), 1);

        let listener = TcpListener::bind(addr).expect("start reconnect fixture");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept reconnect");
            let mut ws = accept(stream).expect("accept websocket");
            let mut seq = Vec::new();
            for _ in 0..2 {
                let msg = ws.read().expect("read impulse");
                let text = msg.into_text().expect("text impulse");
                let value: Value = serde_json::from_str(&text).expect("parse impulse");
                seq.push(value["data"]["event"]["seq"].as_i64().unwrap());
            }
            seq
        });

        store
            .append_event("run-1", &json!({"seq": 2}))
            .expect("reconnect and flush queue");

        assert_eq!(server.join().unwrap(), vec![1, 2]);
        assert_eq!(store.pending_len(), 0);
    }

    #[test]
    fn outbox_capacity_is_fail_closed() {
        let addr = reserve_address();
        let url = format!("ws://{}", addr);
        let store = LiminalStore::with_url(&url).expect("create offline store");

        for seq in 0..MAX_PENDING_IMPULSES {
            store
                .append_event("run-capacity", &json!({"seq": seq}))
                .expect("queue within bounded capacity");
        }
        let err = store
            .append_event("run-capacity", &json!({"seq": MAX_PENDING_IMPULSES}))
            .unwrap_err();
        assert!(err.to_string().contains("outbox is full"));
        assert_eq!(store.pending_len(), MAX_PENDING_IMPULSES);
    }
}
