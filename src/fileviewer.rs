use std::{fs::DirEntry, mem::take, path::PathBuf};

use crate::event::{Drawable, EventListener, GuiEvent};

use eyre::{Result, eyre};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, List, ListState, Paragraph, StatefulWidget, Widget},
};
use tokio::sync::oneshot;

pub struct FileViewer {
    cur_dir: PathBuf,
    contents: Vec<DirEntry>,
    list_contents: Vec<String>,
    list_state: ListState,
    tx: Option<oneshot::Sender<PathBuf>>,
}

impl FileViewer {
    pub fn new() -> Result<(FileViewer, oneshot::Receiver<PathBuf>)> {
        let cur_dir =
            std::env::current_dir().map_err(|e| eyre!("Error reading current directory: {}", e))?;
        let contents: Vec<DirEntry> = cur_dir.read_dir()?.filter_map(|r| r.ok()).collect();
        let list_contents = contents
            .iter()
            .map(|e| {
                if e.metadata().unwrap().is_dir() {
                    format!("🗀 {}", e.file_name().display())
                } else {
                    format!("{}", e.file_name().display())
                }
            })
            .collect();

        let (tx, rx) = oneshot::channel();
        let tx = Some(tx);
        Ok((
            Self {
                cur_dir,
                contents,
                list_contents,
                list_state: ListState::default(),
                tx,
            },
            rx,
        ))
    }

    fn go_parent(&mut self) -> Result<()> {
        self.update_dir(
            self.cur_dir
                .parent()
                .ok_or(eyre!("Directory has no parent!"))?
                .to_path_buf(),
        )
    }

    fn handle_file(&mut self, sel: usize) -> Result<()> {
        let f = self.contents[sel].path();
        // specifically chooses to traverse symlinks
        let m = std::fs::metadata(&f)?;
        if m.is_dir() {
            self.update_dir(f)?;
        } else {
            todo!();
        }
        Ok(())
    }

    fn update_dir(&mut self, path: PathBuf) -> Result<()> {
        self.contents = path.read_dir()?.filter_map(|r| r.ok()).collect();
        self.list_contents = self
            .contents
            .iter()
            .map(|e| {
                if e.metadata().unwrap().is_dir() {
                    format!("🗀 {}", e.file_name().display())
                } else {
                    format!("{}", e.file_name().display())
                }
            })
            .collect();
        self.cur_dir = path;
        Ok(())
    }
}

impl Drawable for FileViewer {
    fn draw(&mut self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer) {
        let text = self.list_contents.iter().map(Text::raw);
        let list = List::new(text)
            .highlight_style(Style::default().reversed())
            .block(Block::bordered());
        <List as StatefulWidget>::render(list, area, buf, &mut self.list_state);
    }

    fn alive(&self) -> bool {
        self.tx.is_some()
    }
}

impl EventListener for FileViewer {
    fn listen(&mut self, e: &GuiEvent) -> bool {
        use GuiEvent::Crossterm;
        use crossterm::event::{
            Event::Key,
            KeyCode::{Down, Enter, Left, Right, Up},
            KeyEvent,
        };
        match e {
            Crossterm(Key(KeyEvent { code: Left, .. })) => todo!(),
            Crossterm(Key(KeyEvent {
                code: Right | Enter,
                ..
            })) => {
                let Some(selected) = self.list_state.selected() else {
                    return false;
                };
                let selected = &self.contents[selected];
                if let Some(tx) = self.tx.take() {
                    tx.send(selected.path());
                }
                true
            }
            Crossterm(Key(KeyEvent { code: Up, .. })) => {
                self.list_state.select_previous();
                true
            }
            Crossterm(Key(KeyEvent { code: Down, .. })) => {
                self.list_state.select_next();
                true
            }
            _ => false,
        }
    }
}

pub struct CmdInput {
    contents: String,
    tx: Option<oneshot::Sender<String>>,
}

impl CmdInput {
    pub fn new(default: String) -> (CmdInput, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                contents: default,
                tx: Some(tx),
            },
            rx,
        )
    }
}

impl Drawable for CmdInput {
    fn draw(&mut self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer) {
        let cursor = Span::raw("█").style(Style::default().add_modifier(Modifier::SLOW_BLINK));
        let line = Line::from(vec![Span::raw(&self.contents), cursor]);
        Paragraph::new(Text::from(line))
            .block(Block::bordered())
            .left_aligned()
            .render(area, buf);
    }

    fn alive(&self) -> bool {
        self.tx.is_some()
    }
}

impl EventListener for CmdInput {
    fn listen(&mut self, e: &GuiEvent) -> bool {
        use GuiEvent::Crossterm;
        use crossterm::event::{
            Event::Key,
            KeyCode::{Backspace, Char, Enter},
            KeyEvent, KeyModifiers,
        };
        match e {
            Crossterm(Key(KeyEvent {
                code: Char(c),
                modifiers,
                ..
            })) if modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                self.contents.push(*c);
                true
            }
            Crossterm(Key(KeyEvent {
                code: Enter,
                modifiers: KeyModifiers::NONE,
                ..
            })) => {
                if self.contents.is_empty() {
                    return false;
                }
                if let Some(tx) = self.tx.take() {
                    tx.send(take(&mut self.contents));
                }
                true
            }
            Crossterm(Key(KeyEvent {
                code: Backspace,
                modifiers: KeyModifiers::NONE,
                ..
            })) => {
                if self.contents.is_empty() {
                    return false;
                }
                self.contents.pop();
                true
            }
            _ => false,
        }
    }
}
