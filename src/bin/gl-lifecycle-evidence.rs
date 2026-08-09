use anyhow::{Context, Result};
use gl::events::EventType;
use gl::isolate::ns::{self, NamespaceSnapshot};
use gl::process::ProcessRunner;
use gl::seed::{
    EntrypointConfig, LimitsConfig, LoggingConfig, MountConfig, NetConfig, RootfsConfig,
    SecurityConfig, Seed, SeedMeta, StoreConfig, UserConfig,
};
use gl::store::{RunStatus, SeedRecord, Store};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct EvidenceStore {
    expected_host: NamespaceSnapshot,
    events: Mutex<Vec<Value>>,
    statuses: Mutex<Vec<String>>,
    host_side_checks: Mutex<u64>,
}

impl EvidenceStore {
    fn new(expected_host: NamespaceSnapshot) -> Self {
        Self {
            expected_host,
            events: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
            host_side_checks: Mutex::new(0),
        }
    }

    fn assert_host_side(&self) -> Result<()> {
        let current = ns::namespace_snapshot().context("store-side namespace snapshot")?;
        if current != self.expected_host {
            anyhow::bail!(
                "Store crossed workload namespace boundary: expected {:?}, got {:?}",
                self.expected_host,
                current
            );
        }
        *self.host_side_checks.lock().unwrap() += 1;
        Ok(())
    }
}

impl Store for EvidenceStore {
    fn upsert_seed(&self, _s: SeedRecord) -> Result<()> {
        self.assert_host_side()
    }

    fn create_run(&self, _run_id: &str, _seed_id: &str, _start_ts: &str) -> Result<()> {
        self.assert_host_side()
    }

    fn append_event(&self, _run_id: &str, event: &Value) -> Result<()> {
        self.assert_host_side()?;
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn update_run_status(
        &self,
        _run_id: &str,
        status: RunStatus,
        _end_ts: Option<&str>,
    ) -> Result<()> {
        self.assert_host_side()?;
        self.statuses.lock().unwrap().push(format!("{:?}", status));
        Ok(())
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().context("usage: gl-lifecycle-evidence <rootful|rootless> <rootfs>")?;
    let rootfs = PathBuf::from(
        args.next()
            .context("usage: gl-lifecycle-evidence <rootful|rootless> <rootfs>")?,
    );
    let rootless = match mode.as_str() {
        "rootful" => false,
        "rootless" => true,
        other => anyhow::bail!("unknown lifecycle evidence mode: {other}"),
    };

    let host_before = ns::namespace_snapshot().context("host namespace snapshot before run")?;
    let host_uid = ns::get_uid();
    let host_gid = ns::get_gid();
    let store = Arc::new(EvidenceStore::new(host_before.clone()));

    let seed = Seed {
        api_version: "v0".to_string(),
        kind: "Seed".to_string(),
        meta: SeedMeta {
            name: format!("namespace-evidence-{mode}"),
            id: format!("namespace-evidence-{mode}-{}", std::process::id()),
        },
        rootfs: RootfsConfig { path: rootfs },
        entrypoint: EntrypointConfig {
            cmd: vec!["/bin/busybox".to_string(), "true".to_string()],
            env: vec![],
            cwd: "/".to_string(),
        },
        limits: LimitsConfig::default(),
        net: NetConfig { enable: true },
        mounts: Vec::<MountConfig>::new(),
        security: SecurityConfig::default(),
        user: UserConfig {
            uid: 1000,
            gid: 1000,
            map_rootless: rootless,
        },
        logging: LoggingConfig::default(),
        store: StoreConfig::default(),
    };

    let runner = ProcessRunner::new(seed, store.clone());
    let exit_code = runner.run().context("ProcessRunner evidence run")?;
    if exit_code != 0 {
        anyhow::bail!("evidence workload exited with code {exit_code}");
    }

    let host_after = ns::namespace_snapshot().context("host namespace snapshot after run")?;
    if host_after != host_before {
        anyhow::bail!("host namespace IDs changed across ProcessRunner run");
    }

    let events = store.events.lock().unwrap().clone();
    let statuses = store.statuses.lock().unwrap().clone();
    let host_side_checks = *store.host_side_checks.lock().unwrap();

    let ns_event = events
        .iter()
        .find(|event| event["event"] == json!(EventType::NsCreated))
        .context("missing NS_CREATED evidence")?;
    let namespaces: NamespaceSnapshot = serde_json::from_value(ns_event["data"]["namespaces"].clone())
        .context("decode workload namespace snapshot")?;

    if ns_event["data"]["namespace_pid"] != 1 {
        anyhow::bail!("workload did not report namespace PID 1");
    }
    if namespaces.pid == host_before.pid
        || namespaces.mnt == host_before.mnt
        || namespaces.uts == host_before.uts
        || namespaces.ipc == host_before.ipc
        || namespaces.net == host_before.net
    {
        anyhow::bail!(
            "workload namespace post-condition failed: host={:?} workload={:?}",
            host_before,
            namespaces
        );
    }

    if rootless {
        if namespaces.user == host_before.user {
            anyhow::bail!("rootless workload did not receive a new user namespace");
        }
        let idmap_event = events
            .iter()
            .find(|event| event["event"] == json!(EventType::IdmapApplied))
            .context("missing IDMAP_APPLIED evidence")?;
        if idmap_event["data"]["uid"] != 1000 || idmap_event["data"]["gid"] != 1000 {
            anyhow::bail!("mapped workload identity is not 1000:1000");
        }
        let expected_uid = format!("1000 {} 1", host_uid);
        let expected_gid = format!("1000 {} 1", host_gid);
        if idmap_event["data"]["uid_map"] != expected_uid
            || idmap_event["data"]["gid_map"] != expected_gid
        {
            anyhow::bail!(
                "rootless map evidence mismatch: uid_map={} gid_map={}",
                idmap_event["data"]["uid_map"],
                idmap_event["data"]["gid_map"]
            );
        }
    } else if namespaces.user != host_before.user {
        anyhow::bail!("rootful path unexpectedly changed user namespace");
    }

    let start = events
        .iter()
        .find(|event| event["event"] == json!(EventType::ProcessStart))
        .context("missing PROCESS_START evidence")?;
    if start["data"]["pid1"] != true || start["data"]["namespace_pid"] != 1 {
        anyhow::bail!("PROCESS_START does not prove PID-1 semantics");
    }

    if !statuses.iter().any(|status| status == "Running")
        || !statuses.iter().any(|status| status == "Exited(0)")
    {
        anyhow::bail!("run status sequence missing Running/Exited(0): {:?}", statuses);
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "lifecycle_postcondition": "PASS",
            "mode": mode,
            "exit_code": exit_code,
            "host_namespace_stable": true,
            "store_host_side_checks": host_side_checks,
            "host_namespaces": host_before,
            "workload_namespaces": namespaces,
            "workload_pid1": true,
            "network_namespace_isolated": true,
            "rootless_user_namespace": rootless,
            "statuses": statuses,
            "event_count": events.len(),
        }))?
    );

    Ok(())
}
