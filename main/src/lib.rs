use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_std::sync::Mutex;
use futures::channel::mpsc;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

use chrono::Local;
use futures::prelude::*;
use protobuf_model::{LogEntry, LogLevel};

use prost::*;

use async_tungstenite::{async_std::connect_async, tungstenite::Message as WebsocketMessage};
use url::Url;

mod protobuf_model;

const PING_TIMEOUT: Duration = Duration::from_secs(30);
const PING_INTERVAL: Duration = Duration::from_secs(5);
const BUFFER_SIZE: usize = 100_000;

enum MessageToSend {
    Pong,
    Payload(Arc<LogEntry>),
}

pub struct WebsocketLogger {
    level: LevelFilter,
    send_queue: Arc<Mutex<mpsc::Sender<Arc<LogEntry>>>>,
}

lazy_static::lazy_static! {
    static ref HOSTNAME: String = hostname::get()
        .ok()
        .and_then(|s|s.to_str().map(|s|s.into()))
        .unwrap_or_else(||"<unknown host>".into());
}

impl WebsocketLogger {
    #[must_use = "You must call init() to begin logging"]
    pub fn new(log_target_url: &str) -> Result<WebsocketLogger, String> {
        //
        let log_target_url = Url::parse(log_target_url)
            .map_err(|err| format!("could not parse ws logget target url: {}", err))?;

        let (send_queue, receiver) = mpsc::channel::<Arc<LogEntry>>(BUFFER_SIZE);

        async_std::task::spawn(run_websocket(receiver, log_target_url));

        Ok(WebsocketLogger {
            level: LevelFilter::Debug,
            send_queue: Arc::new(Mutex::new(send_queue)),
        })
    }

    #[must_use = "You must call init() to begin logging"]
    pub fn with_level(mut self, level: LevelFilter) -> WebsocketLogger {
        self.level = level;
        self
    }

    /// 'Init' the actual logger, instantiate it and configure it,
    /// this method MUST be called in order for the logger to be effective.
    pub fn init(self) -> Result<(), SetLoggerError> {
        //#[cfg(all(windows, feature = "colored"))]
        //set_up_color_terminal();

        log::set_max_level(self.level);
        log::set_boxed_logger(Box::new(self))?;
        Ok(())
    }
}

impl Log for WebsocketLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        &metadata.level().to_level_filter() <= &self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_string = { record.level().to_string() };
            let target = if !record.target().is_empty() {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };

            let now = Local::now().naive_local();

            println!(
                "{} {:<5} [{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S,%3f"),
                level_string,
                target,
                record.args()
            );

            let entry = Arc::new(LogEntry {
                message_type: 0,
                host: HOSTNAME.clone(),
                micro_second_timestamp: now.timestamp() * 1_000_000
                    + now.timestamp_subsec_micros() as i64,
                log_level: (match record.level() {
                    Level::Error => LogLevel::Error,
                    Level::Warn => LogLevel::Warn,
                    Level::Info => LogLevel::Info,
                    Level::Debug => LogLevel::Debug,
                    Level::Trace => LogLevel::Trace,
                }) as i32,
                target: target.to_string(),
                args: format!("{}", record.args()),
            });

            let send_queue = self.send_queue.clone();

            async_std::task::spawn(async move {
                let send = async {
                    let mut lck = send_queue.lock().await;
                    let _ = lck.send(entry).await;
                    drop(lck);
                };

                futures::select! {
                    _ = send.fuse() => (),
                    _ = async_std::task::sleep(Duration::from_secs(1)).fuse() => (),
                }
            });
        }
    }

    fn flush(&self) {}
}

async fn run_websocket(rx: mpsc::Receiver<Arc<LogEntry>>, mut log_target_url: Url) {
    //
    let (mut pong_tx, pong_rx) = mpsc::channel::<MessageToSend>(BUFFER_SIZE);
    //
    let mut source = futures::stream::select(pong_rx, rx.map(|r| MessageToSend::Payload(r)));

    log_target_url.set_path("ws-logger-target");

    loop {
        let ws_stream = match connect_async(&log_target_url).await {
            Ok((wss, _)) => wss,
            Err(err) => {
                println!(
                    "WS LOGGER: could not etablish web socket connection to '{}': {}",
                    log_target_url, err
                );
                async_std::task::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let (mut sink, mut read) = ws_stream.split();

        let last_ping = Mutex::new(Instant::now());

        let mut pong_tx_1 = pong_tx.clone();

        let handle_incoming_payload = async {
            //
            while let Some(msg_result) = read.next().await {
                match msg_result {
                    Err(err) => {
                        println!("WS LOGGER: could not receive websocket message: {:?}", err);
                        break;
                    }
                    Ok(msg) => match msg {
                        WebsocketMessage::Ping(_) => {
                            let now = Instant::now();
                            let mut lck = last_ping.lock().await;
                            *lck = now;
                            drop(lck);

                            let _ = pong_tx_1.send(MessageToSend::Pong).await;
                        }
                        // WebsocketMessage::Binary(data) => {
                        //     let r: ForwardedRequest = ForwardedRequest::decode(data.as_slice())
                        //         .map_err(|err| {
                        //             format!("could not parse incoming message: {:?}", err)
                        //         })?;

                        //     //println!("WS LOGGER: Received req: {:?}", r);

                        //     sender.send(r).await.map_err(|err| {
                        //         format!("could send incoming request internal: {:?}", err)
                        //     })?;
                        // }
                        _ => {
                            println!("WS LOGGER: Received unexpected message: {:?}", msg);
                        }
                    },
                }
            }
        };

        let send_payload = async {
            //
            while let Some(msg) = source.next().await {
                //
                match msg {
                    MessageToSend::Payload(response) => {
                        let mut data: Vec<u8> = Vec::new();

                        match response.encode(&mut data) {
                            Err(err) => {
                                println!("WS LOGGER: could not encode protobuf response: {:?}", err)
                            }

                            Ok(()) => {
                                if let Err(_) = sink.send(WebsocketMessage::Binary(data)).await {
                                    // we re-enqueue the message here
                                    let _ = pong_tx.send(MessageToSend::Payload(response)).await;
                                    break;
                                }
                            }
                        }
                    }
                    MessageToSend::Pong => {
                        let _ = sink.send(WebsocketMessage::Pong(vec![])).await;
                    }
                };
            }
        };

        let do_ping_pong = async {
            loop {
                async_std::task::sleep(PING_INTERVAL).await;

                let now = Instant::now();
                let lck = last_ping.lock().await;
                let connection_broken = now.duration_since(*lck) > PING_TIMEOUT;
                drop(lck);

                if connection_broken {
                    println!(
                        "WS LOGGER: closing connection because last ping is more then {:?} ago",
                        PING_TIMEOUT
                    );
                    break;
                }
            }
        };

        futures::select! {
            _ = handle_incoming_payload.fuse() => (),
            _ = send_payload.fuse() => (),
            _ = do_ping_pong.fuse() => (),
        };

        async_std::task::sleep(Duration::from_secs(1)).await;
    }
}
