use std::time::Duration;

use color_eyre::{
    Report, Result,
    eyre::{self, OptionExt},
};
use eyre::eyre;
use futures::{FutureExt, StreamExt, future::OptionFuture};
use ratatui::crossterm::event::Event as CrosstermEvent;
use serialport::SerialPort;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc,
};
use tokio_serial::SerialStream;

use tokio::time::sleep as tokio_sleep;

#[derive(Debug)]
pub enum ToAppMsg {
    Crossterm(CrosstermEvent),
    App(AppEvent),
    RecieveSerial(Result<FromSerialData>),
    SerialConnected(String),
    SerialGone,
    Log(String),
}

#[derive(Debug)]
pub enum FromAppMsg {
    ConnectDevice(SerialStream),
    WriteSerial(ToSerialData),
}

#[derive(Debug)]
pub enum AppEvent {
    RequestAvailableDevices,
    SelectDevice(String),
    ConnectDevice(SerialStream),
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
        let actor = ManagerTask::new(to_app.clone(), from_app);
        tokio::spawn(async { actor.run().await });
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
        }
    }

    async fn run(mut self) -> Result<()> {
        use FromAppMsg::{ConnectDevice, WriteSerial};
        let mut reader = crossterm::event::EventStream::new();
        // this is kinda stupid but is necessary to get rust to shut up
        let mut reset_serial = false;
        let mut new_serial: Option<SerialStream> = None;
        loop {
            let no_device = eyre!("No device connected!");
            if reset_serial {
                self.serial = None;
                let _ = self.to_app.send(ToAppMsg::SerialGone);
                reset_serial = false;
            }
            if let Some(se) = new_serial.take() {
                let _ = self.to_app.send(ToAppMsg::SerialConnected(
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

            let serial_read: OptionFuture<_> = se_rx.map(|rx| rx.recv()).into();
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
                ConnectDevice(serial) => {
                   new_serial = Some(serial);
                }
               }
              },
              r = serial_read => {
                match r{
                  // serial does not exist, which is normal
                  None => { },
                  // serial is disconnected, in which case we need to tell the app and clear the serial
                  Some(None) => { let _ = self.to_app.send(ToAppMsg::SerialGone); reset_serial = true;  }
                  Some(Some(data)) => {let _ = self.to_app.send(ToAppMsg::RecieveSerial(data));}
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
                    }
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
