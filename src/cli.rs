use clap::{ArgAction, Parser};
use color_eyre::Result;
use eyre::eyre;
use notify::event::DataChange;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::{fmt::Display, path::PathBuf};

use crate::device_finder::{Baud, DeviceConfig};

#[derive(Debug, Parser)]
#[command(version, about, long_about)]
/// Seterm configuration is done primarily through TUI, although defaults can be set via the commandline.
/// Use the keyboard to enter input, and use alt+? to view the help menu. Use ctrl+c to quit the application,
/// or ESC to close a popup. Use ctrl+f to find and connect a device, and ctrl+u to select a file to upload.
/// Use ctrl+d to toggle DTR, and ctrl+r to toggle rts.
pub struct CliConfiguration {
    #[arg(long, help = "Default binary to upload")]
    pub watch_path: Option<PathBuf>,
    #[arg(short = 'c', long, help = "Default upload command")]
    pub default_cmd: Option<String>,
    #[command(flatten)]
    pub device: DeviceOptions,
}

impl Display for Baud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", *self as usize)
    }
}

#[derive(Clone, Debug, Parser)]
pub struct DeviceOptions {
    #[arg(long, help = "Device serial port")]
    pub path: Option<PathBuf>,
    #[arg(short = 'b', long, value_parser = parse_baud, default_value = "1152k")]
    pub baud: Baud,
    #[arg(short = 'd', long, value_parser = parse_data, default_value = "8", help = "Bits per word")]
    pub bits: DataBits,
    #[arg(short = 'f', long, value_parser = parse_flow, default_value = "none", help = "How control flow works")]
    pub flow: FlowControl,
    #[arg(short = 'p', long, value_parser = parse_parity, default_value = "none", help = "If parity bits are emitted")]
    pub parity: Parity,
    #[arg(short = 's', long, value_parser = parse_stop, default_value = "1", help = "How many bits to end a word")]
    pub stop: StopBits,
    #[arg(short = 'r', long="no-dtr", long, default_value_t = true, action = ArgAction::SetFalse, help = "Whether DTR is asserted on start or not")]
    pub dtr: bool,
}

impl Default for DeviceOptions {
    fn default() -> Self {
        Self {
            path: None,
            baud: Baud::B1152,
            bits: DataBits::Eight,
            flow: FlowControl::None,
            parity: Parity::None,
            stop: StopBits::One,
            dtr: true,
        }
    }
}

impl DeviceOptions {
    pub fn to_config(&mut self) -> Option<DeviceConfig> {
        let path = self.path.take()?;
        Some(DeviceConfig {
            path,
            baud: self.baud,
            bits: self.bits,
            flow: self.flow,
            parity: self.parity,
            stop: self.stop,
            dtr: self.dtr,
        })
    }

    pub fn to_config_path(&self, path: PathBuf) -> DeviceConfig {
        DeviceConfig {
            path,
            baud: self.baud,
            bits: self.bits,
            flow: self.flow,
            parity: self.parity,
            stop: self.stop,
            dtr: self.dtr,
        }
    }
}

fn parse_baud(arg: &str) -> Result<Baud> {
    use Baud::{B48, B96, B192, B384, B576, B1152};
    match arg {
        "4800" | "48k" => Ok(B48),
        "9600" | "96k" => Ok(B96),
        "19200" | "192k" => Ok(B192),
        "38400" | "384k" => Ok(B384),
        "57600" | "576k" => Ok(B576),
        "115200" | "1152k" => Ok(B1152),
        _ => Err(eyre!(
            "Valid baud rates are: 48k, 96k, 192k, 384k, 576k, 1152k"
        )),
    }
}

fn parse_data(arg: &str) -> Result<DataBits> {
    match arg {
        "5" => Ok(DataBits::Five),
        "6" => Ok(DataBits::Six),
        "7" => Ok(DataBits::Seven),
        "8" => Ok(DataBits::Eight),
        _ => Err(eyre!("Not a valid number of data bits: (5, 6, 7, 8)")),
    }
}

fn parse_flow(arg: &str) -> Result<FlowControl> {
    match arg {
        "none" => Ok(FlowControl::None),
        "software" | "xonxoff" => Ok(FlowControl::Software),
        "hardware" | "rtscts" => Ok(FlowControl::Hardware),
        _ => Err(eyre!("Not a valid flow control (none, software, hardware)")),
    }
}

fn parse_parity(arg: &str) -> Result<Parity> {
    match arg {
        "none" => Ok(Parity::None),
        "odd" => Ok(Parity::Odd),
        "even" => Ok(Parity::Even),
        _ => Err(eyre!("Not a valid parity (none, odd, even)")),
    }
}

fn parse_stop(arg: &str) -> Result<StopBits> {
    match arg {
        "1" => Ok(StopBits::One),
        "2" => Ok(StopBits::Two),
        _ => Err(eyre!("Not a valid number of stop bits (1 or 2)")),
    }
}
