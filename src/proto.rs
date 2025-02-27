//! Protocol definitions necessary for this MITM proxy

// ---------- wl_display ---------

use byteorder::{ByteOrder, NativeEndian};
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

pub trait WlParsedMessage<'a>: Sized {
    fn opcode() -> u16;
    fn object_type() -> WlObjectType;
    fn try_from_msg<'obj>(
        objects: &'obj WlObjects,
        msg: &'a WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<Self> {
        // Verify object type and opcode
        if objects.lookup_object(msg.obj_id) != Some(Self::object_type()) {
            return WaylandProtocolParsingOutcome::IncorrectObject;
        }

        if msg.opcode != Self::opcode() {
            return WaylandProtocolParsingOutcome::IncorrectOpcode;
        }

        Self::try_from_msg_impl(msg)
    }

    fn try_from_msg_impl(msg: &'a WlRawMsg) -> WaylandProtocolParsingOutcome<Self>;
}

wayland_proto_gen!("proto/wayland.xml");

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;
/// Opcode for binding the wl_registry object
pub const WL_DISPLAY_GET_REGISTRY_OPCODE: u16 = 1;

pub struct WlDisplayGetRegistry {
    pub registry_new_id: u32,
}

impl WlParsedMessage<'_> for WlDisplayGetRegistry {
    fn object_type() -> WlObjectType {
        WL_DISPLAY
    }

    fn opcode() -> u16 {
        WL_DISPLAY_GET_REGISTRY_OPCODE
    }

    fn try_from_msg_impl(msg: &WlRawMsg) -> WaylandProtocolParsingOutcome<WlDisplayGetRegistry> {
        let payload = msg.payload();

        if payload.len() != 4 {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        }

        WaylandProtocolParsingOutcome::Ok(WlDisplayGetRegistry {
            registry_new_id: NativeEndian::read_u32(msg.payload()),
        })
    }
}

// ---------- wl_registry ---------

/// Opcode for server->client "global" events
pub const WL_REGISTRY_GLOBAL_OPCODE: u16 = 0;
/// Opcode for client->server "bind" requests
pub const WL_REGISTRY_BIND_OPCODE: u16 = 0;

pub struct WlRegistryGlobalEvent<'a> {
    pub name: u32,
    pub interface: &'a str,
    pub version: u32,
}

impl<'a> WlParsedMessage<'a> for WlRegistryGlobalEvent<'a> {
    fn opcode() -> u16 {
        WL_REGISTRY_GLOBAL_OPCODE
    }

    fn object_type() -> WlObjectType {
        WL_REGISTRY
    }

    fn try_from_msg_impl(
        msg: &'a WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<WlRegistryGlobalEvent<'a>> {
        let payload = msg.payload();

        if payload.len() < 8 {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        }

        let name = NativeEndian::read_u32(&payload[0..4]);
        let interface_len = NativeEndian::read_u32(&payload[4..8]);

        if interface_len + 4 >= payload.len() as u32 {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        }

        let version = NativeEndian::read_u32(&payload[payload.len() - 4..]);
        // -1 because of 0-terminator
        let Ok(interface) = std::str::from_utf8(&payload[8..8 + interface_len as usize - 1]) else {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        };

        WaylandProtocolParsingOutcome::Ok(WlRegistryGlobalEvent {
            name,
            interface,
            version,
        })
    }
}

pub struct WlRegistryBind {
    pub name: u32,
    pub new_id: u32,
}

impl<'a> WlParsedMessage<'a> for WlRegistryBind {
    fn opcode() -> u16 {
        WL_REGISTRY_BIND_OPCODE
    }

    fn object_type() -> WlObjectType {
        WL_REGISTRY
    }

    fn try_from_msg_impl(msg: &'a WlRawMsg) -> WaylandProtocolParsingOutcome<WlRegistryBind> {
        let payload = msg.payload();

        if payload.len() < 8 {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        }

        let name = NativeEndian::read_u32(&payload[..4]);
        let new_id = NativeEndian::read_u32(&payload[4..8]);
        WaylandProtocolParsingOutcome::Ok(WlRegistryBind { name, new_id })
    }
}
