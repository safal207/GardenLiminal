use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::seed::{Seed, Garden};
use crate::store::StoreKind;
use crate::process::ProcessRunner;
use crate::pod::PodSupervisor;

#[derive(Parser)]
#[command(name = "gl")]
#[command(about = "GardenLiminal - Process isolation runtime", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate and print normalized seed configuration
    Inspect {
        /// Path to seed.yaml file
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Prepare environment for seed execution (validate paths, cgroups)
    Prepare {
        /// Path to seed.yaml file
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Run a process in isolation according to seed configuration
    Run {
        /// Path to seed.yaml file
        #[arg(short, long)]
        file: PathBuf,

        /// Storage backend to use (mem or liminal)
        #[arg(long, default_value = "mem")]
        store: String,
    },
    /// Garden (Pod) commands
    Garden {
        #[command(subcommand)]
        command: GardenCommands,
    },
    /// OCI Image commands
    Image {
        #[command(subcommand)]
        command: ImageCommands,
    },
    /// Volume commands
    Volume {
        #[command(subcommand)]
        command: VolumeCommands,
    },
    /// Secret commands
    Secret {
        #[command(subcommand)]
        command: SecretCommands,
    },
    /// Network commands
    Net {
        #[command(subcommand)]
        command: NetCommands,
    },
}

#[derive(Subcommand)]
enum GardenCommands {
    /// Validate and print normalized garden configuration
    Inspect {
        /// Path to garden.yaml file
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Run a pod according to garden configuration
    Run {
        /// Path to garden.yaml file
        #[arg(short, long)]
        file: PathBuf,

        /// Storage backend to use (mem or liminal)
        #[arg(long, default_value = "mem")]
        store: String,

        /// Metrics collection interval in seconds
        #[arg(long, default_value = "2")]
        metrics_interval: u64,
    },
    /// Show pod metrics snapshot
    Stats {
        /// Path to garden.yaml file
        #[arg(short, long)]
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum ImageCommands {
    /// Import an OCI image from tar or directory
    Import {
        /// Path to OCI image (tar, tar.gz, or directory)
        source: PathBuf,

        /// Optional store path (default: ./oci-store)
        #[arg(long, default_value = "./oci-store")]
        store_path: PathBuf,
    },
    /// List imported OCI images
    List {
        /// Optional store path (default: ./oci-store)
        #[arg(long, default_value = "./oci-store")]
        store_path: PathBuf,
    },
}

#[derive(Subcommand)]
enum VolumeCommands {
    /// Create a named volume
    Create {
        /// Volume name
        name: String,

        /// Size limit (e.g., "10Gi")
        #[arg(long)]
        size: Option<String>,
    },
    /// List all named volumes
    #[command(name = "ls")]
    List,
    /// Remove a named volume
    #[command(name = "rm")]
    Remove {
        /// Volume name
        name: String,
    },
}

#[derive(Subcommand)]
enum SecretCommands {
    /// Create a secret from literal value
    Create {
        /// Secret name
        name: String,

        /// Key-value pair (key=value)
        #[arg(long, value_name = "KEY=VALUE")]
        from_literal: String,

        /// Secret version (default: "1")
        #[arg(long, default_value = "1")]
        version: String,
    },
    /// Get secret metadata
    Get {
        /// Secret name
        name: String,

        /// Secret version
        #[arg(long, default_value = "1")]
        version: String,
    },
    /// Remove a secret
    #[command(name = "rm")]
    Remove {
        /// Secret name
        name: String,

        /// Secret version
        #[arg(long, default_value = "1")]
        version: String,
    },
}

#[derive(Subcommand)]
enum NetCommands {
    /// Show network status
    Status,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect { file } => cmd_inspect(file),
        Commands::Prepare { file } => cmd_prepare(file),
        Commands::Run { file, store } => cmd_run(file, store),
        Commands::Garden { command } => match command {
            GardenCommands::Inspect { file } => cmd_garden_inspect(file),
            GardenCommands::Run {
                file,
                store,
                metrics_interval,
            } => cmd_garden_run(file, store, metrics_interval),
            GardenCommands::Stats { file } => cmd_garden_stats(file),
        },
        Commands::Image { command } => match command {
            ImageCommands::Import { source, store_path } => cmd_image_import(source, store_path),
            ImageCommands::List { store_path } => cmd_image_list(store_path),
        },
        Commands::Volume { command } => match command {
            VolumeCommands::Create { name, size } => cmd_volume_create(name, size),
            VolumeCommands::List => cmd_volume_list(),
            VolumeCommands::Remove { name } => cmd_volume_remove(name),
        },
        Commands::Secret { command } => match command {
            SecretCommands::Create { name, from_literal, version } => {
                cmd_secret_create(name, from_literal, version)
            }
            SecretCommands::Get { name, version } => cmd_secret_get(name, version),
            SecretCommands::Remove { name, version } => cmd_secret_remove(name, version),
        },
        Commands::Net { command } => match command {
            NetCommands::Status => cmd_net_status(),
        },
    }
}

fn cmd_inspect(file: PathBuf) -> Result<()> {
    tracing::info!("Inspecting seed file: {}", file.display());

    let seed = Seed::from_file(&file)?;
    seed.validate()?;

    // Print normalized JSON
    let json = serde_json::to_string_pretty(&seed)?;
    println!("{}", json);

    tracing::info!("Seed validation passed");
    Ok(())
}

fn cmd_prepare(file: PathBuf) -> Result<()> {
    tracing::info!("Preparing environment for seed: {}", file.display());

    let seed = Seed::from_file(&file)?;
    seed.validate()?;

    // Check rootfs path exists
    if !seed.rootfs.path.exists() {
        anyhow::bail!("Rootfs path does not exist: {}", seed.rootfs.path.display());
    }

    tracing::info!("Rootfs path verified: {}", seed.rootfs.path.display());

    // Check if cgroups v2 is available
    if !std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        tracing::warn!("cgroups v2 not detected at /sys/fs/cgroup");
    } else {
        tracing::info!("cgroups v2 detected");
    }

    println!("✓ Seed configuration valid");
    println!("✓ Rootfs path exists: {}", seed.rootfs.path.display());
    println!("✓ Ready to run");

    Ok(())
}

fn cmd_run(file: PathBuf, store_kind: String) -> Result<()> {
    tracing::info!("Running seed from: {}", file.display());

    let seed = Seed::from_file(&file)?;
    seed.validate()?;

    // Create store
    let store_type = StoreKind::from_str(&store_kind)?;
    let store = store_type.create()?;

    // Run the process
    let runner = ProcessRunner::new(seed, store);
    let exit_code = runner.run()?;

    tracing::info!("Process exited with code: {}", exit_code);
    std::process::exit(exit_code);
}

// ============================================================================
// Garden (Pod) Commands
// ============================================================================

fn cmd_garden_inspect(file: PathBuf) -> Result<()> {
    tracing::info!("Inspecting garden file: {}", file.display());

    let garden = Garden::from_file(&file)?;
    garden.validate()?;

    // Print normalized JSON
    let json = serde_json::to_string_pretty(&garden)?;
    println!("{}", json);

    tracing::info!("Garden validation passed");
    Ok(())
}

fn cmd_garden_run(file: PathBuf, store_kind: String, metrics_interval: u64) -> Result<()> {
    tracing::info!("Running garden from: {}", file.display());

    let garden = Garden::from_file(&file)?;
    garden.validate()?;

    // Create store
    let store_type = StoreKind::from_str(&store_kind)?;
    let store = store_type.create()?;

    // Create pod supervisor
    let mut supervisor = PodSupervisor::new(garden, store)?;

    // Start pod
    supervisor.start()?;

    tracing::info!("Pod started successfully");

    // Main loop - tick and handle signals
    // For MVP, just wait a bit and then stop
    std::thread::sleep(std::time::Duration::from_secs(5));

    // Stop gracefully
    supervisor.stop_graceful(std::time::Duration::from_secs(10))?;

    let exit_code = supervisor.get_exit_code();

    tracing::info!("Pod exited with code: {}", exit_code);
    std::process::exit(exit_code);
}

fn cmd_image_import(source: PathBuf, store_path: PathBuf) -> Result<()> {
    use crate::store::oci::OCIManager;

    tracing::info!("Importing OCI image from: {}", source.display());
    tracing::info!("Store path: {}", store_path.display());

    let mut manager = OCIManager::new(store_path)?;

    let manifest_digest = manager.import(&source)?;

    println!("Successfully imported OCI image");
    println!("Manifest digest: {}", manifest_digest);
    println!();
    println!("To use this image in a Garden, reference it in rootfs config:");
    println!("  rootfs:");
    println!("    oci:");
    println!("      manifest: \"{}\"", manifest_digest);

    Ok(())
}

fn cmd_image_list(store_path: PathBuf) -> Result<()> {
    tracing::info!("Listing OCI images in: {}", store_path.display());

    // For MVP, just show store path and CAS contents
    println!("OCI Image Store: {}", store_path.display());
    println!();

    // Check if store exists
    if !store_path.exists() {
        println!("Store directory does not exist yet.");
        println!("Import an image with: gl image import <path>");
        return Ok(());
    }

    // List any index.json files or imported manifests
    // For MVP, just show directory structure
    println!("Store contents:");
    if let Ok(entries) = std::fs::read_dir(&store_path) {
        for entry in entries.flatten() {
            println!("  - {}", entry.file_name().to_string_lossy());
        }
    }

    Ok(())
}

// ============================================================================
// Volume Commands
// ============================================================================

fn cmd_volume_create(name: String, size: Option<String>) -> Result<()> {
    use crate::volumes::named;

    tracing::info!("Creating named volume: {}", name);

    named::ensure_named_volume(&name, size.as_deref())?;

    println!("✓ Created named volume: {}", name);
    if let Some(s) = size {
        println!("  Size limit: {}", s);
    }
    println!("\nTo use this volume in a Garden, add to volumes:");
    println!("  volumes:");
    println!("    - name: {}", name);
    println!("      namedVolume:");
    println!("        volumeName: {}", name);

    Ok(())
}

fn cmd_volume_list() -> Result<()> {
    use crate::volumes::named;

    tracing::info!("Listing named volumes");

    let volumes = named::list_named_volumes()?;

    if volumes.is_empty() {
        println!("No named volumes found.");
        println!("\nCreate a volume with: gl volume create <name>");
        return Ok(());
    }

    println!("Named Volumes:");
    println!();
    for vol in volumes {
        println!("  - {}", vol);
    }

    Ok(())
}

fn cmd_volume_remove(name: String) -> Result<()> {
    use crate::volumes::named;

    tracing::info!("Removing named volume: {}", name);

    named::delete_named_volume(&name)?;

    println!("✓ Removed named volume: {}", name);

    Ok(())
}

// ============================================================================
// Secret Commands
// ============================================================================

fn cmd_secret_create(name: String, from_literal: String, version: String) -> Result<()> {
    use crate::secrets::keystore;

    tracing::info!("Creating secret: {}@{}", name, version);

    // Parse key=value format
    let parts: Vec<&str> = from_literal.splitn(2, '=').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid literal format. Expected: key=value");
    }

    let key = parts[0];
    let value = parts[1];

    keystore::create_secret_from_literal(&name, &version, vec![(key, value)])?;

    println!("✓ Created secret: {}@{}", name, version);
    println!("  Key: {}", key);
    println!("\nTo use this secret in a Garden, add to volumes:");
    println!("  volumes:");
    println!("    - name: {}-vol", name);
    println!("      secret:");
    println!("        secretRef: {}@{}", name, version);

    Ok(())
}

fn cmd_secret_get(name: String, version: String) -> Result<()> {
    use crate::secrets::keystore;

    tracing::info!("Getting secret metadata: {}@{}", name, version);

    let secret = keystore::load_secret(&name, &version)?;

    println!("Secret: {}@{}", secret.name, secret.version);
    println!("Keys:");
    for item in &secret.items {
        println!("  - {} (value masked)", item.key);
    }

    Ok(())
}

fn cmd_secret_remove(name: String, version: String) -> Result<()> {
    use crate::secrets::keystore;

    tracing::info!("Removing secret: {}@{}", name, version);

    keystore::delete_secret(&name, &version)?;

    println!("✓ Removed secret: {}@{}", name, version);

    Ok(())
}

// ============================================================================
// Network Commands
// ============================================================================

fn cmd_net_status() -> Result<()> {
    use crate::isolate::net;
    use crate::isolate::dns;

    tracing::info!("Checking network status");

    println!("GardenLiminal Network Status");
    println!();

    // Bridge status
    println!("Bridge:");
    match net::ensure_garden_bridge() {
        Ok(info) => {
            println!("  Name: {}", info.name);
            println!("  IP: {}/{}", info.ip, info.prefix_len);
            println!("  Status: ✓ Active");
        }
        Err(e) => {
            println!("  Status: ✗ Error: {}", e);
        }
    }

    println!();

    // IPAM status
    println!("IPAM:");
    match net::get_ipam_stats() {
        Ok(stats) => {
            println!("  Pool: {}", stats.pool_cidr);
            println!("  Allocated: {}", stats.allocated_count);
            println!("  Available: {}", stats.available_count);
        }
        Err(e) => {
            println!("  Status: ✗ Error: {}", e);
        }
    }

    println!();

    // DNS status
    println!("DNS:");
    match dns::get_dns_status() {
        Ok(status) => {
            println!("  Server: {}", status.listen_addr);
            println!("  Zone: {}", status.zone);
            println!("  Records: {}", status.record_count);
            println!("  Status: ✓ Running");
        }
        Err(e) => {
            println!("  Status: ✗ Error: {}", e);
        }
    }

    Ok(())
}

// ============================================================================
// Garden Stats Command
// ============================================================================

fn cmd_garden_stats(file: PathBuf) -> Result<()> {
    use crate::metrics::MetricsCollector;

    tracing::info!("Collecting metrics for garden: {}", file.display());

    let garden = Garden::from_file(&file)?;
    garden.validate()?;

    let garden_id = &garden.meta.name;

    println!("Pod Metrics: {}", garden_id);
    println!();

    // Collect metrics for each container
    for container in &garden.containers {
        let collector = MetricsCollector::new(garden_id, &container.name);

        match collector.collect() {
            Ok(metrics) => {
                println!("Container: {}", metrics.container_name);
                println!("  Timestamp: {}", metrics.timestamp);

                if let Some(mem) = metrics.memory_current {
                    let mb = mem as f64 / 1024.0 / 1024.0;
                    println!("  Memory: {:.2} MiB", mb);
                }

                if let Some(max) = metrics.memory_max {
                    if max != u64::MAX {
                        let mb = max as f64 / 1024.0 / 1024.0;
                        println!("  Memory Limit: {:.2} MiB", mb);
                    } else {
                        println!("  Memory Limit: unlimited");
                    }
                }

                if let Some(cpu) = metrics.cpu_usage_usec {
                    let secs = cpu as f64 / 1_000_000.0;
                    println!("  CPU Usage: {:.2} sec", secs);
                }

                if let Some(pids) = metrics.pids_current {
                    println!("  PIDs: {}", pids);
                }

                println!();
            }
            Err(e) => {
                println!("Container: {} - Error: {}", container.name, e);
                println!();
            }
        }
    }

    Ok(())
}
