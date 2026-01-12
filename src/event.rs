use std::{path::Path, time::Duration};

use color_eyre::{
    Report, Result,
    eyre::{self, OptionExt},
};
use eyre::{Context, eyre};
use futures::{FutureExt, StreamExt};
use notify::{RecommendedWatcher, Watcher};
use ratatui::{buffer::Buffer, crossterm::event::Event as CrosstermEvent, layout::Rect};
use serialport::SerialPort;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::{mpsc, oneshot},
};
use tokio_serial::SerialStream;

use tokio::time::sleep as tokio_sleep;

use crate::device_finder::DeviceConfig;

pub trait EventListener {
    fn listen(&mut self, e: &GuiEvent) -> bool;
}

pub trait Drawable {
    fn alive(&self) -> bool;
    fn draw(&mut self, area: Rect, buf: &mut Buffer);
}

pub trait Reactive: EventListener + Drawable + Send {}

impl<T> Reactive for T where T: EventListener + Drawable + Send {}

pub enum ToAppEvent {
    App(AppEvent),
    Popup(Box<dyn Reactive>),
    Gui(GuiEvent),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Info,
    Debug,
}

#[derive(Clone, Debug)]
pub enum GuiEvent {
    Crossterm(CrosstermEvent),
    Log(Severity, String),
    Serial(FromSerialData),
}

#[derive(Debug)]
pub enum AppEvent {
    RequestSerial,
    SerialConnect(mpsc::UnboundedSender<ToSerialData>, DeviceConfig),
    SendSerial(ToSerialData),
    RequestUpload,
    SendUpload(oneshot::Sender<()>),
    Watcher(FromFileWatcher),
    Leave,
    Quit,
}

#[derive(Clone, Debug)]
pub enum ToSerialData {
    Data(String),
    CTS(bool),
    DTR(bool),
    Disconnect,
    RequestStatus,
}

#[derive(Clone, Debug)]
pub enum FromSerialData {
    Connect(String),
    Data(Vec<u8>),
    Status { dtr: bool, cts: bool },
    Gone,
}

#[derive(Clone, Debug)]
pub enum FromFileWatcher {
    DisonnectRequest,
    ReconnectRequest,
}

#[derive(Clone, Debug)]
pub struct Messenger(mpsc::UnboundedSender<ToAppEvent>);

impl Messenger {
    pub fn new(m: mpsc::UnboundedSender<ToAppEvent>) -> Self {
        Self(m)
    }

    pub fn send_term(&self, e: CrosstermEvent) {
        _ = self.0.send(ToAppEvent::Gui(GuiEvent::Crossterm(e)));
    }
    pub fn send_app(&self, e: AppEvent) {
        _ = self.0.send(ToAppEvent::App(e));
    }
    pub fn new_component(&self, c: Box<dyn Reactive>) {
        _ = self.0.send(ToAppEvent::Popup(c));
    }
    pub fn log(&self, s: Severity, e: String) {
        _ = self.0.send(ToAppEvent::Gui(GuiEvent::Log(s, e)));
    }
    pub fn send_serial(&self, d: FromSerialData) {
        _ = self.0.send(ToAppEvent::Gui(GuiEvent::Serial(d)));
    }
    pub fn send_file(&self, f: FromFileWatcher) {
        _ = self.0.send(ToAppEvent::App(AppEvent::Watcher(f)));
    }

    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

pub fn crossterm_handler(to_app: Messenger) {
    tokio::spawn(async move {
        let mut reader = crossterm::event::EventStream::new();
        loop {
            let crossterm_event = reader.next().fuse();
            if let Some(Ok(evt)) = crossterm_event.await {
                if to_app.is_closed() {
                    break;
                }
                to_app.send_term(evt);
            }
        }
    });
}

pub fn serial_handler(
    mut device: SerialStream,
    data_tx: Messenger,
) -> mpsc::UnboundedSender<ToSerialData> {
    use Severity::Error;
    use ToSerialData::{CTS, DTR, Data, Disconnect, RequestStatus};
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        data_tx.send_serial(FromSerialData::Connect(
            device.name().unwrap_or("Virtual".into()),
        ));
        let mut buf = [0; 128];
        let mut alive = true;
        while alive {
            // I hate the formatting, and macros are atrocious
            // unfortunately, there's literally no better way to represent the problem
            select! {
               _ = data_tx.0.closed() => break,
              read = device.read(&mut buf) => { let len = match read {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = data_tx.log(Error, format!("Error reading serial: {}",e)) ;
                        break;
                    }
                };
                    if len == 0 {
                        let _ = data_tx.log(Error, "Serial has been disconnected!".into());
                        break;
                    } else {
                        data_tx.send_serial(FromSerialData::Data(Vec::from(&buf[0..len])));
                    }
                },
                event = event_rx.recv() => {
                let err: Report = match event {
                    Some(Data(s)) => {
                        let Err(e) = device.write_all(s.as_bytes()).await else {
                            continue;
                        };
                        e.into()
                    }
                    Some(DTR(b)) => {
                        let Err(e) = device.write_data_terminal_ready(b) else {
                            continue;
                        };
                        e.into()
                    }
                    Some(CTS(b)) => {
                        let Err(e) = device.write_request_to_send(b) else {
                            continue;
                        };
                        e.into()
                    }
                    Some(Disconnect) =>{
                        alive = false; continue;
                    }
                    Some(RequestStatus) =>{
                        let dtr = match device.read_data_set_ready(){
                            Ok(d) => d,
                            Err(e) => { data_tx.log(Error, format!("{}", e)); true }
                        };
                        let cts = match device.read_clear_to_send(){
                            Ok(d) => d,
                            Err(e) => { data_tx.log(Error, format!("{}", e)); true }
                        };

                        data_tx.send_serial(
                            FromSerialData::Status{ dtr, cts }
                        );
                        continue;
                    }
                    None => eyre!("Serial terminal closed!"),
                };
                let _ =  data_tx.log(Error, format!("{}", err));
                break;
            }
            };
        }
        data_tx.send_serial(FromSerialData::Gone);
    });

    event_tx
}

