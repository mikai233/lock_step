use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{error, info, warn};
use rand::RngCore;
use tokio::sync::Mutex;

use proto::message::{
    CreateRoomReq, CreateRoomResp, FrameReq, JoinRoomReq, JoinRoomResp, LoadingProgressUpdateReq,
    LoadingProgressUpdateResp, LoginReq, LoginResp, PingReq, PingResp, ReconnectReq, StartGameReq,
    StartGameResp, StopGameReq, StopGameResp,
};

use crate::player::{Player, LOGIC_FRAME_RATE};
use crate::room::{Room, RoomStatus};

pub async fn handle_login_req(player: &mut Player, req: LoginReq) -> anyhow::Result<()> {
    player.id = req.player_id;
    player.write_msg(Box::new(LoginResp::new())).await?;
    Ok(())
}

pub async fn handle_create_room_req(player: &mut Player, req: CreateRoomReq) -> anyhow::Result<()> {
    let mut resp = CreateRoomResp::new();
    let mut room_id = rand::thread_rng().next_u32();
    let (tick_tx, tick_rx) = tokio::sync::mpsc::channel(100);
    {
        let mut m = player.manager.lock().await;
        while m.rooms.contains_key(&room_id) {
            room_id = rand::thread_rng().next_u32();
        }
        player.room_id = room_id;
        let mut new_room = Room::new(tick_rx);
        resp.sead = new_room.random_seed;
        new_room.client_tx.insert(player.id, player.tx.clone());
        player.room_tx = Some(new_room.tx.clone());
        let new_room = Arc::new(Mutex::new(new_room));
        m.rooms.insert(room_id, new_room.clone());
        tokio::spawn(async move {
            loop {
                let mut room = new_room.lock().await;
                if !room.client_tx.is_empty() {
                    match room.tick().await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("{}", e);
                            break;
                        }
                    };
                } else {
                    info!("room: {} tick done", room_id);
                    break;
                }
            }
        });
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1) / LOGIC_FRAME_RATE);
        loop {
            interval.tick().await;
            match tick_tx.send(()).await {
                Ok(_) => {}
                Err(e) => {
                    info!("send tick err:{}", e);
                    break;
                }
            };
        }
    });
    resp.room_id = room_id;
    player.write_msg(Box::new(resp)).await?;
    Ok(())
}

pub async fn handle_loading_progress_update_req(
    player: &mut Player,
    req: LoadingProgressUpdateReq,
) -> anyhow::Result<()> {
    if let Some(room_tx) = &player.room_tx {
        room_tx.send(Box::new(req))?;
    };
    player
        .write_msg(Box::new(LoadingProgressUpdateResp::new()))
        .await?;
    Ok(())
}

pub async fn handle_start_game_req(player: &mut Player, req: StartGameReq) -> anyhow::Result<()> {
    if let Some(room_tx) = &player.room_tx {
        room_tx.send(Box::new(req))?;
    };
    player.write_msg(Box::new(StartGameResp::new())).await?;
    Ok(())
}

pub async fn handle_frame_req(player: &mut Player, req: FrameReq) -> anyhow::Result<()> {
    if let Some(room_tx) = &player.room_tx {
        room_tx.send(Box::new(req))?;
    };
    Ok(())
}

pub async fn handle_join_room_req(player: &mut Player, req: JoinRoomReq) -> anyhow::Result<()> {
    let mut resp = JoinRoomResp::new();
    {
        let manager = player.manager.lock().await;
        if let Some(room) = manager.rooms.get(&req.room_id) {
            let mut room = room.lock().await;
            if room.status == RoomStatus::Waiting {
                room.client_tx.insert(player.id, player.tx.clone());
                player.room_tx = Some(room.tx.clone());
                resp.room_id = req.room_id;
                resp.seat_id = 1;
                resp.sead = room.random_seed;
            } else {
                warn!(
                    "player:{} try to join a non waiting room:{}",
                    player.id, req.room_id
                );
            }
        }
    }
    player.write_msg(Box::new(resp)).await?;
    Ok(())
}

pub async fn handle_ping_req(player: &mut Player, req: PingReq) -> anyhow::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    player.last_ping = now;
    let r = tokio::spawn(async {});
    player.write_msg(Box::new(PingResp::new())).await?;
    Ok(())
}

pub async fn handle_reconnect_req(player: &mut Player, req: ReconnectReq) -> anyhow::Result<()> {
    let room_id = req.room_id;
    {
        let mut m = player.manager.lock().await;
        if let Some(room) = m.rooms.get_mut(&req.room_id) {
            let mut room = room.lock().await;
            room.client_tx.insert(player.id, player.tx.clone());
            player.room_tx = Option::from(room.tx.clone());
            info!("player: {} reconnect", player.id);
        } else {
            warn!("player: {} room: {} not exists", player.id, req.room_id);
        }
    }
    if let Some(room_tx) = &player.room_tx {
        room_tx.send(Box::new(req))?;
        info!(
            "player: {} send reconnect msg to room: {}",
            player.id, room_id
        );
    };
    Ok(())
}

pub async fn handle_stop_game_req(player: &mut Player, req: StopGameReq) -> anyhow::Result<()> {
    if let Some(room_tx) = &player.room_tx {
        room_tx.send(Box::new(req))?;
        player.write_msg(Box::new(StopGameResp::new())).await?;
    };
    Ok(())
}
