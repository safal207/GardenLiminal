use anyhow::{Context, Result};
use chrono::Utc;
use nix::sched::clone;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{self, Pid};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use uuid::Uuid;

use crate::events::EventBuilder;
use crate::isolate::{cgroups, idmap, ns, IsolationConfig};
use crate::seed::Seed;
use crate::store::{RunStatus, SeedRecord, Store};

const CHILD_STACK_BYTES: usize = 1024 * 1024;
const CHILD_START_BYTE: u8 = b'G';
const CHILD_BOOTSTRAP_FAILURE: isize = 125;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChildNotice {
    NamespaceReady {
        ids: ns::NamespaceIds,
        namespace_pid: i32,
    },
    IsolationReady,
    ProcessStart {
        namespace_pid: i32,
    },
    Failed {
        error: String,
    },
}

/// Owns the clone(2) stack for exactly as long as the workload task may still
/// execute on it. Any early return after clone therefore terminates and reaps
/// the child before the backing stack memory can be dropped.
struct ClonedWorkload {
    pid: Pid,
    _stack: Vec<u8>,
    cgroup_seed_id: Option<String>,
    reaped: bool,
}

impl ClonedWorkload {
    fn new(pid: Pid, stack: Vec<u8>, cgroup_seed_id: Option<String>) -> Self {
        Self {
            pid,
            _stack: stack,
            cgroup_seed_id,
            reaped: false,
        }
    }

    fn pid(&self) -> Pid {
        self.pid
    }

    fn wait_terminal(&mut self) -> Result<WaitStatus> {
        let status = wait_for_terminal(self.pid)?;
        self.reaped = true;
        Ok(status)
    }

    fn terminate(&self) {
        let _ = kill(self.pid, Signal::SIGKILL);
    }
}

impl Drop for ClonedWorkload {
    fn drop(&mut self) {
        if !self.reaped {
            match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
                    self.reaped = true;
                }
                Err(nix::errno::Errno::ECHILD) => {
                    // The task is no longer a waitable child, so it cannot
                    // still be executing on this supervisor-owned stack.
                    self.reaped = true;
                }
                Ok(_) | Err(_) => {
                    let _ = kill(self.pid, Signal::SIGKILL);
                    if let Err(err) = wait_for_terminal(self.pid) {
                        tracing::warn!(
                            child_pid = self.pid.as_raw(),
                            error = %err,
                            "Failed to reap cloned workload during guard cleanup"
                        );
                    } else {
                        self.reaped = true;
                    }
                }
            }
        }

        if let Some(seed_id) = self.cgroup_seed_id.take() {
            if let Err(err) = cgroups::cleanup_cgroup(&seed_id) {
                tracing::warn!(
                    seed_id = %seed_id,
                    error = %err,
                    "Failed to cleanup workload cgroup from clone guard"
                );
            }
        }
    }
}

/// Process runner that orchestrates execution while keeping the host-side
/// supervisor outside workload namespaces and workload cgroups.
pub struct ProcessRunner {
    seed: Seed,
    store: Arc<dyn Store>,
    run_id: String,
}

impl ProcessRunner {
    pub fn new(seed: Seed, store: Arc<dyn Store>) -> Self {
        let run_id = Uuid::new_v4().to_string();
        Self {
            seed,
            store,
            run_id,
        }
    }

