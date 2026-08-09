use anyhow::{Context, Result};
use chrono::Utc;
use nix::sys::signal::{killpg, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{self, ForkResult, Pid};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use uuid::Uuid;

use crate::events::{Event, EventBuilder, EventType};
use crate::isolate::{cgroups, idmap, ns, IsolationConfig};
use crate::seed::Seed;
use crate::store::{RunStatus, SeedRecord, Store};

const BOOTSTRAP_FAILURE_EXIT: i32 = 125;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SupervisorMessage {
    MappingRequest,
    MappingComplete { uid_map: String, gid_map: String },
    Event { event: Event },
    Outcome { code: i32 },
}

/// Process runner that orchestrates execution.
///
/// Trust boundary:
/// - this host supervisor owns durable audit/store connectivity;
/// - a bootstrap child enters workload namespaces;
/// - rootless ID maps are written by this host-side parent after user-ns entry;
/// - the bootstrap forks PID 1 in the new PID namespace;
/// - workload lifecycle events cross back over an inherited AF_UNIX socket;
/// - the control socket is close-on-exec and is never exposed to workload code.
pub struct ProcessRunner {
    seed: Seed,
    store: Arc<dyn Store>,
    run_id: String,
}

impl ProcessRunner {
    pub fn new(seed: Seed, store: Arc<dyn Store>) -> Self {
        Self {
            seed,
            store,
            run_id: Uuid::new_v4().to_string(),
        }
    }

    /// Run a Seed while keeping the host supervisor outside workload
    /// namespaces and outside the workload cgroup.
    pub fn run(self) -> Result<i32> {
        let events = EventBuilder::new(self.run_id.clone(), self.seed.meta.id.clone());

        self.store_seed_manifest()?;
        let start_ts = Utc::now().to_rfc3339();
        self.store
            .create_run(&self.run_id, &self.seed.meta.id, &start_ts)?;
        self.append_event(&events.run_created())?;
        self.append_event(&events.seed_loaded())?;

        let iso_config = IsolationConfig::new(&self.seed, self.run_id.clone());
        iso_config
            .apply_parent()
            .context("Failed to prepare host-side isolation resources")?;

        let host_ns_before = ns::namespace_snapshot().context("Capture host namespaces before run")?;
        let host_uid = ns::get_uid();
        let host_gid = ns::get_gid();

        let (mut parent_channel, child_channel) =
            UnixStream::pair().context("Create supervisor control socket")?;
        set_cloexec(&parent_channel)?;
        set_cloexec(&child_channel)?;

        let seed_clone = self.seed.clone();
        let run_id_clone = self.run_id.clone();

        match unsafe { unistd::fork() }.context("Failed to fork namespace bootstrap")? {
            ForkResult::Parent { child } => {
                drop(child_channel);

                unistd::setpgid(child, child)
                    .context("Failed to establish bootstrap process group")?;

                if uses_cgroups(&self.seed) {
                    if let Err(err) = cgroups::move_seed_pid_to_cgroup(&self.seed.meta.id, child.as_raw()) {
                        return self.abort_supervision(
                            child,
                            format!("Failed to move bootstrap into workload cgroup: {err:#}"),
                            &host_ns_before,
                        );
                    }
                    self.append_event(&events.cgroup_applied())?;
                }

                if self.seed.user.map_rootless {
                    if let Err(err) = self.complete_rootless_mapping(
                        child,
                        &mut parent_channel,
                        host_uid,
                        host_gid,
                    ) {
                        return self.abort_supervision(
                            child,
                            format!("Rootless mapping handshake failed: {err:#}"),
                            &host_ns_before,
                        );
                    }
                }

                self.supervise_bootstrap(child, parent_channel, host_ns_before)
            }
            ForkResult::Child => {
                drop(parent_channel);
                let code = Self::bootstrap_workload(seed_clone, run_id_clone, child_channel);
                std::process::exit(code);
            }
        }
    }

    fn complete_rootless_mapping(
        &self,
        bootstrap_pid: Pid,
        channel: &mut UnixStream,
        host_uid: u32,
        host_gid: u32,
    ) -> Result<()> {
        match read_message(channel).context("Read rootless mapping request")? {
            SupervisorMessage::MappingRequest => {}
            other => anyhow::bail!("Expected mapping_request, got {:?}", other),
        }

        let applied = idmap::configure_child_uid_gid_mapping(
            bootstrap_pid.as_raw(),
            &self.seed.user,
            host_uid,
            host_gid,
        )
        .context("Configure child rootless UID/GID map from host supervisor")?;

        send_message(
            channel,
            &SupervisorMessage::MappingComplete {
                uid_map: applied.uid_map,
                gid_map: applied.gid_map,
            },
        )
        .context("Send mapping completion to namespace bootstrap")?;
        Ok(())
    }

    fn bootstrap_workload(seed: Seed, run_id: String, mut channel: UnixStream) -> i32 {
        let events = EventBuilder::new(run_id.clone(), seed.meta.id.clone());

        let setup = (|| -> Result<Option<(String, String)>> {
            ns::create_namespaces(seed.net.enable, seed.user.map_rootless)
                .context("Failed to create workload namespaces")?;

            if seed.user.map_rootless {
                send_message(&mut channel, &SupervisorMessage::MappingRequest)
                    .context("Request rootless mapping from host supervisor")?;
                match read_message(&mut channel).context("Wait for rootless mapping completion")? {
                    SupervisorMessage::MappingComplete { uid_map, gid_map } => {
                        Ok(Some((uid_map, gid_map)))
                    }
                    other => anyhow::bail!("Expected mapping_complete, got {:?}", other),
                }
            } else {
                Ok(None)
            }
        })();

        let rootless_maps = match setup {
            Ok(maps) => maps,
            Err(err) => {
                let _ = send_event(
                    &mut channel,
                    events.process_failed(&format!("Namespace bootstrap failed: {err:#}")),
                );
                let _ = send_message(
                    &mut channel,
                    &SupervisorMessage::Outcome {
                        code: BOOTSTRAP_FAILURE_EXIT,
                    },
                );
                return BOOTSTRAP_FAILURE_EXIT;
            }
        };

        // CLONE_NEWPID applies to children, not to the caller of unshare().
        // This second fork creates the explicit PID-1 workload process.
        match unsafe { unistd::fork() } {
            Ok(ForkResult::Parent { child }) => {
                let (code, failure_event) = match wait_for_pid(child) {
                    Ok(WaitStatus::Exited(_, code)) => (code, None),
                    Ok(WaitStatus::Signaled(_, signal, _)) => {
                        let code = 128 + signal as i32;
                        (
                            code,
                            Some(events.process_failed(&format!(
                                "Workload PID 1 killed by signal: {:?}",
                                signal
                            ))),
                        )
                    }
                    Ok(other) => (
                        BOOTSTRAP_FAILURE_EXIT,
                        Some(events.process_failed(&format!(
                            "Unexpected PID-1 wait status: {:?}",
                            other
                        ))),
                    ),
                    Err(err) => (
                        BOOTSTRAP_FAILURE_EXIT,
                        Some(events.process_failed(&format!(
                            "Failed waiting for workload PID 1: {err:#}"
                        ))),
                    ),
                };

                if let Some(evt) = failure_event {
                    let _ = send_event(&mut channel, evt);
                } else {
                    let _ = send_event(&mut channel, events.process_exit(code));
                }
                let _ = send_message(&mut channel, &SupervisorMessage::Outcome { code });
                code
            }
            Ok(ForkResult::Child) => {
                if let Err(err) = Self::workload_exec(seed, run_id, rootless_maps, &mut channel) {
                    let _ = send_event(
                        &mut channel,
                        events.process_failed(&format!("Workload bootstrap/exec failed: {err:#}")),
                    );
                    BOOTSTRAP_FAILURE_EXIT
                } else {
                    unreachable!("successful exec does not return")
                }
            }
            Err(err) => {
                let _ = send_event(
                    &mut channel,
                    events.process_failed(&format!("Failed to fork workload PID 1: {err}")),
                );
                let _ = send_message(
                    &mut channel,
                    &SupervisorMessage::Outcome {
                        code: BOOTSTRAP_FAILURE_EXIT,
                    },
                );
                BOOTSTRAP_FAILURE_EXIT
            }
        }
    }

    fn workload_exec(
        seed: Seed,
        run_id: String,
        rootless_maps: Option<(String, String)>,
        channel: &mut UnixStream,
    ) -> Result<()> {
        let events = EventBuilder::new(run_id.clone(), seed.meta.id.clone());

        let namespace_pid = unistd::getpid().as_raw();
        if namespace_pid != 1 {
            anyhow::bail!(
                "PID namespace post-condition failed: workload expected PID 1, got {}",
                namespace_pid
            );
        }

        let snapshot = ns::namespace_snapshot().context("Capture workload namespace IDs")?;
        let ns_msg = format!(
            "pid=1, uts, ipc, mnt{}{}",
            if seed.user.map_rootless { ", user" } else { "" },
            if seed.net.enable { ", net" } else { "" }
        );
        let evt = events.ns_created(&ns_msg).with_data(serde_json::json!({
            "namespace_pid": namespace_pid,
            "namespaces": snapshot,
        }));
        send_event(channel, evt)?;

        let iso_config = IsolationConfig::new(&seed, run_id.clone());
        iso_config
            .apply_child()
            .context("Failed to apply workload isolation")?;

        send_event(channel, events.mount_done("Mounts configured and root pivoted"))?;

        if let Some((uid_map, gid_map)) = rootless_maps {
            let evt = events.idmap_applied().with_data(serde_json::json!({
                "uid": unistd::getuid().as_raw(),
                "gid": unistd::getgid().as_raw(),
                "uid_map": uid_map,
                "gid_map": gid_map,
            }));
            send_event(channel, evt)?;
        }

        if !seed.security.drop_caps.is_empty() {
            send_event(channel, events.caps_dropped())?;
        }
        if seed.security.seccomp_profile.is_some() {
            send_event(channel, events.seccomp_enabled())?;
        }

        std::env::set_current_dir(&seed.entrypoint.cwd)
            .with_context(|| format!("Failed to chdir to {}", seed.entrypoint.cwd))?;
        for env_var in &seed.entrypoint.env {
            if let Some(eq_pos) = env_var.find('=') {
                std::env::set_var(&env_var[..eq_pos], &env_var[eq_pos + 1..]);
            }
        }

        let evt = events.process_start(namespace_pid).with_data(serde_json::json!({
            "namespace_pid": namespace_pid,
            "pid1": true,
        }));
        send_event(channel, evt)?;

        let program = CString::new(seed.entrypoint.cmd[0].as_str())
            .context("Invalid program path")?;
        let args: Result<Vec<CString>> = seed
            .entrypoint
            .cmd
            .iter()
            .map(|s| CString::new(s.as_str()).context("Invalid argument"))
            .collect();
        let args = args?;

        unistd::execv(&program, &args).context("Failed to exec workload")?;
        unreachable!("exec should not return")
    }

    fn supervise_bootstrap(
        self,
        bootstrap_pid: Pid,
        channel: UnixStream,
        host_ns_before: ns::NamespaceSnapshot,
    ) -> Result<i32> {
        tracing::info!("Supervising namespace bootstrap PID {}", bootstrap_pid);

        let mut outcome = None;
        let mut last_failure = None::<String>;
        let mut running = false;
        let reader = BufReader::new(channel);

        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    return self.abort_supervision(
                        bootstrap_pid,
                        format!("Supervisor control-channel read failed: {err}"),
                        &host_ns_before,
                    );
                }
            };

            let message: SupervisorMessage = match serde_json::from_str(&line) {
                Ok(message) => message,
                Err(err) => {
                    return self.abort_supervision(
                        bootstrap_pid,
                        format!("Invalid supervisor control message: {err}"),
                        &host_ns_before,
                    );
                }
            };

            match message {
                SupervisorMessage::Event { event } => {
                    if event.event == EventType::ProcessFailed {
                        last_failure = event
                            .error
                            .clone()
                            .or_else(|| event.msg.clone())
                            .or_else(|| Some("workload failed".to_string()));
                    }
                    if event.event == EventType::ProcessStart && !running {
                        self.store
                            .update_run_status(&self.run_id, RunStatus::Running, None)?;
                        running = true;
                    }
                    self.append_event(&event)?;
                }
                SupervisorMessage::Outcome { code } => outcome = Some(code),
                unexpected @ (SupervisorMessage::MappingRequest
                | SupervisorMessage::MappingComplete { .. }) => {
                    return self.abort_supervision(
                        bootstrap_pid,
                        format!("Unexpected mapping message after handshake: {:?}", unexpected),
                        &host_ns_before,
                    );
                }
            }
        }

        let bootstrap_wait = wait_for_pid(bootstrap_pid)
            .context("Failed waiting for namespace bootstrap")?;
        let code = outcome.unwrap_or_else(|| match bootstrap_wait {
            WaitStatus::Exited(_, code) => code,
            WaitStatus::Signaled(_, signal, _) => 128 + signal as i32,
            _ => BOOTSTRAP_FAILURE_EXIT,
        });

        if outcome.is_none() {
            last_failure.get_or_insert_with(|| {
                format!("Bootstrap closed control channel without outcome: {:?}", bootstrap_wait)
            });
        }

        let host_ns_after = ns::namespace_snapshot().context("Capture host namespaces after run")?;
        if host_ns_after != host_ns_before {
            last_failure = Some(format!(
                "Host supervisor namespace IDs changed across workload run: before={:?} after={:?}",
                host_ns_before, host_ns_after
            ));
        }

        let end_ts = Utc::now().to_rfc3339();
        if let Some(error) = last_failure {
            self.store.update_run_status(
                &self.run_id,
                RunStatus::Failed(error),
                Some(&end_ts),
            )?;
        } else {
            self.store.update_run_status(
                &self.run_id,
                RunStatus::Exited(code),
                Some(&end_ts),
            )?;
        }

        self.cleanup()?;
        Ok(code)
    }

    fn abort_supervision(
        &self,
        bootstrap_pid: Pid,
        error: String,
        host_ns_before: &ns::NamespaceSnapshot,
    ) -> Result<i32> {
        let _ = killpg(bootstrap_pid, Signal::SIGKILL);
        let _ = wait_for_pid(bootstrap_pid);

        let events = EventBuilder::new(self.run_id.clone(), self.seed.meta.id.clone());
        let failure = events.process_failed(&error);
        let _ = self.append_event(&failure);
        let end_ts = Utc::now().to_rfc3339();
        let _ = self.store.update_run_status(
            &self.run_id,
            RunStatus::Failed(error.clone()),
            Some(&end_ts),
        );

        if let Ok(after) = ns::namespace_snapshot() {
            if &after != host_ns_before {
                tracing::error!(?host_ns_before, ?after, "Host namespace changed during failed supervision");
            }
        }
        let _ = self.cleanup();
        anyhow::bail!(error)
    }

    fn append_event(&self, event: &Event) -> Result<()> {
        self.store
            .append_event(&self.run_id, &event.to_json()?)
            .context("Persist lifecycle event")
    }

    fn store_seed_manifest(&self) -> Result<()> {
        let yaml = serde_yaml::to_string(&self.seed)
            .context("Failed to serialize seed to YAML")?;
        let record = SeedRecord {
            id: self.seed.meta.id.clone(),
            name: self.seed.meta.name.clone(),
            manifest_yaml: yaml,
            created_at: Utc::now().to_rfc3339(),
        };
        self.store.upsert_seed(record)?;
        Ok(())
    }

    fn cleanup(&self) -> Result<()> {
        if let Err(e) = cgroups::cleanup_cgroup(&self.seed.meta.id) {
            tracing::warn!("Failed to cleanup cgroups: {}", e);
        }
        Ok(())
    }
}

