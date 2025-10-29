use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{SecretData, SecretItem};

/// Secret keystore (in-memory for MVP)
/// In production, this would integrate with Liminal-DB for encrypted storage
pub struct SecretKeystore {
    secrets: Arc<Mutex<HashMap<String, SecretData>>>,
}

impl SecretKeystore {
    pub fn new() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Store a secret
    pub fn store_secret(&self, secret: SecretData) -> Result<()> {
        let key = format!("{}@{}", secret.name, secret.version);

        let mut secrets = self.secrets.lock().unwrap();
        secrets.insert(key, secret.clone());

        tracing::debug!("Stored secret: {}@{}", secret.name, secret.version);

        Ok(())
    }

    /// Load a secret
    pub fn load_secret(&self, name: &str, version: &str) -> Result<SecretData> {
        let key = format!("{}@{}", name, version);

        let secrets = self.secrets.lock().unwrap();

        secrets.get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", key))
    }

    /// Delete a secret
    pub fn delete_secret(&self, name: &str, version: &str) -> Result<()> {
        let key = format!("{}@{}", name, version);

        let mut secrets = self.secrets.lock().unwrap();

        if secrets.remove(&key).is_some() {
            tracing::info!("Deleted secret: {}", key);
            Ok(())
        } else {
            anyhow::bail!("Secret not found: {}", key)
        }
    }

    /// List all secrets
    pub fn list_secrets(&self) -> Vec<(String, String)> {
        let secrets = self.secrets.lock().unwrap();

        secrets.keys()
            .filter_map(|k| {
                let parts: Vec<&str> = k.split('@').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Global keystore instance (singleton)
lazy_static::lazy_static! {
    static ref GLOBAL_KEYSTORE: SecretKeystore = SecretKeystore::new();
}

/// Create a secret from literal key-value pairs
pub fn create_secret_from_literal(
    name: &str,
    version: &str,
    items: Vec<(&str, &str)>,
) -> Result<()> {
    let secret_items: Vec<SecretItem> = items
        .into_iter()
        .map(|(key, value)| SecretItem {
            key: key.to_string(),
            value: value.as_bytes().to_vec(),
        })
        .collect();

    let secret = SecretData {
        name: name.to_string(),
        version: version.to_string(),
        items: secret_items,
    };

    GLOBAL_KEYSTORE.store_secret(secret)?;

    Ok(())
}

/// Load a secret (used by materialize_secret)
pub fn load_secret(name: &str, version: &str) -> Result<SecretData> {
    GLOBAL_KEYSTORE.load_secret(name, version)
}

/// Delete a secret
pub fn delete_secret(name: &str, version: &str) -> Result<()> {
    GLOBAL_KEYSTORE.delete_secret(name, version)
}

/// List all secrets
pub fn list_secrets() -> Vec<(String, String)> {
    GLOBAL_KEYSTORE.list_secrets()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keystore() {
        let keystore = SecretKeystore::new();

        let secret = SecretData {
            name: "test-secret".to_string(),
            version: "1".to_string(),
            items: vec![
                SecretItem {
                    key: "username".to_string(),
                    value: b"admin".to_vec(),
                },
                SecretItem {
                    key: "password".to_string(),
                    value: b"supersecret".to_vec(),
                },
            ],
        };

        // Store
        keystore.store_secret(secret.clone()).unwrap();

        // Load
        let loaded = keystore.load_secret("test-secret", "1").unwrap();
        assert_eq!(loaded.name, "test-secret");
        assert_eq!(loaded.version, "1");
        assert_eq!(loaded.items.len(), 2);

        // Delete
        keystore.delete_secret("test-secret", "1").unwrap();

        // Should not exist
        assert!(keystore.load_secret("test-secret", "1").is_err());
    }

    #[test]
    fn test_create_from_literal() {
        create_secret_from_literal(
            "api-key",
            "1",
            vec![("key", "abc123")],
        ).unwrap();

        let loaded = load_secret("api-key", "1").unwrap();
        assert_eq!(loaded.items[0].key, "key");
        assert_eq!(loaded.items[0].value, b"abc123");
    }
}
