use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};

/// Container metrics from cgroups
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetrics {
    pub timestamp: String,
    pub container_name: String,

    /// Memory usage in bytes
    pub memory_current: Option<u64>,

    /// Memory limit in bytes
    pub memory_max: Option<u64>,

    /// CPU usage in microseconds
    pub cpu_usage_usec: Option<u64>,

    /// Number of PIDs
    pub pids_current: Option<u64>,
}

/// Metrics collector for a container
pub struct MetricsCollector {
    cgroup_path: PathBuf,
    container_name: String,
}

impl MetricsCollector {
    /// Create a new metrics collector for a container
    pub fn new(garden_id: &str, container_name: &str) -> Self {
        let cgroup_path = Path::new("/sys/fs/cgroup")
            .join("garden")
            .join(garden_id)
            .join(container_name);

        Self {
            cgroup_path,
            container_name: container_name.to_string(),
        }
    }

    /// Collect current metrics
    pub fn collect(&self) -> Result<ContainerMetrics> {
        let timestamp = chrono::Utc::now().to_rfc3339();

        let memory_current = self.read_memory_current().ok();
        let memory_max = self.read_memory_max().ok();
        let cpu_usage_usec = self.read_cpu_usage().ok();
        let pids_current = self.read_pids_current().ok();

        Ok(ContainerMetrics {
            timestamp,
            container_name: self.container_name.clone(),
            memory_current,
            memory_max,
            cpu_usage_usec,
            pids_current,
        })
    }

    /// Read memory.current
    fn read_memory_current(&self) -> Result<u64> {
        let path = self.cgroup_path.join("memory.current");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        content
            .trim()
            .parse()
            .context("Failed to parse memory.current")
    }

    /// Read memory.max
    fn read_memory_max(&self) -> Result<u64> {
        let path = self.cgroup_path.join("memory.max");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let trimmed = content.trim();

        // memory.max can be "max" for unlimited
        if trimmed == "max" {
            return Ok(u64::MAX);
        }

        trimmed.parse().context("Failed to parse memory.max")
    }

    /// Read cpu.stat and extract usage_usec
    fn read_cpu_usage(&self) -> Result<u64> {
        let path = self.cgroup_path.join("cpu.stat");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        // Parse cpu.stat which has format:
        // usage_usec 123456
        // user_usec 78901
        // system_usec 44555
        for line in content.lines() {
            if let Some(usage) = line.strip_prefix("usage_usec ") {
                return usage.trim().parse().context("Failed to parse cpu usage");
            }
        }

        anyhow::bail!("usage_usec not found in cpu.stat")
    }

    /// Read pids.current
    fn read_pids_current(&self) -> Result<u64> {
        let path = self.cgroup_path.join("pids.current");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        content
            .trim()
            .parse()
            .context("Failed to parse pids.current")
    }
}

/// Metrics collector thread for periodic collection
pub struct MetricsCollectorThread {
    collectors: Vec<MetricsCollector>,
    interval: Duration,
    run_id: String,
    garden_id: String,
}

impl MetricsCollectorThread {
    pub fn new(
        run_id: String,
        garden_id: String,
        container_names: Vec<String>,
        interval_secs: u64,
    ) -> Self {
        let collectors = container_names
            .iter()
            .map(|name| MetricsCollector::new(&garden_id, name))
            .collect();

        Self {
            collectors,
            interval: Duration::from_secs(interval_secs),
            run_id,
            garden_id,
        }
    }

    /// Start collecting metrics in a background thread
    pub fn start<F>(self, mut callback: F) -> thread::JoinHandle<()>
    where
        F: FnMut(ContainerMetrics) + Send + 'static,
    {
        thread::spawn(move || {
            loop {
                for collector in &self.collectors {
                    match collector.collect() {
                        Ok(metrics) => {
                            callback(metrics);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to collect metrics for {}: {}",
                                collector.container_name,
                                e
                            );
                        }
                    }
                }

                thread::sleep(self.interval);
            }
        })
    }
}

/// Format metrics as JSON
pub fn metrics_to_json(metrics: &ContainerMetrics) -> serde_json::Value {
    serde_json::to_value(metrics).unwrap_or_else(|_| serde_json::json!({}))
}

// ============================================================================
// Prometheus Metrics Exporter
// ============================================================================

/// Metrics registry for Prometheus exposition
pub struct MetricsRegistry {
    metrics: Arc<Mutex<HashMap<String, Vec<ContainerMetrics>>>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Update metrics for a pod
    pub fn update_pod_metrics(&self, garden_id: &str, metrics: Vec<ContainerMetrics>) {
        let mut m = self.metrics.lock().unwrap();
        m.insert(garden_id.to_string(), metrics);
    }

