use anyhow::Result;
use clap::{Parser, Subcommand};
use snap_back_core as core;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Snap Back — Btrfs backup and recovery for CachyOS")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Status,
    Drives,
    PlanDrive {
        #[arg(long)]
        device: PathBuf,
    },
    SetupDrive {
        #[arg(long)]
        token: String,
        #[arg(long)]
        confirm: String,
    },
    UseDrive {
        uuid: String,
    },
    SetBackupDrives {
        #[arg(long = "drive", required = true)]
        drives: Vec<String>,
    },
    Configure {
        #[arg(long)]
        destination: PathBuf,
    },
    Backup {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        scheduled: bool,
        #[arg(long = "drive")]
        drives: Vec<String>,
        #[arg(long)]
        save_targets: bool,
    },
    Browse {
        point: String,
        source: String,
        #[arg(default_value = "")]
        path: String,
    },
    RestoreFile {
        point: String,
        source: String,
        path: String,
        #[arg(long)]
        to: PathBuf,
    },
    PrepareRecovery {
        point: String,
    },
    CancelRecovery,
    RecoverOffline {
        #[arg(long, default_value = "/etc/snap-back-recovery.json")]
        plan: PathBuf,
    },
    Schedule {
        #[arg(value_parser=["on","off"])]
        state: String,
    },
    Reboot,
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Configure { destination } => message(core::setup::configure(&destination)?),
        Commands::Drives => println!("{}", serde_json::to_string(&core::drives::drives()?)?),
        Commands::PlanDrive { device } => println!(
            "{}",
            serde_json::to_string(&core::drives::plan_drive(&device)?)?
        ),
        Commands::SetupDrive { token, confirm } => {
            message(core::drives::setup_drive(&token, &confirm)?)
        }
        Commands::UseDrive { uuid } => message(core::drives::use_drive(&uuid)?),
        Commands::SetBackupDrives { drives } => message(core::drives::set_backup_drives(&drives)?),
        Commands::Status => println!("{}", serde_json::to_string(&core::status()?)?),
        Commands::Backup {
            label,
            scheduled,
            drives,
            save_targets,
        } => println!(
            "{}",
            serde_json::to_string(&core::backup(label, scheduled, &drives, save_targets)?)?
        ),
        Commands::Browse {
            point,
            source,
            path,
        } => println!(
            "{}",
            serde_json::to_string(&core::browse(&point, &source, &path)?)?
        ),
        Commands::RestoreFile {
            point,
            source,
            path,
            to,
        } => message(core::restore_file(&point, &source, &path, &to)?),
        Commands::PrepareRecovery { point } => message(core::prepare_recovery(&point)?),
        Commands::CancelRecovery => {
            message(core::cancel_recovery()?);
        }
        Commands::RecoverOffline { plan } => {
            core::root()?;
            core::recovery::recover_offline(&plan)?;
        }
        Commands::Schedule { state } => message(core::schedule(state == "on")?),
        Commands::Reboot => {
            core::root()?;
            core::command("systemctl", &["reboot", "--no-block"])?;
            message("Rebooting into recovery".into());
        }
    }
    Ok(())
}
fn message(message: String) {
    println!("{}", serde_json::json!({"message":message}));
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
