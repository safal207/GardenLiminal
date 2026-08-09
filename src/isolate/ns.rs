use anyhow::{Context, Result};
use nix::sched::CloneFlags;
use nix::unistd;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceIds {
    pub user: String,
    pub pid: String,
    pub uts: String,
    pub ipc: String,
    pub mnt: String,
    pub net: String,
}

impl NamespaceIds {
    pub fn summary(&self) -> String {
        format!(
            "user={} pid={} uts={} ipc={} mnt={} net={}",
            self.user, self.pid, self.uts, self.ipc, self.mnt, self.net
        )
    }
}

/// Namespace flags used when creating the isolated workload task.
///
/// PID/UTS/IPC/mount namespaces are always isolated. A network namespace is
/// optional. A user namespace is created only for rootless Seeds; rootful
/// Seeds retain the caller's user namespace instead of entering an unmapped
/// user namespace.
pub fn workload_clone_flags(map_rootless: bool, enable_net: bool) -> CloneFlags {
    let mut flags = CloneFlags::CLONE_NEWPID
        | CloneFlags::CLONE_NEWUTS
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWNS;

    if map_rootless {
        flags |= CloneFlags::CLONE_NEWUSER;
    }
    if enable_net {
        flags |= CloneFlags::CLONE_NEWNET;
    }

    flags
}

pub fn workload_namespace_names(map_rootless: bool, enable_net: bool) -> String {
    let mut names = Vec::new();
    if map_rootless {
        names.push("user");
    }
    names.extend(["pid", "uts", "ipc", "mnt"]);
    if enable_net {
        names.push("net");
    }
    names.join(", ")
}

/// Read namespace identities for the calling task.
///
/// `/proc/thread-self` is deliberate: namespace membership can be task-local,
/// so evidence must describe the task making the observation rather than a
/// different thread-group member.
pub fn current_namespace_ids() -> Result<NamespaceIds> {
    Ok(NamespaceIds {
        user: namespace_link("user")?,
        pid: namespace_link("pid")?,
        uts: namespace_link("uts")?,
        ipc: namespace_link("ipc")?,
        mnt: namespace_link("mnt")?,
        net: namespace_link("net")?,
    })
}

fn namespace_link(name: &str) -> Result<String> {
    let path = format!("/proc/thread-self/ns/{name}");
    let target = fs::read_link(&path)
        .with_context(|| format!("Failed to read namespace link {path}"))?;
    Ok(target.to_string_lossy().into_owned())
}

/// Set hostname in UTS namespace.
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

/// Get current UID.
pub fn get_uid() -> u32 {
    unistd::getuid().as_raw()
}

/// Get current GID.
pub fn get_gid() -> u32 {
    unistd::getgid().as_raw()
}

/// Write to a file (helper for uid_map/gid_map).
pub fn write_file(path: &str, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("Failed to write to {}", path))?;
    Ok(())
}

/// Enter an existing network namespace.
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
    fn rootful_workload_does_not_request_user_namespace() {
        let flags = workload_clone_flags(false, false);
        assert!(!flags.contains(CloneFlags::CLONE_NEWUSER));
        assert!(flags.contains(CloneFlags::CLONE_NEWPID));
        assert!(flags.contains(CloneFlags::CLONE_NEWUTS));
        assert!(flags.contains(CloneFlags::CLONE_NEWIPC));
        assert!(flags.contains(CloneFlags::CLONE_NEWNS));
        assert!(!flags.contains(CloneFlags::CLONE_NEWNET));
    }

    #[test]
    fn rootless_networked_workload_requests_expected_namespaces() {
        let flags = workload_clone_flags(true, true);
        assert!(flags.contains(CloneFlags::CLONE_NEWUSER));
        assert!(flags.contains(CloneFlags::CLONE_NEWPID));
        assert!(flags.contains(CloneFlags::CLONE_NEWUTS));
        assert!(flags.contains(CloneFlags::CLONE_NEWIPC));
        assert!(flags.contains(CloneFlags::CLONE_NEWNS));
        assert!(flags.contains(CloneFlags::CLONE_NEWNET));
    }
}
