use std::mem::take;

use crate::{
    event::{pseudo_serial, AppEvent, EventHandler, FromAppMsg, ToAppMsg},
    ui::render_ui,
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::ScrollbarState,
    DefaultTerminal,
};

use color_eyre::{Report, Result};

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

#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub counter: u8,
    pub events: EventHandler,
    pub term_input: String,
    pub term_state: TerminalStatus,
    pub status: Status,
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
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        use crossterm::event::{Event::Key, KeyEventKind::Press};
        use ToAppMsg::{App, Crossterm};
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
                ToAppMsg::RecieveSerial(s) => self.handle_serial(s),
                ToAppMsg::SerialGone => self.status.device = "None".into(),
                ToAppMsg::SerialConnected(s) => self.status.device = s,
            }
        }
        Ok(())
    }

    pub fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        use KeyCode::{Char, Enter, Esc};
        match key_event.code {
            Esc => self.events.send_self(AppEvent::Quit),
            Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send_self(AppEvent::Quit)
            }
            Char('s') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send(FromAppMsg::ConnectDevice(pseudo_serial()));
            }
            Char('x') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send(FromAppMsg::DisconnectSerial);
            }
            Char('d' | 'D') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status.dtr = !self.status.dtr;
            }
            Char('f' | 'F') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status.cts = !self.status.cts;
            }
            Char(c) if !key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.term_input.push(c)
            }
            Enter => self.enter_input(),
            _ => {}
        }
        Ok(())
    }

    fn handle_serial(&mut self, message: Result<String>) {
        match message {
            Ok(s) => {
                for line in s.split_inclusive('\n') {
                    if self.term_state.text.last().unwrap().ends_with('\n') {
                        self.term_state.text.push(line.into());
                    } else {
                        self.term_state.text.last_mut().unwrap().push_str(line);
                    }
                }
            }
            Err(e) => self.log_err(e),
        }
    }

    fn enter_input(&mut self) {
        if self.term_input.is_empty() {
            return;
        }
        self.events
            .send(FromAppMsg::WriteSerial(crate::event::ToSerialData::Data(
                take(&mut self.term_input),
            )));
    }

    fn log_err(&mut self, report: Report) {
        self.status.log.push(report.to_string());
    }
}
