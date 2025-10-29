use anyhow::{Context, Result};
use std::fs;
use std::process::Command;
use serde::{Deserialize, Serialize};

/// Bridge configuration
pub const BRIDGE_NAME: &str = "gl0";
pub const BRIDGE_IP: &str = "10.44.0.1/16"; // Changed to /16 for larger pool
pub const DEFAULT_SUBNET: &str = "10.44.0.0/16";

/// IPAM - IP address management with allocation tracking
/// Pool: 10.44.0.0/16 (10.44.1.0 - 10.44.255.254)
pub struct IpAllocator {
    allocated: std::collections::HashSet<String>,
    next_subnet: u8, // 10.44.X.0/24
    next_host: u8,   // 10.44.X.Y
}

impl IpAllocator {
    pub fn new() -> Self {
        Self {
            allocated: std::collections::HashSet::new(),
            next_subnet: 1, // Start from 10.44.1.0
            next_host: 10,  // Start from .10 within subnet
        }
    }

    /// Allocate next available IP (returns IP without CIDR for simplicity)
    pub fn allocate(&mut self, pod_id: &str) -> Result<String> {
        // Try to allocate from current subnet
        loop {
            if self.next_subnet >= 255 {
                anyhow::bail!("IP pool exhausted (all subnets used)");
            }

            if self.next_host >= 254 {
                // Move to next subnet
                self.next_subnet += 1;
                self.next_host = 10;
                continue;
            }

            let ip = format!("10.44.{}.{}", self.next_subnet, self.next_host);
            self.next_host += 1;

            // Check if already allocated
            if !self.allocated.contains(&ip) {
                self.allocated.insert(ip.clone());
                tracing::debug!("Allocated IP {} to pod {}", ip, pod_id);
                return Ok(ip);
            }
        }
    }

    /// Release an IP back to the pool
    pub fn release(&mut self, ip: &str) -> Result<()> {
        if self.allocated.remove(ip) {
            tracing::debug!("Released IP {}", ip);
            Ok(())
        } else {
            tracing::warn!("Attempted to release non-allocated IP: {}", ip);
            Ok(()) // Not an error, just a warning
        }
    }

    /// Get count of allocated IPs
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }

    /// Get list of allocated IPs
    pub fn allocated_ips(&self) -> Vec<String> {
        self.allocated.iter().cloned().collect()
    }

    /// Check if IP is allocated
    pub fn is_allocated(&self, ip: &str) -> bool {
        self.allocated.contains(ip)
    }
}

/// Ensure bridge exists and is configured
pub fn ensure_bridge() -> Result<()> {
    // Check if bridge already exists
    if bridge_exists()? {
        tracing::debug!("Bridge {} already exists", BRIDGE_NAME);
        return Ok(());
    }

    tracing::info!("Creating bridge {}", BRIDGE_NAME);

    // Create bridge
    run_cmd("ip", &["link", "add", BRIDGE_NAME, "type", "bridge"])
        .context("Failed to create bridge")?;

    // Set bridge up
    run_cmd("ip", &["link", "set", BRIDGE_NAME, "up"])
        .context("Failed to bring bridge up")?;

    // Assign IP to bridge
    run_cmd("ip", &["addr", "add", BRIDGE_IP, "dev", BRIDGE_NAME])
        .context("Failed to assign IP to bridge")?;

    tracing::info!("Bridge {} created and configured with IP {}", BRIDGE_NAME, BRIDGE_IP);

    Ok(())
}

/// Check if bridge exists
fn bridge_exists() -> Result<bool> {
    let output = Command::new("ip")
        .args(&["link", "show", BRIDGE_NAME])
        .output()
        .context("Failed to check bridge existence")?;

    Ok(output.status.success())
}

/// Create veth pair and attach to bridge
pub fn setup_veth_pair(container_name: &str, pod_netns: &str) -> Result<(String, String)> {
    let veth_host = format!("veth-{}", &container_name[..std::cmp::min(8, container_name.len())]);
    let veth_pod = "eth0".to_string();

    tracing::debug!("Creating veth pair: {} <-> {}", veth_host, veth_pod);

    // Create veth pair
    run_cmd(
        "ip",
        &["link", "add", &veth_host, "type", "veth", "peer", "name", &veth_pod],
    )
    .context("Failed to create veth pair")?;

    // Move pod-side veth to network namespace
    run_cmd("ip", &["link", "set", &veth_pod, "netns", pod_netns])
        .context("Failed to move veth to netns")?;

    // Attach host-side veth to bridge
    run_cmd("ip", &["link", "set", &veth_host, "master", BRIDGE_NAME])
        .context("Failed to attach veth to bridge")?;

    // Bring up host-side veth
    run_cmd("ip", &["link", "set", &veth_host, "up"])
        .context("Failed to bring up host veth")?;

    tracing::info!("Created veth pair: {} (host) <-> {} (pod)", veth_host, veth_pod);

    Ok((veth_host, veth_pod))
}

