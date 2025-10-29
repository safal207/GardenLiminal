use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// DNS server for service discovery
/// Listens on gl0 bridge (10.44.0.1:53) and resolves service names
/// Format: service-name.pod-name.garden -> pod IP
pub struct DnsServer {
    bind_addr: String,
    registry: Arc<Mutex<ServiceRegistry>>,
    upstream_dns: Vec<String>,
}

/// Service registry for DNS resolution
#[derive(Debug, Clone)]
struct ServiceRegistry {
    /// Map: "service-name.pod-name.garden" -> IP
    services: HashMap<String, String>,
}

impl ServiceRegistry {
    fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    fn register(&mut self, fqdn: &str, ip: &str) {
        tracing::debug!("DNS: Registering {} -> {}", fqdn, ip);
        self.services.insert(fqdn.to_string(), ip.to_string());
    }

    fn unregister(&mut self, fqdn: &str) {
        tracing::debug!("DNS: Unregistering {}", fqdn);
        self.services.remove(fqdn);
    }

    fn lookup(&self, fqdn: &str) -> Option<String> {
        self.services.get(fqdn).cloned()
    }

    fn list(&self) -> Vec<(String, String)> {
        self.services.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

impl DnsServer {
    /// Create new DNS server
    pub fn new(bind_ip: &str, port: u16, upstream_dns: Vec<String>) -> Self {
        Self {
            bind_addr: format!("{}:{}", bind_ip, port),
            registry: Arc::new(Mutex::new(ServiceRegistry::new())),
            upstream_dns,
        }
    }

    /// Start DNS server in background thread
    /// For MVP, this is a stub. Full implementation would:
    /// 1. Listen on UDP 53
    /// 2. Parse DNS queries
    /// 3. Check registry for service names
    /// 4. Forward unknown queries to upstream
    /// 5. Send DNS responses
    pub fn start(self) -> Result<JoinHandle<()>> {
        tracing::info!("Starting DNS server on {}", self.bind_addr);

        let handle = thread::spawn(move || {
            // MVP: Just log that DNS would be running
            // Full implementation would use trust-dns or similar
            tracing::info!("DNS server thread started (MVP stub)");
            tracing::debug!("Would listen on: {}", self.bind_addr);
            tracing::debug!("Upstream DNS: {:?}", self.upstream_dns);

            // In production, would:
            // let socket = UdpSocket::bind(&self.bind_addr).unwrap();
            // loop {
            //     let mut buf = [0u8; 512];
            //     let (amt, src) = socket.recv_from(&mut buf).unwrap();
            //     let query = parse_dns_query(&buf[..amt]);
            //     let response = self.handle_query(&query);
            //     socket.send_to(&response, src).unwrap();
            // }

            // For MVP, just keep thread alive
            loop {
                thread::sleep(std::time::Duration::from_secs(60));
            }
        });

        Ok(handle)
    }

    /// Register a service in DNS
    pub fn register_service(
        registry: Arc<Mutex<ServiceRegistry>>,
        service_name: &str,
        pod_name: &str,
        ip: &str,
    ) -> Result<()> {
        let fqdn = format!("{}.{}.garden", service_name, pod_name);

        let mut reg = registry.lock().unwrap();
        reg.register(&fqdn, ip);

        Ok(())
    }

    /// Unregister all services for a pod
    pub fn unregister_pod_services(
        registry: Arc<Mutex<ServiceRegistry>>,
        pod_name: &str,
    ) -> Result<()> {
        let suffix = format!(".{}.garden", pod_name);

        let mut reg = registry.lock().unwrap();
        let to_remove: Vec<String> = reg.services
            .keys()
            .filter(|k| k.ends_with(&suffix))
            .cloned()
            .collect();

        for fqdn in to_remove {
            reg.unregister(&fqdn);
        }

        Ok(())
    }

    /// Get registry handle
    pub fn registry(&self) -> Arc<Mutex<ServiceRegistry>> {
        Arc::clone(&self.registry)
    }
}

/// Write /etc/hosts entry for a service (fallback to DNS)
pub fn write_hosts_entry(service_name: &str, pod_name: &str, ip: &str) -> Result<()> {
    let fqdn = format!("{}.{}.garden", service_name, pod_name);
    let hosts_path = "/etc/hosts";

    // Read existing hosts
    let existing = std::fs::read_to_string(hosts_path).unwrap_or_default();

    // Check if entry already exists
    if existing.contains(&fqdn) {
        return Ok(());
    }

    // Append new entry
    let entry = format!("{} {}\n", ip, fqdn);
    std::fs::write(hosts_path, format!("{}{}", existing, entry))
        .with_context(|| format!("Failed to write {}", hosts_path))?;

    tracing::debug!("Added /etc/hosts entry: {} -> {}", fqdn, ip);

    Ok(())
}

/// Write /etc/resolv.conf to use gl DNS
pub fn write_resolv_conf(dns_servers: &[String]) -> Result<()> {
    let resolv_conf = "/etc/resolv.conf";

    let mut content = String::new();
    for server in dns_servers {
        content.push_str(&format!("nameserver {}\n", server));
    }
    content.push_str("options ndots:5\n"); // For service discovery

    std::fs::write(resolv_conf, content)
        .with_context(|| format!("Failed to write {}", resolv_conf))?;

    tracing::debug!("Configured DNS: {:?}", dns_servers);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_registry() {
        let mut registry = ServiceRegistry::new();

        registry.register("echo.echo-pod.garden", "10.44.1.10");
        registry.register("api.web-pod.garden", "10.44.1.11");

        assert_eq!(registry.lookup("echo.echo-pod.garden"), Some("10.44.1.10".to_string()));
        assert_eq!(registry.lookup("api.web-pod.garden"), Some("10.44.1.11".to_string()));
        assert_eq!(registry.lookup("nonexistent.garden"), None);

        registry.unregister("echo.echo-pod.garden");
        assert_eq!(registry.lookup("echo.echo-pod.garden"), None);
    }

    #[test]
    fn test_fqdn_format() {
        let fqdn = format!("{}.{}.garden", "echo", "echo-pod");
        assert_eq!(fqdn, "echo.echo-pod.garden");
    }
}
