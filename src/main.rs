use clap::Parser;
use eyre::eyre;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{app::App, cli::CliConfiguration};

pub mod app;
pub mod cli;
pub mod device_finder;
pub mod event;
pub mod fileviewer;
pub mod ui;
pub mod notif;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    if let Some(path) = std::env::var_os("LOG_PATH") {
        let log = std::fs::File::create(path)?;
        fmt::Subscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .with_writer(log)
            .compact()
            .init();
    }

    let args = CliConfiguration::parse();

    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new()
        .run(
            terminal,
            args.device,
            args.default_cmd.unwrap_or_default(),
            None,
        )
        .await;
    ratatui::restore();
    result
}
