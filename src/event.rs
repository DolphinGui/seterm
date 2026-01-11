use std::{
    borrow::Cow,
    path::Path,
    process::ExitStatus,
    rc::Rc,
    sync::{Arc, OnceLock},
    time::Duration,
};

use color_eyre::{
    Report, Result,
    eyre::{self, OptionExt},
};
use eyre::eyre;
use futures::{
    FutureExt, StreamExt,
    future::{OptionFuture, pending},
};
use notify::Watcher;
use ratatui::crossterm::event::Event as CrosstermEvent;
use serialport::{DataBits, SerialPort};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::{broadcast, mpsc},
};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use tokio::time::sleep as tokio_sleep;

#[derive(Debug)]
pub enum ToAppMsg {
    Crossterm(CrosstermEvent),
    App(AppEvent),
    RecieveSerial(Result<FromSerialData>),
    SerialConnected(String),
    SerialGone,
    Log(String),
    LogResult(ExitStatus, String, String),
}

#[derive(Debug)]
pub enum FromAppMsg {
    ConnectDevice(SerialStream, String),
    WriteSerial(ToSerialData),
}

#[derive(Debug)]
pub enum AppEvent {
    SelectDevice(String),
    ConnectDevice(SerialStream, String),
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

enum ToFileWatcher {
    DoneDisconnect,
}

enum FromFileWatcher {
    LogErr(String),
    LogResult(ExitStatus, String, String),
    DisonnectRequest,
    ReconnectRequest,
}

#[derive(Clone, Debug)]
pub enum FromSerialData {
    Data(Vec<u8>),
    Status { dtr: bool, cts: bool },
}

#[derive(Debug)]
pub struct EventHandler {
    pub to_self: mpsc::UnboundedSender<ToAppMsg>,
    from_mgr: mpsc::UnboundedReceiver<ToAppMsg>,
    to_mgr: mpsc::UnboundedSender<FromAppMsg>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (to_app, from_mgr) = mpsc::unbounded_channel();
        let (to_mgr, from_app) = mpsc::unbounded_channel();
        let to_app2 = to_app.clone();
        tokio::spawn(async move { ManagerTask::new(to_app2, from_app).run().await });
        Self {
            to_self: to_app,
            from_mgr,
            to_mgr,
        }
    }

    pub async fn next(&mut self) -> color_eyre::Result<ToAppMsg> {
        self.from_mgr
            .recv()
            .await
            .ok_or_eyre("Failed to receive event")
    }

    pub fn send_self(&mut self, app_event: AppEvent) {
        let _ = self.to_self.send(ToAppMsg::App(app_event));
    }

