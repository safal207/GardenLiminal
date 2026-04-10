use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use std::path::Path;

use super::IsolationConfig;

/// Setup mounts for the container
pub fn setup_mounts(config: &IsolationConfig) -> Result<()> {
    // Make current mount namespace private to avoid propagation to host
    make_private_mounts()?;

    // Pivot root into the container's rootfs
    pivot_root_to(&config.seed.rootfs.path)?;

    // Mount additional filesystems requested in the manifest
    for mount_cfg in &config.seed.mounts {
        match mount_cfg.mount_type.as_str() {
            "proc"  => mount_proc(&mount_cfg.target)?,
            "tmpfs" => mount_tmpfs(&mount_cfg.target)?,
            "bind"  => {
                if let Some(ref source) = mount_cfg.source {
                    mount_bind(source, &mount_cfg.target)?;
                }
            }
            _ => tracing::warn!("Unknown mount type: {}", mount_cfg.mount_type),
        }
    }

    Ok(())
}

/// Make all mounts private to avoid propagation to the host namespace
fn make_private_mounts() -> Result<()> {
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("Failed to make mounts private")?;

    tracing::debug!("Mount namespace made private");
    Ok(())
}

/// Switch the root filesystem using pivot_root(2).
///
/// pivot_root is the correct way to change the root for a containerised
/// process — unlike chroot it actually moves the root mount point inside
/// the mount namespace, making it impossible to escape back to the host
/// filesystem via open("/proc/1/root") tricks.
///
/// Steps:
///   1. Bind-mount new_root onto itself so it becomes a separate mount point
///      (pivot_root requires new_root to be a mount, not just a directory).
///   2. Create a temporary directory inside new_root for the old root.
///   3. Call pivot_root(new_root, put_old).
///   4. chdir("/") — we are now inside the new root.
///   5. Lazy-unmount the old root so the container cannot see the host fs.
///   6. Remove the temporary directory.
pub fn pivot_root_to(new_root: &Path) -> Result<()> {
    if !new_root.exists() {
        anyhow::bail!("Rootfs path does not exist: {}", new_root.display());
    }

    // 1. Bind-mount new_root onto itself
    mount(
        Some(new_root),
        new_root,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .with_context(|| format!("Failed to bind-mount rootfs: {}", new_root.display()))?;

    tracing::debug!("Bind-mounted rootfs: {}", new_root.display());

    // 2. Create a temporary directory for the old root inside new_root
    let put_old = new_root.join(".old_root");
    std::fs::create_dir_all(&put_old)
        .with_context(|| format!("Failed to create put_old dir: {}", put_old.display()))?;

    // 3. pivot_root via raw syscall (nix 0.29 does not wrap this syscall)
    {
        use std::ffi::CString;
        let new_root_c = CString::new(new_root.to_str().context("Invalid new_root path")?)
            .context("Failed to convert new_root to CString")?;
        let put_old_c = CString::new(put_old.to_str().context("Invalid put_old path")?)
            .context("Failed to convert put_old to CString")?;
        let ret = unsafe {
            nix::libc::syscall(
                nix::libc::SYS_pivot_root,
                new_root_c.as_ptr(),
                put_old_c.as_ptr(),
            )
        };
        if ret != 0 {
            anyhow::bail!(
                "pivot_root({}, {}) failed: {}",
                new_root.display(),
                put_old.display(),
                std::io::Error::last_os_error()
            );
        }
    }

    tracing::debug!("pivot_root to: {}", new_root.display());

    // 4. chdir to new root
    std::env::set_current_dir("/").context("Failed to chdir to / after pivot_root")?;

    // 5. Lazy-unmount old root — the container can no longer reach the host filesystem
    umount2("/.old_root", MntFlags::MNT_DETACH)
        .context("Failed to unmount old root after pivot_root")?;

    // 6. Remove the now-empty put_old directory
    std::fs::remove_dir("/.old_root")
        .context("Failed to remove /.old_root after unmount")?;

    tracing::debug!("Old root unmounted and removed — container is fully isolated");

    Ok(())
}

/// Fallback chroot for contexts where pivot_root is not available
/// (e.g. pod containers that share the parent's mount namespace setup).
pub fn do_chroot(rootfs: &Path) -> Result<()> {
    use std::ffi::CString;

    std::env::set_current_dir(rootfs)
        .with_context(|| format!("Failed to chdir to {}", rootfs.display()))?;

    let rootfs_c = CString::new(rootfs.to_str().context("Invalid rootfs path")?)
        .context("Failed to convert rootfs to CString")?;

    let ret = unsafe { nix::libc::chroot(rootfs_c.as_ptr()) };
    if ret != 0 {
        anyhow::bail!("chroot failed: {}", std::io::Error::last_os_error());
    }

    std::env::set_current_dir("/").context("Failed to chdir to / after chroot")?;

    tracing::debug!("chroot to: {}", rootfs.display());
    Ok(())
}

/// Mount /proc filesystem (required for ps, top, /proc/self, etc.)
pub fn mount_proc(target: &str) -> Result<()> {
    let target_path = Path::new(target);

    if !target_path.exists() {
        std::fs::create_dir_all(target_path)
            .with_context(|| format!("Failed to create mount point: {}", target))?;
    }

    mount(
        Some("proc"),
        target,
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .with_context(|| format!("Failed to mount proc at {}", target))?;

    tracing::debug!("Mounted proc at: {}", target);
    Ok(())
}

/// Mount tmpfs at target
fn mount_tmpfs(target: &str) -> Result<()> {
    let target_path = Path::new(target);

    if !target_path.exists() {
        std::fs::create_dir_all(target_path)
            .with_context(|| format!("Failed to create mount point: {}", target))?;
    }

    mount(
        Some("tmpfs"),
        target,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        None::<&str>,
    )
    .with_context(|| format!("Failed to mount tmpfs at {}", target))?;

    tracing::debug!("Mounted tmpfs at: {}", target);
    Ok(())
}

/// Bind mount source to target
fn mount_bind(source: &str, target: &str) -> Result<()> {
    let target_path = Path::new(target);

    if !target_path.exists() {
        std::fs::create_dir_all(target_path)
            .with_context(|| format!("Failed to create mount point: {}", target))?;
    }

    mount(
        Some(source),
        target,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )
    .with_context(|| format!("Failed to bind mount {} → {}", source, target))?;

    tracing::debug!("Bind mounted {} → {}", source, target);
    Ok(())
}
