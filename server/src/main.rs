extern crate core;

use std::sync::Arc;

use log::info;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::net::new_connection;
use crate::room::RoomManager;

mod handler;
mod net;
mod player;
mod room;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = "127.0.0.1:7789";
    log4rs::init_file("log4rs.yaml", Default::default()).unwrap();
    let mut cfg = tokio_kcp::KcpConfig::default();
    cfg.flush_write = true;
    cfg.flush_acks_input = true;
    cfg.stream = true;
    cfg.nodelay = tokio_kcp::KcpNoDelayConfig::fastest();
    let manager = Arc::new(Mutex::new(RoomManager::new()));
    let mut listener = tokio_kcp::KcpListener::bind(cfg, addr).await?;
    info!("server listening on: {}", addr);
    loop {
        let (socket, _) = listener.accept().await?;
        new_connection(manager.clone(), socket).await;
    }
}
