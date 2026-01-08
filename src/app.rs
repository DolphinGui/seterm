use crate::{
    event::{AppEvent, Event, EventHandler},
    ui::render_ui,
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::ScrollbarState,
    DefaultTerminal,
};

#[derive(Debug)]
pub struct Status {
    pub cts: bool,
    pub dtr: bool,
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
        Self {
            running: true,
            counter: 0,
            events: EventHandler::new(),
            term_input: String::new(),
            term_state: TerminalStatus {
                text: Vec::new(),
                scroll_index: 0,
                scroll_state: ScrollbarState::new(0).viewport_content_length(1),
            },
            status: Status {
                cts: false,
                dtr: false,
            },
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        use crossterm::event::{Event::Key, KeyEventKind::Press};
        use Event::{App, Crossterm, Tick};
        while self.running {
            terminal.draw(|frame| render_ui(&mut self, frame))?;
            match self.events.next().await? {
                Tick => self.tick(),
                Crossterm(Key(event)) => {
                    if event.kind == Press {
                        self.handle_key_events(event)?
                    }
                }
                Crossterm(_) => {}
                App(app_event) => match app_event {
                    AppEvent::Increment => self.increment_counter(),
                    AppEvent::Decrement => self.decrement_counter(),
                    AppEvent::Quit => self.quit(),
                },
            }
        }
        Ok(())
    }

    pub fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        use KeyCode::{Char, Esc, Left, Right};
        match key_event.code {
            Esc => self.events.send(AppEvent::Quit),
            Char('c' | 'C') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.events.send(AppEvent::Quit)
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
            Right => self.events.send(AppEvent::Increment),
            Left => self.events.send(AppEvent::Decrement),
            _ => {}
        }
        Ok(())
    }

    pub fn tick(&self) {}

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn increment_counter(&mut self) {
        self.counter = self.counter.saturating_add(1);
    }

    pub fn decrement_counter(&mut self) {
        self.counter = self.counter.saturating_sub(1);
    }
}