pub fn pseudo_serial() -> SerialStream {
    let (computer, device) = SerialStream::pair().unwrap();
    tokio::spawn(async {
        let mut periods = 0;
        let (mut reader, mut writer) = tokio::io::split(device);
        let mut buffer = [0; 128];
        loop {
            let write = writer
                .write_all(format!("{} seconds has passed\n", periods).as_bytes())
                .await;
            if write.is_err() {
                break;
            }
            let read = reader.read(&mut buffer);
            select! {
             Ok(bytes) = read=>{ {
                if writer
                    .write_all("Received the following: ".as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if writer.write_all(&buffer[0..bytes]).await.is_err() {
                    break;
                };
            }},
            _ = tokio_sleep(Duration::from_millis(1000)) => {} }
            periods += 1;
        }
    });
    computer
}

struct WatcherImpl(mpsc::UnboundedSender<notify::Result<notify::Event>>);

impl notify::EventHandler for WatcherImpl {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        _ = self.0.send(event);
    }
}

struct UploaderImpl {
    _watcher: RecommendedWatcher,
    events: mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
    to_dash: Messenger,
    deadman: oneshot::Receiver<()>,
    cmd: Vec<String>,
}

pub fn new_filewatcher(file: &Path, cmd: String, events: Messenger) -> Result<oneshot::Sender<()>> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (killswitch, dead) = oneshot::channel();
    let mut watcher = notify::recommended_watcher(WatcherImpl(tx))?;
    watcher.watch(file, notify::RecursiveMode::Recursive)?;
    let cmd = shlex::split(&cmd).ok_or_eyre("Unable to parse command")?;
    tokio::spawn(async {
        UploaderImpl {
            _watcher: watcher,
            events: rx,
            to_dash: events,
            cmd,
            deadman: dead,
        }
        .run()
        .await;
    });

    Ok(killswitch)
}

impl UploaderImpl {
    async fn exec(&self) -> Result<()> {
        let out = tokio::process::Command::new(&self.cmd[0])
            .args(&self.cmd[1..])
            .output()
            .await
            .wrap_err("Unable to execute command")?;
        self.to_dash.log(
            Severity::Info,
            format!("{}: {}", String::from_utf8_lossy(&out.stdout), out.status),
        );
        if !out.stderr.is_empty() {
            self.to_dash.log(
                Severity::Error,
                String::from_utf8_lossy(&out.stderr).to_string(),
            );
        }
        Ok(())
    }

    async fn run(&mut self) {
        use notify::EventKind::{Any, Create, Modify};
        loop {
            if self.deadman.try_recv().is_ok() {
                return;
            }
            match self.events.recv().await.unwrap() {
                Ok(notify::Event {
                    kind: Any | Create(..) | Modify(..),
                    ..
                }) => {
                    if let Err(e) = self.exec().await {
                        self.to_dash.log(Severity::Error, e.to_string())
                    }
                }
                Err(e) => self
                    .to_dash
                    .log(Severity::Error, format!("Error watching file: {}", e)),
                _ => {}
            };
        }
    }
}
