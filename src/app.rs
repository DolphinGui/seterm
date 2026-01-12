use std::mem::take;

use crate::{
    device_finder::{Baud, DeviceConfig, DeviceConfigurer, DeviceFinder},
    event::{
        AppEvent, FromFileWatcher, FromSerialData, GuiEvent, Messenger, Reactive, Severity,
        ToAppEvent, ToSerialData, new_filewatcher, pseudo_serial, serial_handler,
    },
    fileviewer::{CmdInput, FileViewer},
    ui::Dashboard,
};

use eyre::OptionExt;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    widgets::ScrollbarState,
};

use color_eyre::{Report, Result, eyre::WrapErr};
use tokio::sync::{mpsc, oneshot};

pub struct App {
    running: bool,
    to_self: Messenger,
    inbox: mpsc::UnboundedReceiver<ToAppEvent>,
    stack: Vec<Box<dyn Reactive>>,
    serial: Option<mpsc::UnboundedSender<ToSerialData>>,
    serial_cfg: Option<DeviceConfig>,
    uploader: Option<oneshot::Sender<()>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let tx = Messenger::new(tx);
        Self {
            running: true,
            to_self: tx.clone(),
            inbox: rx,
            stack: vec![Box::new(Dashboard::new(tx))],
            serial: None,
            serial_cfg: None,
            uploader: None,
        }
    }

    pub async fn run(
        mut self,
        mut terminal: DefaultTerminal,
        default_baud: Baud,
        try_device: Option<String>,
        default_cmd: String,
    ) -> color_eyre::Result<()> {
        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            match self.next().await? {
                ToAppEvent::Gui(g) => self.handle_key_events(g),
                ToAppEvent::App(AppEvent::Leave) => _ = self.stack.pop(),
                ToAppEvent::App(AppEvent::Quit) => self.running = false,
                ToAppEvent::App(AppEvent::RequestSerial) => self.connect_serial(default_baud)?,
                ToAppEvent::App(AppEvent::RequestUpload) => {
                    self.upload_file(default_cmd.clone(), true)?
                }
                ToAppEvent::App(AppEvent::SendSerial(s)) => {
                    self.send_serial(s);
                }
                ToAppEvent::App(AppEvent::SerialConnect(s, c)) => {
                    self.serial = Some(s);
                    self.serial_cfg = Some(c);
                }
                ToAppEvent::App(AppEvent::SendUpload(u)) => {
                    self.uploader = Some(u);
                }
                ToAppEvent::App(AppEvent::Watcher(w)) => self.handle_watcher(w),
                ToAppEvent::Popup(reactive) => self.stack.push(reactive),
            }
        }
        Ok(())
    }

    fn handle_key_events(&mut self, event: GuiEvent) {
        use crate::event::GuiEvent::Crossterm;
        use crossterm::event::{Event::Key, KeyEventKind::Press};
        let event = match event {
            Crossterm(Key(event)) if event.kind != Press => {
                return;
            }
            e => e,
        };

        for component in self.stack.iter_mut().rev() {
            if component.listen(&event) {
                break;
            }
        }
    }

    fn handle_watcher(&mut self, w: FromFileWatcher) {
        match w {
            FromFileWatcher::DisonnectRequest => {
                if self.serial.is_none() {
                    self.to_self.log(
                        Severity::Error,
                        "Cannot flash when no device is connected".into(),
                    );
                    self.uploader.take().map(|u| u.send(()));
                }
                self.serial = None;
            }
            FromFileWatcher::ReconnectRequest => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.stack.retain(|i| i.alive());
        for component in &mut self.stack {
            component.draw(frame.area(), frame.buffer_mut());
        }
    }

    fn connect_serial(&mut self, baud: Baud) -> Result<()> {
        use crate::event::Severity;
        let app = self.to_self.clone();
        tokio::spawn(async move {
            let r: Result<()> = async {
                let (finder, rx) = DeviceFinder::new().wrap_err("Could not list serial ports")?;
                app.new_component(Box::new(finder));
                let Ok(path) = rx.await else { return Ok(()) };
                let (popup, config) = DeviceConfigurer::new(path, baud);
                app.new_component(Box::new(popup));
                let Ok(config) = config.await else {
                    return Ok(());
                };
                let serial = config
                    .clone()
                    .to_serial()
                    .wrap_err("Could not connect to serial port")?;
                let serial = serial_handler(serial, app.clone());
                _ = app.send_app(AppEvent::SerialConnect(serial, config));
                Ok(())
            }
            .await;
            if let Err(e) = r {
                _ = app.log(Severity::Error, format!("{}", e));
            }
        });
        Ok(())
    }

    fn upload_file(&mut self, path: String, autorun: bool) -> Result<()> {
        use crate::event::Severity;
        let to_dash = self.to_self.clone();
        tokio::spawn(async move {
            let Ok((finder, f)) = FileViewer::new() else {
                _ = to_dash.log(Severity::Error, "Could not open working directory".into());
                return;
            };
            to_dash.new_component(Box::new(finder));
            let Ok(file) = f.await else {
                return;
            };
            let (input, cmd) = CmdInput::new(Default::default());
            to_dash.new_component(Box::new(input));
            let Ok(cmd) = cmd.await else {
                return;
            };
            let Ok(watcher) = new_filewatcher(&file, cmd, to_dash.clone()) else {
                return;
            };
            to_dash.send_app(AppEvent::SendUpload(watcher));
        });
        Ok(())
    }
    async fn next(&mut self) -> color_eyre::Result<ToAppEvent> {
        self.inbox
            .recv()
            .await
            .ok_or_eyre("Failed to receive event")
    }

    fn send_self(&mut self, app_event: AppEvent) {
        let _ = self.to_self.send_app(app_event);
    }

    fn send_serial(&mut self, data: ToSerialData) {
        if let Some(se) = self.serial.as_ref() {
            if se.send(data).is_err() {
                self.serial = None;
            }
        } else {
            self.to_self.log(
                crate::event::Severity::Error,
                "Not currently connected to a device".into(),
            );
        }
    }
}

fn render_popup(popup: &mut dyn Reactive, area: Rect, buf: &mut Buffer) {
    let x_margin = area.width / 4;
    let y_margin = area.height / 4;
    let area = area.inner(ratatui::layout::Margin {
        horizontal: x_margin,
        vertical: y_margin,
    });

    popup.draw(area, buf);
}