    pub fn send(&mut self, message: FromAppMsg) {
        let _ = self.to_mgr.send(message);
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

struct ManagerTask {
    to_app: mpsc::UnboundedSender<ToAppMsg>,
    from_app: mpsc::UnboundedReceiver<FromAppMsg>,
    serial: Option<SerialHandler>,
    uploader: Option<FileWatcher>,
}

// the struct tokio_serial is nice as a builder
// but is terrible for actually storing, which is kinda
// of dumb since it contains very little actual stored information
struct SerialConfig {
    path: String,
    baud: u32,
    data: tokio_serial::DataBits,
    flow: tokio_serial::FlowControl,
    parity: tokio_serial::Parity,
    stop: tokio_serial::StopBits,
}

impl ManagerTask {
    fn new(
        to_app: mpsc::UnboundedSender<ToAppMsg>,
        from_app: mpsc::UnboundedReceiver<FromAppMsg>,
    ) -> Self {
        Self {
            to_app,
            from_app,
            serial: None,
            uploader: None,
        }
    }

    fn get_config(serial: &SerialStream, path: String) -> Result<SerialConfig> {
        let baud = serial.baud_rate()?;
        let data = serial.data_bits()?;
        let flow = serial.flow_control()?;
        let parity = serial.parity()?;
        let stop = serial.stop_bits()?;
        Ok(SerialConfig {
            path,
            baud,
            data,
            flow,
            parity,
            stop,
        })
    }

    fn create_stream(cfg: &SerialConfig) -> Result<SerialStream> {
        tokio_serial::new(cfg.path.clone(), cfg.baud)
            .data_bits(cfg.data)
            .flow_control(cfg.flow)
            .parity(cfg.parity)
            .stop_bits(cfg.stop)
            .open_native_async()
            .map_err(|e| e.into())
    }

    async fn run(mut self) -> Result<()> {
        use FromAppMsg::{ConnectDevice, WriteSerial};
        use FromFileWatcher::{DisonnectRequest, LogErr, LogResult, ReconnectRequest};
        let mut reader = crossterm::event::EventStream::new();
        // this is kinda stupid but is necessary to get rust to shut up
        let mut reset_serial = false;
        let mut reset_upload = false;
        let mut new_serial: Option<(SerialStream, String)> = None;
        let mut old_config: Option<SerialConfig> = None;
        loop {
            let no_device = eyre!("No device connected!");
            if reset_serial {
                self.serial = None;
                let _ = self.to_app.send(ToAppMsg::SerialGone);
                reset_serial = false;
            }

            if reset_upload {
                self.uploader = None;
                reset_upload = false;
            }

            if let Some((se, name)) = new_serial.take() {
                match Self::get_config(&se, name) {
                    Ok(s) => old_config = Some(s),
                    Err(e) => {
                        _ = self
                            .to_app
                            .send(ToAppMsg::Log(format!("Failed to connect to serial: {}", e)));
                        continue;
                    }
                };
                _ = self.to_app.send(ToAppMsg::SerialConnected(
                    se.name().unwrap_or("Virtual".into()),
                ));
                self.serial = Some(SerialHandler::new(se));
            }
            let (se_tx, se_rx) = self
                .serial
                .as_mut()
                .map(
                    |SerialHandler {
                         app_tx: tx,
                         app_rx: rx,
                     }| (tx, rx),
                )
                .unzip();

            let serial_read = async {
                match se_rx {
                    Some(rx) => rx.recv().await,
                    None => pending().await,
                }
            };

            let file_read = async {
                match self.uploader.as_mut() {
                    Some(u) => u.from_watcher.recv().await,
                    None => pending().await,
                }
            };

            let crossterm_event = reader.next().fuse();
            tokio::select! {
              _ = self.to_app.closed() => {
                break;
              }
              Some(Ok(evt)) = crossterm_event => {
                let _ = self.to_app.send(ToAppMsg::Crossterm(evt));
              }
              Some(e) = self.from_app.recv() => {
               match e {
                WriteSerial(s) => {
                   let _ = se_tx
                     .ok_or(no_device)
                     .and_then(|serial| serial.send(s).map_err(|e| e.into()))
                     .map_err(|e| self.to_app.send(ToAppMsg::RecieveSerial(Err(e))));
                }
                ConnectDevice(serial, name) => {
                   new_serial = Some((serial, name));
                }
               }
              },
              r = serial_read => {
                match r{
                  None => {  _ = self.to_app.send(ToAppMsg::SerialGone); reset_serial = true;  }
                  Some(data) => {  _ = self.to_app.send(ToAppMsg::RecieveSerial(data));}
                }
              }
              f = file_read =>{
                  match f {
                    None => { reset_upload = true; },
                    Some(DisonnectRequest) => { reset_serial = true;  },
                    Some(ReconnectRequest) => {
                        let Some(ref cfg) = old_config else {
                            continue;
                        };
                        let Ok(stream) = Self::create_stream(cfg) else{
                          _ = self.to_app.send(ToAppMsg::Log("Failed to construct serial configuration".into()));
                          continue;
                        };
                        new_serial = Some((stream, cfg.path.clone()));
                    },
                    Some(LogErr(e) ) => { _ = self.to_app.send(ToAppMsg::Log(e))  },
                    Some(LogResult(s, o, e)) => { _ = self.to_app.send(ToAppMsg::LogResult(s, o, e))  },
                }
              }
            };
        }
        Ok(())
    }
}

struct SerialHandler {
    app_tx: mpsc::UnboundedSender<ToSerialData>,
    app_rx: mpsc::UnboundedReceiver<Result<FromSerialData>>,
}

impl SerialHandler {
    fn new(mut device: SerialStream) -> Self {
        use ToSerialData::{CTS, DTR, Data, Disconnect, RequestStatus};
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut buf = [0; 128];
            let mut alive = true;
            while alive {
                // I hate the formatting, and macros are atrocious
                // unfortunately, there's literally no better way to represent the problem
                select! {
                   _ = data_tx.closed() => break,
                  read = device.read(&mut buf) => { let len = match read {
                        Ok(l) => l,
                        Err(e) => {
                            let _ = data_tx.send(Err(e.into()));
                            break;
                        }
                    };
                    if len == 0 {
                        let _ = data_tx.send(Err(eyre!("Out of bytes to read!")));
                        break;
                    } else {
                        // If we failed to send, either the client's dead or something weird has happened, so die early
                        if data_tx.send(Ok(FromSerialData::Data(Vec::from(&buf[0..len])))).is_err() {break};
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
                                Err(e) => { _ = data_tx.send(Err(e.into())); true }
                            };
                            let cts = match device.read_clear_to_send(){
                                Ok(d) => d,
                                Err(e) => { _ = data_tx.send(Err(e.into())); true }
                            };

                            if data_tx.send(
                              Ok(
                                FromSerialData::Status{ dtr, cts }
                              )
                            ).is_err() {break};
                            continue;
                        }
                        None => eyre!("Serial terminal closed!"),
                    };
                    let _ =  data_tx.send(Err(err));
                    break;
                }
                };
            }
        });

        Self {
            app_tx: event_tx,
            app_rx: data_rx,
        }
    }
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

