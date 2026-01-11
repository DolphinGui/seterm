use std::path::PathBuf;

use clap::Parser;
use eyre::eyre;

use crate::{app::App, cli::CliConfiguration};

pub mod app;
pub mod cli;
pub mod device_finder;
pub mod event;
pub mod ui;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let args = CliConfiguration::parse();

    color_eyre::install()?;
    let terminal = ratatui::init();
    let dev = args.device.map(|p| p.into_os_string().into_string());
    let dev = match dev {
        Some(Ok(d)) => Some(d),
        Some(Err(_)) => return Err(eyre!("Unable to parse non-utf8 strings!")),
        None => None,
    };
    let result = App::new().run(terminal, args.default_baud, dev).await;
    ratatui::restore();
    result
}
