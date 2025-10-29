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

// ============================================================================
// Iteration 2: Garden (Pod) Types
// ============================================================================

/// Garden (Pod) manifest - multiple containers with shared network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Garden {
    #[serde(rename = "apiVersion")]
    pub api_version: String,

    pub kind: String,

    pub meta: SeedMeta,

    pub net: GardenNetConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default = "default_restart_policy")]
    #[serde(rename = "restartPolicy")]
    pub restart_policy: String,

    #[serde(default)]
    pub services: Vec<ServiceSpec>,

    pub containers: Vec<Container>,

    #[serde(default)]
    pub volumes: Vec<VolumeSpec>,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub store: StoreConfig,
}

fn default_restart_policy() -> String {
    "Never".to_string()
}

/// Container within a Garden/Pod
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub name: String,

    pub rootfs: ContainerRootfsConfig,

    pub entrypoint: EntrypointConfig,

    #[serde(default)]
    pub limits: LimitsConfig,

    #[serde(default)]
    pub mounts: Vec<MountConfig>,

    #[serde(default)]
    pub user: UserConfig,

    #[serde(default)]
    pub ports: Vec<u16>,

    #[serde(default)]
    #[serde(rename = "volumeMounts")]
    pub volume_mounts: Vec<VolumeMount>,
}

/// Rootfs configuration for a container (path OR layers)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContainerRootfsConfig {
    Path { path: PathBuf },
    Layers(RootfsLayersConfig),
}

/// OverlayFS layers configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootfsLayersConfig {
    pub layers: LayersSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayersSpec {
    #[serde(default)]
    pub lower: Vec<String>,

    pub upper: String,

    pub work: String,
}

/// Garden network configuration (bridge mode with IP allocation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenNetConfig {
    #[serde(default = "default_net_preset")]
    pub preset: String,

    #[serde(default)]
    pub ip: Option<String>,

    #[serde(default = "default_dns")]
    pub dns: Vec<String>,
}

fn default_net_preset() -> String {
    "bridge".to_string()
}

fn default_dns() -> Vec<String> {
    vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()]
}

impl Default for GardenNetConfig {
    fn default() -> Self {
        Self {
            preset: default_net_preset(),
            ip: None,
            dns: default_dns(),
        }
    }
}

/// Restart policy enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

impl RestartPolicy {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "Never" => Ok(RestartPolicy::Never),
            "OnFailure" => Ok(RestartPolicy::OnFailure),
            "Always" => Ok(RestartPolicy::Always),
            _ => anyhow::bail!("Unknown restart policy: {}", s),
        }
    }
}

impl Garden {
    /// Load garden from YAML file
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read garden file: {}", path.display()))?;

        let garden: Garden = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse garden YAML: {}", path.display()))?;

        Ok(garden)
    }

    /// Validate garden configuration
    pub fn validate(&self) -> Result<()> {
        // Check API version
        if self.api_version != "v0" {
            anyhow::bail!("Unsupported API version: {}", self.api_version);
        }

        // Check kind
        if self.kind != "Garden" {
            anyhow::bail!("Invalid kind: {}, expected 'Garden'", self.kind);
        }

        // Check containers
        if self.containers.is_empty() {
            anyhow::bail!("Garden must have at least one container");
        }

        // Validate each container
        for (idx, container) in self.containers.iter().enumerate() {
            if container.name.is_empty() {
                anyhow::bail!("Container {} has empty name", idx);
            }

            if container.entrypoint.cmd.is_empty() {
                anyhow::bail!("Container {} has empty command", container.name);
            }

            // Validate memory limits
            if let Some(ref mem_max) = container.limits.memory.max {
                parse_memory_string(mem_max)
                    .with_context(|| format!("Invalid memory limit for container {}: {}", container.name, mem_max))?;
            }

            // Validate rootfs
            match &container.rootfs {
                ContainerRootfsConfig::Path { path } => {
                    if !path.exists() {
                        tracing::warn!("Rootfs path for container {} does not exist: {}", container.name, path.display());
                    }
                }
                ContainerRootfsConfig::Layers(layers_config) => {
                    if layers_config.layers.upper.is_empty() {
                        anyhow::bail!("Container {} has empty upper layer", container.name);
                    }
                    if layers_config.layers.work.is_empty() {
                        anyhow::bail!("Container {} has empty work layer", container.name);
                    }
                }
            }
        }

        // Validate restart policy
        RestartPolicy::from_str(&self.restart_policy)?;

        // Validate network config
        if let Some(ref ip) = self.net.ip {
            // Basic IP validation (should contain /)
            if !ip.contains('/') {
                anyhow::bail!("IP address must be in CIDR format (e.g., 10.44.0.10/24)");
            }
        }

        Ok(())
    }

    /// Get restart policy as enum
    pub fn get_restart_policy(&self) -> Result<RestartPolicy> {
        RestartPolicy::from_str(&self.restart_policy)
    }
}

// ============================================================================
// Iteration 4: Services, Volumes, Secrets
// ============================================================================

/// Service definition for service discovery and DNS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub port: u16,
    #[serde(rename = "targetContainer")]
    pub target_container: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "TCP".to_string()
}

/// Volume definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub name: String,
    #[serde(flatten)]
    pub volume_type: VolumeType,
}

/// Volume types (emptyDir, hostPath, namedVolume, config, secret)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VolumeType {
    EmptyDir(EmptyDirVolume),
    HostPath(HostPathVolume),
    NamedVolume(NamedVolume),
    Config(ConfigVolume),
    Secret(SecretVolume),
}

/// emptyDir volume (tmpfs or disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyDirVolume {
    #[serde(default = "default_empty_dir_medium")]
    pub medium: String, // "disk" or "tmpfs"
    #[serde(rename = "sizeLimit")]
    pub size_limit: Option<String>, // e.g., "256Mi"
}

fn default_empty_dir_medium() -> String {
    "disk".to_string()
}

/// hostPath volume (bind mount from host)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPathVolume {
    pub path: PathBuf,
    #[serde(default)]
    #[serde(rename = "readOnly")]
    pub read_only: bool,
}

/// namedVolume (persistent volume managed by gl)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedVolume {
    #[serde(rename = "volumeName")]
    pub name: String,
    #[serde(rename = "sizeLimit")]
    pub size_limit: Option<String>,
}

/// config volume (in-memory config files)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVolume {
    pub items: Vec<ConfigItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigItem {
    pub path: String,
    pub content: String,
}

/// secret volume (references secret from store)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretVolume {
    #[serde(rename = "ref")]
    pub secret_ref: String, // "name@version" format
}

/// Volume mount in container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub name: String,
    #[serde(rename = "mountPath")]
    pub mount_path: String,
    #[serde(default)]
    #[serde(rename = "readOnly")]
    pub read_only: bool,
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

    #[test]
    fn test_restart_policy_from_str() {
        assert_eq!(RestartPolicy::from_str("Never").unwrap(), RestartPolicy::Never);
        assert_eq!(RestartPolicy::from_str("OnFailure").unwrap(), RestartPolicy::OnFailure);
        assert_eq!(RestartPolicy::from_str("Always").unwrap(), RestartPolicy::Always);
        assert!(RestartPolicy::from_str("Unknown").is_err());
    }
}
