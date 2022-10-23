use std::sync::Arc;

use futures::SinkExt;
use log::{error, info};
use tokio::sync::Mutex;
use tokio_kcp::KcpStream;
use tokio_util::codec::Framed;

use proto::codec::ProtoCodec;
use proto::tool::MsgType;

use crate::player::Player;
use crate::RoomManager;

pub async fn new_connection(manager: Arc<Mutex<RoomManager>>, conn: KcpStream) {
    let framed = Framed::new(conn, ProtoCodec::new(MsgType::SC));
    let mut player = Player::new(manager, framed);
    tokio::spawn(async move {
        match player.process_server_msg().await {
            Ok(_) => {}
            Err(e) => {
                error!("{}", e);
            }
        };
        info!("player:{} session closed", player.id);
    });
}
