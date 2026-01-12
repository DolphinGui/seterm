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
    let dev = args.device.map(|p| p.into_os_string().into_string());
    let dev = match dev {
        Some(Ok(d)) => Some(d),
        Some(Err(_)) => return Err(eyre!("Unable to parse non-utf8 strings!")),
        None => None,
    };
    let result = App::new()
        .run(terminal, args.default_baud, dev, "".into())
        .await;
    ratatui::restore();
    result
}
