use anyhow::{bail, Context, Result};
use std::io;

const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
const CAPABILITY_WORDS: usize = 2;
const PR_CAP_AMBIENT_VALUE: nix::libc::c_int = 47;
const PR_CAP_AMBIENT_CLEAR_ALL_VALUE: nix::libc::c_ulong = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Drop the capabilities requested by the Seed and verify the kernel state.
///
/// The previous MVP implementation only logged what it would drop while the
/// runtime emitted `CAPS_DROPPED`. That produced false security evidence. This
/// implementation removes every requested capability from:
///
/// - the process bounding set;
/// - effective capabilities;
/// - permitted capabilities;
/// - inheritable capabilities;
/// - the ambient set.
///
/// `no_new_privs` is applied by the caller before this function, so a later
/// exec cannot use setuid or file capabilities to regain what was removed.
pub fn drop_capabilities(caps_to_drop: &[String]) -> Result<()> {
    if caps_to_drop.is_empty() {
        tracing::debug!("No capabilities requested for dropping");
        return Ok(());
    }

    let capabilities = caps_to_drop
        .iter()
        .map(|name| {
            cap_name_to_num(name)
                .with_context(|| format!("Unknown capability requested for drop: {name}"))
        })
        .collect::<Result<Vec<_>>>()?;

    clear_ambient_capabilities()?;

    // Remove capabilities from the bounding set while CAP_SETPCAP is still
    // available. This prevents a later exec from reconstructing them.
    for capability in &capabilities {
        let result = unsafe {
            nix::libc::prctl(
                nix::libc::PR_CAPBSET_DROP,
                *capability as nix::libc::c_ulong,
                0,
                0,
                0,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error()).with_context(|| {
                format!("Failed to drop capability {capability} from bounding set")
            });
        }
    }

    let mut header = CapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [CapData::default(); CAPABILITY_WORDS];
    capget(&mut header, &mut data)?;

    for capability in &capabilities {
        clear_capability(&mut data, *capability)?;
    }

    capset(&header, &data)?;

    // Re-read and verify before the caller is allowed to emit CAPS_DROPPED.
    let mut observed = [CapData::default(); CAPABILITY_WORDS];
    capget(&mut header, &mut observed)?;
    for capability in &capabilities {
        if capability_is_present(&observed, *capability)? {
            bail!("Capability {capability} remained present after capset");
        }
    }

    tracing::debug!(
        "Dropped and verified capabilities: {:?}",
        caps_to_drop
    );
    Ok(())
}

fn clear_ambient_capabilities() -> Result<()> {
    let result = unsafe {
        nix::libc::prctl(
            PR_CAP_AMBIENT_VALUE,
            PR_CAP_AMBIENT_CLEAR_ALL_VALUE,
            0,
            0,
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error())
            .context("Failed to clear ambient capabilities");
    }
    Ok(())
}

fn capget(header: &mut CapHeader, data: &mut [CapData; CAPABILITY_WORDS]) -> Result<()> {
    let result = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_capget,
            header as *mut CapHeader,
            data.as_mut_ptr(),
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error()).context("capget failed");
    }
    Ok(())
}

fn capset(header: &CapHeader, data: &[CapData; CAPABILITY_WORDS]) -> Result<()> {
    let result = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_capset,
            header as *const CapHeader,
            data.as_ptr(),
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error()).context("capset failed");
    }
    Ok(())
}

fn clear_capability(data: &mut [CapData; CAPABILITY_WORDS], capability: u32) -> Result<()> {
    let (index, mask) = capability_position(capability)?;
    data[index].effective &= !mask;
    data[index].permitted &= !mask;
    data[index].inheritable &= !mask;
    Ok(())
}

fn capability_is_present(data: &[CapData; CAPABILITY_WORDS], capability: u32) -> Result<bool> {
    let (index, mask) = capability_position(capability)?;
    Ok((data[index].effective | data[index].permitted | data[index].inheritable) & mask != 0)
}

fn capability_position(capability: u32) -> Result<(usize, u32)> {
    let index = (capability / 32) as usize;
    if index >= CAPABILITY_WORDS {
        bail!("Capability number {capability} exceeds supported Linux v3 range");
    }
    Ok((index, 1_u32 << (capability % 32)))
}

/// Get capability name to number mapping.
///
/// Seeds historically used both `SYS_ADMIN` and `CAP_SYS_ADMIN`; accept both
/// forms but never silently ignore an unknown name.
fn cap_name_to_num(name: &str) -> Result<u32> {
    let normalized = name.strip_prefix("CAP_").unwrap_or(name);
    let number = match normalized {
        "CHOWN" => 0,
        "DAC_OVERRIDE" => 1,
        "DAC_READ_SEARCH" => 2,
        "FOWNER" => 3,
        "FSETID" => 4,
        "KILL" => 5,
        "SETGID" => 6,
        "SETUID" => 7,
        "SETPCAP" => 8,
        "LINUX_IMMUTABLE" => 9,
        "NET_BIND_SERVICE" => 10,
        "NET_BROADCAST" => 11,
        "NET_ADMIN" => 12,
        "NET_RAW" => 13,
        "IPC_LOCK" => 14,
        "IPC_OWNER" => 15,
        "SYS_MODULE" => 16,
        "SYS_RAWIO" => 17,
        "SYS_CHROOT" => 18,
        "SYS_PTRACE" => 19,
        "SYS_PACCT" => 20,
        "SYS_ADMIN" => 21,
        "SYS_BOOT" => 22,
        "SYS_NICE" => 23,
        "SYS_RESOURCE" => 24,
        "SYS_TIME" => 25,
        "SYS_TTY_CONFIG" => 26,
        "MKNOD" => 27,
        "LEASE" => 28,
        "AUDIT_WRITE" => 29,
        "AUDIT_CONTROL" => 30,
        "SETFCAP" => 31,
        "MAC_OVERRIDE" => 32,
        "MAC_ADMIN" => 33,
        "SYSLOG" => 34,
        "WAKE_ALARM" => 35,
        "BLOCK_SUSPEND" => 36,
        "AUDIT_READ" => 37,
        _ => bail!("Unsupported capability name: {name}"),
    };
    Ok(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_seed_and_kernel_capability_names() {
        assert_eq!(cap_name_to_num("NET_ADMIN").unwrap(), 12);
        assert_eq!(cap_name_to_num("CAP_NET_ADMIN").unwrap(), 12);
        assert_eq!(cap_name_to_num("SYS_ADMIN").unwrap(), 21);
        assert_eq!(cap_name_to_num("CAP_SYS_ADMIN").unwrap(), 21);
    }

    #[test]
    fn rejects_unknown_capability_names() {
        assert!(cap_name_to_num("CAP_NOT_REAL").is_err());
    }

    #[test]
    fn clears_effective_permitted_and_inheritable_bits() {
        let mut data = [
            CapData {
                effective: u32::MAX,
                permitted: u32::MAX,
                inheritable: u32::MAX,
            },
            CapData {
                effective: u32::MAX,
                permitted: u32::MAX,
                inheritable: u32::MAX,
            },
        ];
        clear_capability(&mut data, 12).unwrap();
        assert!(!capability_is_present(&data, 12).unwrap());
        assert!(capability_is_present(&data, 21).unwrap());
    }

    #[test]
    fn handles_second_capability_word() {
        let mut data = [CapData::default(); CAPABILITY_WORDS];
        data[1].effective = 1 << (37 - 32);
        assert!(capability_is_present(&data, 37).unwrap());
        clear_capability(&mut data, 37).unwrap();
        assert!(!capability_is_present(&data, 37).unwrap());
    }
}
