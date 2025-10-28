use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::store::cas::CAS;

/// OCI Image Index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageIndex {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,

    pub manifests: Vec<ImageManifestDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManifestDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,

    pub digest: String,

    pub size: u64,

    #[serde(default)]
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
}

/// OCI Image Manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,

    pub config: Descriptor,

    pub layers: Vec<Descriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,

    pub digest: String,

    pub size: u64,
}

/// OCI Image Config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageConfig {
    pub architecture: String,
    pub os: String,

    #[serde(default)]
    pub config: Option<ContainerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    #[serde(rename = "Env", default)]
    pub env: Vec<String>,

    #[serde(rename = "Cmd", default)]
    pub cmd: Vec<String>,

    #[serde(rename = "WorkingDir", default)]
    pub working_dir: String,
}

/// OCI Image Manager
pub struct OCIManager {
    store_path: PathBuf,
    cas: CAS,
}

impl OCIManager {
    pub fn new(store_path: PathBuf) -> Result<Self> {
        // Create store directory if it doesn't exist
        if !store_path.exists() {
            fs::create_dir_all(&store_path)
                .with_context(|| format!("Failed to create OCI store: {}", store_path.display()))?;
        }

        Ok(Self {
            store_path,
            cas: CAS::new(),
        })
    }

    /// Import an OCI image from a tar archive or directory
    pub fn import(&mut self, source: &Path) -> Result<String> {
        tracing::info!("Importing OCI image from: {}", source.display());

        // Check if source is a directory or tar
        if source.is_dir() {
            self.import_directory(source)
        } else {
            self.import_tar(source)
        }
    }

    /// Import from OCI layout directory
    fn import_directory(&mut self, dir: &Path) -> Result<String> {
        // Read index.json
        let index_path = dir.join("index.json");
        let index_content = fs::read_to_string(&index_path)
            .with_context(|| format!("Failed to read index.json from {}", dir.display()))?;

        let index: ImageIndex = serde_json::from_str(&index_content)
            .context("Failed to parse index.json")?;

        // For MVP, just take the first manifest
        let manifest_desc = index
            .manifests
            .first()
            .context("No manifests found in index")?;

        let manifest_digest = &manifest_desc.digest;

        tracing::info!("Found manifest: {}", manifest_digest);

        // Read manifest
        let manifest_blob_path = self.blob_path_from_digest(dir, manifest_digest);
        let manifest_content = fs::read_to_string(&manifest_blob_path)
            .with_context(|| format!("Failed to read manifest blob: {}", manifest_blob_path.display()))?;

        let manifest: ImageManifest = serde_json::from_str(&manifest_content)
            .context("Failed to parse manifest")?;

        // Register layers in CAS
        for layer in &manifest.layers {
            let layer_blob_path = self.blob_path_from_digest(dir, &layer.digest);

            if layer_blob_path.exists() {
                self.cas.register(&layer.digest, layer_blob_path)?;
                tracing::debug!("Registered layer: {}", layer.digest);
            } else {
                tracing::warn!("Layer blob not found: {}", layer_blob_path.display());
            }
        }

        Ok(manifest_digest.clone())
    }

    /// Import from tar archive
    fn import_tar(&mut self, _tar_path: &Path) -> Result<String> {
        // TODO: Extract tar and call import_directory
        // For MVP, just error
        anyhow::bail!("TAR import not yet implemented (use OCI layout directory)")
    }

    /// Get blob path from digest
    fn blob_path_from_digest(&self, base: &Path, digest: &str) -> PathBuf {
        // Format: blobs/sha256/<hash>
        if let Some(hash) = digest.strip_prefix("sha256:") {
            base.join("blobs").join("sha256").join(hash)
        } else {
            base.join("blobs").join(digest)
        }
    }

    /// Unpack layers for a manifest
    pub fn unpack(&self, manifest_digest: &str) -> Result<Vec<PathBuf>> {
        tracing::info!("Unpacking manifest: {}", manifest_digest);

        // For MVP, just return registered layer paths from CAS
        // In production, would extract tarballs to separate directories

        let layers = vec![]; // TODO: Get from CAS

        Ok(layers)
    }

    /// Get CAS reference
    pub fn cas(&self) -> &CAS {
        &self.cas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_oci_manager_creation() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let manager = OCIManager::new(temp_dir.path().to_path_buf())?;

        assert!(temp_dir.path().exists());

        Ok(())
    }
}
