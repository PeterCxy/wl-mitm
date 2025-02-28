//! Protocol definitions necessary for this MITM proxy

// ---------- wl_display ---------

use byteorder::ByteOrder;
use protogen::wayland_proto_gen;

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjectTypeId, WlObjects},
};

macro_rules! reject_malformed {
    ($e:expr) => {
        if let crate::proto::WaylandProtocolParsingOutcome::MalformedMessage = $e {
            return false;
        } else if let crate::proto::WaylandProtocolParsingOutcome::Ok(e) = $e {
            Some(e)
        } else {
            None
        }
    };
}

macro_rules! decode_and_match_msg {
    ($objects:expr, match $msg:ident {$($t:ty => $act:block$(,)?)+}) => {
        $(
            if let Some($msg) = reject_malformed!(<$t as crate::proto::WlParsedMessage>::try_from_msg(&$objects, $msg)) {
                $act
            }
        )+
    };
}

pub enum WaylandProtocolParsingOutcome<T> {
    Ok(T),
    MalformedMessage,
    IncorrectObject,
    IncorrectOpcode,
}

pub trait WlParsedMessage<'a> {
    fn opcode() -> u16;
    fn object_type() -> WlObjectType;
    fn try_from_msg<'obj>(
        objects: &'obj WlObjects,
        msg: &'a WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<Self>
    where
        Self: Sized,
    {
        // Verify object type and opcode
        if objects.lookup_object(msg.obj_id) != Some(Self::object_type()) {
            return WaylandProtocolParsingOutcome::IncorrectObject;
        }

        if msg.opcode != Self::opcode() {
            return WaylandProtocolParsingOutcome::IncorrectOpcode;
        }

        Self::try_from_msg_impl(msg)
    }

    fn try_from_msg_impl(msg: &'a WlRawMsg) -> WaylandProtocolParsingOutcome<Self>
    where
        Self: Sized;
}

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;

wayland_proto_gen!("proto/wayland.xml");
