use anyhow::{Context, Result};
use nix::mount::{mount, MsFlags};
use std::fs;
use std::path::{Path, PathBuf};

use crate::seed::LayersSpec;

/// OverlayFS mount configuration
pub struct OverlayMount {
    pub lower_dirs: Vec<PathBuf>,
    pub upper_dir: PathBuf,
    pub work_dir: PathBuf,
    pub merged_dir: PathBuf,
}

impl OverlayMount {
    /// Create overlay mount configuration from layers spec
    pub fn from_layers(layers: &LayersSpec, container_name: &str) -> Result<Self> {
        // Convert string paths to PathBuf
        let lower_dirs: Vec<PathBuf> = layers
            .lower
            .iter()
            .map(|s| PathBuf::from(s))
            .collect();

        let upper_dir = PathBuf::from(&layers.upper);
        let work_dir = PathBuf::from(&layers.work);

        // Create merged dir in /tmp for this container
        let merged_dir = PathBuf::from(format!("/tmp/gl-merged-{}", container_name));

        Ok(Self {
            lower_dirs,
            upper_dir,
            work_dir,
            merged_dir,
        })
    }

    /// Prepare directories for overlay mount
    pub fn prepare(&self) -> Result<()> {
        // Create upper dir if it doesn't exist
        if !self.upper_dir.exists() {
            fs::create_dir_all(&self.upper_dir)
                .with_context(|| format!("Failed to create upper dir: {}", self.upper_dir.display()))?;
            tracing::debug!("Created upper dir: {}", self.upper_dir.display());
        }

        // Create work dir if it doesn't exist
        if !self.work_dir.exists() {
            fs::create_dir_all(&self.work_dir)
                .with_context(|| format!("Failed to create work dir: {}", self.work_dir.display()))?;
            tracing::debug!("Created work dir: {}", self.work_dir.display());
        }

        // Verify lower dirs exist
        for lower in &self.lower_dirs {
            if !lower.exists() {
                anyhow::bail!("Lower dir does not exist: {}", lower.display());
            }
        }

        // Create merged dir
        if !self.merged_dir.exists() {
            fs::create_dir_all(&self.merged_dir)
                .with_context(|| format!("Failed to create merged dir: {}", self.merged_dir.display()))?;
            tracing::debug!("Created merged dir: {}", self.merged_dir.display());
        }

        Ok(())
    }

    /// Mount the overlay filesystem
    pub fn mount(&self) -> Result<PathBuf> {
        // Prepare directories
        self.prepare()?;

        // Build overlay options string
        // Format: lowerdir=lower1:lower2,upperdir=upper,workdir=work
        let lowerdir = self
            .lower_dirs
            .iter()
            .map(|p| p.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(":");

        let upperdir = self.upper_dir.to_str().context("Invalid upper dir path")?;
        let workdir = self.work_dir.to_str().context("Invalid work dir path")?;

        let options = format!("lowerdir={},upperdir={},workdir={}", lowerdir, upperdir, workdir);

        tracing::debug!("Mounting overlay with options: {}", options);

        // Mount overlay
        mount(
            Some("overlay"),
            &self.merged_dir,
            Some("overlay"),
            MsFlags::empty(),
            Some(options.as_str()),
        )
        .with_context(|| format!("Failed to mount overlay at {}", self.merged_dir.display()))?;

        tracing::info!("OverlayFS mounted at: {}", self.merged_dir.display());

        Ok(self.merged_dir.clone())
    }

    /// Unmount the overlay filesystem
    pub fn unmount(&self) -> Result<()> {
        use nix::mount::umount;

        if self.merged_dir.exists() {
            umount(&self.merged_dir)
                .with_context(|| format!("Failed to unmount overlay at {}", self.merged_dir.display()))?;

            tracing::debug!("Unmounted overlay at: {}", self.merged_dir.display());

            // Remove merged dir
            fs::remove_dir(&self.merged_dir)
                .with_context(|| format!("Failed to remove merged dir: {}", self.merged_dir.display()))?;
        }

        Ok(())
    }
}

/// Prepare rootfs from either path or overlay layers
pub fn prepare_rootfs(
    container_name: &str,
    rootfs_config: &crate::seed::ContainerRootfsConfig,
) -> Result<PathBuf> {
    use crate::seed::ContainerRootfsConfig;

    match rootfs_config {
        ContainerRootfsConfig::Path { path } => {
            // Simple path-based rootfs
            if !path.exists() {
                anyhow::bail!("Rootfs path does not exist: {}", path.display());
            }
            Ok(path.clone())
        }
        ContainerRootfsConfig::Layers(layers_config) => {
            // OverlayFS-based rootfs
            let overlay = OverlayMount::from_layers(&layers_config.layers, container_name)?;
            overlay.mount()
        }
    }
}

/// Cleanup rootfs (unmount overlay if needed)
pub fn cleanup_rootfs(
    container_name: &str,
    rootfs_config: &crate::seed::ContainerRootfsConfig,
) -> Result<()> {
    use crate::seed::ContainerRootfsConfig;

    match rootfs_config {
        ContainerRootfsConfig::Path { .. } => {
            // Nothing to cleanup for path-based rootfs
            Ok(())
        }
        ContainerRootfsConfig::Layers(layers_config) => {
            // Unmount overlay
            let overlay = OverlayMount::from_layers(&layers_config.layers, container_name)?;
            overlay.unmount()
        }
    }
}
