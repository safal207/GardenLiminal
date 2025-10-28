use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd;
use std::path::Path;

use super::IsolationConfig;

/// Setup mounts for the container
pub fn setup_mounts(config: &IsolationConfig) -> Result<()> {
    // Make current mount namespace private
    make_private_mounts()?;

    // Mount rootfs
    mount_rootfs(&config.seed.rootfs.path)?;

    // Change root to rootfs
    change_root(&config.seed.rootfs.path)?;

    // Mount /proc if requested
    for mount_cfg in &config.seed.mounts {
        match mount_cfg.mount_type.as_str() {
            "proc" => mount_proc(&mount_cfg.target)?,
            "tmpfs" => mount_tmpfs(&mount_cfg.target)?,
            "bind" => {
                if let Some(ref source) = mount_cfg.source {
                    mount_bind(source, &mount_cfg.target)?;
                }
            }
            _ => tracing::warn!("Unknown mount type: {}", mount_cfg.mount_type),
        }
    }

    Ok(())
}

/// Make all mounts private to avoid propagation
fn make_private_mounts() -> Result<()> {
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("Failed to make mounts private")?;

    tracing::debug!("Made mount namespace private");
    Ok(())
}

/// Bind mount rootfs
fn mount_rootfs(rootfs: &Path) -> Result<()> {
    // For MVP, we'll use bind mount + chroot
    // In production, pivot_root is preferred

    if !rootfs.exists() {
        anyhow::bail!("Rootfs path does not exist: {}", rootfs.display());
    }

    tracing::debug!("Rootfs ready at: {}", rootfs.display());
    Ok(())
}

/// Change root to new rootfs (using chroot for MVP)
fn change_root(rootfs: &Path) -> Result<()> {
    use std::ffi::CString;

    // Change directory to rootfs
    std::env::set_current_dir(rootfs)
        .with_context(|| format!("Failed to chdir to {}", rootfs.display()))?;

    // Chroot to rootfs using libc directly
    let rootfs_c = CString::new(rootfs.to_str().context("Invalid rootfs path")?)
        .context("Failed to convert rootfs to CString")?;

    unsafe {
        let ret = nix::libc::chroot(rootfs_c.as_ptr());
        if ret != 0 {
            anyhow::bail!("Failed to chroot: {}", std::io::Error::last_os_error());
        }
    }

    // Change to working directory
    std::env::set_current_dir("/")
        .context("Failed to chdir to / after chroot")?;

    tracing::debug!("Changed root to: {}", rootfs.display());

    Ok(())
}

/// Perform chroot to a rootfs directory
pub fn do_chroot(rootfs: &Path) -> Result<()> {
    use std::ffi::CString;

    // Change directory to rootfs
    std::env::set_current_dir(rootfs)
        .with_context(|| format!("Failed to chdir to {}", rootfs.display()))?;

    // Chroot to rootfs using libc directly
    let rootfs_c = CString::new(rootfs.to_str().context("Invalid rootfs path")?)
        .context("Failed to convert rootfs to CString")?;

    unsafe {
        let ret = nix::libc::chroot(rootfs_c.as_ptr());
        if ret != 0 {
            anyhow::bail!("Failed to chroot: {}", std::io::Error::last_os_error());
        }
    }

    // Change to root directory after chroot
    std::env::set_current_dir("/")
        .context("Failed to chdir to / after chroot")?;

    tracing::debug!("Chrooted to: {}", rootfs.display());

    Ok(())
}

/// Mount /proc filesystem
pub fn mount_proc(target: &str) -> Result<()> {
    let target_path = Path::new(target);

    // Create target directory if it doesn't exist
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

/// Mount tmpfs
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

/// Bind mount a directory
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
    .with_context(|| format!("Failed to bind mount {} to {}", source, target))?;

    tracing::debug!("Bind mounted {} to {}", source, target);

    Ok(())
}