    /// Run the process with full isolation.
    ///
    /// The supervisor creates the workload task directly with clone(2)
    /// namespace flags. The child starts blocked on a local control socket while
    /// the supervisor installs any rootless UID/GID map and cgroup membership.
    /// Only then is the child released to apply mount/capability/seccomp policy
    /// and exec the workload. Lifecycle evidence is written to Store only by
    /// the host-side supervisor.
    pub fn run(self) -> Result<i32> {
        let events = EventBuilder::new(self.run_id.clone(), self.seed.meta.id.clone());
        let supervisor_ns_before = ns::current_namespace_ids()
            .context("Failed to capture supervisor namespace state before clone")?;

        self.store_seed_manifest()?;

        let start_ts = Utc::now().to_rfc3339();
        self.store
            .create_run(&self.run_id, &self.seed.meta.id, &start_ts)?;

        self.append_event(&events.run_created())?;
        self.append_event(&events.seed_loaded())?;

        let iso_config = IsolationConfig::new(&self.seed, self.run_id.clone());
        let cgroup_path = iso_config
            .apply_parent()
            .context("Failed to prepare parent-side isolation resources")?;

        let parent_uid = ns::get_uid();
        let parent_gid = ns::get_gid();
        let flags = ns::workload_clone_flags(self.seed.user.map_rootless, self.seed.net.enable);

        let (mut supervisor_control, mut child_control) =
            UnixStream::pair().context("Failed to create supervisor/child control socket")?;
        set_cloexec(&supervisor_control)?;
        set_cloexec(&child_control)?;

        // clone() duplicates all file descriptors. The child explicitly closes
        // its duplicate of the supervisor endpoint so EOF and CLOEXEC semantics
        // on the child endpoint remain meaningful.
        let supervisor_peer_fd = supervisor_control.as_raw_fd();
        let seed_for_child = self.seed.clone();
        let run_id_for_child = self.run_id.clone();
        let mut child_stack = vec![0u8; CHILD_STACK_BYTES];

        let child_pid = match unsafe {
            clone(
                Box::new(move || {
                    nix::libc::close(supervisor_peer_fd);

                    let mut start = [0u8; 1];
                    if let Err(err) = child_control.read_exact(&mut start) {
                        eprintln!("Child bootstrap authorization failed: {err}");
                        return CHILD_BOOTSTRAP_FAILURE;
                    }
                    if start[0] != CHILD_START_BYTE {
                        eprintln!("Child received invalid bootstrap authorization byte");
                        return CHILD_BOOTSTRAP_FAILURE;
                    }

                    match Self::child_exec_static(
                        &seed_for_child,
                        &run_id_for_child,
                        &mut child_control,
                    ) {
                        Ok(()) => 0,
                        Err(err) => {
                            let rendered = format!("{err:#}");
                            let _ = send_notice(
                                &mut child_control,
                                &ChildNotice::Failed {
                                    error: rendered.clone(),
                                },
                            );
                            eprintln!("Child exec failed: {rendered}");
                            CHILD_BOOTSTRAP_FAILURE
                        }
                    }
                }),
                &mut child_stack,
                flags,
                Some(Signal::SIGCHLD as i32),
            )
        } {
            Ok(pid) => pid,
            Err(err) => {
                if cgroup_path.is_some() {
                    let _ = self.cleanup();
                }
                return Err(err).context("Failed to clone isolated workload task");
            }
        };

        let cgroup_seed_id = cgroup_path
            .as_ref()
            .map(|_| self.seed.meta.id.clone());
        let mut child = ClonedWorkload::new(child_pid, child_stack, cgroup_seed_id);

        // Do not release the child until all host-side transitions that target
        // its host PID are complete.
        if self.seed.user.map_rootless {
            if let Err(err) = idmap::map_child_from_parent(
                child.pid(),
                &self.seed.user,
                parent_uid,
                parent_gid,
            ) {
                return self.abort_blocked_child(
                    child,
                    supervisor_control,
                    events,
                    format!("Failed to install rootless UID/GID mapping: {err:#}"),
                );
            }
        }

        if let Some(ref path) = cgroup_path {
            if let Err(err) = cgroups::move_pid_to_path(path, child.pid().into()) {
                return self.abort_blocked_child(
                    child,
                    supervisor_control,
                    events,
                    format!("Failed to move workload into prepared cgroup: {err:#}"),
                );
            }
            self.append_event(&events.cgroup_applied())?;
        }

        self.store
            .update_run_status(&self.run_id, RunStatus::Running, None)?;

        if let Err(err) = supervisor_control.write_all(&[CHILD_START_BYTE]) {
            return self.abort_blocked_child(
                child,
                supervisor_control,
                events,
                format!("Failed to release isolated workload: {err}"),
            );
        }
        supervisor_control
            .shutdown(Shutdown::Write)
            .context("Failed to close supervisor control write side")?;

        let mut reader = BufReader::new(supervisor_control);
        let mut saw_namespace = false;
        let mut saw_isolation = false;
        let mut saw_process_start = false;
        let mut reported_failure: Option<String> = None;
        let mut protocol_error: Option<String> = None;
        let mut line = String::new();

        loop {
            line.clear();
            let read = match reader.read_line(&mut line) {
                Ok(read) => read,
                Err(err) => {
                    protocol_error = Some(format!("Failed reading child evidence channel: {err}"));
                    break;
                }
            };
            if read == 0 {
                break;
            }

            let notice: ChildNotice = match serde_json::from_str(line.trim_end()) {
                Ok(notice) => notice,
                Err(err) => {
                    protocol_error = Some(format!("Invalid child evidence message: {err}"));
                    break;
                }
            };

            match notice {
                ChildNotice::NamespaceReady { ids, namespace_pid } => {
                    if let Err(err) = validate_workload_namespace_boundary(
                        &supervisor_ns_before,
                        &ids,
                        self.seed.user.map_rootless,
                        self.seed.net.enable,
                        namespace_pid,
                    ) {
                        protocol_error = Some(format!("Namespace boundary verification failed: {err:#}"));
                        break;
                    }

                    let msg = format!(
                        "{}; namespace_pid={}; {}",
                        ns::workload_namespace_names(
                            self.seed.user.map_rootless,
                            self.seed.net.enable
                        ),
                        namespace_pid,
                        ids.summary()
                    );
                    self.append_event(&events.ns_created(&msg))?;
                    if self.seed.user.map_rootless {
                        self.append_event(&events.idmap_applied())?;
                    }
                    saw_namespace = true;
                }
                ChildNotice::IsolationReady => {
                    self.append_event(&events.mount_done("Mounts configured"))?;
                    if !self.seed.security.drop_caps.is_empty() {
                        self.append_event(&events.caps_dropped())?;
                    }
                    if self.seed.security.seccomp_profile.is_some() {
                        self.append_event(&events.seccomp_enabled())?;
                    }
                    saw_isolation = true;
                }
                ChildNotice::ProcessStart { namespace_pid } => {
                    if namespace_pid != 1 {
                        protocol_error = Some(format!(
                            "Workload process is not PID 1 in its PID namespace: observed {namespace_pid}"
                        ));
                        break;
                    }
                    self.append_event(&events.process_start(namespace_pid))?;
                    saw_process_start = true;
                }
                ChildNotice::Failed { error } => {
                    reported_failure = Some(error);
                    break;
                }
            }
        }

        if let Some(ref error) = protocol_error {
            tracing::error!(%error, child_pid = child.pid().as_raw(), "Failing closed on child evidence protocol error");
            child.terminate();
        }

        let wait_status = child.wait_terminal()?;
        let supervisor_ns_after = ns::current_namespace_ids()
            .context("Failed to capture supervisor namespace state after workload")?;

        let namespace_invariance_error = if supervisor_ns_after != supervisor_ns_before {
            Some(format!(
                "Host supervisor namespace IDs changed across workload run: before={:?} after={:?}",
                supervisor_ns_before, supervisor_ns_after
            ))
        } else {
            None
        };

        let result = if let Some(error) = namespace_invariance_error
            .or(protocol_error)
            .or(reported_failure)
        {
            self.record_failed_run(&events, &error)?;
            match wait_status {
                WaitStatus::Exited(_, code) => code,
                WaitStatus::Signaled(_, signal, _) => 128 + signal as i32,
                _ => CHILD_BOOTSTRAP_FAILURE as i32,
            }
        } else if !saw_namespace || !saw_isolation || !saw_process_start {
            let error = format!(
                "Child evidence incomplete before exec: namespace={} isolation={} process_start={}",
                saw_namespace, saw_isolation, saw_process_start
            );
            self.record_failed_run(&events, &error)?;
            CHILD_BOOTSTRAP_FAILURE as i32
        } else {
            match wait_status {
                WaitStatus::Exited(_, exit_code) => {
                    self.append_event(&events.process_exit(exit_code))?;
                    let end_ts = Utc::now().to_rfc3339();
                    self.store.update_run_status(
                        &self.run_id,
                        RunStatus::Exited(exit_code),
                        Some(&end_ts),
                    )?;
                    exit_code
                }
                WaitStatus::Signaled(_, signal, _) => {
                    let error = format!("Killed by signal: {signal:?}");
                    self.record_failed_run(&events, &error)?;
                    128 + signal as i32
                }
                other => {
                    let error = format!("Unexpected terminal wait status: {other:?}");
                    self.record_failed_run(&events, &error)?;
                    CHILD_BOOTSTRAP_FAILURE as i32
                }
            }
        };

        // `child` remains in scope through every Store write above. Its Drop
        // now only releases the clone stack (the child is already reaped) and
        // performs best-effort cgroup cleanup.
        drop(child);

        Ok(result)
    }

