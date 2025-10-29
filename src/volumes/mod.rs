pub mod emptydir;
pub mod hostpath;
pub mod named;

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::seed::{VolumeMount, VolumeSpec, VolumeType};

/// Volume attachment result
pub struct AttachedVolume {
    pub name: String,
    pub source_path: PathBuf,
    pub mount_path: String,
    pub read_only: bool,
}

/// Attach a volume for a container
/// Returns the source path to bind-mount
pub fn attach_volume(
    volume_spec: &VolumeSpec,
    garden_id: &str,
    container_name: &str,
) -> Result<PathBuf> {
    match &volume_spec.volume_type {
        VolumeType::EmptyDir(empty_dir) => {
            emptydir::create_emptydir(
                &volume_spec.name,
                garden_id,
                container_name,
                empty_dir,
            )
        }
        VolumeType::HostPath(host_path) => {
            hostpath::validate_hostpath(&host_path.path)?;
            Ok(host_path.path.clone())
        }
        VolumeType::NamedVolume(named) => {
            named::ensure_named_volume(&named.name, named.size_limit.as_deref())
        }
        VolumeType::Config(config) => {
            // Materialize config to tmpfs
            let tmpfs_path = emptydir::create_tmpfs_for_config(
                &volume_spec.name,
                garden_id,
                container_name,
            )?;

            // Write config files
            for item in &config.items {
                let file_path = tmpfs_path.join(&item.path);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&file_path, &item.content)
                    .with_context(|| format!("Failed to write config file: {}", file_path.display()))?;

                // Set read-only permissions (0444)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(0o444);
                    std::fs::set_permissions(&file_path, perms)?;
                }
            }

            Ok(tmpfs_path)
        }
        VolumeType::Secret(_secret) => {
            // Create tmpfs for secret
            let tmpfs_path = emptydir::create_tmpfs_for_secret(
                &volume_spec.name,
                garden_id,
                container_name,
            )?;

            // Secret values would be written here
            // For MVP, just return the path
            // Full implementation in src/secrets/mod.rs

            Ok(tmpfs_path)
        }
    }
}

/// Detach and cleanup a volume
pub fn detach_volume(
    volume_spec: &VolumeSpec,
    garden_id: &str,
    container_name: &str,
) -> Result<()> {
    match &volume_spec.volume_type {
        VolumeType::EmptyDir(_) | VolumeType::Config(_) | VolumeType::Secret(_) => {
            emptydir::cleanup_emptydir(&volume_spec.name, garden_id, container_name)?;
        }
        VolumeType::HostPath(_) => {
            // Nothing to cleanup for hostPath
        }
        VolumeType::NamedVolume(_) => {
            // Named volumes persist, no cleanup
        }
    }

    Ok(())
}

/// Mount a volume inside container rootfs
pub fn mount_volume_in_container(
    source: &PathBuf,
    mount_point: &str,
    read_only: bool,
) -> Result<()> {
    use std::process::Command;

    // Create mount point if it doesn't exist
    let mount_path = std::path::Path::new(mount_point);
    if !mount_path.exists() {
        std::fs::create_dir_all(mount_path)
            .with_context(|| format!("Failed to create mount point: {}", mount_point))?;
    }

    // Bind mount
    let mut args = vec!["--bind"];
    if read_only {
        args.push("--read-only");
    }
    args.push(source.to_str().unwrap());
    args.push(mount_point);

    let output = Command::new("mount")
        .args(&args)
        .output()
        .context("Failed to execute mount command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Mount failed: {}", stderr);
    }

    tracing::debug!("Mounted {} -> {} (ro={})", source.display(), mount_point, read_only);

    Ok(())
}
