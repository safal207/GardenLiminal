use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::seed::{EmptyDirVolume, parse_memory_string};

/// Base directory for emptyDir volumes
const EMPTYDIR_BASE: &str = "/var/lib/gl/state";

/// Create emptyDir volume (disk or tmpfs)
pub fn create_emptydir(
    volume_name: &str,
    garden_id: &str,
    container_name: &str,
    config: &EmptyDirVolume,
) -> Result<PathBuf> {
    let path = get_emptydir_path(volume_name, garden_id, container_name);

    // Create directory
    std::fs::create_dir_all(&path)
        .with_context(|| format!("Failed to create emptyDir: {}", path.display()))?;

    // Mount tmpfs if requested
    if config.medium == "tmpfs" {
        mount_tmpfs(&path, config.size_limit.as_deref())?;
    }

    tracing::debug!("Created emptyDir: {} (medium={})", path.display(), config.medium);

    Ok(path)
}

/// Create tmpfs for config volumes
pub fn create_tmpfs_for_config(
    volume_name: &str,
    garden_id: &str,
    container_name: &str,
) -> Result<PathBuf> {
    let path = get_emptydir_path(volume_name, garden_id, container_name);

    std::fs::create_dir_all(&path)?;
    mount_tmpfs(&path, Some("64Mi"))?; // Config usually small

    tracing::debug!("Created tmpfs for config: {}", path.display());

    Ok(path)
}

/// Create tmpfs for secrets
pub fn create_tmpfs_for_secret(
    volume_name: &str,
    garden_id: &str,
    container_name: &str,
) -> Result<PathBuf> {
    let path = get_emptydir_path(volume_name, garden_id, container_name);

    std::fs::create_dir_all(&path)?;
    mount_tmpfs(&path, Some("16Mi"))?; // Secrets should be small

    // Set strict permissions (0700)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&path, perms)?;
    }

    tracing::debug!("Created tmpfs for secret: {}", path.display());

    Ok(path)
}

/// Cleanup emptyDir volume
pub fn cleanup_emptydir(
    volume_name: &str,
    garden_id: &str,
    container_name: &str,
) -> Result<()> {
    let path = get_emptydir_path(volume_name, garden_id, container_name);

    if !path.exists() {
        return Ok(());
    }

    // Unmount if it's a tmpfs
    let _ = unmount_if_mounted(&path);

    // Remove directory
    std::fs::remove_dir_all(&path)
        .with_context(|| format!("Failed to remove emptyDir: {}", path.display()))?;

    tracing::debug!("Cleaned up emptyDir: {}", path.display());

    Ok(())
}

/// Get path for emptyDir
fn get_emptydir_path(
    volume_name: &str,
    garden_id: &str,
    container_name: &str,
) -> PathBuf {
    PathBuf::from(EMPTYDIR_BASE)
        .join(garden_id)
        .join(container_name)
        .join(format!("vol_{}", volume_name))
}

/// Mount tmpfs at path
fn mount_tmpfs(path: &PathBuf, size_limit: Option<&str>) -> Result<()> {
    // Prepare size option string (must be owned to avoid lifetime issues)
    let size_opt = if let Some(limit) = size_limit {
        let bytes = parse_memory_string(limit)?;
        format!("size={}", bytes)
    } else {
        "size=256M".to_string() // Default 256MB
    };

    let path_str = path.to_str().unwrap();
    let args = vec!["-t", "tmpfs", "-o", &size_opt, "tmpfs", path_str];

    let output = Command::new("mount")
        .args(&args)
        .output()
        .context("Failed to mount tmpfs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tmpfs mount failed: {}", stderr);
    }

    tracing::debug!("Mounted tmpfs at {}", path.display());

    Ok(())
}

/// Unmount if path is a mount point
fn unmount_if_mounted(path: &PathBuf) -> Result<()> {
    // Check if mounted
    let check = Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .status();

    if let Ok(status) = check {
        if status.success() {
            // Is a mount point, unmount it
            let output = Command::new("umount")
                .arg(path)
                .output()
                .context("Failed to unmount")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Unmount failed: {}", stderr);
            }

            tracing::debug!("Unmounted {}", path.display());
        }
    }

    Ok(())
}
