use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seed {
    #[serde(rename = "apiVersion")]
    pub api_version: String,

    pub kind: String,

    pub meta: SeedMeta,

    pub rootfs: RootfsConfig,

    pub entrypoint: EntrypointConfig,

    #[serde(default)]
    pub limits: LimitsConfig,

    #[serde(default)]
    pub net: NetConfig,

    #[serde(default)]
    pub mounts: Vec<MountConfig>,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub user: UserConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub store: StoreConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedMeta {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootfsConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrypointConfig {
    pub cmd: Vec<String>,

    #[serde(default)]
    pub env: Vec<String>,

    #[serde(default = "default_cwd")]
    pub cwd: String,
}

fn default_cwd() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitsConfig {
    #[serde(default)]
    pub cpu: CpuLimit,

    #[serde(default)]
    pub memory: MemoryLimit,

    #[serde(default)]
    pub pids: PidsLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuLimit {
    #[serde(default)]
    pub shares: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryLimit {
    #[serde(default)]
    pub max: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PidsLimit {
    #[serde(default)]
    pub max: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetConfig {
    #[serde(default)]
    pub enable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    #[serde(rename = "type")]
    pub mount_type: String,

    #[serde(default)]
    pub source: Option<String>,

    pub target: String,

    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityConfig {
    #[serde(default)]
    pub hostname: Option<String>,

    #[serde(default)]
    pub drop_caps: Vec<String>,

    #[serde(default)]
    pub seccomp_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserConfig {
    #[serde(default = "default_uid")]
    pub uid: u32,

    #[serde(default = "default_gid")]
    pub gid: u32,

    #[serde(default)]
    pub map_rootless: bool,
}

fn default_uid() -> u32 {
    1000
}

fn default_gid() -> u32 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingConfig {
    #[serde(default = "default_log_mode")]
    pub mode: String,
}

fn default_log_mode() -> String {
    "json".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoreConfig {
    #[serde(default = "default_store_kind")]
    pub kind: String,
}

fn default_store_kind() -> String {
    "mem".to_string()
}

impl Seed {
    /// Load seed from YAML file
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read seed file: {}", path.display()))?;

        let seed: Seed = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse seed YAML: {}", path.display()))?;

        Ok(seed)
    }

    /// Validate seed configuration
    pub fn validate(&self) -> Result<()> {
        // Check API version
        if self.api_version != "v0" {
            anyhow::bail!("Unsupported API version: {}", self.api_version);
        }

        // Check kind
        if self.kind != "Seed" {
            anyhow::bail!("Invalid kind: {}, expected 'Seed'", self.kind);
        }

        // Check entrypoint
        if self.entrypoint.cmd.is_empty() {
            anyhow::bail!("Entrypoint cmd cannot be empty");
        }

        // Check rootfs path (convert to absolute if needed)
        if !self.rootfs.path.exists() {
            tracing::warn!("Rootfs path does not exist: {}", self.rootfs.path.display());
        }

        // Validate memory limit format if provided
        if let Some(ref mem_max) = self.limits.memory.max {
            parse_memory_string(mem_max)
                .with_context(|| format!("Invalid memory limit format: {}", mem_max))?;
        }

        Ok(())
    }
}

/// Parse memory strings like "128Mi", "1Gi", "512M" to bytes
pub fn parse_memory_string(s: &str) -> Result<u64> {
    let s = s.trim();

    if let Some(rest) = s.strip_suffix("Ki") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1024);
    }
    if let Some(rest) = s.strip_suffix("Mi") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1024 * 1024);
    }
    if let Some(rest) = s.strip_suffix("Gi") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1024 * 1024 * 1024);
    }
    if let Some(rest) = s.strip_suffix("K") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1000);
    }
    if let Some(rest) = s.strip_suffix("M") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1000 * 1000);
    }
    if let Some(rest) = s.strip_suffix("G") {
        let val: u64 = rest.parse()?;
        return Ok(val * 1000 * 1000 * 1000);
    }

    // Just bytes
    s.parse().context("Invalid number format")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_string() {
        assert_eq!(parse_memory_string("128Mi").unwrap(), 128 * 1024 * 1024);
        assert_eq!(parse_memory_string("1Gi").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_memory_string("512M").unwrap(), 512 * 1000 * 1000);
        assert_eq!(parse_memory_string("1024").unwrap(), 1024);
    }
}
