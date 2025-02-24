//! Protocol definitions necessary for this MITM proxy

// ---------- wl_display ---------

use byteorder::{ByteOrder, NativeEndian};

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjects},
};

pub enum WaylandProtocolParsingOutcome<T> {
    Ok(T),
    MalformedMessage,
    IncorrectObject,
    IncorrectOpcode,
}

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;
/// Opcode for binding the wl_registry object
pub const WL_DISPLAY_GET_REGISTRY_OPCODE: u16 = 1;

pub struct WlDisplayGetRegistry {
    pub registry_new_id: u32,
}

impl WlDisplayGetRegistry {
    pub fn try_from_msg(
        objects: &WlObjects,
        msg: &WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<WlDisplayGetRegistry> {
        if objects.lookup_object(msg.obj_id) != Some(WlObjectType::WlDisplay) {
            return WaylandProtocolParsingOutcome::IncorrectObject;
        }

        if msg.opcode != WL_DISPLAY_GET_REGISTRY_OPCODE {
            return WaylandProtocolParsingOutcome::IncorrectOpcode;
        }

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

impl<'a> WlRegistryGlobalEvent<'a> {
    pub fn try_from_msg<'obj>(
        objects: &'obj WlObjects,
        msg: &'a WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<WlRegistryGlobalEvent<'a>> {
        if objects.lookup_object(msg.obj_id) != Some(WlObjectType::WlRegistry) {
            return WaylandProtocolParsingOutcome::IncorrectObject;
        }

        if msg.opcode != WL_REGISTRY_GLOBAL_OPCODE {
            return WaylandProtocolParsingOutcome::IncorrectOpcode;
        }

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
        let Ok(interface) = std::str::from_utf8(&payload[8..8 + interface_len as usize]) else {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        };

        WaylandProtocolParsingOutcome::Ok(WlRegistryGlobalEvent {
            name,
            interface,
            version,
        })
    }
}