fn send_event(channel: &mut UnixStream, event: Event) -> Result<()> {
    send_message(channel, &SupervisorMessage::Event { event })
}

fn send_message(channel: &mut UnixStream, message: &SupervisorMessage) -> Result<()> {
    serde_json::to_writer(&mut *channel, message).context("Serialize supervisor message")?;
    channel.write_all(b"\n").context("Write supervisor message delimiter")?;
    channel.flush().context("Flush supervisor message")?;
    Ok(())
}

fn read_message(channel: &mut UnixStream) -> Result<SupervisorMessage> {
    let clone = channel.try_clone().context("Clone supervisor channel for read")?;
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).context("Read supervisor message")?;
    if bytes == 0 {
        anyhow::bail!("Supervisor channel closed before expected message");
    }
    serde_json::from_str(line.trim_end()).context("Decode supervisor message")
}

fn set_cloexec(stream: &UnixStream) -> Result<()> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFD) };
    if flags < 0 {
        anyhow::bail!("F_GETFD failed: {}", std::io::Error::last_os_error());
    }
    let ret = unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFD, flags | nix::libc::FD_CLOEXEC) };
    if ret < 0 {
        anyhow::bail!("F_SETFD(FD_CLOEXEC) failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn uses_cgroups(seed: &Seed) -> bool {
    seed.limits.cpu.shares.is_some()
        || seed.limits.memory.max.is_some()
        || seed.limits.pids.max.is_some()
}

fn wait_for_pid(pid: Pid) -> Result<WaitStatus> {
    loop {
        match waitpid(pid, None) {
            Ok(WaitStatus::Stopped(_, _)) | Ok(WaitStatus::Continued(_)) => continue,
            Ok(status) => return Ok(status),
            Err(nix::errno::Errno::EINTR) => continue,
            Err(err) => return Err(err).context("waitpid failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_message_roundtrip_preserves_event() {
        let event = EventBuilder::new("run".into(), "seed".into())
            .process_start(1)
            .with_data(serde_json::json!({"pid1": true}));
        let encoded = serde_json::to_string(&SupervisorMessage::Event { event }).unwrap();
        let decoded: SupervisorMessage = serde_json::from_str(&encoded).unwrap();
        match decoded {
            SupervisorMessage::Event { event } => {
                assert_eq!(event.event, EventType::ProcessStart);
                assert_eq!(event.data.unwrap()["pid1"], true);
            }
            _ => panic!("expected event message"),
        }
    }

    #[test]
    fn mapping_handshake_messages_roundtrip() {
        let request = serde_json::to_string(&SupervisorMessage::MappingRequest).unwrap();
        assert!(matches!(
            serde_json::from_str::<SupervisorMessage>(&request).unwrap(),
            SupervisorMessage::MappingRequest
        ));

        let complete = SupervisorMessage::MappingComplete {
            uid_map: "1000 501 1".into(),
            gid_map: "1000 20 1".into(),
        };
        let encoded = serde_json::to_string(&complete).unwrap();
        match serde_json::from_str::<SupervisorMessage>(&encoded).unwrap() {
            SupervisorMessage::MappingComplete { uid_map, gid_map } => {
                assert_eq!(uid_map, "1000 501 1");
                assert_eq!(gid_map, "1000 20 1");
            }
            _ => panic!("expected mapping_complete"),
        }
    }
}
