use std::collections::{BTreeMap, HashMap};
use std::ops::Not;
use std::sync::Arc;

use anyhow::anyhow;
use log::{error, warn};
use protobuf::{MessageDyn, MessageField};
use rand::random;
use tokio::select;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;

use proto::cs::MsgCS;
use proto::message::{
    Frame, FrameReq, LoadingProgressUpdateReq, ReconnectReq, ReconnectResp, SCFrame,
    SCLoadingProgressUpdate, SCStartGame, SCStopGame,
};
use proto::tool::get_cs_proto_enum;

use crate::player::{Player, UnboundMessageReceiver, UnboundMessageSender};

pub struct RoomManager {
    pub players: HashMap<u32, Player>,
    pub rooms: HashMap<u32, Arc<Mutex<Room>>>,
}

impl RoomManager {
    pub fn new() -> Self {
        RoomManager {
            players: HashMap::new(),
            rooms: HashMap::new(),
        }
    }
}

pub struct Room {
    frame_id: u32,
    pub random_seed: i32,
    pub client_tx: HashMap<u32, UnboundMessageSender>,
    pub tx: UnboundMessageSender,
    rx: UnboundMessageReceiver,
    tick_rx: Receiver<()>,
    pub status: RoomStatus,
    frames: BTreeMap<u32, Frame>,
}

impl Room {
    pub fn new(tick_rx: Receiver<()>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Room {
            frame_id: 1,
            random_seed: random(),
            client_tx: Default::default(),
            tx,
            rx,
            tick_rx,
            status: RoomStatus::Waiting,
            frames: Default::default(),
        }
    }

    pub async fn broadcast_msg(&mut self, msg: Box<dyn MessageDyn>) {
        for (_, tx) in &mut self.client_tx {
            if tx.is_closed().not() {
                match tx.send(msg.clone_box()) {
                    Ok(_) => {}
                    Err(e) => {
                        error!("broadcast msg err:{}", e)
                    }
                };
            }
        }
    }

    pub async fn broadcast_frame(&mut self, frame: Frame) {
        let mut sc_frame = SCFrame::new();
        sc_frame.frame = MessageField::some(frame);
        self.broadcast_msg(Box::new(sc_frame)).await;
    }

    pub fn send_to_player(&mut self, room_seat_id: i32, msg: &dyn MessageDyn) {}

    pub async fn tick(&mut self) -> anyhow::Result<()> {
        select! {
            Some(_) = self.tick_rx.recv() => {
                if self.client_tx.is_empty() {
                    self.tick_rx.close();
                    return Ok(());
                }
                if self.status == RoomStatus::Gaming {
                    let f = self.frames.entry(self.frame_id).or_insert(Frame::new());
                    f.frame_id = self.frame_id;
                    match self.frames.last_key_value() {
                        None => {}
                        Some((_, recent_frame)) => {
                            self.broadcast_frame(recent_frame.clone()).await
                        }
                    }
                    self.frame_id += 1;
                   let mut new_frame= Frame::new();
                    new_frame.frame_id=self.frame_id;
                    self.frames.insert(self.frame_id,new_frame);
                }
            }
            Some(proto_msg) = self.rx.recv() => {
                let msg_type = get_cs_proto_enum(proto_msg.as_ref())?;
                let msg_type = msg_type.ok_or(anyhow!("the message:{} enum not found in cs",proto_msg.descriptor_dyn().name()))?;
                dispatch_room_msg(self, msg_type, proto_msg).await?;
            }
        }
        Ok(())
    }
}

async fn dispatch_room_msg(
    room: &mut Room,
    msg_type: MsgCS,
    message: Box<dyn MessageDyn>,
) -> anyhow::Result<()> {
    match msg_type {
        MsgCS::LoadingProgressUpdateReqE => {
            let message = message.downcast_box::<LoadingProgressUpdateReq>().unwrap();
            let mut sc = SCLoadingProgressUpdate::new();
            sc.seat_id = message.seat_id;
            sc.progress = message.progress;
            room.broadcast_msg(Box::new(sc)).await;
        }
        MsgCS::StartGameReqE => {
            room.status = RoomStatus::Gaming;
            room.broadcast_msg(Box::new(SCStartGame::new())).await;
        }
        MsgCS::FrameReqE => {
            let frame_req = message.downcast_box::<FrameReq>().unwrap();
            if let Some(mut recent) = room.frames.last_entry() {
                let f = frame_req.frame;
                let frame = recent.get_mut();
                if frame.frame_id == f.frame_id {
                    frame.inputs.extend_from_slice(&*f.inputs);
                } else {
                    warn!(
                        "discard expired frame, req frame id:{} current frame id:{}",
                        f.frame_id, frame.frame_id
                    );
                }
            }
        }
        MsgCS::ReconnectReqE => {
            let req = message.downcast_box::<ReconnectReq>().unwrap();
            let per = 100;
            let segment = room.frames.len() / per;
            for i in 0..=segment {
                let mut resp = ReconnectResp::new();
                let low = (i * per) as u32;
                let high = low + per as u32;
                let frames = room.frames.range(low..high);
                for (_, f) in frames {
                    resp.frames.push(f.clone());
                }
                if let Some(tx) = room.client_tx.get_mut(&req.player_id) {
                    tx.send(Box::new(resp))?;
                } else {
                    warn!("reconnect player:{} tx not found in room", req.player_id);
                }
            }
            // for (id, frame) in &room.frames {
            //     resp.frames.push(frame.clone());
            // }
            if let Some(tx) = room.client_tx.get_mut(&req.player_id) {
                tx.send(Box::new(ReconnectResp::new()))?;
            } else {
                warn!("reconnect player:{} tx not found in room", req.player_id);
            }
        }
        MsgCS::StopGameReqE => {
            room.status = RoomStatus::Closing;
            room.broadcast_msg(Box::new(SCStopGame::new())).await;
        }
        _ => {}
    }
    Ok(())
}

#[derive(PartialEq)]
pub enum RoomStatus {
    Waiting,
    Ready,
    Gaming,
    Closing,
}
