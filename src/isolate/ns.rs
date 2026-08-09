use anyhow::{Context, Result};
use nix::sched::{unshare, CloneFlags};
use nix::unistd;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceSnapshot {
    pub user: String,
    pub pid: String,
    pub pid_for_children: String,
    pub mnt: String,
    pub uts: String,
    pub ipc: String,
    pub net: String,
}

/// Create namespaces for the workload bootstrap process.
///
/// `CLONE_NEWPID` affects subsequent children, so callers must fork once more
/// after this function returns to create PID 1 in the new PID namespace.
/// A user namespace is created only for the rootless mapping path; rootful
/// workloads keep the host user namespace while still receiving PID/mount/
/// UTS/IPC and optional network namespaces.
pub fn create_namespaces(enable_net: bool, enable_user: bool) -> Result<()> {
    let mut flags = CloneFlags::CLONE_NEWPID
        | CloneFlags::CLONE_NEWUTS
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWNS;

    if enable_user {
        flags |= CloneFlags::CLONE_NEWUSER;
    }
    if enable_net {
        flags |= CloneFlags::CLONE_NEWNET;
    }

    unshare(flags).context("Failed to unshare workload namespaces")?;

    tracing::debug!(
        "Prepared workload namespaces: pid, uts, ipc, mnt{}{}",
        if enable_user { ", user" } else { "" },
        if enable_net { ", net" } else { "" }
    );

    Ok(())
}

pub fn namespace_snapshot() -> Result<NamespaceSnapshot> {
    Ok(NamespaceSnapshot {
        user: namespace_id("user")?,
        pid: namespace_id("pid")?,
        pid_for_children: namespace_id("pid_for_children")?,
        mnt: namespace_id("mnt")?,
        uts: namespace_id("uts")?,
        ipc: namespace_id("ipc")?,
        net: namespace_id("net")?,
    })
}

pub fn namespace_id(kind: &str) -> Result<String> {
    fs::read_link(format!("/proc/thread-self/ns/{kind}"))
        .with_context(|| format!("Failed to read {kind} namespace id"))
        .map(|path| path.to_string_lossy().into_owned())
}

/// Set hostname in UTS namespace
pub fn set_hostname(hostname: &str) -> Result<()> {
    use std::ffi::CString;

    let hostname_c = CString::new(hostname).context("Invalid hostname")?;

    unsafe {
        let ret = nix::libc::sethostname(
            hostname_c.as_ptr() as *const nix::libc::c_char,
            hostname.len() as nix::libc::size_t,
        );
        if ret != 0 {
            anyhow::bail!("Failed to set hostname: {}", std::io::Error::last_os_error());
        }
    }

    tracing::debug!("Set hostname to: {}", hostname);

    Ok(())
}

/// Set no_new_privs to prevent privilege escalation.
pub fn set_no_new_privs() -> Result<()> {
    unsafe {
        let ret = nix::libc::prctl(
            nix::libc::PR_SET_NO_NEW_PRIVS,
            1 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
            0 as nix::libc::c_ulong,
        );
        if ret != 0 {
            anyhow::bail!("Failed to set no_new_privs: {}", std::io::Error::last_os_error());
        }
    }

    tracing::debug!("Set no_new_privs");

    Ok(())
}

/// Get current UID
pub fn get_uid() -> u32 {
    unistd::getuid().as_raw()
}

/// Get current GID
pub fn get_gid() -> u32 {
    unistd::getgid().as_raw()
}

/// Write to a file (helper for uid_map/gid_map)
pub fn write_file(path: &str, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("Failed to write to {}", path))?;
    Ok(())
}

/// Deny setgroups (required for rootless uid/gid mapping)
pub fn deny_setgroups() -> Result<()> {
    write_file("/proc/self/setgroups", "deny")
}

/// Enter an existing network namespace
pub fn setns_net(netns_path: &str) -> Result<()> {
    use std::os::unix::io::AsRawFd;

    let file = fs::File::open(netns_path)
        .with_context(|| format!("Failed to open netns: {}", netns_path))?;

    let fd = file.as_raw_fd();

    unsafe {
        let ret = nix::libc::setns(fd, nix::libc::CLONE_NEWNET as i32);
        if ret != 0 {
            anyhow::bail!("Failed to setns: {}", std::io::Error::last_os_error());
        }
    }

    tracing::debug!("Entered network namespace: {}", netns_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_snapshot_reports_all_expected_links() {
        let snapshot = namespace_snapshot().expect("namespace snapshot");
        assert!(snapshot.user.starts_with("user:["));
        assert!(snapshot.pid.starts_with("pid:["));
        assert!(snapshot.pid_for_children.starts_with("pid:["));
        assert!(snapshot.mnt.starts_with("mnt:["));
        assert!(snapshot.uts.starts_with("uts:["));
        assert!(snapshot.ipc.starts_with("ipc:["));
        assert!(snapshot.net.starts_with("net:["));
    }
}
