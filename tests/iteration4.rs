// Integration tests for Iteration 4 - Weaver
// Tests volumes, secrets, metrics, and networking features

use anyhow::Result;

// ============================================================================
// Secrets Tests
// ============================================================================

#[test]
fn test_secret_keystore_create_and_load() -> Result<()> {
    use gl::secrets::keystore;

    let name = format!("test-secret-{}", uuid::Uuid::new_v4());
    let version = "1";

    // Create secret
    keystore::create_secret_from_literal(
        &name,
        version,
        vec![
            ("username", "admin"),
            ("password", "s3cr3t"),
        ],
    )?;

    // Load secret
    let secret = keystore::load_secret(&name, version)?;
    assert_eq!(secret.name, name);
    assert_eq!(secret.version, version);
    assert_eq!(secret.items.len(), 2);

    // Delete secret
    keystore::delete_secret(&name, version)?;

    // Should fail to load deleted secret
    assert!(keystore::load_secret(&name, version).is_err());

    Ok(())
}

#[test]
fn test_secret_ref_parsing() -> Result<()> {
    use gl::secrets::parse_secret_ref;

    // Valid format
    let (name, version) = parse_secret_ref("db-creds@1")?;
    assert_eq!(name, "db-creds");
    assert_eq!(version, "1");

    // Invalid formats should fail
    assert!(parse_secret_ref("no-version").is_err());

    Ok(())
}

#[test]
fn test_secret_versions() -> Result<()> {
    use gl::secrets::keystore;

    let name = format!("versioned-{}", uuid::Uuid::new_v4());

    // Create version 1
    keystore::create_secret_from_literal(name.as_str(), "1", vec![("key", "value-v1")])?;

    // Create version 2
    keystore::create_secret_from_literal(name.as_str(), "2", vec![("key", "value-v2")])?;

    // Load version 1
    let secret_v1 = keystore::load_secret(&name, "1")?;
    let v1_value = String::from_utf8(secret_v1.items[0].value.clone())?;
    assert_eq!(v1_value, "value-v1");

    // Load version 2
    let secret_v2 = keystore::load_secret(&name, "2")?;
    let v2_value = String::from_utf8(secret_v2.items[0].value.clone())?;
    assert_eq!(v2_value, "value-v2");

    // Cleanup
    keystore::delete_secret(&name, "1")?;
    keystore::delete_secret(&name, "2")?;

    Ok(())
}

// ============================================================================
// Volume Tests
// ============================================================================

#[test]
#[cfg(target_os = "linux")]
fn test_named_volume_lifecycle() -> Result<()> {
    use gl::volumes::named;

    let vol_name = format!("test-vol-{}", uuid::Uuid::new_v4());

    // Create named volume
    let vol_path = named::ensure_named_volume(&vol_name, Some("10Mi"))?;
    assert!(vol_path.exists());

    // List should contain our volume
    let volumes = named::list_named_volumes()?;
    assert!(volumes.contains(&vol_name));

    // Write data
    std::fs::write(vol_path.join("data.txt"), "persistent")?;

    // Ensure again (should reuse existing)
    let vol_path2 = named::ensure_named_volume(&vol_name, None)?;
    assert_eq!(vol_path, vol_path2);
    assert_eq!(std::fs::read_to_string(vol_path2.join("data.txt"))?, "persistent");

    // Delete volume
    named::delete_named_volume(&vol_name)?;
    assert!(!vol_path.exists());

    Ok(())
}

#[test]
fn test_hostpath_validation() -> Result<()> {
    use gl::volumes::hostpath;
    use tempfile::TempDir;

    let temp_dir = TempDir::new()?;

    // Valid path should pass
    hostpath::validate_hostpath(temp_dir.path())?;

    // Non-existent path should fail
    let bad_path = temp_dir.path().join("does-not-exist");
    assert!(hostpath::validate_hostpath(&bad_path).is_err());

    Ok(())
}

// ============================================================================
// Metrics Tests
// ============================================================================

#[test]
fn test_metrics_serialization() -> Result<()> {
    use gl::metrics::{ContainerMetrics, metrics_to_json};

    let metrics = ContainerMetrics {
        timestamp: "2025-10-29T12:00:00Z".to_string(),
        container_name: "test-container".to_string(),
        memory_current: Some(100_000_000),
        memory_max: Some(256_000_000),
        cpu_usage_usec: Some(5_000_000),
        pids_current: Some(3),
    };

    let json = metrics_to_json(&metrics);
    assert!(json.is_object());
    assert_eq!(json["container_name"], "test-container");
    assert_eq!(json["pids_current"], 3);

    Ok(())
}

