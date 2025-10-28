use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::seed::Seed;
use crate::store::{Store, StoreKind};
use crate::process::ProcessRunner;

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
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect { file } => {
            cmd_inspect(file)
        }
        Commands::Prepare { file } => {
            cmd_prepare(file)
        }
        Commands::Run { file, store } => {
            cmd_run(file, store)
        }
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