    fn child_exec_static(
        seed: &Seed,
        run_id: &str,
        control: &mut UnixStream,
    ) -> Result<()> {
        if seed.user.map_rootless {
            idmap::enter_mapped_identity(&seed.user)
                .context("Rootless child identity transition failed")?;
        }

        let namespace_pid = unistd::getpid().as_raw();
        let ids = ns::current_namespace_ids()
            .context("Failed to capture workload namespace identities")?;
        send_notice(
            control,
            &ChildNotice::NamespaceReady {
                ids,
                namespace_pid,
            },
        )?;

        let iso_config = IsolationConfig::new(seed, run_id.to_string());
        iso_config
            .apply_child()
            .context("Failed to apply child isolation")?;
        send_notice(control, &ChildNotice::IsolationReady)?;

        std::env::set_current_dir(&seed.entrypoint.cwd)
            .with_context(|| format!("Failed to chdir to {}", seed.entrypoint.cwd))?;

        for env_var in &seed.entrypoint.env {
            if let Some(eq_pos) = env_var.find('=') {
                let key = &env_var[..eq_pos];
                let value = &env_var[eq_pos + 1..];
                std::env::set_var(key, value);
            }
        }

        let program = CString::new(seed.entrypoint.cmd[0].as_str())
            .context("Invalid program path")?;
        let args: Result<Vec<CString>> = seed
            .entrypoint
            .cmd
            .iter()
            .map(|s| CString::new(s.as_str()).context("Invalid argument"))
            .collect();
        let args = args?;

        send_notice(
            control,
            &ChildNotice::ProcessStart {
                namespace_pid: unistd::getpid().as_raw(),
            },
        )?;

        // The control socket is FD_CLOEXEC. A successful exec therefore closes
        // the child-side audit channel; all persistent Store connectivity stays
        // with the host supervisor.
        unistd::execv(&program, &args).context("Failed to exec")?;
        unreachable!("exec should not return");
    }

