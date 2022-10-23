use std::fmt::{Display, Formatter};

use anyhow::anyhow;
use protobuf::{EnumFull, MessageDyn};
use protobuf::reflect::EnumValueDescriptor;

use crate::cs::MsgCS;
use crate::sc::MsgSC;

#[derive(Debug, Copy, Clone)]
pub enum MsgType {
    CS,
    SC,
}

impl Display for MsgType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MsgType::CS => write!(f, "cs"),
            MsgType::SC => write!(f, "sc"),
        }
    }
}

pub fn get_proto_id(msg: &dyn MessageDyn, msg_type: MsgType) -> anyhow::Result<i32> {
    let descriptor = get_proto_enum_descriptor(msg, msg_type)?;
    Ok(descriptor.value())
}

pub fn get_proto_enum_descriptor(msg: &dyn MessageDyn, msg_type: MsgType) -> anyhow::Result<EnumValueDescriptor> {
    let type_descriptor = match msg_type {
        MsgType::CS => MsgCS::enum_descriptor(),
        MsgType::SC => MsgSC::enum_descriptor(),
    };
    let msg_descriptor = msg.descriptor_dyn();
    let msg_name = msg_descriptor.name();
    let descriptor = type_descriptor.value_by_name(&*format!("{}E", msg_name)).ok_or(anyhow::Error::msg(format!("{} not found in {}", msg_name, msg_type)))?;
    Ok(descriptor)
}

pub fn get_cs_proto_enum(msg: &dyn MessageDyn) -> anyhow::Result<Option<MsgCS>> {
    let descriptor = get_proto_enum_descriptor(msg, MsgType::CS)?;
    Ok(descriptor.cast())
}

pub fn get_sc_proto_enum(msg: &dyn MessageDyn) -> anyhow::Result<Option<MsgSC>> {
    let descriptor = get_proto_enum_descriptor(msg, MsgType::SC)?;
    Ok(descriptor.cast())
}

// pub fn get_id_proto(id: i32, msg_type: MsgType) -> anyhow::Result<(Option<MsgCS>, Option<MsgSC>)> {
//     let type_descriptor = match msg_type {
//         MsgType::CS => MsgCS::enum_descriptor(),
//         MsgType::SC => MsgSC::enum_descriptor(),
//     };
//     let desc = type_descriptor.value_by_number(id).ok_or(anyhow!("id:{} not found in {}", id, msg_type))?;
//     match msg_type {
//         MsgType::CS => Ok((desc.cast::<MsgCS>(), None)),
//         MsgType::SC => Ok((None, desc.cast::<MsgSC>()))
//     }
// }

pub fn get_cs_id_proto_enum(id: i32) -> anyhow::Result<Option<MsgCS>> {
    let type_descriptor = MsgCS::enum_descriptor();
    let desc = type_descriptor.value_by_number(id).ok_or(anyhow!("id:{} not found in cs", id))?;
    Ok(desc.cast())
}

pub fn get_sc_id_proto_enum(id: i32) -> anyhow::Result<Option<MsgSC>> {
    let type_descriptor = MsgSC::enum_descriptor();
    let desc = type_descriptor.value_by_number(id).ok_or(anyhow!("id:{} not found in sc", id))?;
    Ok(desc.cast())
}