#[test]
fn test_prometheus_export() -> Result<()> {
    use gl::metrics::{MetricsRegistry, ContainerMetrics};

    let registry = MetricsRegistry::new();

    let metrics = vec![
        ContainerMetrics {
            timestamp: "2025-10-29T12:00:00Z".to_string(),
            container_name: "app".to_string(),
            memory_current: Some(100_000_000),
            memory_max: Some(256_000_000),
            cpu_usage_usec: Some(1_000_000),
            pids_current: Some(5),
        },
    ];

    registry.update_pod_metrics("test-pod", metrics);

    let output = registry.export_prometheus();

    // Verify Prometheus format
    assert!(output.contains("# HELP garden_pod_running"));
    assert!(output.contains("# TYPE garden_pod_running gauge"));
    assert!(output.contains("garden_pod_running{garden_id=\"test-pod\"} 1"));
    assert!(output.contains("garden_container_cpu_usage_usec{garden_id=\"test-pod\",container=\"app\"} 1000000"));
    assert!(output.contains("garden_container_mem_current_bytes{garden_id=\"test-pod\",container=\"app\"} 100000000"));

    Ok(())
}

// ============================================================================
// Networking Tests
// ============================================================================

#[test]
fn test_ip_allocator() -> Result<()> {
    use gl::isolate::net::IpAllocator;

    let mut allocator = IpAllocator::new();

    let ip1 = allocator.allocate("pod1")?;
    assert_eq!(ip1, "10.44.1.10");
    assert_eq!(allocator.allocated_count(), 1);
    assert!(allocator.is_allocated(&ip1));

    let ip2 = allocator.allocate("pod2")?;
    assert_eq!(ip2, "10.44.1.11");
    assert_eq!(allocator.allocated_count(), 2);

    allocator.release(&ip1)?;
    assert_eq!(allocator.allocated_count(), 1);
    assert!(!allocator.is_allocated(&ip1));

    Ok(())
}

#[test]
fn test_ipam_stats() -> Result<()> {
    use gl::isolate::net;

    let stats = net::get_ipam_stats()?;

    assert_eq!(stats.pool_cidr, "10.44.0.0/16");
    assert!(stats.available_count > 0);

    Ok(())
}

#[test]
fn test_dns_status() -> Result<()> {
    use gl::isolate::dns;

    let status = dns::get_dns_status()?;

    assert_eq!(status.listen_addr, "10.44.0.1:53");
    assert_eq!(status.zone, "garden");

    Ok(())
}

// ============================================================================
// Schema Tests
// ============================================================================

#[test]
fn test_garden_schema_with_services() -> Result<()> {
    use gl::seed::{Garden, ServiceSpec};

    let yaml = r#"
apiVersion: v1
kind: Garden
meta:
  name: test-garden
  id: test-1
net:
  preset: bridge
services:
  - name: web
    port: 8080
    targetContainer: frontend
    protocol: tcp
containers:
  - name: frontend
    rootfs:
      path: /tmp/rootfs
    command: ["/bin/sh"]
    ports: [8080]
"#;

    let garden: Garden = serde_yaml::from_str(yaml)?;
    assert_eq!(garden.services.len(), 1);
    assert_eq!(garden.services[0].name, "web");
    assert_eq!(garden.services[0].port, 8080);

    Ok(())
}

#[test]
fn test_volume_spec_parsing() -> Result<()> {
    use gl::seed::{Garden, VolumeType};

    let yaml = r#"
apiVersion: v1
kind: Garden
meta:
  name: test-garden
  id: test-2
net:
  preset: bridge
volumes:
  - name: data
    emptyDir:
      medium: disk
  - name: cache
    emptyDir:
      medium: tmpfs
      sizeLimit: "1Gi"
  - name: config
    namedVolume:
      volumeName: my-config
containers:
  - name: app
    rootfs:
      path: /tmp/rootfs
    command: ["/bin/sh"]
    volumeMounts:
      - name: data
        mountPath: /data
      - name: cache
        mountPath: /cache
"#;

    let garden: Garden = serde_yaml::from_str(yaml)?;
    assert_eq!(garden.volumes.len(), 3);

    // Check emptyDir disk
    if let VolumeType::EmptyDir(ref config) = garden.volumes[0].volume_type {
        assert_eq!(config.medium, "disk");
    } else {
        panic!("Expected EmptyDir");
    }

    // Check emptyDir tmpfs with size
    if let VolumeType::EmptyDir(ref config) = garden.volumes[1].volume_type {
        assert_eq!(config.medium, "tmpfs");
        assert_eq!(config.size_limit, Some("1Gi".to_string()));
    } else {
        panic!("Expected EmptyDir");
    }

    Ok(())
}
