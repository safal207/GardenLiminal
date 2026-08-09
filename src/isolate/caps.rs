use anyhow::{Context, Result};
use std::collections::BTreeSet;

// Linux capability ABI v3 supports 64 capability bits via two u32 words.
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
const CAP_WORDS: usize = 2;

// Stable Linux prctl(2) UAPI operation numbers from <linux/prctl.h>.
const PR_CAPBSET_READ_OP: i32 = 23;
const PR_CAPBSET_DROP_OP: i32 = 24;
const PR_CAP_AMBIENT_OP: i32 = 47;
const PR_CAP_AMBIENT_IS_SET_OP: i32 = 1;
const PR_CAP_AMBIENT_LOWER_OP: i32 = 3;

const CAP_SETPCAP: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct UserCapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct UserCapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapabilitySpec {
    canonical: String,
    number: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CapabilityState {
    effective: bool,
    permitted: bool,
    inheritable: bool,
    bounding: bool,
    ambient: bool,
}

/// Apply and verify the requested Linux capability-drop policy.
///
/// For every requested capability, GardenLiminal removes the bit from the
/// effective, permitted, inheritable, bounding and ambient sets. The function
/// verifies kernel-visible post-state before returning success. Any parsing,
/// syscall or verification failure aborts workload bootstrap.
///
/// Both canonical names (`CAP_SYS_ADMIN`) and the historical manifest form
/// (`SYS_ADMIN`) are accepted. Unknown names fail closed.
pub fn drop_capabilities(caps_to_drop: &[String]) -> Result<()> {
    if caps_to_drop.is_empty() {
        tracing::debug!("No capabilities requested for dropping");
        return Ok(());
    }

    // Parse and validate the entire policy before making any irreversible
    // capability changes.
    let last_cap = read_cap_last_cap()?;
    let mut requested = BTreeSet::new();
    for raw in caps_to_drop {
        let spec = parse_capability(raw)?;
        if spec.number > last_cap {
            anyhow::bail!(
                "Capability {} ({}) is not supported by this kernel (cap_last_cap={})",
                spec.canonical,
                spec.number,
                last_cap
            );
        }
        requested.insert(spec.number);
    }

    let mut data = capget_current()?;

    // PR_CAPBSET_DROP requires CAP_SETPCAP in the caller's effective set.
    // Preflight this before any irreversible bounding-set mutation so a
    // partially applied policy is not produced just because the caller lacks
    // authority to finish the requested transition.
    let needs_bounding_drop = requested
        .iter()
        .copied()
        .map(read_bounding)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .any(|present| present);

    if needs_bounding_drop && !has_effective(&data, CAP_SETPCAP) {
        anyhow::bail!(
            "Capability policy requires bounding-set changes but CAP_SETPCAP is not effective; refusing partial enforcement"
        );
    }

    for cap in requested.iter().copied() {
        let before = read_state(&data, cap)?;
        tracing::debug!(capability = cap, ?before, "Capability state before drop");
    }

    // Bounding first: this prevents the requested capability from being
    // regained through file capabilities across a later execve().
    for cap in requested.iter().copied() {
        if read_bounding(cap)? {
            drop_bounding(cap)
                .with_context(|| format!("Failed to drop capability {} from bounding set", cap))?;
        }
    }

    // Ambient is explicit even though lowering permitted/inheritable also
    // forces ambient bits down. This makes the transition intentional and
    // independently verifiable.
    for cap in requested.iter().copied() {
        if read_ambient(cap)? {
            lower_ambient(cap)
                .with_context(|| format!("Failed to lower ambient capability {}", cap))?;
        }
    }

    // Update effective/permitted/inheritable in one capset(2) transition.
    for cap in requested.iter().copied() {
        clear_capability_bits(&mut data, cap)?;
    }
    capset_current(&data)?;

    // Re-read all kernel-facing sets. `CAPS_DROPPED` evidence must only be
    // emitted by the caller after this post-condition succeeds.
    let verified_data = capget_current()?;
    for cap in requested.iter().copied() {
        let after = read_state(&verified_data, cap)?;
        tracing::debug!(capability = cap, ?after, "Capability state after drop");
        if after.effective
            || after.permitted
            || after.inheritable
            || after.bounding
            || after.ambient
        {
            anyhow::bail!(
                "Capability {} remained present after enforcement: {:?}",
                cap,
                after
            );
        }
    }

    tracing::info!(
        capabilities = ?requested,
        "Capability-drop policy enforced and kernel post-state verified"
    );
    Ok(())
}

fn parse_capability(raw: &str) -> Result<CapabilitySpec> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Capability name cannot be empty");
    }

    let upper = trimmed.to_ascii_uppercase();
    let canonical = if upper.starts_with("CAP_") {
        upper
    } else {
        format!("CAP_{}", upper)
    };

    let number = cap_name_to_num(&canonical)
        .with_context(|| format!("Unknown capability name: {}", raw))?;

    Ok(CapabilitySpec { canonical, number })
}

