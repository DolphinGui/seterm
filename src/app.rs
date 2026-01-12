use crate::{
    device_finder::{Baud, DeviceConfig, DeviceConfigurer, DeviceFinder},
    event::{
        AppEvent, FromFileWatcher, FromSerialData, GuiEvent, Messenger, Reactive, Severity,
        ToAppEvent, ToFileWatcher, ToSerialData, crossterm_handler, new_filewatcher,
        serial_handler,
    },
    fileviewer::{CmdInput, FileViewer},
    ui::Dashboard,
};

use crossterm::event::KeyEvent;
use eyre::OptionExt;
use ratatui::{DefaultTerminal, Frame, layout::Rect};

use color_eyre::{Result, eyre::WrapErr};
use tokio::sync::mpsc;
use tracing::{Instrument, instrument, trace};

pub struct App {
    running: bool,
    to_self: Messenger,
    inbox: mpsc::UnboundedReceiver<ToAppEvent>,
    stack: Vec<Box<dyn Reactive>>,
    serial: Option<mpsc::UnboundedSender<ToSerialData>>,
    serial_cfg: Option<DeviceConfig>,
    watcher: Option<mpsc::UnboundedSender<ToFileWatcher>>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("running", &self.running)
            .field("inbox", &self.inbox)
            .field("serial", &self.serial)
            .field("serial_cfg", &self.serial_cfg)
            .field("uploader", &self.watcher)
            .finish()
    }
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
        crossterm_handler(tx.clone());
        Self {
            running: true,
            to_self: tx.clone(),
            inbox: rx,
            stack: vec![Box::new(Dashboard::new(tx))],
            serial: None,
            serial_cfg: None,
            watcher: None,
        }
    }
    #[instrument(skip(terminal))]
    pub async fn run(
        mut self,
        mut terminal: DefaultTerminal,
        default_baud: Baud,
        _try_device: Option<String>,
        default_cmd: String,
    ) -> color_eyre::Result<()> {
        use AppEvent::{
            Leave, Quit, RequestSerial, RequestUpload, SendSerial, SendUpload, SerialConnect,
            Watcher,
        };
        use ToAppEvent::{App, Gui, Popup};
        trace!("Starting main loop!");
        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            match self.next().await? {
                Gui(GuiEvent::Serial(FromSerialData::Gone)) => {
                    self.watcher
                        .as_mut()
                        .inspect(|u| _ = u.send(ToFileWatcher::Disconnected));
                    self.handle_key_events(GuiEvent::Serial(FromSerialData::Gone));
                }
                Gui(g) => self.handle_key_events(g),
                App(Leave) => {
                    _ = self.stack.pop();
                    if self.stack.is_empty() {
                        return Ok(());
                    }
                }
                App(Quit) => self.running = false,
                App(RequestSerial) => self.connect_serial(default_baud),
                App(RequestUpload) => self.upload_file(default_cmd.clone(), true),
                App(SendSerial(s)) => {
                    self.send_serial(s);
                }
                App(SerialConnect(s, c)) => {
                    self.serial = Some(s);
                    self.serial_cfg = Some(c);
                }
                App(SendUpload(u)) => {
                    self.watcher = Some(u);
                }
                App(Watcher(w)) => self.handle_watcher(w),
                Popup(reactive) => self.stack.push(reactive),
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
        if let Crossterm(Key(k)) = event {
            self.handle_keys(k)
        }
    }

    fn handle_keys(&mut self, key: KeyEvent) {
        use crossterm::event::{KeyCode::Char, KeyCode::Esc, KeyModifiers};
        match (key.modifiers, key.code) {
            (_, Esc) => {
                self.to_self.send_app(AppEvent::Leave);
            }
            (KeyModifiers::CONTROL, Char('c')) => {
                self.to_self.send_app(AppEvent::Quit);
            }
            (KeyModifiers::CONTROL, Char('f')) => {
                self.to_self.send_app(AppEvent::RequestSerial);
            }
            (KeyModifiers::CONTROL, Char('u')) => {
                self.to_self.send_app(AppEvent::RequestUpload);
            }
            _ => {}
        }
    }

    fn handle_watcher(&mut self, w: FromFileWatcher) {
        match w {
            FromFileWatcher::DisonnectRequest => {
                let Some(se) = self.serial.as_mut() else {
                    self.to_self.log(
                        Severity::Error,
                        "Cannot flash when no device is connected".into(),
                    );
                    _ = self.watcher.as_mut().unwrap().send(ToFileWatcher::NoDevice);
                    return;
                };
                _ = se.send(ToSerialData::Disconnect);
            }
            FromFileWatcher::ReconnectRequest => {
                let cfg = self.serial_cfg.clone().unwrap();
                let serial = match cfg.to_serial() {
                    Ok(o) => o,
                    Err(e) => {
                        self.to_self.log(
                            Severity::Error,
                            format!("Could not connect to serial: {}", e),
                        );
                        return;
                    }
                };
                self.serial = Some(serial_handler(serial, self.to_self.clone()));
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        trace!("Drawing frame");
        self.stack.retain(|i| i.alive());
        // first element is given more space than others
        // todo maybe consider having stuff store their own space?
        self.stack.first_mut().unwrap().draw(frame.area(), frame);
        for component in &mut self.stack.iter_mut().skip(1) {
            trace!("Drawing popup!");
            render_popup(component.as_mut(), frame.area(), frame);
        }
        trace!("Done Drawing");
    }

    fn connect_serial(&mut self, baud: Baud) {
        use crate::event::Severity;
        let app = self.to_self.clone();
        tokio::spawn(
            async move {
                let r: Result<()> = async {
                    let (finder, rx) = DeviceFinder::new()?;
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
                    app.send_app(AppEvent::SerialConnect(serial, config));
                    app.send_notif(GuiEvent::SerialDone);
                    Ok(())
                }
                .await;
                if let Err(e) = r {
                    app.log(Severity::Error, format!("{}", e));
                }
            }
            .instrument(tracing::info_span!("Serial sequence")),
        );
    }

    fn upload_file(&mut self, _path: String, _autorun: bool) {
        use crate::event::Severity;
        let to_dash = self.to_self.clone();
        tokio::spawn(
            async move {
                let Ok((finder, f)) = FileViewer::new(to_dash.clone()) else {
                    to_dash.log(Severity::Error, "Could not open working directory".into());
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
            }
            .instrument(tracing::info_span!("Watcher sequence")),
        );
    }
    async fn next(&mut self) -> color_eyre::Result<ToAppEvent> {
        self.inbox
            .recv()
            .await
            .ok_or_eyre("Failed to receive event")
    }

    fn send_serial(&mut self, data: ToSerialData) {
        if let Some(se) = self.serial.as_ref() {
            if se.send(data).is_err() {
                self.serial = None;
            }
        } else if data != ToSerialData::Disconnect {
            self.to_self.log(
                crate::event::Severity::Error,
                "Not currently connected to a device".into(),
            );
        }
    }
}

fn render_popup(popup: &mut dyn Reactive, area: Rect, buf: &mut Frame) {
    let x_margin = area.width / 4;
    let y_margin = area.height / 4;
    let area = area.inner(ratatui::layout::Margin {
        horizontal: x_margin,
        vertical: y_margin,
    });

    popup.draw(area, buf);
}
