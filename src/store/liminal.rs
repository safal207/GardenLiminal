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
const LIFECYCLE_PATTERN_PREFIX: &str = "garden.lifecycle.v1:";
const LIFECYCLE_TTL_MS: u64 = 86_400_000;
const LIFECYCLE_STRENGTH: f64 = 0.85;

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
/// GardenLiminal lifecycle records are converted to the application-level
/// LiminalDB `Impulse` schema before being wrapped in the raw WebSocket command:
///
/// ```text
/// {"cmd":"impulse","data":{
///   "kind":"write",
///   "pattern":"garden.lifecycle.v1:<full lifecycle JSON>",
///   "strength":0.85,
///   "ttl_ms":86400000,
///   "tags":["garden","lifecycle","event"]
/// }}
/// ```
///
/// The JSON suffix preserves the complete lifecycle envelope even though the
/// current LiminalDB `Impulse` type has no arbitrary metadata field.
///
/// Transport semantics remain deliberately bounded: WebSocket `send()` success
/// is evidence that the frame was accepted by the local transport, not a durable
/// LiminalDB commit acknowledgement. The current raw impulse protocol has no
/// per-impulse durable ACK, so this adapter does not claim one.
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

    /// Convert the Garden lifecycle envelope into the schema accepted by
    /// LiminalDB's `parse_impulse_json`: pattern is required, while kind,
    /// strength, ttl_ms and tags are explicit for evidence-oriented writes.
    fn encode_lifecycle_impulse(data: Value) -> Result<String> {
        let record_type = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_ascii_lowercase();
        let lifecycle_json = serde_json::to_string(&data)
            .context("Failed to serialize Garden lifecycle envelope")?;
        let pattern = format!("{}{}", LIFECYCLE_PATTERN_PREFIX, lifecycle_json);

        serde_json::to_string(&json!({
            "cmd": "impulse",
            "data": {
                "pattern": pattern,
                "strength": LIFECYCLE_STRENGTH,
                "ttl_ms": LIFECYCLE_TTL_MS,
                "kind": "write",
                "tags": ["garden", "lifecycle", record_type],
            }
        }))
        .context("Failed to serialize LiminalDB impulse command")
    }

    /// Queue an impulse first, then attempt an ordered flush.
    ///
    /// No send error may silently discard the current payload: the FIFO front
    /// is popped only after a successful WebSocket write. If the socket breaks,
    /// it is discarded and the still-pending payload is retried on a later Store
    /// call after reconnect.
    fn send_impulse(&self, data: Value) -> Result<()> {
        let text = Self::encode_lifecycle_impulse(data)?;

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

    fn decode_lifecycle_pattern(frame: &Value) -> Value {
        assert_eq!(frame["cmd"], "impulse");
        assert_eq!(frame["data"]["kind"], "write");
        assert_eq!(frame["data"]["ttl_ms"], LIFECYCLE_TTL_MS);
        assert_eq!(frame["data"]["strength"], LIFECYCLE_STRENGTH);
        let tags = frame["data"]["tags"].as_array().expect("tags array");
        assert!(tags.contains(&json!("garden")));
        assert!(tags.contains(&json!("lifecycle")));

        let pattern = frame["data"]["pattern"].as_str().expect("required pattern");
        let json_suffix = pattern
            .strip_prefix(LIFECYCLE_PATTERN_PREFIX)
            .expect("Garden lifecycle pattern prefix");
        serde_json::from_str(json_suffix).expect("decode embedded lifecycle JSON")
    }

    #[test]
    fn application_schema_has_required_pattern_and_preserves_full_record() {
        let record = json!({
            "type": "EVENT",
            "run_id": "run-schema",
            "event": {"event": "PROCESS_START", "data": {"pid1": true}}
        });
        let encoded = LiminalStore::encode_lifecycle_impulse(record.clone()).unwrap();
        let frame: Value = serde_json::from_str(&encoded).unwrap();
        let decoded = decode_lifecycle_pattern(&frame);
        assert_eq!(decoded, record);
        assert!(frame["data"]["tags"].as_array().unwrap().contains(&json!("event")));
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
                let frame: Value = serde_json::from_str(&text).expect("parse impulse frame");
                let lifecycle = decode_lifecycle_pattern(&frame);
                seq.push(lifecycle["event"]["seq"].as_i64().unwrap());
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
