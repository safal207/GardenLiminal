pub mod keystore;

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::seed::SecretVolume;

/// Secret data structure
#[derive(Debug, Clone)]
pub struct SecretData {
    pub name: String,
    pub version: String,
    pub items: Vec<SecretItem>,
}

#[derive(Debug, Clone)]
pub struct SecretItem {
    pub key: String,
    pub value: Vec<u8>, // Binary data
}

/// Materialize secret to tmpfs with strict permissions
/// Returns path to mounted tmpfs directory
pub fn materialize_secret(
    secret_ref: &SecretVolume,
    garden_id: &str,
    container_name: &str,
) -> Result<PathBuf> {
    // Parse secret reference: "name@version"
    let (name, version) = parse_secret_ref(&secret_ref.secret_ref)?;

    tracing::info!("Materializing secret {}@{} for container {}", name, version, container_name);

    // Create tmpfs mount point
    let tmpfs_path = crate::volumes::emptydir::create_tmpfs_for_secret(
        &format!("secret-{}", name),
        garden_id,
        container_name,
    )?;

    // Load secret from store
    let secret_data = keystore::load_secret(&name, &version)?;

    // Write secret files with strict permissions
    for item in &secret_data.items {
        let file_path = tmpfs_path.join(&item.key);

        // Create parent directories if needed
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write secret value
        std::fs::write(&file_path, &item.value)
            .with_context(|| format!("Failed to write secret file: {}", file_path.display()))?;

        // Set strict permissions (0400 - read-only for owner)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o400);
            std::fs::set_permissions(&file_path, perms)
                .with_context(|| format!("Failed to set permissions on {}", file_path.display()))?;
        }

        // Mask value in logs
        tracing::debug!("Materialized secret key: {} (value masked)", item.key);
    }

    tracing::info!("Secret {}@{} materialized at {}", name, version, tmpfs_path.display());

    Ok(tmpfs_path)
}

/// Cleanup secret tmpfs
pub fn cleanup_secret(
    secret_ref: &SecretVolume,
    garden_id: &str,
    container_name: &str,
) -> Result<()> {
    let (name, _version) = parse_secret_ref(&secret_ref.secret_ref)?;

    crate::volumes::emptydir::cleanup_emptydir(
        &format!("secret-{}", name),
        garden_id,
        container_name,
    )?;

    tracing::debug!("Cleaned up secret {}", name);

    Ok(())
}

/// Parse secret reference "name@version"
fn parse_secret_ref(secret_ref: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = secret_ref.split('@').collect();

    if parts.len() != 2 {
        anyhow::bail!("Invalid secret reference format: {}. Expected 'name@version'", secret_ref);
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_secret_ref() {
        let (name, version) = parse_secret_ref("token@1").unwrap();
        assert_eq!(name, "token");
        assert_eq!(version, "1");

        let (name, version) = parse_secret_ref("api-key@v2").unwrap();
        assert_eq!(name, "api-key");
        assert_eq!(version, "v2");

        assert!(parse_secret_ref("invalid").is_err());
        assert!(parse_secret_ref("too@many@parts").is_err());
    }
}
