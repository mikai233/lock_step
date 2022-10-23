use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use futures::{SinkExt, StreamExt};
use log::{error, info, warn};
use protobuf::{EnumFull, Message, MessageDyn};
use tokio::select;
use tokio::sync::mpsc::{Receiver, Sender, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tokio_kcp::KcpStream;
use tokio_util::codec::Framed;

use proto::codec::ProtoCodec;
use proto::cs::MsgCS;
use proto::message::{CreateRoomReq, FrameReq, JoinRoomReq, LoadingProgressUpdateReq, LoginReq, PingReq, ReconnectReq, SCBlock, StartGameReq, StopGameReq};
use proto::tool::get_cs_id_proto_enum;

use crate::handler::{handle_create_room_req, handle_frame_req, handle_join_room_req, handle_loading_progress_update_req, handle_login_req, handle_ping_req, handle_reconnect_req, handle_start_game_req, handle_stop_game_req};
use crate::room::RoomManager;

pub type MessageReceiver = Receiver<Box<dyn MessageDyn>>;
pub type MessageSender = Sender<Box<dyn MessageDyn>>;
pub type UnboundMessageReceiver = UnboundedReceiver<Box<dyn MessageDyn>>;
pub type UnboundMessageSender = UnboundedSender<Box<dyn MessageDyn>>;

pub const ROOM_SEAT: usize = 5;
//逻辑帧率
pub const LOGIC_FRAME_RATE: u32 = 15;

pub const SESSION_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Player {
    pub id: u32,
    pub frame_id: u32,
    pub tx: UnboundMessageSender,
    pub rx: UnboundMessageReceiver,
    pub connection: Framed<KcpStream, ProtoCodec>,
    pub manager: Arc<Mutex<RoomManager>>,
    pub room_id: u32,
    pub room_tx: Option<UnboundMessageSender>,
    pub last_ping: Duration,
}

pub enum PlayerStatus {
    Ready,
    Gaming,
}

impl Player {
    pub fn new(
        manager: Arc<Mutex<RoomManager>>,
        connection: Framed<KcpStream, ProtoCodec>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Player {
            id: 0,
            frame_id: 0,
            tx,
            rx,
            connection,
            manager,
            room_id: 0,
            room_tx: None,
            last_ping: SystemTime::now().duration_since(UNIX_EPOCH).unwrap(),
        }
    }

    pub async fn write_msg(&mut self, msg: Box<dyn MessageDyn>) -> anyhow::Result<()> {
        self.connection.send(msg).await?;
        Ok(())
    }

    pub async fn process_server_msg(&mut self) -> anyhow::Result<()> {
        loop {
            select! {
                Some(resp) = self.rx.recv() => {
                    match tokio::time::timeout(SESSION_TIMEOUT, self.connection.send(resp)).await {
                        Ok(_) => {}
                        Err(_) => {
                            info!("player:{} send msg time out, close the session",self.id);
                            break;
                        }
                    }
                }
                req = tokio::time::timeout(SESSION_TIMEOUT, self.connection.next()) => match req {
                    Ok(Some(Ok(req))) => {
                        self.dispatch_server_message(req.0, req.1).await?;
                    }
                    Ok(Some(Err(e))) => {
                        error!("process server msg err: {}", e);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        info!("player:{} {}, close the session",e,self.id);
                        break;
                    }
                }
            }
        }
        let mut manager = self.manager.lock().await;
        let rooms = &mut manager.rooms;
        let empty_room = match rooms.get(&self.room_id) {
            None => false,
            Some(room) => {
                let mut room = room.lock().await;
                room.client_tx.remove(&self.id);
                info!(
                    "player: {} disconnect, remove player from room: {}",
                    self.id, self.room_id
                );
                room.client_tx.is_empty()
            }
        };
        if empty_room {
            rooms.remove(&self.room_id);
            info!("there's no clients, destroy room {}", self.room_id);
        }
        Ok(())
    }

    pub async fn dispatch_server_message(
        &mut self,
        id: i32,
        msg_bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        match get_cs_id_proto_enum(id) {
            Ok(Some(msg_cs)) => {
                info!(
                    "player: {} dispatch: {}",
                    self.id,
                    msg_cs.descriptor().name()
                );
                if msg_cs != MsgCS::LoginReqE && self.id == 0 {
                    let mut block = SCBlock::new();
                    block.message = "player not login".to_string();
                    self.write_msg(Box::new(block)).await?;
                    return Err(anyhow!("player not login"));
                }
                match msg_cs {
                    MsgCS::UnusedCS => {}
                    MsgCS::LoginReqE => {
                        handle_login_req(self, LoginReq::parse_from_bytes(&*msg_bytes)?).await?
                    }
                    MsgCS::CreateRoomReqE => {
                        handle_create_room_req(self, CreateRoomReq::parse_from_bytes(&*msg_bytes)?)
                            .await?
                    }
                    MsgCS::JoinRoomReqE => {
                        handle_join_room_req(self, JoinRoomReq::parse_from_bytes(&*msg_bytes)?)
                            .await?
                    }
                    MsgCS::LoadingProgressUpdateReqE => {
                        handle_loading_progress_update_req(
                            self,
                            LoadingProgressUpdateReq::parse_from_bytes(&*msg_bytes)?,
                        )
                            .await?
                    }
                    MsgCS::StartGameReqE => {
                        handle_start_game_req(self, StartGameReq::parse_from_bytes(&*msg_bytes)?)
                            .await?
                    }
                    MsgCS::FrameReqE => {
                        handle_frame_req(self, FrameReq::parse_from_bytes(&*msg_bytes)?).await?
                    }
                    MsgCS::PingReqE => {
                        handle_ping_req(self, PingReq::parse_from_bytes(&*msg_bytes)?).await?
                    }
                    MsgCS::ReconnectReqE => {
                        handle_reconnect_req(self, ReconnectReq::parse_from_bytes(&*msg_bytes)?)
                            .await?;
                    }
                    MsgCS::StopGameReqE => {
                        handle_stop_game_req(self, StopGameReq::parse_from_bytes(&*msg_bytes)?).await?;
                    }
                }
            }
            Ok(None) => {
                warn!("{} not found in cs", id)
            }
            Err(err) => {
                warn!("dispatch server msg err: {}", err);
            }
        };
        Ok(())
    }
}
