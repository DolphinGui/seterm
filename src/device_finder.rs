use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Text,
    widgets::{Block, List, ListState, StatefulWidget, Widget},
};
use serialport::{DataBits, FlowControl, Parity, SerialPortBuilder, SerialPortInfo, StopBits};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

use crate::event::AppEvent;

pub trait EventListener {
    fn listen(&mut self, e: Event) -> Option<AppEvent>;
}

pub trait Drawable {
    fn draw(&mut self, area: Rect, buf: &mut Buffer);
}

pub trait Reactive: EventListener + Drawable {}

impl<T> Reactive for T where T: EventListener + Drawable {}

pub struct DeviceFinder {
    devices: Vec<SerialPortInfo>,
    state: ListState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigSelection {
    Baud,
    Bits,
    Flow,
    Parity,
    Stop,
    Dtr,
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

pub struct DeviceConfigurer {
    selection: ConfigSelection,
    path: String,
    baud: Baud,
    bits: DataBits,
    flow: FlowControl,
    parity: Parity,
    stop: StopBits,
    dtr: bool,
}

impl DeviceFinder {
    pub fn new(devices: Vec<SerialPortInfo>) -> DeviceFinder {
        Self {
            devices,
            state: ListState::default(),
        }
    }
}

impl EventListener for DeviceFinder {
    fn listen(&mut self, e: Event) -> Option<AppEvent> {
        use Event::Key;
        use KeyCode::{Down, Enter, Up};
        match e {
            Key(KeyEvent { code: Up, .. }) => self.state.scroll_up_by(1),
            Key(KeyEvent { code: Down, .. }) => self.state.scroll_down_by(1),
            Key(KeyEvent { code: Enter, .. }) => {
                return self
                    .state
                    .selected()
                    .and_then(|i| self.devices.get(i))
                    .map(|s| AppEvent::SelectDevice(s.port_name.clone()));
            }
            _ => {}
        };
        None
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
        let highlight_style = Style::default().fg(Color::Black).bg(Color::White);
        let l = List::new(text)
            .block(Block::bordered())
            .highlight_style(highlight_style);
        <List as StatefulWidget>::render(l, area, buf, &mut self.state);
    }
}

impl DeviceConfigurer {
    pub fn new(path: String) -> Self {
        Self {
            path,
            selection: ConfigSelection::Baud,
            baud: Baud::B1152,
            bits: DataBits::Eight,
            flow: FlowControl::Hardware,
            parity: Parity::None,
            stop: StopBits::One,
            dtr: true,
        }
    }
}

impl EventListener for DeviceConfigurer {
    fn listen(&mut self, e: Event) -> Option<AppEvent> {
        use Event::Key;
        use KeyCode::{Down, Enter, Up};
        match e {
            Event::FocusGained => todo!(),
            Event::FocusLost => todo!(),
            Key(key_event) => todo!(),
            Event::Mouse(mouse_event) => todo!(),
            Event::Paste(_) => todo!(),
            Event::Resize(_, _) => todo!(),
        }

        todo!()
    }
}
