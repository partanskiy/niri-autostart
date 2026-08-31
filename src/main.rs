mod config;
mod error;
mod event_adapter;
mod ipc;
mod paths;
mod predicate;
mod reconcile;
mod reducer;
mod runtime_state;
mod state;

use clap::Parser;

use crate::config::{Cli, Config, resolve_config_path};
use crate::error::Result;
use crate::event_adapter::EventAdapter;
use crate::ipc::CommandClient;
use crate::reconcile::Reconciler;
use crate::runtime_state::RuntimeState;

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = resolve_config_path(&cli)?;
    let config = Config::load(&config_path)?;
    let runtime_state_path = match &cli.state {
        Some(path) => path.clone(),
        None => RuntimeState::default_path()?,
    };
    let runtime_state = RuntimeState::load(&runtime_state_path)?;

    let events = EventAdapter::connect(std::time::Duration::from_secs(10))?;
    let commands = CommandClient::connect()?;

    let mut reconciler = Reconciler::new(commands, events, runtime_state, runtime_state_path);
    reconciler.run(&config)?;

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("niri-autostart: {err}");
        std::process::exit(1);
    }
}