fn read_cap_last_cap() -> Result<u32> {
    let raw = std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .context("Failed to read /proc/sys/kernel/cap_last_cap")?;
    raw.trim()
        .parse::<u32>()
        .context("Invalid /proc/sys/kernel/cap_last_cap value")
}

fn capget_current() -> Result<[UserCapData; CAP_WORDS]> {
    let mut header = UserCapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [UserCapData::default(); CAP_WORDS];

    let ret = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_capget,
            &mut header as *mut UserCapHeader,
            data.as_mut_ptr(),
        )
    };

    if ret != 0 {
        anyhow::bail!("capget failed: {}", std::io::Error::last_os_error());
    }
    if header.version != LINUX_CAPABILITY_VERSION_3 {
        anyhow::bail!(
            "Kernel returned unexpected capability ABI version: 0x{:x}",
            header.version
        );
    }

    Ok(data)
}

fn capset_current(data: &[UserCapData; CAP_WORDS]) -> Result<()> {
    let mut header = UserCapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };

    let ret = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_capset,
            &mut header as *mut UserCapHeader,
            data.as_ptr(),
        )
    };

    if ret != 0 {
        anyhow::bail!("capset failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn read_bounding(cap: u32) -> Result<bool> {
    let ret = unsafe {
        nix::libc::prctl(
            PR_CAPBSET_READ_OP,
            cap as nix::libc::c_ulong,
            0,
            0,
            0,
        )
    };
    match ret {
        0 => Ok(false),
        1 => Ok(true),
        _ => anyhow::bail!(
            "PR_CAPBSET_READ({}) failed: {}",
            cap,
            std::io::Error::last_os_error()
        ),
    }
}

fn drop_bounding(cap: u32) -> Result<()> {
    let ret = unsafe {
        nix::libc::prctl(
            PR_CAPBSET_DROP_OP,
            cap as nix::libc::c_ulong,
            0,
            0,
            0,
        )
    };
    if ret != 0 {
        anyhow::bail!(
            "PR_CAPBSET_DROP({}) failed: {}",
            cap,
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn read_ambient(cap: u32) -> Result<bool> {
    let ret = unsafe {
        nix::libc::prctl(
            PR_CAP_AMBIENT_OP,
            PR_CAP_AMBIENT_IS_SET_OP as nix::libc::c_ulong,
            cap as nix::libc::c_ulong,
            0,
            0,
        )
    };
    match ret {
        0 => Ok(false),
        1 => Ok(true),
        _ => anyhow::bail!(
            "PR_CAP_AMBIENT_IS_SET({}) failed: {}",
            cap,
            std::io::Error::last_os_error()
        ),
    }
}

fn lower_ambient(cap: u32) -> Result<()> {
    let ret = unsafe {
        nix::libc::prctl(
            PR_CAP_AMBIENT_OP,
            PR_CAP_AMBIENT_LOWER_OP as nix::libc::c_ulong,
            cap as nix::libc::c_ulong,
            0,
            0,
        )
    };
    if ret != 0 {
        anyhow::bail!(
            "PR_CAP_AMBIENT_LOWER({}) failed: {}",
            cap,
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn read_state(data: &[UserCapData; CAP_WORDS], cap: u32) -> Result<CapabilityState> {
    Ok(CapabilityState {
        effective: has_set_bit(data, cap, |word| word.effective)?,
        permitted: has_set_bit(data, cap, |word| word.permitted)?,
        inheritable: has_set_bit(data, cap, |word| word.inheritable)?,
        bounding: read_bounding(cap)?,
        ambient: read_ambient(cap)?,
    })
}

fn has_effective(data: &[UserCapData; CAP_WORDS], cap: u32) -> bool {
    has_set_bit(data, cap, |word| word.effective).unwrap_or(false)
}

fn has_set_bit<F>(
    data: &[UserCapData; CAP_WORDS],
    cap: u32,
    selector: F,
) -> Result<bool>
where
    F: Fn(&UserCapData) -> u32,
{
    let (word, mask) = bit_position(cap)?;
    Ok(selector(&data[word]) & mask != 0)
}

fn clear_capability_bits(data: &mut [UserCapData; CAP_WORDS], cap: u32) -> Result<()> {
    let (word, mask) = bit_position(cap)?;
    data[word].effective &= !mask;
    data[word].permitted &= !mask;
    data[word].inheritable &= !mask;
    Ok(())
}

fn bit_position(cap: u32) -> Result<(usize, u32)> {
    let word = (cap / 32) as usize;
    if word >= CAP_WORDS {
        anyhow::bail!("Capability {} exceeds supported capability ABI width", cap);
    }
    Ok((word, 1u32 << (cap % 32)))
}

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
        "CAP_PERFMON" => Some(38),
        "CAP_BPF" => Some(39),
        "CAP_CHECKPOINT_RESTORE" => Some(40),
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
    fn accepts_canonical_and_historical_names() {
        assert_eq!(parse_capability("CAP_SYS_ADMIN").unwrap().number, 21);
        assert_eq!(parse_capability("sys_admin").unwrap().number, 21);
        assert_eq!(parse_capability(" NET_ADMIN ").unwrap().number, 12);
        assert_eq!(parse_capability("cap_bpf").unwrap().number, 39);
    }

    #[test]
    fn unknown_capability_fails_closed() {
        let err = parse_capability("CAP_NOT_REAL").unwrap_err();
        assert!(err.to_string().contains("Unknown capability"));
    }

    #[test]
    fn empty_capability_name_fails_closed() {
        assert!(parse_capability("  ").is_err());
    }

    #[test]
    fn clear_bits_removes_only_requested_capability() {
        let mut data = [
            UserCapData {
                effective: u32::MAX,
                permitted: u32::MAX,
                inheritable: u32::MAX,
            },
            UserCapData {
                effective: u32::MAX,
                permitted: u32::MAX,
                inheritable: u32::MAX,
            },
        ];

        clear_capability_bits(&mut data, 21).unwrap();
        let mask = 1u32 << 21;
        assert_eq!(data[0].effective & mask, 0);
        assert_eq!(data[0].permitted & mask, 0);
        assert_eq!(data[0].inheritable & mask, 0);
        assert_eq!(data[0].effective | mask, u32::MAX);
        assert_eq!(data[1].effective, u32::MAX);

        clear_capability_bits(&mut data, 39).unwrap();
        let word1_mask = 1u32 << (39 - 32);
        assert_eq!(data[1].effective & word1_mask, 0);
        assert_eq!(data[1].permitted & word1_mask, 0);
        assert_eq!(data[1].inheritable & word1_mask, 0);
    }

    #[test]
    fn bit_position_rejects_out_of_abi_range() {
        assert!(bit_position(64).is_err());
    }

    /// Run only in an isolated privileged Linux process. The test permanently
    /// lowers the process's CAP_NET_RAW state, so normal `cargo test` keeps it
    /// ignored. CI invokes this test in a dedicated `sudo cargo test` process.
    #[test]
    #[ignore = "requires isolated privileged Linux capability environment"]
    fn privileged_kernel_drop_is_verified() {
        const CAP_NET_RAW: u32 = 13;

        let before_data = capget_current().expect("capget before drop");
        assert!(
            has_effective(&before_data, CAP_SETPCAP),
            "privileged fixture requires effective CAP_SETPCAP"
        );
        let before = read_state(&before_data, CAP_NET_RAW).expect("state before drop");
        assert!(before.bounding, "fixture expects CAP_NET_RAW in bounding set");
        assert!(before.effective, "fixture expects CAP_NET_RAW effective");
        assert!(before.permitted, "fixture expects CAP_NET_RAW permitted");

        drop_capabilities(&["CAP_NET_RAW".to_string()])
            .expect("kernel capability drop should succeed");

        let after_data = capget_current().expect("capget after drop");
        let after = read_state(&after_data, CAP_NET_RAW).expect("state after drop");
        assert_eq!(
            after,
            CapabilityState {
                effective: false,
                permitted: false,
                inheritable: false,
                bounding: false,
                ambient: false,
            }
        );
    }
}
