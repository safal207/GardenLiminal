use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Security policy (Pact)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pact {
    pub name: String,
    pub version: String,

    #[serde(default)]
    pub seccomp_profile: Option<SeccompProfile>,

    #[serde(default)]
    pub drop_caps: Vec<String>,

    #[serde(default)]
    pub readonly_paths: Vec<String>,

    #[serde(default)]
    pub masked_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeccompProfile {
    pub default_action: String,

    #[serde(default)]
    pub syscalls: Vec<SyscallRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyscallRule {
    pub names: Vec<String>,
    pub action: String,
}

/// Pact store - manages security policies
#[derive(Debug, Clone)]
pub struct PactStore {
    inner: Arc<Mutex<PactStoreInner>>,
}

#[derive(Debug)]
struct PactStoreInner {
    /// Map from "name@version" to Pact
    pacts: HashMap<String, Pact>,
}

impl PactStore {
    pub fn new() -> Self {
        let mut store = Self {
            inner: Arc::new(Mutex::new(PactStoreInner {
                pacts: HashMap::new(),
            })),
        };

        // Pre-load some default pacts
        store.load_defaults();

        store
    }

    /// Load default pacts
    fn load_defaults(&mut self) {
        // Default "minimal" pact
        let minimal = Pact {
            name: "minimal".to_string(),
            version: "1".to_string(),
            seccomp_profile: None,
            drop_caps: vec![
                "NET_ADMIN".to_string(),
                "SYS_ADMIN".to_string(),
                "SYS_MODULE".to_string(),
            ],
            readonly_paths: vec![],
            masked_paths: vec![],
        };

        let _ = self.register(minimal);

        // "web-api" pact for web services
        let web_api = Pact {
            name: "web-api".to_string(),
            version: "1".to_string(),
            seccomp_profile: Some(SeccompProfile {
                default_action: "SCMP_ACT_ERRNO".to_string(),
                syscalls: vec![
                    SyscallRule {
                        names: vec!["read".to_string(), "write".to_string(), "close".to_string()],
                        action: "SCMP_ACT_ALLOW".to_string(),
                    },
                    SyscallRule {
                        names: vec!["socket".to_string(), "bind".to_string(), "listen".to_string()],
                        action: "SCMP_ACT_ALLOW".to_string(),
                    },
                ],
            }),
            drop_caps: vec![
                "NET_ADMIN".to_string(),
                "SYS_ADMIN".to_string(),
                "SYS_MODULE".to_string(),
                "SYS_PTRACE".to_string(),
            ],
            readonly_paths: vec!["/etc".to_string()],
            masked_paths: vec!["/proc/kcore".to_string()],
        };

        let _ = self.register(web_api);
    }

    /// Register a pact
    pub fn register(&self, pact: Pact) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();

        let key = format!("{}@{}", pact.name, pact.version);
        inner.pacts.insert(key.clone(), pact);

        tracing::debug!("Registered pact: {}", key);

        Ok(())
    }

    /// Get a pact by name@version
    pub fn get(&self, spec: &str) -> Result<Pact> {
        let inner = self.inner.lock().unwrap();

        // Parse spec (e.g., "web-api@1" or just "web-api")
        let key = if spec.contains('@') {
            spec.to_string()
        } else {
            // Default to version "1" if not specified
            format!("{}@1", spec)
        };

        inner
            .pacts
            .get(&key)
            .cloned()
            .with_context(|| format!("Pact not found: {}", spec))
    }

    /// Check if pact exists
    pub fn exists(&self, spec: &str) -> bool {
        let key = if spec.contains('@') {
            spec.to_string()
        } else {
            format!("{}@1", spec)
        };

        let inner = self.inner.lock().unwrap();
        inner.pacts.contains_key(&key)
    }

    /// List all pact names
    pub fn list(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.pacts.keys().cloned().collect()
    }
}

impl Default for PactStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pact_store() {
        let store = PactStore::new();

        // Default pacts should be loaded
        assert!(store.exists("minimal"));
        assert!(store.exists("web-api@1"));

        // Get a pact
        let pact = store.get("web-api@1").unwrap();
        assert_eq!(pact.name, "web-api");
        assert_eq!(pact.version, "1");
        assert!(!pact.drop_caps.is_empty());
    }

    #[test]
    fn test_pact_not_found() {
        let store = PactStore::new();

        assert!(store.get("nonexistent").is_err());
    }
}
