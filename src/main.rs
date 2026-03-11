mod config;
mod error;
mod ipc;
mod predicate;
mod reconcile;
mod reducer;
mod state;

use clap::Parser;

use crate::config::{Cli, Config, resolve_config_path};
use crate::error::Result;
use crate::ipc::{CommandClient, EventStream};
use crate::reconcile::{Reconciler, bootstrap_initial_state};

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = resolve_config_path(&cli)?;
    let config = Config::load(&config_path)?;

    let events = EventStream::connect()?;
    let mut state = bootstrap_initial_state(&events.rx, std::time::Duration::from_secs(10))?;
    let mut commands = CommandClient::connect()?;
    state.set_outputs(commands.outputs()?);

    let mut reconciler = Reconciler::new(commands, events.rx, state);
    reconciler.run(&config)?;

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("niri-autostart: {err}");
        std::process::exit(1);
    }
}
