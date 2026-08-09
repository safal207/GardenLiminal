use anyhow::{Context, Result};
use nix::unistd::Pid;
use std::fs;

use crate::seed::UserConfig;

/// Install a rootless UID/GID mapping for a blocked child from the host-side
/// supervisor.
///
/// The supervisor still has a stable host identity and can target
/// `/proc/<child>/uid_map` and `/proc/<child>/gid_map` before the child is
/// released to perform mounts or policy setup.
pub fn map_child_from_parent(
    child_pid: Pid,
    user_cfg: &UserConfig,
    parent_uid: u32,
    parent_gid: u32,
) -> Result<()> {
    let proc_dir = format!("/proc/{}", child_pid.as_raw());

    let setgroups = format!("{proc_dir}/setgroups");
    fs::write(&setgroups, "deny")
        .with_context(|| format!("Failed to deny setgroups for child {}", child_pid))?;

    let uid_map = format!("{} {} 1\n", user_cfg.uid, parent_uid);
    let uid_path = format!("{proc_dir}/uid_map");
    fs::write(&uid_path, &uid_map)
        .with_context(|| format!("Failed to write UID map for child {}", child_pid))?;

    let gid_map = format!("{} {} 1\n", user_cfg.gid, parent_gid);
    let gid_path = format!("{proc_dir}/gid_map");
    fs::write(&gid_path, &gid_map)
        .with_context(|| format!("Failed to write GID map for child {}", child_pid))?;

    tracing::debug!(
        child_pid = child_pid.as_raw(),
        container_uid = user_cfg.uid,
        container_gid = user_cfg.gid,
        parent_uid,
        parent_gid,
        "Installed child UID/GID map from host supervisor"
    );

    Ok(())
}

/// Switch the calling rootless child to the mapped container identity.
///
/// Creating a user namespace and installing uid_map/gid_map does not itself
/// change the task's current credentials to the mapped Seed UID/GID. The child
/// therefore performs this transition after the parent has released it and
/// before mounts or other workload policy are applied.
pub fn enter_mapped_identity(user_cfg: &UserConfig) -> Result<()> {
    nix::unistd::setgid(nix::unistd::Gid::from_raw(user_cfg.gid))
        .context("Failed to enter mapped GID")?;
    nix::unistd::setuid(nix::unistd::Uid::from_raw(user_cfg.uid))
        .context("Failed to enter mapped UID")?;

    verify_current_identity(user_cfg)
}

/// Verify that the calling task observes the configured container identity.
pub fn verify_current_identity(user_cfg: &UserConfig) -> Result<()> {
    let uid = nix::unistd::getuid().as_raw();
    let gid = nix::unistd::getgid().as_raw();

    if uid != user_cfg.uid || gid != user_cfg.gid {
        anyhow::bail!(
            "Mapped identity mismatch: expected {}:{}, observed {}:{}",
            user_cfg.uid,
            user_cfg.gid,
            uid,
            gid
        );
    }

    tracing::debug!(uid, gid, "Verified rootless mapped identity");
    Ok(())
}
