use anyhow::{Context, Result};
use nix::unistd;

use crate::seed::UserConfig;
use super::ns::{deny_setgroups, write_file, get_uid, get_gid};

/// Apply UID/GID mapping for rootless containers
pub fn apply_uid_gid_mapping(user_cfg: &UserConfig) -> Result<()> {
    // Get parent UID/GID (before entering namespace)
    let parent_uid = get_uid();
    let parent_gid = get_gid();

    // Deny setgroups (required for unprivileged user namespace)
    deny_setgroups().context("Failed to deny setgroups")?;

    // Map UID: container uid -> parent uid
    let uid_map = format!("{} {} 1\n", user_cfg.uid, parent_uid);
    write_file("/proc/self/uid_map", &uid_map)
        .context("Failed to write uid_map")?;

    // Map GID: container gid -> parent gid
    let gid_map = format!("{} {} 1\n", user_cfg.gid, parent_gid);
    write_file("/proc/self/gid_map", &gid_map)
        .context("Failed to write gid_map")?;

    // Set UID/GID in the container
    unistd::setgid(unistd::Gid::from_raw(user_cfg.gid))
        .context("Failed to setgid")?;

    unistd::setuid(unistd::Uid::from_raw(user_cfg.uid))
        .context("Failed to setuid")?;

    tracing::debug!("Applied UID/GID mapping: {}:{} -> {}:{}", user_cfg.uid, user_cfg.gid, parent_uid, parent_gid);

    Ok(())
}
