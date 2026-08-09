use gl::isolate::{idmap, ns};
use gl::seed::UserConfig;
use nix::sched::clone;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{self, Pid};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

const STACK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct Probe {
    namespace_pid: i32,
    uid: u32,
    gid: u32,
    ids: ns::NamespaceIds,
    fresh_tcp_reached_host_listener: bool,
}

/// Keep clone's backing stack alive even if a privileged assertion panics.
/// Dropping this guard terminates and reaps an unreaped child before the stack
/// memory can be released.
struct TestClonedChild {
    pid: Pid,
    _stack: Vec<u8>,
    reaped: bool,
}

impl TestClonedChild {
    fn new(pid: Pid, stack: Vec<u8>) -> Self {
        Self {
            pid,
            _stack: stack,
            reaped: false,
        }
    }

    fn pid(&self) -> Pid {
        self.pid
    }

    fn wait_terminal(&mut self) -> Result<WaitStatus, nix::errno::Errno> {
        loop {
            match waitpid(self.pid, None) {
                Ok(status @ WaitStatus::Exited(_, _))
                | Ok(status @ WaitStatus::Signaled(_, _, _)) => {
                    self.reaped = true;
                    return Ok(status);
                }
                Ok(_) => continue,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(err) => return Err(err),
            }
        }
    }
}

impl Drop for TestClonedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }

        match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
                self.reaped = true;
            }
            Err(nix::errno::Errno::ECHILD) => {
                self.reaped = true;
            }
            Ok(_) | Err(_) => {
                let _ = kill(self.pid, Signal::SIGKILL);
                let _ = self.wait_terminal();
            }
        }
    }
}

#[test]
#[ignore = "requires isolated privileged Linux namespace environment"]
fn privileged_supervisor_stays_outside_workload_namespaces() {
    assert!(
        unistd::geteuid().is_root(),
        "fixture must run as root in an isolated CI process"
    );

    let before = ns::current_namespace_ids().expect("supervisor namespace state before");
    let parent_uid = ns::get_uid();
    let parent_gid = ns::get_gid();
    let user = UserConfig {
        uid: 1000,
        gid: 1000,
        map_rootless: true,
    };

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind host-side audit probe listener");
    let listener_addr = listener.local_addr().expect("listener address");
    let listener_fd = listener.as_raw_fd();

    let (mut parent_control, mut child_control) =
        UnixStream::pair().expect("create namespace test control socket");
    set_cloexec(&parent_control);
    set_cloexec(&child_control);
    let parent_peer_fd = parent_control.as_raw_fd();

    let mut stack = vec![0u8; STACK_BYTES];
    let flags = ns::workload_clone_flags(true, true);
    let user_for_child = user.clone();

    let child_pid = unsafe {
        clone(
            Box::new(move || {
                nix::libc::close(parent_peer_fd);
                nix::libc::close(listener_fd);

                let mut gate = [0u8; 1];
                if child_control.read_exact(&mut gate).is_err() || gate[0] != b'G' {
                    return 125;
                }

                if let Err(err) = idmap::enter_mapped_identity(&user_for_child) {
                    eprintln!("mapped identity transition failed: {err:#}");
                    return 125;
                }

                let ids = match ns::current_namespace_ids() {
                    Ok(ids) => ids,
                    Err(err) => {
                        eprintln!("namespace read failed: {err:#}");
                        return 125;
                    }
                };

                let fresh_tcp_reached_host_listener = TcpStream::connect(listener_addr).is_ok();
                let probe = Probe {
                    namespace_pid: unistd::getpid().as_raw(),
                    uid: unistd::getuid().as_raw(),
                    gid: unistd::getgid().as_raw(),
                    ids,
                    fresh_tcp_reached_host_listener,
                };

                if serde_json::to_writer(&mut child_control, &probe).is_err()
                    || child_control.write_all(b"\n").is_err()
                {
                    return 125;
                }
                0
            }),
            &mut stack,
            flags,
            Some(Signal::SIGCHLD as i32),
        )
    }
    .expect("clone namespaced probe child");

    let mut child = TestClonedChild::new(child_pid, stack);

    idmap::map_child_from_parent(child.pid(), &user, parent_uid, parent_gid)
        .expect("install rootless child mapping from supervisor");

    // The host supervisor remains in its original network namespace and can
    // still create a fresh TCP connection to the host-side listener.
    assert!(
        TcpStream::connect(listener_addr).is_ok(),
        "host-side supervisor lost host network connectivity"
    );

    parent_control.write_all(b"G").expect("release child");
    parent_control.flush().expect("flush child release");

    let mut line = String::new();
    let mut reader = BufReader::new(parent_control);
    reader.read_line(&mut line).expect("read child probe");
    let probe: Probe = serde_json::from_str(line.trim()).expect("parse child probe");

    let status = child.wait_terminal().expect("wait child");
    assert!(matches!(status, WaitStatus::Exited(_, 0)), "child status: {status:?}");

    let after = ns::current_namespace_ids().expect("supervisor namespace state after");
    println!("--- supervisor namespaces before ---\n{:?}", before);
    println!("--- workload probe ---\n{:?}", probe);
    println!("--- supervisor namespaces after ---\n{:?}", after);

    assert_eq!(before, after, "supervisor namespace IDs changed");
    assert_eq!(probe.namespace_pid, 1, "workload must be PID 1");
    assert_eq!(probe.uid, user.uid, "rootless UID mapping mismatch");
    assert_eq!(probe.gid, user.gid, "rootless GID mapping mismatch");

    assert_ne!(probe.ids.user, before.user, "user namespace not isolated");
    assert_ne!(probe.ids.pid, before.pid, "PID namespace not isolated");
    assert_ne!(probe.ids.uts, before.uts, "UTS namespace not isolated");
    assert_ne!(probe.ids.ipc, before.ipc, "IPC namespace not isolated");
    assert_ne!(probe.ids.mnt, before.mnt, "mount namespace not isolated");
    assert_ne!(probe.ids.net, before.net, "network namespace not isolated");

    assert!(
        !probe.fresh_tcp_reached_host_listener,
        "fresh workload TCP unexpectedly reached host-network listener"
    );
}

fn set_cloexec(stream: &UnixStream) {
    let ret = unsafe {
        nix::libc::fcntl(
            stream.as_raw_fd(),
            nix::libc::F_SETFD,
            nix::libc::FD_CLOEXEC,
        )
    };
    assert_ne!(ret, -1, "set FD_CLOEXEC: {}", std::io::Error::last_os_error());
}
