use anyhow::Result;

/// Apply the requested capability-drop policy.
///
/// GardenLiminal does not yet have a kernel-enforced capability implementation.
/// Returning `Ok(())` for a non-empty policy would create false security
/// evidence, because the caller may record `CAPS_DROPPED` even though no Linux
/// capability state changed. Until real enforcement and post-condition
/// verification are implemented, a requested drop policy must fail closed.
pub fn drop_capabilities(caps_to_drop: &[String]) -> Result<()> {
    if caps_to_drop.is_empty() {
        tracing::debug!("No capabilities requested for dropping");
        return Ok(());
    }

    anyhow::bail!(
        "Capability dropping was requested ({:?}) but kernel enforcement is not implemented; refusing to continue",
        caps_to_drop
    );
}

/// Get capability name to number mapping.
///
/// Kept as the basis for the future kernel-enforced implementation. It must not
/// be interpreted as evidence that capability dropping is active today.
#[allow(dead_code)]
fn cap_name_to_num(name: &str) -> Option<u32> {
    match name {
        "CAP_CHOWN" => Some(0),
        "CAP_DAC_OVERRIDE" => Some(1),
        "CAP_DAC_READ_SEARCH" => Some(2),
        "CAP_FOWNER" => Some(3),
        "CAP_FSETID" => Some(4),
        "CAP_KILL" => Some(5),
        "CAP_SETGID" => Some(6),
        "CAP_SETUID" => Some(7),
        "CAP_SETPCAP" => Some(8),
        "CAP_LINUX_IMMUTABLE" => Some(9),
        "CAP_NET_BIND_SERVICE" => Some(10),
        "CAP_NET_BROADCAST" => Some(11),
        "CAP_NET_ADMIN" => Some(12),
        "CAP_NET_RAW" => Some(13),
        "CAP_IPC_LOCK" => Some(14),
        "CAP_IPC_OWNER" => Some(15),
        "CAP_SYS_MODULE" => Some(16),
        "CAP_SYS_RAWIO" => Some(17),
        "CAP_SYS_CHROOT" => Some(18),
        "CAP_SYS_PTRACE" => Some(19),
        "CAP_SYS_PACCT" => Some(20),
        "CAP_SYS_ADMIN" => Some(21),
        "CAP_SYS_BOOT" => Some(22),
        "CAP_SYS_NICE" => Some(23),
        "CAP_SYS_RESOURCE" => Some(24),
        "CAP_SYS_TIME" => Some(25),
        "CAP_SYS_TTY_CONFIG" => Some(26),
        "CAP_MKNOD" => Some(27),
        "CAP_LEASE" => Some(28),
        "CAP_AUDIT_WRITE" => Some(29),
        "CAP_AUDIT_CONTROL" => Some(30),
        "CAP_SETFCAP" => Some(31),
        "CAP_MAC_OVERRIDE" => Some(32),
        "CAP_MAC_ADMIN" => Some(33),
        "CAP_SYSLOG" => Some(34),
        "CAP_WAKE_ALARM" => Some(35),
        "CAP_BLOCK_SUSPEND" => Some(36),
        "CAP_AUDIT_READ" => Some(37),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_is_a_noop() {
        assert!(drop_capabilities(&[]).is_ok());
    }

    #[test]
    fn requested_drop_fails_closed_until_enforced() {
        let requested = vec!["CAP_SYS_ADMIN".to_string()];
        let err = drop_capabilities(&requested).expect_err("unenforced capability policy must fail");
        let message = err.to_string();
        assert!(message.contains("not implemented"));
        assert!(message.contains("CAP_SYS_ADMIN"));
    }
}
