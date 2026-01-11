use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::Text,
    widgets::{
        Block, Borders, List, ListState, Paragraph, Row, StatefulWidget, Table, TableState, Widget,
    },
};
use serialport::{DataBits, FlowControl, Parity, SerialPortInfo, StopBits};
use tokio::sync::mpsc;
use tokio_serial::SerialPortBuilderExt;

use crate::event::{AppEvent, ToAppMsg};

pub trait EventListener {
    fn listen(&mut self, e: Event) -> bool;
}

pub trait Drawable {
    fn draw(&mut self, area: Rect, buf: &mut Buffer);
}

pub trait Reactive: EventListener + Drawable {}

impl<T> Reactive for T where T: EventListener + Drawable {}

pub struct DeviceFinder {
    devices: Vec<SerialPortInfo>,
    state: ListState,
    to_app: mpsc::UnboundedSender<ToAppMsg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Baud {
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

pub struct DeviceConfigurer {
    table_state: TableState,
    path: String,
    baud: Baud,
    bits: DataBits,
    flow: FlowControl,
    parity: Parity,
    stop: StopBits,
    dtr: bool,
    to_app: mpsc::UnboundedSender<ToAppMsg>,
}

impl DeviceFinder {
    pub fn new(
        devices: Vec<SerialPortInfo>,
        to_app: mpsc::UnboundedSender<ToAppMsg>,
    ) -> DeviceFinder {
        Self {
            devices,
            state: ListState::default(),
            to_app,
        }
    }
}

impl EventListener for DeviceFinder {
    fn listen(&mut self, e: Event) -> bool {
        use Event::Key;
        use KeyCode::{Down, Enter, Up};
        let mut handled = true;
        match e {
            Key(KeyEvent { code: Up, .. }) => self.state.scroll_up_by(1),
            Key(KeyEvent { code: Down, .. }) => self.state.scroll_down_by(1),
            Key(KeyEvent { code: Enter, .. }) => {
                if let Some(d) = self.state.selected().and_then(|i| self.devices.get(i)) {
                    _ = self
                        .to_app
                        .send(ToAppMsg::App(AppEvent::SelectDevice(d.port_name.clone())));
                };
            }
            _ => handled = false,
        };
        handled
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
    fn draw(&mut self, area: Rect, buf: &mut Buffer) {
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
        <List as StatefulWidget>::render(l, area, buf, &mut self.state);
    }
}

impl DeviceConfigurer {
    pub fn new(path: String, to_app: mpsc::UnboundedSender<ToAppMsg>) -> Self {
        Self {
            path,
            table_state: TableState::new(),
            baud: Baud::B1152,
            bits: DataBits::Eight,
            flow: FlowControl::None,
            parity: Parity::None,
            stop: StopBits::One,
            dtr: true,
            to_app,
        }
    }

    fn select(&mut self, inc: isize) {
        let col = self.table_state.selected().unwrap_or(1) - 1;
        let index = match col {
            0 => baud_idx(self.baud),
            1 => self.bits as isize,
            2 => self.flow as isize,
            3 => self.parity as isize,
            4 => self.stop as isize,
            5 => {
                if self.dtr {
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
            0 => self.baud = BAUDS[i],
            1 => self.bits = DATABITSS[i],
            2 => self.flow = FLOWCONTROLS[i],
            3 => self.parity = PARITYS[i],
            4 => self.stop = STOPBITSS[i],
            5 => self.dtr = i != 0,
            _ => panic!("Invalid enum passed in"),
        }
    }
}

impl EventListener for DeviceConfigurer {
    fn listen(&mut self, e: Event) -> bool {
        use Event::Key;
        use KeyCode::{Down, Enter, Left, Right, Up};
        let mut handled = true;
        match e {
            Key(KeyEvent { code: Up, .. }) => {
                if self.table_state.selected().unwrap_or(0) <= 1 {
                    self.table_state.select(Some(1))
                } else {
                    self.table_state.scroll_up_by(1)
                }
            }
            Key(KeyEvent { code: Down, .. }) => self.table_state.scroll_down_by(1),
            Key(KeyEvent { code: Left, .. }) => self.select(-1),
            Key(KeyEvent { code: Right, .. }) => self.select(1),
            Key(KeyEvent { code: Enter, .. }) => {
                let config = tokio_serial::new(self.path.clone(), self.baud as u32)
                    .data_bits(self.bits)
                    .flow_control(self.flow)
                    .parity(self.parity)
                    .stop_bits(self.stop)
                    .dtr_on_open(self.dtr);
                match config.open_native_async() {
                    Ok(s) => {
                        _ = self
                            .to_app
                            .send(ToAppMsg::App(AppEvent::ConnectDevice(s, self.path.clone())))
                    }
                    Err(e) => _ = self.to_app.send(ToAppMsg::Log(e.description)),
                };
            }
            _ => handled = false,
        };
        handled
    }
}

impl Drawable for DeviceConfigurer {
    fn draw(&mut self, area: Rect, buf: &mut Buffer) {
        let [opt_area, desc_area] =
            &*Layout::vertical([Constraint::Percentage(80), Constraint::Percentage(20)])
                .split(area)
        else {
            panic!("Device configurer failed to configure");
        };

        let bauds = format!("{}", self.baud as usize);
        let dtr = format!("{}", self.dtr);
        let rows = [
            Row::new([
                Text::raw("Path").left_aligned(),
                Text::raw(&self.path).centered(),
            ]),
            Row::new([
                Text::raw("Baud Rate").left_aligned(),
                Text::raw(&bauds).centered(),
            ]),
            Row::new([
                Text::raw("Bits per Word").left_aligned(),
                Text::raw(DATABIT_STRS[self.bits as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Flow Control").left_aligned(),
                Text::raw(FLOWCONTROL_STRS[self.flow as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Parity Bits").left_aligned(),
                Text::raw(PARITY_STRS[self.parity as usize]).centered(),
            ]),
            Row::new([
                Text::raw("Stop Bits").left_aligned(),
                Text::raw(STOPBIT_STRS[self.stop as usize]).centered(),
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

        <Table as StatefulWidget>::render(table, *opt_area, buf, &mut self.table_state);

        let description = Paragraph::new(
            "Left/Right to change option\nUp/Down to select option\nEnter to connect\nEsc to exit",
        )
        .block(Block::new().borders(Borders::all().difference(Borders::TOP)))
        .centered();
        description.render(*desc_area, buf);
    }
}
