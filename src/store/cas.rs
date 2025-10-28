use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Content-Addressable Storage for OCI layers
/// Maps digest (sha256:...) to local filesystem path
#[derive(Debug, Clone)]
pub struct CAS {
    inner: Arc<Mutex<CASInner>>,
}

#[derive(Debug)]
struct CASInner {
    /// Map from digest to path
    index: HashMap<String, PathBuf>,
}

impl CAS {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CASInner {
                index: HashMap::new(),
            })),
        }
    }

    /// Register a layer by digest
    pub fn register(&self, digest: &str, path: PathBuf) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();

        if !path.exists() {
            anyhow::bail!("Layer path does not exist: {}", path.display());
        }

        inner.index.insert(digest.to_string(), path.clone());

        tracing::debug!("Registered layer {} -> {}", digest, path.display());

        Ok(())
    }

    /// Get path for a digest
    pub fn get(&self, digest: &str) -> Result<PathBuf> {
        let inner = self.inner.lock().unwrap();

        inner
            .index
            .get(digest)
            .cloned()
            .with_context(|| format!("Layer not found in CAS: {}", digest))
    }

    /// Check if digest exists in CAS
    pub fn exists(&self, digest: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.index.contains_key(digest)
    }

    /// List all digests
    pub fn list(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.index.keys().cloned().collect()
    }
}

impl Default for CAS {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cas_register_and_get() -> Result<()> {
        let cas = CAS::new();
        let temp_dir = TempDir::new()?;
        let layer_path = temp_dir.path().join("layer1");
        fs::create_dir(&layer_path)?;

        let digest = "sha256:abc123";
        cas.register(digest, layer_path.clone())?;

        assert!(cas.exists(digest));
        assert_eq!(cas.get(digest)?, layer_path);

        Ok(())
    }
}
