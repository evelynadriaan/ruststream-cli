mod audio;
mod cli;
mod config;
mod daemon;
mod db;
mod download;
mod ipc;
mod models;
mod tui;

use anyhow::Result;
use clap::Parser;
use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;

use cli::{App, Cli, Commands, DaemonCommands};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let app = App::new()?;

    let command = match cli.command {
        Some(command) => command,
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                Commands::Tui
            } else {
                anyhow::bail!(
                    "No subcommand provided in non-interactive mode. Use: clistream help"
                );
            }
        }
    };

    match command {
        Commands::Add { url, alias } => {
            app.add(&url, alias.as_deref())?;
        }
        Commands::Remove { query } => {
            app.remove(&query)?;
        }
        Commands::Play { query } => {
            app.play(&query)?;
        }
        Commands::Pause => {
            app.pause()?;
        }
        Commands::Resume => {
            app.resume()?;
        }
        Commands::Stop => {
            app.stop()?;
        }
        Commands::Seek { position } => {
            app.seek(&position)?;
        }
        Commands::Volume { level } => {
            app.volume(level)?;
        }
        Commands::List => {
            app.list()?;
        }
        Commands::Search { query } => {
            app.search(&query)?;
        }
        Commands::Status => {
            app.status()?;
        }
        Commands::Daemon { command } => match command {
            DaemonCommands::Start => {
                app.daemon_start()?;
            }
            DaemonCommands::Stop => {
                app.daemon_stop()?;
            }
            DaemonCommands::Status => {
                app.daemon_status()?;
            }
            DaemonCommands::Run => {
                app.daemon_run()?;
            }
        },
        Commands::Export { file } => {
            app.export(file.as_deref())?;
        }
        Commands::Import { file } => {
            app.import(&file)?;
        }
        Commands::Check => {
            app.check()?;
        }
        Commands::Tui => {
            // Ensure daemon is running for playback
            let client = ipc::DaemonClient::new(app.config.socket_path());
            if !client.is_daemon_running() {
                daemon::Daemon::start_detached(&app.config)?;
            }
            tui::run(app.config.clone(), app.db)?;
        }
    }

    Ok(())
}
