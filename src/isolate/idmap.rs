use anyhow::{Context, Result};
use nix::unistd;

use crate::seed::UserConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedIdMap {
    pub uid_map: String,
    pub gid_map: String,
}

/// Configure a bootstrap child's one-entry UID/GID map from its parent in the
/// parent user namespace.
///
/// This is the canonical rootless mapping boundary: the child first enters a
/// new user namespace, then blocks on the supervisor handshake. The host-side
/// supervisor writes and reads back `/proc/<pid>/{uid_map,gid_map}` before the
/// bootstrap is allowed to fork workload PID 1.
pub fn configure_child_uid_gid_mapping(
    child_pid: i32,
    user_cfg: &UserConfig,
    host_uid: u32,
    host_gid: u32,
) -> Result<AppliedIdMap> {
    if child_pid <= 0 {
        anyhow::bail!("Invalid bootstrap PID for idmap: {}", child_pid);
    }

    let proc_dir = format!("/proc/{child_pid}");
    let uid_map_path = format!("{proc_dir}/uid_map");
    let gid_map_path = format!("{proc_dir}/gid_map");
    let setgroups_path = format!("{proc_dir}/setgroups");

    let uid_map = uid_map_line(user_cfg.uid, host_uid);
    std::fs::write(&uid_map_path, &uid_map)
        .with_context(|| format!("Failed to write {uid_map_path}"))?;

    // Linux requires an unprivileged parent to disable setgroups before it may
    // write a gid map for the child user namespace.
    std::fs::write(&setgroups_path, "deny")
        .with_context(|| format!("Failed to write {setgroups_path}"))?;

    let gid_map = gid_map_line(user_cfg.gid, host_gid);
    std::fs::write(&gid_map_path, &gid_map)
        .with_context(|| format!("Failed to write {gid_map_path}"))?;

    let observed_uid = std::fs::read_to_string(&uid_map_path)
        .with_context(|| format!("Failed to read back {uid_map_path}"))?;
    let observed_gid = std::fs::read_to_string(&gid_map_path)
        .with_context(|| format!("Failed to read back {gid_map_path}"))?;

    verify_single_map("uid_map", &observed_uid, user_cfg.uid, host_uid)?;
    verify_single_map("gid_map", &observed_gid, user_cfg.gid, host_gid)?;

    tracing::debug!(
        child_pid,
        "Configured and verified rootless UID/GID map {}:{} -> host {}:{}",
        user_cfg.uid,
        user_cfg.gid,
        host_uid,
        host_gid
    );

    Ok(AppliedIdMap {
        uid_map: observed_uid.trim().to_string(),
        gid_map: observed_gid.trim().to_string(),
    })
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

fn verify_single_map(name: &str, observed: &str, inside: u32, outside: u32) -> Result<()> {
    let fields: Vec<&str> = observed.split_whitespace().collect();
    if fields.len() != 3
        || fields[0] != inside.to_string()
        || fields[1] != outside.to_string()
        || fields[2] != "1"
    {
        anyhow::bail!(
            "{} post-condition mismatch: expected '{} {} 1', got {:?}",
            name,
            inside,
            outside,
            observed.trim()
        );
    }
    Ok(())
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

    #[test]
    fn single_map_verifier_rejects_extra_ranges() {
        assert!(verify_single_map("uid_map", "1000 501 1\n1001 502 1\n", 1000, 501).is_err());
        assert!(verify_single_map("uid_map", "1000 501 1\n", 1000, 501).is_ok());
    }
}