/// Configure network inside pod namespace
pub fn configure_pod_network(pod_ip: &str, dns_servers: &[String]) -> Result<()> {
    // Bring up loopback
    run_cmd("ip", &["link", "set", "lo", "up"])
        .context("Failed to bring up loopback")?;

    // Assign IP to eth0
    run_cmd("ip", &["addr", "add", pod_ip, "dev", "eth0"])
        .context("Failed to assign IP to eth0")?;

    // Bring up eth0
    run_cmd("ip", &["link", "set", "eth0", "up"])
        .context("Failed to bring up eth0")?;

    // Add default route
    let gateway = "10.44.0.1";
    run_cmd("ip", &["route", "add", "default", "via", gateway])
        .context("Failed to add default route")?;

    // Configure DNS
    setup_dns(dns_servers)?;

    tracing::info!("Configured pod network: IP={}, Gateway={}", pod_ip, gateway);

    Ok(())
}

/// Setup DNS in /etc/resolv.conf
fn setup_dns(dns_servers: &[String]) -> Result<()> {
    let resolv_conf = "/etc/resolv.conf";

    let mut content = String::new();
    for server in dns_servers {
        content.push_str(&format!("nameserver {}\n", server));
    }

    fs::write(resolv_conf, content)
        .with_context(|| format!("Failed to write {}", resolv_conf))?;

    tracing::debug!("Configured DNS: {:?}", dns_servers);

    Ok(())
}

/// Create network namespace
pub fn create_netns(name: &str) -> Result<()> {
    // Check if netns already exists
    if netns_exists(name)? {
        tracing::warn!("Network namespace {} already exists, reusing", name);
        return Ok(());
    }

    run_cmd("ip", &["netns", "add", name])
        .context("Failed to create network namespace")?;

    tracing::debug!("Created network namespace: {}", name);

    Ok(())
}

/// Check if network namespace exists
fn netns_exists(name: &str) -> Result<bool> {
    let netns_path = format!("/var/run/netns/{}", name);
    Ok(std::path::Path::new(&netns_path).exists())
}

/// Delete network namespace
pub fn delete_netns(name: &str) -> Result<()> {
    if !netns_exists(name)? {
        return Ok(());
    }

    run_cmd("ip", &["netns", "del", name])
        .context("Failed to delete network namespace")?;

    tracing::debug!("Deleted network namespace: {}", name);

    Ok(())
}

/// Execute command in network namespace
pub fn exec_in_netns(netns: &str, cmd: &str, args: &[&str]) -> Result<()> {
    let mut full_args = vec!["netns", "exec", netns, cmd];
    full_args.extend_from_slice(args);

    run_cmd("ip", &full_args)
        .with_context(|| format!("Failed to execute command in netns {}", netns))?;

    Ok(())
}

/// Helper to run command and check result
fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute: {} {:?}", program, args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Command failed: {} {:?}\nStderr: {}",
            program,
            args,
            stderr
        );
    }

    Ok(())
}

/// Cleanup veth interface
pub fn cleanup_veth(veth_name: &str) -> Result<()> {
    // Check if veth exists
    let check = Command::new("ip")
        .args(&["link", "show", veth_name])
        .output();

    if let Ok(output) = check {
        if output.status.success() {
            run_cmd("ip", &["link", "del", veth_name])
                .with_context(|| format!("Failed to delete veth {}", veth_name))?;

            tracing::debug!("Deleted veth interface: {}", veth_name);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_allocator() {
        let mut allocator = IpAllocator::new();

        let ip1 = allocator.allocate("pod1").unwrap();
        assert_eq!(ip1, "10.44.1.10");

        let ip2 = allocator.allocate("pod2").unwrap();
        assert_eq!(ip2, "10.44.1.11");

        // Test release
        allocator.release(&ip1).unwrap();
        assert_eq!(allocator.allocated_count(), 1);
        assert!(!allocator.is_allocated(&ip1));
        assert!(allocator.is_allocated(&ip2));
    }
}

// ============================================================================
// CLI Status Helpers
// ============================================================================

/// Bridge information for status reporting
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub name: String,
    pub ip: String,
    pub prefix_len: u8,
}

/// IPAM statistics for status reporting
#[derive(Debug, Serialize, Deserialize)]
pub struct IpamStats {
    pub pool_cidr: String,
    pub allocated_count: usize,
    pub available_count: usize,
}

/// Get bridge information for CLI status
pub fn ensure_garden_bridge() -> Result<BridgeInfo> {
    // Ensure bridge exists
    ensure_bridge()?;

    // Parse BRIDGE_IP to extract IP and prefix
    let parts: Vec<&str> = BRIDGE_IP.split('/').collect();
    let ip = parts[0].to_string();
    let prefix_len = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(16);

    Ok(BridgeInfo {
        name: BRIDGE_NAME.to_string(),
        ip,
        prefix_len,
    })
}

/// Get IPAM statistics for CLI status
/// Note: This returns theoretical stats since we don't have global state
pub fn get_ipam_stats() -> Result<IpamStats> {
    // For CLI, return theoretical pool stats
    // In production, would query actual allocator state
    let pool_cidr = DEFAULT_SUBNET.to_string();

    // Calculate theoretical available IPs
    // 10.44.0.0/16 = 65536 IPs
    // - 256 for 10.44.0.x (reserved)
    // - 1 for broadcast per subnet
    // ≈ 65000 usable
    let available_count = 65000;

    Ok(IpamStats {
        pool_cidr,
        allocated_count: 0, // Would need global state to track
        available_count,
    })
}
