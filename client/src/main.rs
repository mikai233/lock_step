use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::time::Duration;

use futures::SinkExt;
use log::{error, info, warn};
use protobuf::{EnumFull, Message, MessageDyn, MessageField};
use rand::random;
use tokio::macros::support::thread_rng_n;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::{io, select};
use tokio_kcp::KcpStream;
use tokio_stream::StreamExt;
use tokio_util::codec::{BytesCodec, Framed, FramedRead};

use proto::codec::ProtoCodec;
use proto::message::{
    CreateRoomReq, CreateRoomResp, Frame, FrameReq, Input, JoinRoomReq, JoinRoomResp, LoginReq,
    PingReq, ReconnectReq, ReconnectResp, SCBlock, SCFrame, StartGameReq, StopGameReq,
};
use proto::sc::MsgSC;
use proto::tool::{get_sc_id_proto_enum, MsgType};

#[derive(PartialEq, Debug)]
enum ClientStatus {
    Idle,
    Gaming,
    Reconnecting,
}

struct Client {
    player_id: u32,
    conn: Framed<KcpStream, ProtoCodec>,
    tx: Sender<Box<dyn MessageDyn>>,
    rx: Receiver<Box<dyn MessageDyn>>,
    status: ClientStatus,
    state: HashMap<u32, State>,
    frame_id: u32,
    pending: Vec<Frame>,
}

#[derive(Default)]
struct State {
    x: i32,
    y: i32,
}

impl Display for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "x:{} y:{}", self.x, self.y)
    }
}

