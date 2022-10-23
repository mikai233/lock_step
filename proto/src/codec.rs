use std::fmt::{Display, Formatter};
use std::io;

use bytes::{BufMut, BytesMut};
use protobuf::MessageDyn;
use tokio_util::codec::{Decoder, Encoder};

use crate::tool::{get_proto_id, MsgType};

pub struct ProtoCodec {
    codec_type: MsgType,
}

impl ProtoCodec {
    pub fn new(codec_type: MsgType) -> Self {
        ProtoCodec {
            codec_type
        }
    }
}

impl Decoder for ProtoCodec {
    type Item = (i32, Vec<u8>);
    type Error = ProtoCodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let buf_len = src.len();
        if buf_len < 2 {
            return Ok(None);
        }
        let mut package_len_bytes = [0u8; 2];
        package_len_bytes.copy_from_slice(&src[..2]);
        let package_len = u16::from_be_bytes(package_len_bytes) as usize;
        return if buf_len < package_len {
            src.reserve(package_len - buf_len);
            Ok(None)
        } else {
            let src = src.split_to(package_len);
            let mut id_bytes = [0u8; 2];
            id_bytes.copy_from_slice(&src[2..4]);
            let id = u16::from_be_bytes(id_bytes) as i32;
            let mut msg_bytes = vec![0u8; package_len - 4];
            msg_bytes.copy_from_slice(&src[4..package_len]);
            Ok(Some((id, msg_bytes)))
        };
    }
}

impl Encoder<Box<dyn MessageDyn>> for ProtoCodec {
    type Error = ProtoCodecError;

    fn encode(&mut self, msg: Box<dyn MessageDyn>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let id = get_proto_id(&*msg, self.codec_type.into())?;
        let body = msg.write_to_bytes_dyn()?;
        let package_len = 2 + 2 + body.len();
        dst.put_u16(package_len as u16);
        dst.put_u16(id as u16);
        dst.put_slice(body.as_slice());
        Ok(())
    }
}

#[derive(Debug)]
pub enum ProtoCodecError {
    Protobuf(protobuf::Error),
    Anyhow(anyhow::Error),
    Io(io::Error),
}

impl Display for ProtoCodecError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoCodecError::Io(e) => {
                write!(f, "{}", e)
            }
            ProtoCodecError::Protobuf(e) => {
                write!(f, "{}", e)
            }
            ProtoCodecError::Anyhow(e) => {
                write!(f, "{}", e)
            }
        }
    }
}

impl From<io::Error> for ProtoCodecError {
    fn from(e: io::Error) -> ProtoCodecError {
        ProtoCodecError::Io(e)
    }
}

impl From<anyhow::Error> for ProtoCodecError {
    fn from(value: anyhow::Error) -> Self {
        ProtoCodecError::Anyhow(value)
    }
}

impl From<protobuf::Error> for ProtoCodecError {
    fn from(value: protobuf::Error) -> Self {
        ProtoCodecError::Protobuf(value)
    }
}

impl std::error::Error for ProtoCodecError {}