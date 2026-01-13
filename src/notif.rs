use ratatui::widgets::{Block, Clear, Paragraph};

use crate::event::{Drawable, EventListener};

pub struct Notification {
    content: String,
}

impl Notification {
    pub fn new(content: String) -> Self {
        Self { content }
    }
}

impl Drawable for Notification {
    fn alive(&self) -> bool {
        true
    }

    fn draw(&mut self, area: ratatui::prelude::Rect, frame: &mut ratatui::Frame) {
        frame.render_widget(Clear, area);
        let p = Paragraph::new(self.content.clone())
            .block(Block::bordered())
            .centered()
            .wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(p, area);
    }
}

impl EventListener for Notification {
    fn listen(&mut self, _: &crate::event::GuiEvent) -> bool {
        false
    }
}
