mod archive;
mod catalog;
#[cfg(windows)]
mod cuda;
mod gpu_lock;
pub mod model;
mod platform;
mod runtime;
mod runtime_notices;

use clap::{Parser, Subcommand};
use model::{ModelId, PlanAction};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "framegen-manager",
    version,
    about = "Install the mpv frame generation runtime"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Status,
    Plan {
        #[arg(long, value_enum, default_value = "install")]
        action: CliAction,
    },
    Install,
    Repair,
    Remove {
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Clone, clap::ValueEnum)]
pub enum CliAction {
    Install,
    Repair,
    Remove,
}

impl From<CliAction> for PlanAction {
    fn from(value: CliAction) -> Self {
        match value {
            CliAction::Install => Self::Install,
            CliAction::Repair => Self::Repair,
            CliAction::Remove => Self::Remove,
        }
    }
}

fn print_json(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn run_cli(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Status => print_json(&runtime::status()?),
        Command::Plan { action } => {
            print_json(&runtime::plan(vec![ModelId::Rife426], action.into())?)
        }
        Command::Install => print_json(&runtime::install(vec![ModelId::Rife426])?),
        Command::Repair => print_json(&runtime::repair(vec![ModelId::Rife426])?),
        Command::Remove { confirm } => {
            if !confirm {
                anyhow::bail!("removal requires --confirm after inspecting `plan --action remove`")
            }
            print_json(&runtime::remove()?)
        }
    }
}

pub fn run(cli: Cli) {
    if let Some(command) = cli.command {
        if let Err(error) = run_cli(command) {
            eprintln!("{}", serde_json::json!({ "error": format!("{error:#}") }));
            std::process::exit(1);
        }
        return;
    }
    eprintln!("A command is required. Use `framegen-manager.exe --help`.");
    std::process::exit(2);
}
