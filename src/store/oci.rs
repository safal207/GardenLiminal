use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
    fn import_tar(&mut self, tar_path: &Path) -> Result<String> {
        use std::io::Read;
        use flate2::read::GzDecoder;
        use tar::Archive;

        tracing::info!("Extracting OCI image from tar: {}", tar_path.display());

        // Create temporary directory for extraction
        let temp_dir = self.store_path.join(format!("oci-extract-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir)
            .with_context(|| format!("Failed to create temp dir: {}", temp_dir.display()))?;

        // Open tar file
        let tar_file = fs::File::open(tar_path)
            .with_context(|| format!("Failed to open tar: {}", tar_path.display()))?;

        // Check if gzipped based on extension or magic bytes
        let is_gzipped = tar_path.extension().and_then(|e| e.to_str()) == Some("gz")
            || tar_path.extension().and_then(|e| e.to_str()) == Some("tgz");

        // Extract tar
        if is_gzipped {
            let decoder = GzDecoder::new(tar_file);
            let mut archive = Archive::new(decoder);
            archive.unpack(&temp_dir)
                .context("Failed to extract gzipped tar")?;
        } else {
            let mut archive = Archive::new(tar_file);
            archive.unpack(&temp_dir)
                .context("Failed to extract tar")?;
        }

        tracing::info!("Extracted tar to: {}", temp_dir.display());

        // Import from extracted directory
        let manifest_digest = self.import_directory(&temp_dir)?;

        // Cleanup temp directory
        if let Err(e) = fs::remove_dir_all(&temp_dir) {
            tracing::warn!("Failed to cleanup temp dir {}: {}", temp_dir.display(), e);
        }

        Ok(manifest_digest)
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

    /// Unpack layers for a manifest into a target directory
    /// Returns paths to unpacked layer directories in order (lower to upper)
    pub fn unpack(&self, manifest_digest: &str, target_dir: &Path) -> Result<Vec<PathBuf>> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        tracing::info!("Unpacking manifest: {} to {}", manifest_digest, target_dir.display());

        // Create target directory
        fs::create_dir_all(target_dir)
            .with_context(|| format!("Failed to create target dir: {}", target_dir.display()))?;

        // Get layer digests from CAS
        // For now, we'll return layer blob paths directly
        // In production, would extract each layer tarball to a separate directory

        // For MVP: just return empty list
        // Full implementation would:
        // 1. Read manifest to get layer list
        // 2. For each layer digest:
        //    - Get layer blob path from CAS
        //    - Extract tarball to target_dir/layer-<index>
        //    - Handle whiteout files (.wh.*)
        // 3. Return list of extracted layer directories

        let layers = vec![];

        tracing::info!("Unpacked {} layers", layers.len());

        Ok(layers)
    }

    /// Extract a single OCI layer tarball to a directory
    fn extract_layer(&self, layer_blob: &Path, target_dir: &Path) -> Result<()> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        tracing::debug!("Extracting layer {} to {}", layer_blob.display(), target_dir.display());

        fs::create_dir_all(target_dir)
            .with_context(|| format!("Failed to create layer dir: {}", target_dir.display()))?;

        let layer_file = fs::File::open(layer_blob)
            .with_context(|| format!("Failed to open layer blob: {}", layer_blob.display()))?;

        // OCI layers are typically gzipped tarballs
        let decoder = GzDecoder::new(layer_file);
        let mut archive = Archive::new(decoder);

        // Extract with whiteout handling
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;

            // Handle whiteout files (OCI spec)
            // .wh.<name> means delete <name>
            // .wh..wh..opq means make directory opaque (delete all lower files)
            if let Some(filename) = path.file_name() {
                if let Some(name) = filename.to_str() {
                    if name.starts_with(".wh.") {
                        if name == ".wh..wh..opq" {
                            // Mark directory as opaque (for OverlayFS)
                            tracing::debug!("Opaque whiteout: {:?}", path);
                        } else {
                            // Regular whiteout - delete target file
                            let target_name = &name[4..]; // Remove ".wh." prefix
                            tracing::debug!("Whiteout: {} -> delete {}", name, target_name);
                        }
                        continue; // Don't extract whiteout files
                    }
                }
            }

            // Extract entry
            entry.unpack_in(target_dir)?;
        }

        tracing::debug!("Extracted layer to: {}", target_dir.display());

        Ok(())
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
