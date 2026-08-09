use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::IsolationConfig;
use crate::seed::parse_memory_string;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Prepare the cgroup and limits for a Seed workload.
///
/// This deliberately does not move the calling process. The host supervisor
/// creates/configures the cgroup, then moves the namespace-bootstrap child into
/// it after `fork()`. Descendants inherit that cgroup while the supervisor
/// remains outside the workload resource boundary.
pub fn setup_cgroups(config: &IsolationConfig) -> Result<()> {
    let cgroup_path = get_cgroup_path(&config.seed.meta.id)?;
    create_cgroup(&cgroup_path)?;

    if let Some(shares) = config.seed.limits.cpu.shares {
        set_cpu_weight(&cgroup_path, shares)?;
    }

    if let Some(ref max) = config.seed.limits.memory.max {
        let bytes = parse_memory_string(max)
            .with_context(|| format!("Failed to parse memory limit: {}", max))?;
        set_memory_max(&cgroup_path, bytes)?;
    }

    if let Some(max) = config.seed.limits.pids.max {
        set_pids_max(&cgroup_path, max)?;
    }

    tracing::debug!("Prepared workload cgroup at: {}", cgroup_path.display());
    Ok(())
}

pub fn move_seed_pid_to_cgroup(seed_id: &str, pid: i32) -> Result<()> {
    let path = get_cgroup_path(seed_id)?;
    move_pid_to_path(&path, pid)
}

fn get_cgroup_path(seed_id: &str) -> Result<PathBuf> {
    Ok(Path::new(CGROUP_ROOT).join("gl").join(seed_id))
}

fn create_cgroup(path: &Path) -> Result<()> {
    let parent = path.parent().context("No parent for cgroup path")?;
    if !parent.exists() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cgroup parent: {}", parent.display()))?;
    }

    if !path.exists() {
        fs::create_dir(path)
            .with_context(|| format!("Failed to create cgroup: {}", path.display()))?;
    }

    tracing::debug!("Created cgroup at: {}", path.display());
    Ok(())
}

fn set_cpu_weight(cgroup_path: &Path, shares: u64) -> Result<()> {
    let cpu_weight_path = cgroup_path.join("cpu.weight");
    let weight = ((shares * 10000) / 1024).clamp(1, 10000);
    fs::write(&cpu_weight_path, format!("{}\n", weight))
        .with_context(|| format!("Failed to set cpu.weight to {}", weight))?;
    tracing::debug!("Set CPU weight to: {}", weight);
    Ok(())
}

fn set_memory_max(cgroup_path: &Path, bytes: u64) -> Result<()> {
    let memory_max_path = cgroup_path.join("memory.max");
    fs::write(&memory_max_path, format!("{}\n", bytes))
        .with_context(|| format!("Failed to set memory.max to {}", bytes))?;
    tracing::debug!("Set memory.max to: {} bytes", bytes);
    Ok(())
}

fn set_pids_max(cgroup_path: &Path, max: u64) -> Result<()> {
    let pids_max_path = cgroup_path.join("pids.max");
    fs::write(&pids_max_path, format!("{}\n", max))
        .with_context(|| format!("Failed to set pids.max to {}", max))?;
    tracing::debug!("Set pids.max to: {}", max);
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

/// Setup cgroup for a container in a garden (pod)
/// Creates cgroup at: /sys/fs/cgroup/garden/<garden_id>/<container_name>
pub fn setup_cgroup_for_container(
    garden_id: &str,
    container_name: &str,
    limits: &crate::seed::LimitsConfig,
) -> Result<()> {
    let cgroup_path = get_container_cgroup_path(garden_id, container_name)?;
    create_cgroup(&cgroup_path)?;

    if let Some(shares) = limits.cpu.shares {
        set_cpu_weight(&cgroup_path, shares)?;
    }

    if let Some(ref max) = limits.memory.max {
        let bytes = parse_memory_string(max)
            .with_context(|| format!("Failed to parse memory limit: {}", max))?;
        set_memory_max(&cgroup_path, bytes)?;
    }

    if let Some(max) = limits.pids.max {
        set_pids_max(&cgroup_path, max)?;
    }

    tracing::debug!("Applied container cgroups at: {}", cgroup_path.display());
    Ok(())
}

fn get_container_cgroup_path(garden_id: &str, container_name: &str) -> Result<PathBuf> {
    Ok(Path::new(CGROUP_ROOT)
        .join("garden")
        .join(garden_id)
        .join(container_name))
}

/// Move a PID into a cgroup
pub fn move_pid_to_cgroup(cgroup_path: &str, pid: i32) -> Result<()> {
    move_pid_to_path(Path::new(cgroup_path), pid)
}

fn move_pid_to_path(cgroup_path: &Path, pid: i32) -> Result<()> {
    let procs_path = cgroup_path.join("cgroup.procs");
    fs::write(&procs_path, format!("{}\n", pid))
        .with_context(|| format!("Failed to add PID {} to cgroup {}", pid, cgroup_path.display()))?;
    tracing::debug!("Moved PID {} to cgroup {}", pid, cgroup_path.display());
    Ok(())
}
