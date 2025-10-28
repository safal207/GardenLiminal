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
