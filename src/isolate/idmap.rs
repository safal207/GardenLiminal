use anyhow::{Context, Result};
use nix::unistd;

use crate::seed::UserConfig;
use super::ns::{deny_setgroups, write_file};

/// Configure the one-entry UID/GID map immediately after entering a new user
/// namespace. The host IDs must be captured before the namespace transition.
pub fn configure_uid_gid_mapping(
    user_cfg: &UserConfig,
    host_uid: u32,
    host_gid: u32,
) -> Result<()> {
    let uid_map = uid_map_line(user_cfg.uid, host_uid);
    write_file("/proc/self/uid_map", &uid_map)
        .context("Failed to write uid_map")?;

    // Required before an unprivileged process may write gid_map.
    deny_setgroups().context("Failed to deny setgroups")?;
    let gid_map = gid_map_line(user_cfg.gid, host_gid);
    write_file("/proc/self/gid_map", &gid_map)
        .context("Failed to write gid_map")?;

    tracing::debug!(
        "Configured UID/GID map: namespace {}:{} -> host {}:{}",
        user_cfg.uid,
        user_cfg.gid,
        host_uid,
        host_gid
    );
    Ok(())
}

/// Enter the mapped workload identity after privileged namespace/mount setup is
/// complete but before no_new_privs, capability enforcement and seccomp.
pub fn enter_mapped_identity(user_cfg: &UserConfig) -> Result<()> {
    unistd::setgid(unistd::Gid::from_raw(user_cfg.gid))
        .context("Failed to set mapped gid")?;
    unistd::setuid(unistd::Uid::from_raw(user_cfg.uid))
        .context("Failed to set mapped uid")?;

    if unistd::getuid().as_raw() != user_cfg.uid || unistd::getgid().as_raw() != user_cfg.gid {
        anyhow::bail!(
            "Mapped identity post-condition failed: expected {}:{}, got {}:{}",
            user_cfg.uid,
            user_cfg.gid,
            unistd::getuid().as_raw(),
            unistd::getgid().as_raw()
        );
    }

    tracing::debug!("Entered mapped identity {}:{}", user_cfg.uid, user_cfg.gid);
    Ok(())
}

pub fn read_uid_map() -> Result<String> {
    std::fs::read_to_string("/proc/self/uid_map").context("Failed to read uid_map")
}

pub fn read_gid_map() -> Result<String> {
    std::fs::read_to_string("/proc/self/gid_map").context("Failed to read gid_map")
}

fn uid_map_line(container_uid: u32, host_uid: u32) -> String {
    format!("{} {} 1\n", container_uid, host_uid)
}

fn gid_map_line(container_gid: u32, host_gid: u32) -> String {
    format!("{} {} 1\n", container_gid, host_gid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_id_maps_are_explicit_and_bounded() {
        assert_eq!(uid_map_line(1000, 501), "1000 501 1\n");
        assert_eq!(gid_map_line(1000, 20), "1000 20 1\n");
    }
}
