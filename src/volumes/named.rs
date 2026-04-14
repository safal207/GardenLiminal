use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::seed::parse_memory_string;

/// Base directory for named volumes
const NAMED_VOLUMES_BASE: &str = "/var/lib/gl/volumes";

/// Ensure named volume exists
/// Named volumes persist across pod restarts
pub fn ensure_named_volume(name: &str, size_limit: Option<&str>) -> Result<PathBuf> {
    ensure_named_volume_in(Path::new(NAMED_VOLUMES_BASE), name, size_limit)
}

/// Ensure named volume exists under a specific base directory.
/// This keeps tests and non-root environments off the system volume path.
pub fn ensure_named_volume_in(
    base_dir: &Path,
    name: &str,
    size_limit: Option<&str>,
) -> Result<PathBuf> {
    let path = get_named_volume_path_in(base_dir, name);

    if path.exists() {
        tracing::debug!("Using existing named volume: {}", path.display());
        return Ok(path);
    }

    // Create directory
    std::fs::create_dir_all(&path)
        .with_context(|| format!("Failed to create named volume: {}", path.display()))?;

    // Apply size limit via quota (if specified and supported)
    if let Some(limit) = size_limit {
        let _bytes = parse_memory_string(limit)?;
        // TODO: Apply quota with setquota or similar
        // For MVP, just log the limit
        tracing::debug!("Named volume {} size limit: {}", name, limit);
    }

    tracing::info!("Created named volume: {} at {}", name, path.display());

    Ok(path)
}

/// Delete a named volume
pub fn delete_named_volume(name: &str) -> Result<()> {
    delete_named_volume_in(Path::new(NAMED_VOLUMES_BASE), name)
}

/// Delete a named volume under a specific base directory.
pub fn delete_named_volume_in(base_dir: &Path, name: &str) -> Result<()> {
    let path = get_named_volume_path_in(base_dir, name);

    if !path.exists() {
        tracing::warn!("Named volume does not exist: {}", name);
        return Ok(());
    }

    std::fs::remove_dir_all(&path)
        .with_context(|| format!("Failed to delete named volume: {}", path.display()))?;

    tracing::info!("Deleted named volume: {}", name);

    Ok(())
}

/// List all named volumes
pub fn list_named_volumes() -> Result<Vec<String>> {
    list_named_volumes_in(Path::new(NAMED_VOLUMES_BASE))
}

/// List named volumes under a specific base directory.
pub fn list_named_volumes_in(base_dir: &Path) -> Result<Vec<String>> {
    let base = base_dir.to_path_buf();

    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut volumes = Vec::new();

    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                volumes.push(name.to_string());
            }
        }
    }

    Ok(volumes)
}

/// Get path for named volume
fn get_named_volume_path(name: &str) -> PathBuf {
    get_named_volume_path_in(Path::new(NAMED_VOLUMES_BASE), name)
}

fn get_named_volume_path_in(base_dir: &Path, name: &str) -> PathBuf {
    base_dir.join(name)
}
