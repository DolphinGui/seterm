use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Text,
    widgets::{Block, List, ListState, StatefulWidget, Widget},
};

use crate::event::AppEvent;

pub struct DeviceFinder {
    devices: Vec<String>,
    state: ListState,
}

pub trait EventListener {
    fn listen(&mut self, e: Event) -> Option<AppEvent>;
}

pub trait Drawable {
    fn draw(&mut self, area: Rect, buf: &mut Buffer);
}

pub trait Reactive: EventListener + Drawable {}

impl<T> Reactive for T where T: EventListener + Drawable {}

impl DeviceFinder {
    pub fn new(devices: Vec<String>) -> DeviceFinder {
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
                    .map(|s| AppEvent::SelectDevice(s.clone()));
            }
            _ => {}
        };
        None
    }
}

impl Drawable for DeviceFinder {
    fn draw(&mut self, area: Rect, buf: &mut Buffer) {
        let text: Vec<_> = self.devices.iter().map(Text::raw).collect();
        let highlight_style = Style::default().fg(Color::Black).bg(Color::White);
        let l = List::new(text)
            .block(Block::bordered())
            .highlight_style(highlight_style);
        <List as StatefulWidget>::render(l, area, buf, &mut self.state);
    }
}
