use std::sync::Arc;

use color_eyre::{
    eyre::{self, OptionExt},
    Report, Result,
};
use eyre::{eyre, Error};
use futures::{FutureExt, StreamExt};
use ratatui::crossterm::event::Event as CrosstermEvent;
use serialport::SerialPort;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc,
};
use tokio_serial::SerialStream;

#[derive(Debug)]
pub enum ToAppMsg {
    Crossterm(CrosstermEvent),
    App(AppEvent),
    RecieveSerial(Result<String>),
}

#[derive(Debug)]
pub enum FromAppMsg {
    ConnectDevice(SerialStream),
    WriteSerial(ToSerialData),
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    Quit,
}

#[derive(Clone, Debug)]
pub enum ToSerialData {
    Data(String),
    RTS(bool),
    DTR(bool),
}

#[derive(Debug)]
pub struct EventHandler {
    to_self: mpsc::UnboundedSender<ToAppMsg>,
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
        self.to_mgr.send(message);
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
        loop {
            let crossterm_event = reader.next().fuse();
            tokio::select! {
              _ = self.to_app.closed() => {
                break;
              }
              Some(Ok(evt)) = crossterm_event => {
                self.send(ToAppMsg::Crossterm(evt));
              }
              Some(e) = self.from_app.recv() => {
            match e {
                WriteSerial(s) => {
                    let Some(ref serial) = self.serial else {
                        self.to_app
                            .send(ToAppMsg::RecieveSerial(Err(eyre!("No device connected"))));
                        continue;
                    };
                    serial.app_tx.send(s);
                }
                ConnectDevice(serial) => {
                    self.serial = Some(SerialHandler::new(serial));
                }
            }
              }
            else => { break; }
            };
        }
        Ok(())
    }

    fn send(&self, event: ToAppMsg) {
        // Ignores the result because shutting down the app drops the receiver, which causes the send
        // operation to fail. This is expected behavior and should not panic.
        let _ = self.to_app.send(event);
    }
}

struct SerialHandler {
    app_tx: mpsc::UnboundedSender<ToSerialData>,
    app_rx: mpsc::UnboundedReceiver<Result<String>>,
}

impl SerialHandler {
    fn new(mut device: SerialStream) -> Self {
        use ToSerialData::{Data, DTR, RTS};
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut buf = [0; 128];
            loop {
                // I hate the formatting, and macros are atrocious
                // unfortunately, there's literally no better way to represent the problem
                select! {
                   _ = data_tx.closed() => break,
                        read = device.read(&mut buf) => { let len = match read {
                        Ok(l) => l,
                        Err(e) => {
                            data_tx.send(Err(e.into()));
                            break;
                        }
                    };
                    if len == 0 {
                        data_tx.send(Err(eyre!("Out of bytes to read!")));
                    } else {
                        let v = Vec::from(&buf[0..len]);
                        data_tx.send(String::from_utf8(v).map_err(|e| e.into()));
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
                        Some(RTS(b)) => {
                            let Err(e) = device.write_request_to_send(b) else {
                                continue;
                            };
                            e.into()
                        }
                        None => eyre!("Serial terminal closed!"),
                    };
                    data_tx.send(Err(err));
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
