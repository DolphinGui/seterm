use std::mem::take;

use crossterm::event::{KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget,
    },
};
use tracing::{instrument, trace};

use crate::event::{
    AppEvent, Drawable, EventListener, FromSerialData, GuiEvent, Messenger, Severity, ToSerialData,
};

#[derive(Debug)]
pub struct Dashboard {
    alive: bool,
    term_input: String,
    term_state: TerminalStatus,
    status: Status,
    to_app: Messenger,
}

#[derive(Default)]
struct Status {
    rts: bool,
    dtr: bool,
    device: String,
    log: Vec<(Severity, String)>,
}

#[derive(Default)]
struct TerminalStatus {
    text: Vec<String>,
    scroll_index: usize,
    scroll_state: ScrollbarState,
}

impl EventListener for Dashboard {
    fn listen(&mut self, e: &GuiEvent) -> bool {
        use GuiEvent::{Crossterm, Log, Serial};
        match e {
            Log(sev, st) => {
                self.status.log.push((*sev, st.clone()));
                true
            }
            Crossterm(c) => self.handle_term(c),
            Serial(s) => self.handle_serial(s),
            GuiEvent::SerialDone => false,
        }
    }
}

impl Dashboard {
    pub fn new(to_app: Messenger) -> Self {
        Self {
            alive: true,
            term_input: Default::default(),
            term_state: Default::default(),
            status: Default::default(),
            to_app,
        }
    }

    fn handle_term(&mut self, e: &crossterm::event::Event) -> bool {
        use crossterm::event::Event::Key;
        if let Key(k) = e {
            self.handle_keybinds(*k);
        }
        true
    }

    fn handle_keybinds(&mut self, event: KeyEvent) -> bool {
        let KeyEvent {
            code, modifiers, ..
        } = event;
        use AppEvent::SendSerial;
        use ToSerialData::{DTR, RTS};
        use crossterm::event::{
            KeyCode::{Backspace, Char, Enter},
            KeyEvent,
        };
        match (modifiers, code) {
            (KeyModifiers::NONE | KeyModifiers::SHIFT, Char(c)) => {
                self.term_input.push(c);
            }
            (KeyModifiers::NONE, Backspace) => {
                _ = self.term_input.pop();
            }
            (KeyModifiers::NONE, Enter) => {
                self.term_input.push('\n');
                self.send_serial();
            }
            (KeyModifiers::CONTROL, Char('d')) => {
                self.status.dtr = !self.status.dtr;
                self.to_app.send_app(SendSerial(DTR(self.status.dtr)));
            }
            (KeyModifiers::CONTROL, Char('r')) => {
                self.status.rts = !self.status.rts;
                self.to_app.send_app(SendSerial(RTS(self.status.rts)));
            }
            _ => return false,
        }
        true
    }

    fn handle_serial(&mut self, se: &FromSerialData) -> bool {
        match se {
            FromSerialData::Data(items) => {
                for line in String::from_utf8_lossy(items).split_inclusive('\n') {
                    match self.term_state.text.last_mut() {
                        Some(l) if l.ends_with('\n') => {
                            self.term_state.text.push(line.into());
                        }
                        Some(l) => {
                            l.push_str(line);
                        }
                        None => self.term_state.text.push(line.into()),
                    }
                }
            }
            FromSerialData::Connect(s) => self.status.device = s.clone(),
            FromSerialData::Gone => self.status.device.clear(),
        };
        true
    }

    fn send_serial(&mut self) {
        use crate::event::{AppEvent::SendSerial, ToSerialData::Data};
        self.to_app
            .send_app(SendSerial(Data(take(&mut self.term_input))));
    }
}

impl Drawable for Dashboard {
    fn alive(&self) -> bool {
        self.alive
    }
    #[instrument(skip(frame))]
    fn draw(&mut self, area: Rect, frame: &mut Frame) {
        trace!("Drawing dashboard");
        use ratatui::layout::Direction;
        let a = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Min(20)])
            .split(area);
        let [bigger, status_area] = &*a else {
            panic!("Bigger area should have 1 item");
        };

        let left_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(90), Constraint::Min(1)])
            .split(*bigger);

        let [term, input] = &*left_area else {
            // really should not happen unless above fails somehow
            panic!("Layout should have 2 items only");
        };
        let buf = frame.buffer_mut();

        render_terminal_block(&mut self.term_state, *term, buf);
        trace!("Drawing terminal");
        render_input_block(&self.term_input, *input, buf);
        trace!("Drawing input");
        render_status_block(&self.status, *status_area, buf);
        trace!("Drawing status");
    }
}

fn render_input_block(input: &str, area: Rect, frame: &mut Buffer) {
    let cursor = Span::raw("█").style(Style::default().add_modifier(Modifier::SLOW_BLINK));
    let line = Line::from(vec![Span::raw(input), cursor]);
    Paragraph::new(Text::from(line))
        .block(Block::bordered())
        .left_aligned()
        .render(area, frame);
}

fn render_log<T: Iterator>(lines: T, area: Rect, buf: &mut Buffer)
where
    <T as std::iter::Iterator>::Item: ratatui::widgets::Widget,
{
    let rows = area.rows().rev();
    let lines = lines.zip(rows).take((area.height).into());
    for (text, row) in lines {
        text.render(row, buf);
    }
}

fn render_status_block(stat: &Status, area: Rect, frame: &mut Buffer) {
    let [stats, log_area] =
        &*Layout::vertical([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area)
    else {
        panic!("Status constraint failed!");
    };

    const ON: &str = "●";
    const OFF: &str = "○";
    let rts = if stat.rts { ON } else { OFF };
    let dtr = if stat.dtr { ON } else { OFF };

    let log_block = Block::bordered();
    let log_zone = log_block.inner(*log_area);
    log_block.render(*log_area, frame);
    let lines = stat
        .log
        .iter()
        .rev()
        .map(|(sev, str)| render_text(*sev, str));
    render_log(lines, log_zone, frame);

    let status = format!("RTS: {}\nDTR: {}\nConnected: {}", rts, dtr, stat.device,);

    let status_block = Paragraph::new(status).block(Block::bordered()).centered();
    status_block.render(*stats, frame);
}

fn render_text(sev: Severity, t: &str) -> Text<'_> {
    let color = match sev {
        Severity::Error => ratatui::style::Color::Red,
        Severity::Info => ratatui::style::Color::default(),
        Severity::Debug => ratatui::style::Color::LightGreen,
    };
    Text::styled(t, Style::new().fg(color))
}

fn render_terminal_block(input: &mut TerminalStatus, area: Rect, frame: &mut Buffer) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("#");
    input.scroll_state = input
        .scroll_state
        .content_length(input.text.len())
        .position(input.text.len() - input.scroll_index);
    <Scrollbar as StatefulWidget>::render(scrollbar, area, frame, &mut input.scroll_state);
    let block = Block::bordered();
    let text_area = block.inner(area);
    block.render(area, frame);
    let lines = input.text.iter().rev().skip(input.scroll_index);
    render_log(lines, text_area, frame);
}

impl std::fmt::Debug for TerminalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalStatus")
            .field("text_size", &self.text.len())
            .field("scroll_index", &self.scroll_index)
            .field("scroll_state", &self.scroll_state)
            .finish()
    }
}

impl std::fmt::Debug for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Status")
            .field("rts", &self.rts)
            .field("dtr", &self.dtr)
            .field("device", &self.device)
            .field("log_size", &self.log.len())
            .finish()
    }
}
