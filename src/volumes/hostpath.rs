use anyhow::{Context, Result};
use std::path::Path;

/// Validate that hostPath exists and is accessible
pub fn validate_hostpath(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("hostPath does not exist: {}", path.display());
    }

    // Check if readable
    std::fs::metadata(path)
        .with_context(|| format!("Cannot access hostPath: {}", path.display()))?;

    tracing::debug!("Validated hostPath: {}", path.display());

    Ok(())
}