    /// Export metrics in Prometheus format
    pub fn export_prometheus(&self) -> String {
        let metrics = self.metrics.lock().unwrap();
        let mut output = String::new();

        // Pod running status
        output.push_str("# HELP garden_pod_running Whether the pod is running (1=running, 0=stopped)\n");
        output.push_str("# TYPE garden_pod_running gauge\n");
        for (garden_id, _) in metrics.iter() {
            output.push_str(&format!("garden_pod_running{{garden_id=\"{}\"}} 1\n", garden_id));
        }
        output.push('\n');

        // Container CPU usage
        output.push_str("# HELP garden_container_cpu_usage_usec Container CPU usage in microseconds\n");
        output.push_str("# TYPE garden_container_cpu_usage_usec counter\n");
        for (garden_id, containers) in metrics.iter() {
            for container in containers {
                if let Some(cpu) = container.cpu_usage_usec {
                    output.push_str(&format!(
                        "garden_container_cpu_usage_usec{{garden_id=\"{}\",container=\"{}\"}} {}\n",
                        garden_id, container.container_name, cpu
                    ));
                }
            }
        }
        output.push('\n');

        // Container memory current
        output.push_str("# HELP garden_container_mem_current_bytes Container current memory usage in bytes\n");
        output.push_str("# TYPE garden_container_mem_current_bytes gauge\n");
        for (garden_id, containers) in metrics.iter() {
            for container in containers {
                if let Some(mem) = container.memory_current {
                    output.push_str(&format!(
                        "garden_container_mem_current_bytes{{garden_id=\"{}\",container=\"{}\"}} {}\n",
                        garden_id, container.container_name, mem
                    ));
                }
            }
        }
        output.push('\n');

        // Container memory limit
        output.push_str("# HELP garden_container_mem_max_bytes Container memory limit in bytes\n");
        output.push_str("# TYPE garden_container_mem_max_bytes gauge\n");
        for (garden_id, containers) in metrics.iter() {
            for container in containers {
                if let Some(mem) = container.memory_max {
                    output.push_str(&format!(
                        "garden_container_mem_max_bytes{{garden_id=\"{}\",container=\"{}\"}} {}\n",
                        garden_id, container.container_name, mem
                    ));
                }
            }
        }
        output.push('\n');

        // Container PIDs
        output.push_str("# HELP garden_container_pids_current Number of PIDs in container\n");
        output.push_str("# TYPE garden_container_pids_current gauge\n");
        for (garden_id, containers) in metrics.iter() {
            for container in containers {
                if let Some(pids) = container.pids_current {
                    output.push_str(&format!(
                        "garden_container_pids_current{{garden_id=\"{}\",container=\"{}\"}} {}\n",
                        garden_id, container.container_name, pids
                    ));
                }
            }
        }

        output
    }
}

/// Simple HTTP server for metrics endpoint
/// Listens on 127.0.0.1:9464/metrics
pub fn start_metrics_server(registry: Arc<MetricsRegistry>) -> Result<thread::JoinHandle<()>> {
    let handle = thread::spawn(move || {
        let listener = TcpListener::bind("127.0.0.1:9464").expect("Failed to bind metrics server");
        tracing::info!("Metrics server listening on http://127.0.0.1:9464/metrics");

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let registry = Arc::clone(&registry);
                    thread::spawn(move || handle_metrics_request(stream, registry));
                }
                Err(e) => {
                    tracing::warn!("Failed to accept connection: {}", e);
                }
            }
        }
    });

    Ok(handle)
}

fn handle_metrics_request(mut stream: TcpStream, registry: Arc<MetricsRegistry>) {
    let mut buffer = [0u8; 1024];
    if let Ok(size) = stream.read(&mut buffer) {
        let request = String::from_utf8_lossy(&buffer[..size]);

        // Simple HTTP request parsing
        if request.starts_with("GET /metrics") {
            let metrics = registry.export_prometheus();

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                metrics.len(),
                metrics
            );

            let _ = stream.write_all(response.as_bytes());
        } else {
            // 404 for other paths
            let response = "HTTP/1.1 404 Not Found\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_struct() {
        let metrics = ContainerMetrics {
            timestamp: "2025-10-28T12:00:00Z".to_string(),
            container_name: "test".to_string(),
            memory_current: Some(1024 * 1024),
            memory_max: Some(128 * 1024 * 1024),
            cpu_usage_usec: Some(1000000),
            pids_current: Some(5),
        };

        let json = metrics_to_json(&metrics);
        assert!(json.is_object());
        assert_eq!(json["container_name"], "test");
    }
}