struct FileWatcher {
    from_watcher: mpsc::Receiver<FromFileWatcher>,
    to_watcher: mpsc::Sender<ToFileWatcher>,
    watcher: notify::RecommendedWatcher,
}

struct WatcherImpl {
    to_app: mpsc::Sender<FromFileWatcher>,
    from_app: mpsc::Receiver<ToFileWatcher>,
    cmd: Vec<String>,
}

impl notify::EventHandler for WatcherImpl {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        use FromFileWatcher::{DisonnectRequest, LogErr, LogResult, ReconnectRequest};
        use notify::{
            Event,
            EventKind::{Create, Modify},
            event::{CreateKind::File, ModifyKind::Data},
        };
        use std::process::{Command, Output};
        match event {
            Err(e) => {
                _ = self.to_app.send(FromFileWatcher::LogErr(e.to_string()));
            }
            Ok(Event {
                kind: Modify(Data(..)) | Create(File),
                ..
            }) => {
                _ = self.to_app.blocking_send(DisonnectRequest);
                // wait for disconnect to finish
                let r = self.from_app.blocking_recv();
                if r.is_none() {
                    // we're already dead but haven't realized it
                    return;
                }
                let out = Command::new(&self.cmd[0]).args(&self.cmd[1..]).output();
                match out {
                    // This only watches 1 file, so we don't bother checking which file it was
                    Ok(Output {
                        status,
                        stdout,
                        stderr,
                    }) => {
                        let stdout = String::from_utf8_lossy(&stdout);
                        let stderr = String::from_utf8_lossy(&stderr);
                        _ = self.to_app.blocking_send(LogResult(
                            status,
                            stderr.into(),
                            stdout.into(),
                        ));
                    }
                    Err(e) => _ = self.to_app.blocking_send(LogErr(e.to_string().into())),
                };
                _ = self.to_app.blocking_send(ReconnectRequest);
            }
            _ => {}
        };
    }
}

impl FileWatcher {
    pub fn new_pair(file: &Path, mut cmd: Vec<String>) -> Result<Self> {
        let filename = file.to_str().ok_or_eyre("Firmware filename is not UTF8")?;
        *cmd.iter_mut()
            .find(|p| *p == "#BIN#")
            .ok_or_eyre("Firmware arguments do not contain #FILE#")? = filename.into();
        let (to_app, from_watcher) = mpsc::channel(8);
        let (to_watcher, from_app) = mpsc::channel(8);

        let watcher = WatcherImpl {
            to_app,
            from_app,
            cmd,
        };
        let mut watcher = notify::recommended_watcher(watcher)?;
        watcher.watch(file, notify::RecursiveMode::Recursive)?;
        Ok(Self {
            to_watcher,
            from_watcher,
            watcher,
        })
    }
}
