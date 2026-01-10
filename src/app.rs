use std::mem::take;

use crate::{
    device_finder::{DeviceConfigurer, DeviceFinder, Reactive},
    event::{
        AppEvent, EventHandler, FromAppMsg, FromSerialData, ToAppMsg, ToSerialData, pseudo_serial,
    },
    ui::render_ui,
};

use ratatui::{
    DefaultTerminal,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::{ScrollbarState, Widget},
};

use color_eyre::{Report, Result};
use tokio_serial::SerialStream;

#[derive(Debug)]
pub struct Status {
    pub cts: bool,
    pub dtr: bool,
    pub device: String,
    pub log: Vec<String>,
}

#[derive(Debug)]
pub struct TerminalStatus {
    // should be using something like smol_string, which is immutable and may have better perf
    // but for now we don't care
    pub text: Vec<String>,
    pub scroll_index: usize,
    pub scroll_state: ScrollbarState,
}

pub struct App {
    pub running: bool,
    pub counter: u8,
    pub events: EventHandler,
    pub term_input: String,
    pub term_state: TerminalStatus,
    pub status: Status,
    pub popup: Option<Box<dyn Reactive>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            counter: 0,
            events: EventHandler::new(),
            term_input: String::new(),
            term_state: TerminalStatus {
                text: vec!["".into()],
                scroll_index: 0,
                scroll_state: ScrollbarState::new(1).viewport_content_length(1),
            },
            status: Status {
                cts: false,
                dtr: false,
                device: "None".into(),
                log: vec![],
            },
            popup: None,
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        use ToAppMsg::{App, Crossterm, Log, RecieveSerial, SerialConnected, SerialGone};
        use crossterm::event::{Event::Key, KeyEventKind::Press};
        while self.running {
            terminal.draw(|frame| render_ui(&mut self, frame))?;
            match self.events.next().await? {
                Crossterm(Key(event)) => {
                    if event.kind == Press {
                        self.handle_key_events(event)?
                    }
                }
                Crossterm(_) => {}
                App(AppEvent::Quit) => self.running = false,
                RecieveSerial(s) => self.handle_serial(s),
                SerialGone => self.status.device = "None".into(),
                SerialConnected(s) => self.status.device = s,
                App(AppEvent::SelectDevice(s)) => {
                    self.popup = Some(Box::new(DeviceConfigurer::new(
                        s,
                        self.events.to_self.clone(),
                    )))
                }
                App(AppEvent::ConnectDevice(d)) => {
                    self.events.send(FromAppMsg::ConnectDevice(d));
                    self.popup = None;
                }
                App(AppEvent::RequestAvailableDevices) => todo!(),
                App(AppEvent::Leave) => self.popup = None,
                Log(m) => self.log_err_str(&m),
            }
        }
        Ok(())
    }

    pub fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        use KeyCode::{Char, Enter, Esc};
        if let Some(popup) = self.popup.as_mut()
            && popup.listen(crossterm::event::Event::Key(key_event))
        {
            return Ok(());
        };
        match key_event.code {
            Esc => {
                if self.popup.is_some() {
                    self.popup = None;
                } else {
                    self.events.send_self(AppEvent::Quit)
                }
            }
            Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send_self(AppEvent::Quit)
            }
            Char('s') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send(FromAppMsg::ConnectDevice(pseudo_serial()));
            }
            Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events
                    .send(FromAppMsg::WriteSerial(ToSerialData::Disconnect));
            }
            Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status.dtr = !self.status.dtr;
                self.events
                    .send(FromAppMsg::WriteSerial(ToSerialData::DTR(self.status.dtr)));
            }
            Char('t') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status.cts = !self.status.cts;
                self.events
                    .send(FromAppMsg::WriteSerial(ToSerialData::CTS(self.status.cts)));
            }
            Char('f') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.find_devices();
            }
            Char(c) if !key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.term_input.push(c)
            }
            Enter => self.enter_input(),
            _ => {}
        }
        Ok(())
    }

    fn handle_serial(&mut self, message: Result<FromSerialData>) {
        use FromSerialData::{Data, Status};
        match message {
            Ok(Data(s)) => {
                for line in String::from_utf8_lossy(&s).split_inclusive('\n') {
                    if self.term_state.text.last().unwrap().ends_with('\n') {
                        self.term_state.text.push(line.into());
                    } else {
                        self.term_state.text.last_mut().unwrap().push_str(line);
                    }
                }
            }
            Ok(Status { dtr, cts }) => {
                self.status.dtr = dtr;
                self.status.cts = cts;
            }
            Err(e) => self.log_err(e),
        }
    }

    fn enter_input(&mut self) {
        self.term_input.push('\n');
        self.events
            .send(FromAppMsg::WriteSerial(ToSerialData::Data(take(
                &mut self.term_input,
            ))));
    }

    fn log_err(&mut self, report: Report) {
        self.status.log.push(report.to_string());
    }

    fn log_err_str(&mut self, report: &str) {
        self.status.log.push(report.into());
    }

    fn find_devices(&mut self) {
        use serialport::SerialPortType::{BluetoothPort, PciPort, Unknown, UsbPort};
        if self.popup.is_some() {
            return;
        }
        let devices = tokio_serial::available_ports();
        let devices: Vec<_> = match devices {
            Ok(d) => d,
            Err(e) => {
                self.log_err(e.into());
                return;
            }
        }
        .into_iter()
        .filter(|e| match e.port_type {
            UsbPort(_) | BluetoothPort => true,
            PciPort | Unknown => false,
        })
        .collect();
        if devices.is_empty() {
            self.log_err_str("No devices found");
        } else {
            self.popup = Some(Box::new(DeviceFinder::new(
                devices,
                self.events.to_self.clone(),
            )));
        }
    }
}