    fn abort_blocked_child(
        self,
        mut child: ClonedWorkload,
        control: UnixStream,
        events: EventBuilder,
        error: String,
    ) -> Result<i32> {
        // Closing the authorization channel lets a still-blocked child observe
        // EOF. The guard is retained until waitpid has observed termination.
        drop(control);
        if child.wait_terminal().is_err() {
            child.terminate();
            let _ = child.wait_terminal();
        }

        self.record_failed_run(&events, &error)?;
        Ok(CHILD_BOOTSTRAP_FAILURE as i32)
    }

    fn record_failed_run(&self, events: &EventBuilder, error: &str) -> Result<()> {
        self.append_event(&events.process_failed(error))?;
        let end_ts = Utc::now().to_rfc3339();
        self.store.update_run_status(
            &self.run_id,
            RunStatus::Failed(error.to_string()),
            Some(&end_ts),
        )?;
        Ok(())
    }

    fn append_event(&self, event: &crate::events::Event) -> Result<()> {
        self.store.append_event(&self.run_id, &event.to_json()?)
    }

    fn store_seed_manifest(&self) -> Result<()> {
        let yaml = serde_yaml::to_string(&self.seed)
            .context("Failed to serialize seed YAML")?;

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

fn send_notice(control: &mut UnixStream, notice: &ChildNotice) -> Result<()> {
    serde_json::to_writer(&mut *control, notice).context("Failed to serialize child notice")?;
    control
        .write_all(b"\n")
        .context("Failed to write child notice delimiter")?;
    control.flush().context("Failed to flush child notice")?;
    Ok(())
}

fn set_cloexec(stream: &UnixStream) -> Result<()> {
    let fd = stream.as_raw_fd();
    let ret = unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFD, nix::libc::FD_CLOEXEC) };
    if ret == -1 {
        anyhow::bail!(
            "Failed to set FD_CLOEXEC on control socket: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn wait_for_terminal(child_pid: Pid) -> Result<WaitStatus> {
    loop {
        match waitpid(child_pid, None) {
            Ok(status @ WaitStatus::Exited(_, _)) | Ok(status @ WaitStatus::Signaled(_, _, _)) => {
                return Ok(status)
            }
            Ok(WaitStatus::Stopped(_, _))
            | Ok(WaitStatus::Continued(_))
            | Ok(WaitStatus::StillAlive) => continue,
            Ok(_) => continue,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(err) => anyhow::bail!("waitpid failed: {err}"),
        }
    }
}

fn validate_workload_namespace_boundary(
    supervisor: &ns::NamespaceIds,
    workload: &ns::NamespaceIds,
    map_rootless: bool,
    enable_net: bool,
    namespace_pid: i32,
) -> Result<()> {
    if namespace_pid != 1 {
        anyhow::bail!("expected workload PID 1, observed {namespace_pid}");
    }

    for (name, parent, child) in [
        ("pid", &supervisor.pid, &workload.pid),
        ("uts", &supervisor.uts, &workload.uts),
        ("ipc", &supervisor.ipc, &workload.ipc),
        ("mnt", &supervisor.mnt, &workload.mnt),
    ] {
        if parent == child {
            anyhow::bail!("workload did not enter a distinct {name} namespace: {child}");
        }
    }

    if map_rootless {
        if supervisor.user == workload.user {
            anyhow::bail!("rootless workload did not enter a distinct user namespace");
        }
    } else if supervisor.user != workload.user {
        anyhow::bail!("rootful workload unexpectedly changed user namespace");
    }

    if enable_net {
        if supervisor.net == workload.net {
            anyhow::bail!("networked workload did not enter a distinct network namespace");
        }
    } else if supervisor.net != workload.net {
        anyhow::bail!("network-disabled workload unexpectedly changed network namespace");
    }

    Ok(())
}
