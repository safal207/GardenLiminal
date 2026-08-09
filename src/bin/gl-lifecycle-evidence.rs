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
    fn upsert_seed(&self, _s: SeedRecord) -> Result<()> { self.assert_host_side() }
    fn create_run(&self, _run_id: &str, _seed_id: &str, _start_ts: &str) -> Result<()> {
        self.assert_host_side()
    }
    fn append_event(&self, _run_id: &str, event: &Value) -> Result<()> {
        self.assert_host_side()?;
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
    fn update_run_status(&self, _run_id: &str, status: RunStatus, _end_ts: Option<&str>) -> Result<()> {
        self.assert_host_side()?;
        self.statuses.lock().unwrap().push(format!("{:?}", status));
        Ok(())
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().context("usage: gl-lifecycle-evidence <rootful|rootless> <rootfs>")?;
    let rootfs = PathBuf::from(args.next().context("usage: gl-lifecycle-evidence <rootful|rootless> <rootfs>")?);
    let rootless = match mode.as_str() {
        "rootful" => false,
        "rootless" => true,
        other => anyhow::bail!("unknown lifecycle evidence mode: {other}"),
    };

    let host_before = ns::namespace_snapshot().context("host namespace snapshot before run")?;
    let host_uid = ns::get_uid();
    let host_gid = ns::get_gid();
    let store = Arc::new(EvidenceStore::new(host_before.clone()));

    // The bounded rootless mode maps namespace root 0:0 to the current host
    // UID/GID. Rootful evidence keeps the historical default 1000:1000 because
    // no user-namespace mapping is requested on that path.
    let (workload_uid, workload_gid) = if rootless { (0, 0) } else { (1000, 1000) };

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
            uid: workload_uid,
            gid: workload_gid,
            map_rootless: rootless,
        },
        logging: LoggingConfig::default(),
        store: StoreConfig::default(),
    };

    let exit_code = ProcessRunner::new(seed, store.clone())
        .run()
        .context("ProcessRunner evidence run")?;

    let host_after = ns::namespace_snapshot().context("host namespace snapshot after run")?;
    let events = store.events.lock().unwrap().clone();
    let statuses = store.statuses.lock().unwrap().clone();
    let host_side_checks = *store.host_side_checks.lock().unwrap();

    if exit_code != 0 {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "lifecycle_postcondition": "FAIL",
                "mode": mode,
                "exit_code": exit_code,
                "host_namespace_stable": host_after == host_before,
                "store_host_side_checks": host_side_checks,
                "host_namespaces_before": host_before,
                "host_namespaces_after": host_after,
                "statuses": statuses,
                "events": events,
            }))?
        );
        anyhow::bail!("evidence workload exited with code {exit_code}");
    }

    if host_after != host_before {
        anyhow::bail!("host namespace IDs changed across ProcessRunner run");
    }

    let ns_event = events.iter()
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
        anyhow::bail!("workload namespace post-condition failed: host={:?} workload={:?}", host_before, namespaces);
    }

    if rootless {
        if namespaces.user == host_before.user {
            anyhow::bail!("rootless workload did not receive a new user namespace");
        }
        let idmap_event = events.iter()
            .find(|event| event["event"] == json!(EventType::IdmapApplied))
            .context("missing IDMAP_APPLIED evidence")?;
        if idmap_event["data"]["uid"] != 0 || idmap_event["data"]["gid"] != 0 {
            anyhow::bail!("bounded rootless workload identity is not namespace root 0:0");
        }
        let expected_uid = format!("0 {} 1", host_uid);
        let expected_gid = format!("0 {} 1", host_gid);
        if idmap_event["data"]["uid_map"] != expected_uid
            || idmap_event["data"]["gid_map"] != expected_gid
        {
            anyhow::bail!(
                "rootless map evidence mismatch: uid_map={} gid_map={}",
                idmap_event["data"]["uid_map"], idmap_event["data"]["gid_map"]
            );
        }
    } else if namespaces.user != host_before.user {
        anyhow::bail!("rootful path unexpectedly changed user namespace");
    }

    let start = events.iter()
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
            "rootless_mapping_contract": if rootless { "namespace-root-single-id" } else { "none" },
            "statuses": statuses,
            "event_count": events.len(),
        }))?
    );

    Ok(())
}
