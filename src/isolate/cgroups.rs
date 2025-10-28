use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::IsolationConfig;
use crate::seed::parse_memory_string;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Setup cgroups v2 for the container
pub fn setup_cgroups(config: &IsolationConfig) -> Result<()> {
    let cgroup_path = get_cgroup_path(&config.seed.meta.id)?;

    // Create cgroup directory
    create_cgroup(&cgroup_path)?;

    // Apply CPU limits
    if let Some(shares) = config.seed.limits.cpu.shares {
        set_cpu_weight(&cgroup_path, shares)?;
    }

    // Apply memory limits
    if let Some(ref max) = config.seed.limits.memory.max {
        let bytes = parse_memory_string(max)
            .with_context(|| format!("Failed to parse memory limit: {}", max))?;
        set_memory_max(&cgroup_path, bytes)?;
    }

    // Apply PID limits
    if let Some(max) = config.seed.limits.pids.max {
        set_pids_max(&cgroup_path, max)?;
    }

    // Add current process to cgroup
    add_process_to_cgroup(&cgroup_path)?;

    tracing::debug!("Applied cgroups at: {}", cgroup_path.display());

    Ok(())
}

/// Get cgroup path for seed
fn get_cgroup_path(seed_id: &str) -> Result<PathBuf> {
    // Use a dedicated subtree for gl containers
    // Format: /sys/fs/cgroup/gl/<seed_id>
    let path = Path::new(CGROUP_ROOT).join("gl").join(seed_id);
    Ok(path)
}

/// Create cgroup directory
fn create_cgroup(path: &Path) -> Result<()> {
    // Ensure parent gl directory exists
    let parent = path.parent().context("No parent for cgroup path")?;
    if !parent.exists() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cgroup parent: {}", parent.display()))?;
    }

    // Create seed-specific cgroup
    if !path.exists() {
        fs::create_dir(path)
            .with_context(|| format!("Failed to create cgroup: {}", path.display()))?;
    }

    tracing::debug!("Created cgroup at: {}", path.display());

    Ok(())
}

/// Set CPU weight (shares)
/// cgroups v2 uses cpu.weight (range 1-10000, default 100)
fn set_cpu_weight(cgroup_path: &Path, shares: u64) -> Result<()> {
    let cpu_weight_path = cgroup_path.join("cpu.weight");

    // Convert shares to weight (shares are typically 0-1024, weight is 1-10000)
    let weight = (shares * 10000) / 1024;
    let weight = weight.max(1).min(10000);

    fs::write(&cpu_weight_path, format!("{}\n", weight))
        .with_context(|| format!("Failed to set cpu.weight to {}", weight))?;

    tracing::debug!("Set CPU weight to: {}", weight);

    Ok(())
}

/// Set memory limit
fn set_memory_max(cgroup_path: &Path, bytes: u64) -> Result<()> {
    let memory_max_path = cgroup_path.join("memory.max");

    fs::write(&memory_max_path, format!("{}\n", bytes))
        .with_context(|| format!("Failed to set memory.max to {}", bytes))?;

    tracing::debug!("Set memory.max to: {} bytes", bytes);

    Ok(())
}

/// Set PIDs limit
fn set_pids_max(cgroup_path: &Path, max: u64) -> Result<()> {
    let pids_max_path = cgroup_path.join("pids.max");

    fs::write(&pids_max_path, format!("{}\n", max))
        .with_context(|| format!("Failed to set pids.max to {}", max))?;

    tracing::debug!("Set pids.max to: {}", max);

    Ok(())
}

/// Add current process to cgroup
fn add_process_to_cgroup(cgroup_path: &Path) -> Result<()> {
    let procs_path = cgroup_path.join("cgroup.procs");
    let pid = std::process::id();

    fs::write(&procs_path, format!("{}\n", pid))
        .with_context(|| format!("Failed to add PID {} to cgroup", pid))?;

    tracing::debug!("Added PID {} to cgroup", pid);

    Ok(())
}

/// Cleanup cgroup (call after process exits)
pub fn cleanup_cgroup(seed_id: &str) -> Result<()> {
    let cgroup_path = get_cgroup_path(seed_id)?;

    if cgroup_path.exists() {
        fs::remove_dir(&cgroup_path)
            .with_context(|| format!("Failed to remove cgroup: {}", cgroup_path.display()))?;

        tracing::debug!("Cleaned up cgroup: {}", cgroup_path.display());
    }

    Ok(())
}