impl Client {
    pub fn new(conn: Framed<KcpStream, ProtoCodec>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        Self {
            player_id: 0,
            conn,
            tx,
            rx,
            status: ClientStatus::Idle,
            state: HashMap::new(),
            frame_id: 0,
            pending: vec![],
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    log4rs::init_file("log4rs.yaml", Default::default()).unwrap();
    let mut cfg = tokio_kcp::KcpConfig::default();
    cfg.flush_write = true;
    cfg.flush_acks_input = true;
    cfg.stream = true;
    cfg.nodelay = tokio_kcp::KcpNoDelayConfig::fastest();
    let stream = tokio_kcp::KcpStream::connect(&cfg, "127.0.0.1:7789".parse()?).await?;
    let framed = Framed::new(stream, ProtoCodec::new(MsgType::CS));
    let mut client = Client::new(framed);
    let stdin = FramedRead::new(io::stdin(), BytesCodec::new());
    let mut stdin = stdin.map(|i| i.map(|bytes| bytes.freeze()));

    loop {
        select! {
            input = stdin.next() => match input {
                Some(Ok(input)) => {
                    process_client_input(input, &mut client).await?;
                }
                Some(Err(e)) => {
                    error!("{}", e);
                }
                None => break,
            },
            resp = client.conn.next() => match resp {
                Some(Ok(resp)) => {
                    let (id, msg_bytes) = resp;
                    let sc = get_sc_id_proto_enum(id)?;
                    dispatch_client_msg(&mut client, sc.unwrap(), msg_bytes).await?
                }
                Some(Err(e)) => {
                    error!("{}", e);
                }
                None => break,
            },
            req = client.rx.recv() => match req {
                Some(req) => {
                    client.conn.send(req).await?;
                }
                None => break,
            },
            ctr = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }
    Ok(())
}

async fn dispatch_client_msg(
    client: &mut Client,
    sc_type: MsgSC,
    msg_bytes: Vec<u8>,
) -> anyhow::Result<()> {
    if sc_type != MsgSC::SCFrameE && sc_type != MsgSC::PingRespE {
        info!("receive server msg {}", sc_type.descriptor().name());
    }
    match sc_type {
        MsgSC::UnusedSC => {}
        MsgSC::LoginRespE => {
            client.conn.send(Box::new(PingReq::new())).await?;
        }
        MsgSC::CreateRoomRespE => {
            let create_resp = CreateRoomResp::parse_from_bytes(&*msg_bytes)?;
            info!("create room {}", create_resp.room_id);
        }
        MsgSC::JoinRoomRespE => {
            let join_resp = JoinRoomResp::parse_from_bytes(&*msg_bytes)?;
            if join_resp.room_id != 0 {
                info!("successfully join room {}", join_resp.room_id);
            } else {
                warn!("failed to join room")
            }
        }
        MsgSC::LoadingProgressUpdateRespE => {}
        MsgSC::StartGameRespE => {
            client.status = ClientStatus::Gaming;
        }
        MsgSC::FrameRespE => {}
        MsgSC::PingRespE => {
            let tx = client.tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = tx.send(Box::new(PingReq::new())).await;
            });
        }
        MsgSC::ReconnectRespE => {
            let mut resp = ReconnectResp::parse_from_bytes(&*msg_bytes)?;
            info!("receive reconnect frames:{}", resp.frames.len());
            //追帧
            resp.frames.extend_from_slice(&*client.pending);
            for f in &resp.frames {
                for i in &f.inputs {
                    let mut s = client.state.entry(i.id).or_insert(State::default());
                    s.x = s.x.wrapping_add(i.x);
                    s.y = s.y.wrapping_add(i.y);
                }
            }
            if resp.frames.is_empty() {
                client.status = ClientStatus::Gaming;
                client.pending.clear();
                info!("追帧完成");
            }
        }
        MsgSC::SCFrameE => {
            let sc_frame = SCFrame::parse_from_bytes(&*msg_bytes)?;
            let f = sc_frame.frame;
            // info!("frame id:{}", f.frame_id);
            client.frame_id = f.frame_id;
            if client.status == ClientStatus::Reconnecting {
                client.pending.push(f.unwrap());
                return Ok(());
            }
            for i in &f.inputs {
                let mut s = client.state.entry(i.id).or_insert(State::default());
                s.x = s.x.wrapping_add(i.x);
                s.y = s.y.wrapping_add(i.y);
            }
            let mut frame_req = FrameReq::new();
            let mut frame = Frame::new();
            frame.frame_id = client.frame_id + 1;
            for _ in 0..thread_rng_n(10) {
                let x: i32 = random();
                let y: i32 = random();
                let mut input = Input::new();
                input.x = x;
                input.y = y;
                input.id = client.player_id;
                frame.inputs.push(input);
            }
            frame_req.frame = MessageField::some(frame);
            client.conn.send(Box::new(frame_req)).await?;
        }
        MsgSC::SCStartGameE => {
            client.status = ClientStatus::Gaming;
        }
        MsgSC::SCLoadingProgressUpdateE => {}
        MsgSC::SCBlockE => {
            let info = SCBlock::parse_from_bytes(&*msg_bytes)?;
            info!("{}", info.message);
        }
        MsgSC::StopGameRespE | MsgSC::SCStopGameE => {
            for (player_id, state) in &client.state {
                info!("player:{} state:{}", player_id, state);
            }
        }
    }
    Ok(())
}

async fn process_client_input(input: bytes::Bytes, client: &mut Client) -> anyhow::Result<()> {
    let cmd = String::from_utf8_lossy(input.as_ref())
        .trim_end()
        .to_string();
    if cmd.starts_with("login") {
        let cmd = cmd.splitn(2, ' ').collect::<Vec<&str>>();
        let player_id: u32 = cmd[1].parse()?;
        info!("login with player id {}", player_id);
        let mut login_req = LoginReq::new();
        login_req.player_id = player_id;
        client.player_id = player_id;
        client.conn.send(Box::new(login_req)).await?;
    }
    if cmd == "create" {
        let create_req = CreateRoomReq::new();
        client.conn.send(Box::new(create_req)).await?;
    }
    if cmd.starts_with("join") {
        let cmd = cmd.splitn(2, ' ').collect::<Vec<&str>>();
        let room_id: u32 = cmd[1].parse()?;
        let mut join_req = JoinRoomReq::new();
        join_req.room_id = room_id;
        client.conn.send(Box::new(join_req)).await?;
    }
    if cmd == "start" {
        let start_req = StartGameReq::new();
        client.conn.send(Box::new(start_req)).await?;
    }

    if cmd.starts_with("reconnect") {
        let cmd = cmd.splitn(2, ' ').collect::<Vec<&str>>();
        let room_id: u32 = cmd[1].parse()?;
        let mut req = ReconnectReq::new();
        req.player_id = client.player_id;
        req.room_id = room_id;
        client.status = ClientStatus::Reconnecting;
        client.conn.send(Box::new(req)).await?;
    }

    if cmd == "stop" {
        let stop_req = Box::new(StopGameReq::new());
        client.conn.send(stop_req).await?;
    }
    Ok(())
}
