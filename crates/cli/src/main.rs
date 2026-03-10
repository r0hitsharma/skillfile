mod commands;
mod patch;

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use skillfile_core::error::SkillfileError;

#[derive(Parser)]
#[command(
    name = "skillfile",
    about = "Tool-agnostic AI skill & agent manager",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch community entries into .skillfile/ (without deploying)
    Sync {
        /// Show planned actions without fetching
        #[arg(long)]
        dry_run: bool,
        /// Sync only this named entry
        #[arg(long, value_name = "NAME")]
        entry: Option<String>,
        /// Re-resolve all refs and update the lock
        #[arg(long)]
        update: bool,
    },

    /// Show state of all entries
    Status {
        /// Check current upstream SHA (makes API calls)
        #[arg(long)]
        check_upstream: bool,
    },

    /// Configure install targets interactively
    Init,

    /// Fetch entries and deploy to platform directories
    Install {
        /// Show planned actions without fetching or installing
        #[arg(long)]
        dry_run: bool,
        /// Re-resolve all refs and update the lock
        #[arg(long)]
        update: bool,
    },

    /// Add an entry to the Skillfile
    Add {
        #[command(subcommand)]
        source: AddSource,
    },

    /// Remove an entry from the Skillfile
    Remove {
        /// Entry name to remove
        name: String,
    },

    /// Check the Skillfile for errors
    Validate,

    /// Sort and canonicalize the Skillfile in-place
    Sort {
        /// Print sorted output without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Capture your edits to an installed entry so they survive upstream updates
    Pin {
        /// Entry name to pin
        name: String,
        /// Show what would be pinned without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Discard pinned customisations and restore pure upstream on next install
    Unpin {
        /// Entry name to unpin
        name: String,
    },

    /// Show local changes (or upstream delta after a conflict)
    Diff {
        /// Entry name
        name: String,
    },

    /// Merge upstream changes with your customisations after a conflict
    Resolve {
        /// Entry name to resolve
        name: Option<String>,
        /// Clear pending conflict state without merging
        #[arg(long)]
        abort: bool,
    },
}

#[derive(Subcommand)]
enum AddSource {
    /// Add a GitHub-hosted entry
    Github {
        /// skill or agent
        #[arg(value_name = "TYPE")]
        entity_type: String,
        /// GitHub repository (e.g. VoltAgent/repo)
        #[arg(value_name = "OWNER/REPO")]
        owner_repo: String,
        /// Path to the .md file within the repo
        #[arg(value_name = "PATH")]
        path: String,
        /// Branch, tag, or SHA (default: main)
        #[arg(value_name = "REF")]
        ref_: Option<String>,
        /// Override name (default: filename stem)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Add a local file entry
    Local {
        /// skill or agent
        #[arg(value_name = "TYPE")]
        entity_type: String,
        /// Path to the .md file relative to repo root
        #[arg(value_name = "PATH")]
        path: String,
        /// Override name (default: filename stem)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Add a URL entry
    Url {
        /// skill or agent
        #[arg(value_name = "TYPE")]
        entity_type: String,
        /// Direct URL to the .md file
        #[arg(value_name = "URL")]
        url: String,
        /// Override name (default: filename stem)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

fn run() -> Result<(), SkillfileError> {
    let cli = Cli::parse();
    let repo_root = PathBuf::from(".");

    match cli.command {
        Command::Sync {
            dry_run,
            entry,
            update,
        } => {
            skillfile_sources::sync::cmd_sync(&repo_root, dry_run, entry.as_deref(), update)?;
        }
        Command::Status { check_upstream } => {
            commands::status::cmd_status(&repo_root, check_upstream)?;
        }
        Command::Init => {
            commands::init::cmd_init(&repo_root)?;
        }
        Command::Install { dry_run, update } => {
            skillfile_deploy::install::cmd_install(&repo_root, dry_run, update)?;
        }
        Command::Add { source } => {
            let entry = match source {
                AddSource::Github {
                    entity_type,
                    owner_repo,
                    path,
                    ref_,
                    name,
                } => commands::add::entry_from_github(
                    &entity_type,
                    &owner_repo,
                    &path,
                    ref_.as_deref(),
                    name.as_deref(),
                ),
                AddSource::Local {
                    entity_type,
                    path,
                    name,
                } => commands::add::entry_from_local(&entity_type, &path, name.as_deref()),
                AddSource::Url {
                    entity_type,
                    url,
                    name,
                } => commands::add::entry_from_url(&entity_type, &url, name.as_deref()),
            };
            commands::add::cmd_add(entry, &repo_root)?;
        }
        Command::Remove { name } => {
            commands::remove::cmd_remove(&name, &repo_root)?;
        }
        Command::Validate => {
            commands::validate::cmd_validate(&repo_root)?;
        }
        Command::Sort { dry_run } => {
            commands::sort::cmd_sort(&repo_root, dry_run)?;
        }
        Command::Pin { name, dry_run } => {
            commands::pin::cmd_pin(&name, &repo_root, dry_run)?;
        }
        Command::Unpin { name } => {
            commands::pin::cmd_unpin(&name, &repo_root)?;
        }
        Command::Diff { name } => {
            commands::diff::cmd_diff(&name, &repo_root)?;
        }
        Command::Resolve { name, abort } => {
            commands::resolve::cmd_resolve(name.as_deref(), abort, &repo_root)?;
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        let msg = e.to_string();
        if !msg.is_empty() {
            eprintln!("error: {msg}");
        }
        process::exit(1);
    }
}
