use std::mem::take;

use clap::ValueEnum;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::Text,
    widgets::{
        Block, Borders, Clear, List, ListState, Paragraph, Row, StatefulWidget, Table, TableState,
        Widget,
    },
};
use serialport::{DataBits, FlowControl, Parity, SerialPortInfo, StopBits};
use tokio::sync::oneshot;

use color_eyre::Result;
use eyre::eyre;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use crate::event::{Drawable, EventListener, GuiEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Baud {
    B48 = 4800,
    B96 = 9600,
    B192 = 19200,
    B384 = 38400,
    B576 = 57600,
    B1152 = 115200,
}

const BAUDS: [Baud; 6] = [
    Baud::B48,
    Baud::B96,
    Baud::B192,
    Baud::B384,
    Baud::B576,
    Baud::B1152,
];

fn baud_idx(b: Baud) -> isize {
    match b {
        Baud::B48 => 0,
        Baud::B96 => 1,
        Baud::B192 => 2,
        Baud::B384 => 3,
        Baud::B576 => 4,
        Baud::B1152 => 5,
    }
}

const DATABITSS: [DataBits; 4] = [
    DataBits::Five,
    DataBits::Six,
    DataBits::Seven,
    DataBits::Eight,
];
const DATABIT_STRS: [&str; 4] = ["5", "6", "7", "8"];

const FLOWCONTROLS: [FlowControl; 3] = [
    FlowControl::None,
    FlowControl::Software,
    FlowControl::Hardware,
];
const FLOWCONTROL_STRS: [&str; 3] = ["None", "Software", "Hardware"];

const PARITYS: [Parity; 3] = [Parity::None, Parity::Odd, Parity::Even];
const PARITY_STRS: [&str; 3] = ["None", "Odd", "Even"];

const STOPBITSS: [StopBits; 2] = [StopBits::One, StopBits::Two];
const STOPBIT_STRS: [&str; 2] = ["1", "2"];

pub struct DeviceFinder {
    devices: Vec<SerialPortInfo>,
    state: ListState,
    tx: Option<oneshot::Sender<String>>,
}

impl DeviceFinder {
    pub fn new() -> Result<(DeviceFinder, oneshot::Receiver<String>)> {
        let devices: Vec<_> = tokio_serial::available_ports()?
            .into_iter()
            .filter(|i| {
                matches!(
                    &i.port_type,
                    serialport::SerialPortType::UsbPort(_)
                        | serialport::SerialPortType::BluetoothPort
                )
            })
            .collect();
        if devices.is_empty() {
            return Err(eyre!("Found no serial devices"));
        }
        let (tx, rx) = oneshot::channel();

        Ok((
            Self {
                devices,
                state: ListState::default(),
                tx: Some(tx),
            },
            rx,
        ))
    }
}

impl EventListener for DeviceFinder {
    fn listen(&mut self, e: &GuiEvent) -> bool {
        use GuiEvent::{Crossterm, SerialDone};
        use KeyCode::{Down, Enter, Up};
        use crossterm::event::Event::Key;
        match e {
            Crossterm(Key(KeyEvent { code: Up, .. })) => self.state.scroll_up_by(1),
            Crossterm(Key(KeyEvent { code: Down, .. })) => self.state.scroll_down_by(1),
            Crossterm(Key(KeyEvent { code: Enter, .. })) => {
                if let Some(d) = self.state.selected().and_then(|i| self.devices.get(i))
                    && let Some(tx) = self.tx.take()
                {
                    _ = tx.send(d.port_name.clone());
                };
            }
            SerialDone => {
                self.tx = None;
                return false;
            }
            _ => return false,
        };
        true
    }
}

fn format_device_info(info: &SerialPortInfo) -> String {
    use serialport::SerialPortType::{BluetoothPort, PciPort, Unknown, UsbPort};
    match info.port_type {
        UsbPort(ref usb) => {
            let values: Vec<_> = [&usb.serial_number, &usb.product, &usb.manufacturer]
                .iter()
                .filter_map(|a| a.as_deref())
                .collect();
            format!("{} ({})", info.port_name, values.join(", "))
        }
        PciPort => format!("{} (PCI)", info.port_name),
        BluetoothPort => format!("{} (Bluetooth)", info.port_name),
        Unknown => info.port_name.clone(),
    }
}

impl Drawable for DeviceFinder {
    fn draw(&mut self, area: Rect, frame: &mut Frame) {
        let text: Vec<_> = self
            .devices
            .iter()
            .map(format_device_info)
            .map(Text::raw)
            .collect();
        let highlight_style = Style::default().reversed();
        let l = List::new(text)
            .block(Block::bordered())
            .highlight_style(highlight_style);

        frame.render_widget(Clear, area);
        frame.render_stateful_widget(l, area, &mut self.state);
    }
    fn alive(&self) -> bool {
        self.tx.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct DeviceConfig {
    path: String,
    baud: Baud,
    bits: DataBits,
    flow: FlowControl,
    parity: Parity,
    stop: StopBits,
    dtr: bool,
}

impl DeviceConfig {
    pub fn to_serial(self) -> Result<SerialStream> {
        tokio_serial::new(self.path, self.baud as u32)
            .data_bits(self.bits)
            .flow_control(self.flow)
            .parity(self.parity)
            .stop_bits(self.stop)
            .dtr_on_open(self.dtr)
            .open_native_async()
            .map_err(|e| e.into())
    }
}

pub struct DeviceConfigurer {
    config: DeviceConfig,
    table_state: TableState,
    tx: Option<oneshot::Sender<DeviceConfig>>,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self::new(String::default(), Baud::B1152)
    }
}

impl DeviceConfig {
    fn new(path: String, baud: Baud) -> Self {
        Self {
            path,
            baud,
            bits: DataBits::Eight,
            flow: FlowControl::None,
            parity: Parity::None,
            stop: StopBits::One,
            dtr: true,
        }
    }
}

impl DeviceConfigurer {
    pub fn new(path: String, baud: Baud) -> (Self, oneshot::Receiver<DeviceConfig>) {
        let (tx, rx) = oneshot::channel();
        let tx = Some(tx);
        (
            Self {
                config: DeviceConfig::new(path, baud),
                table_state: TableState::new(),
                tx,
            },
            rx,
        )
    }

    fn select(&mut self, inc: isize) {
        let col = self.table_state.selected().unwrap_or(1) - 1;
        let index = match col {
            0 => baud_idx(self.config.baud),
            1 => self.config.bits as isize,
            2 => self.config.flow as isize,
            3 => self.config.parity as isize,
            4 => self.config.stop as isize,
            5 => {
                if self.config.dtr {
                    1
                } else {
                    0
                }
            }
            _ => panic!("Invalid enum passed in"),
        };
        let max = match col {
            0 => BAUDS.len(),
            1 => DATABITSS.len(),
            2 => FLOWCONTROLS.len(),
            3 => PARITYS.len(),
            4 => STOPBITSS.len(),
            5 => 2,
            _ => panic!("Invalid enum passed in"),
        };
        let i: usize = index
            .strict_add(inc)
            .rem_euclid(max as isize)
            .try_into()
            .unwrap();

        match col {
            0 => self.config.baud = BAUDS[i],
            1 => self.config.bits = DATABITSS[i],
            2 => self.config.flow = FLOWCONTROLS[i],
            3 => self.config.parity = PARITYS[i],
            4 => self.config.stop = STOPBITSS[i],
            5 => self.config.dtr = i != 0,
            _ => panic!("Invalid enum passed in"),
        }
    }
}

impl EventListener for DeviceConfigurer {
    fn listen(&mut self, e: &GuiEvent) -> bool {
        use GuiEvent::{Crossterm, SerialDone};
        use KeyCode::{Down, Enter, Left, Right, Up};
        use crossterm::event::Event::Key;
        match e {
            Crossterm(Key(KeyEvent { code: Up, .. })) => {
                if self.table_state.selected().unwrap_or(0) <= 1 {
                    self.table_state.select(Some(1))
                } else {
                    self.table_state.scroll_up_by(1)
                }
            }
            Crossterm(Key(KeyEvent { code: Down, .. })) => self.table_state.scroll_down_by(1),
            Crossterm(Key(KeyEvent { code: Left, .. })) => self.select(-1),
            Crossterm(Key(KeyEvent { code: Right, .. })) => self.select(1),
            Crossterm(Key(KeyEvent { code: Enter, .. })) => {
                if let Some(tx) = self.tx.take() {
                    _ = tx.send(take(&mut self.config));
                }
            }
            SerialDone => {
                self.tx = None;
                return false;
            }
            _ => {
                return false;
            }
        };
        true
    }
}

impl Drawable for DeviceConfigurer {
    fn draw(&mut self, area: Rect, frame: &mut Frame) {
        let [opt_area, desc_area] =
            &*Layout::vertical([Constraint::Percentage(80), Constraint::Percentage(20)])
                .split(area)
        else {
            panic!("Device configurer failed to configure");
        };

        let bauds = format!("{}", self.config.baud as usize);
        let dtr = format!("{}", self.config.dtr);
        let rows = [
            Row::new([
                Text::raw("Path").left_aligned(),
                Text::raw(&self.config.path).centered(),
            ]),
            Row::new([
                Text::raw("Baud Rate").left_aligned(),
                Text::raw(&bauds).centered(),
            ]),
            Row::new([
                Text::raw("Bits per Word").left_aligned(),
                Text::raw(DATABIT_STRS[self.config.bits as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Flow Control").left_aligned(),
                Text::raw(FLOWCONTROL_STRS[self.config.flow as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Parity Bits").left_aligned(),
                Text::raw(PARITY_STRS[self.config.parity as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Stop Bits").left_aligned(),
                Text::raw(STOPBIT_STRS[self.config.stop as usize]).centered(),
            ]),
            Row::new([
                Text::raw("DTR on start").left_aligned(),
                Text::raw(&dtr).centered(),
            ]),
        ];
        let widths = [Constraint::Percentage(30), Constraint::Percentage(70)];
        let table = Table::new(rows, widths)
            .block(Block::new().borders(Borders::all().difference(Borders::BOTTOM)))
            .row_highlight_style(Style::new().reversed());

        frame.render_widget(Clear, area);
        frame.render_stateful_widget(table, *opt_area, &mut self.table_state);

        let description = Paragraph::new(
            "Left/Right to change option\nUp/Down to select option\nEnter to connect\nEsc to exit",
        )
        .block(Block::new().borders(Borders::all().difference(Borders::TOP)))
        .centered();

        frame.render_widget(description, *desc_area);
    }

    fn alive(&self) -> bool {
        self.tx.is_some()
    }
}
