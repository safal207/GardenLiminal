use anyhow::{Context, Result};
use nix::sched::{unshare, CloneFlags};
use nix::unistd;
use std::fs;

fn namespace_flags(enable_net: bool, enable_user: bool) -> CloneFlags {
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

    flags
}

/// Create namespaces for isolation.
///
/// Rootful workloads do not enter a new user namespace. They already execute
/// with the privileges required to configure the remaining namespaces, and an
/// unmapped rootful user namespace turns UID 0 into an overflow identity for
/// host-backed root filesystems. Rootless workloads opt into CLONE_NEWUSER and
/// must apply their UID/GID mapping before touching the rootfs.
pub fn create_namespaces(enable_net: bool, enable_user: bool) -> Result<()> {
    let flags = namespace_flags(enable_net, enable_user);
    unshare(flags).context("Failed to unshare namespaces")?;

    tracing::debug!(
        "Created namespaces: {}pid, uts, ipc, mnt{}",
        if enable_user { "user, " } else { "" },
        if enable_net { ", net" } else { "" }
    );

    Ok(())
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

/// Set no_new_privs to prevent privilege escalation
pub fn set_no_new_privs() -> Result<()> {
    unsafe {
        let ret = nix::libc::prctl(nix::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
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
    fn rootful_mode_omits_user_namespace() {
        let flags = namespace_flags(false, false);
        assert!(!flags.contains(CloneFlags::CLONE_NEWUSER));
        assert!(flags.contains(CloneFlags::CLONE_NEWPID));
        assert!(flags.contains(CloneFlags::CLONE_NEWUTS));
        assert!(flags.contains(CloneFlags::CLONE_NEWIPC));
        assert!(flags.contains(CloneFlags::CLONE_NEWNS));
        assert!(!flags.contains(CloneFlags::CLONE_NEWNET));
    }

    #[test]
    fn rootless_and_network_modes_are_explicit() {
        let flags = namespace_flags(true, true);
        assert!(flags.contains(CloneFlags::CLONE_NEWUSER));
        assert!(flags.contains(CloneFlags::CLONE_NEWNET));
    }
}
