use std::{thread, time::Duration};

use log::LevelFilter;
use ws_logger::*;

fn main() {
    //env für Websocket "FLEX_WS_LOGGER_TARGET"

    WebsocketLogger::new("ws://localhost")
        .unwrap()
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap();

    for i in 0..1_0 {
        log::warn!("IT WORKS warn {}", i);

        log::error!("IT WORKS error {}", i);

        log::trace!("IT WORKS trace {}", i);

        log::info!("IT WORKS info {}", i);

        log::debug!("IT WORKS debug {}", i);
    }

    thread::sleep(Duration::from_secs(60));
}
