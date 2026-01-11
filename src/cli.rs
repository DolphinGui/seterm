use clap::Parser;
use color_eyre::Result;
use eyre::eyre;
use std::{fmt::Display, path::PathBuf};

use crate::device_finder::Baud;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct CliConfiguration {
    #[arg(long)]
    pub watch_path: Option<PathBuf>,
    #[arg(long)]
    pub device: Option<PathBuf>,
    #[arg(short = 'b', long, value_parser = parse_baud, default_value_t = Baud::B1152)]
    pub default_baud: Baud,
}

impl Display for Baud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", *self as usize)
    }
}

fn parse_baud(arg: &str) -> Result<Baud> {
    use Baud::{B48, B96, B192, B384, B576, B1152};
    let n: usize = arg
        .parse()
        .map_err(|e| eyre!("Issue parsing baud rate: {}!", e))?;
    match n {
        4800 | 48 => Ok(B48),
        9600 | 96 => Ok(B96),
        19200 | 192 => Ok(B192),
        38400 | 384 => Ok(B384),
        57600 | 576 => Ok(B576),
        115200 | 1152 => Ok(B1152),
        _ => Err(eyre!("Not a valid baud rate!")),
    }
}
