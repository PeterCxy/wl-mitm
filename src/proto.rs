//! Protocol definitions necessary for this MITM proxy

// ---------- wl_display ---------

use byteorder::ByteOrder;
use protogen::wayland_proto_gen;

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjectTypeId, WlObjects},
};

macro_rules! bubble_malformed {
    ($e:expr) => {{
        let e = $e;
        if let crate::proto::WaylandProtocolParsingOutcome::MalformedMessage = e {
            return WaylandProtocolParsingOutcome::MalformedMessage;
        } else {
            e
        }
    }};
}

macro_rules! match_decoded {
    (match $decoded:ident {$($t:ty => $act:block$(,)?)+}) => {
        if let crate::proto::WaylandProtocolParsingOutcome::MalformedMessage = $decoded {
            return false;
        }

        if let crate::proto::WaylandProtocolParsingOutcome::Ok($decoded) = $decoded {
            $(
                if let Some($decoded) = $decoded.downcast_ref::<$t>() {
                    $act
                }
            )+
        }
    };
}

#[derive(PartialEq, Eq)]
pub enum WlMsgType {
    Request,
    Event,
}

pub enum WaylandProtocolParsingOutcome<T> {
    Ok(T),
    MalformedMessage,
    IncorrectObject,
    IncorrectOpcode,
    Unknown,
}

pub trait WlParsedMessage<'a> {
    fn opcode() -> u16
    where
        Self: Sized;
    fn object_type() -> WlObjectType
    where
        Self: Sized;
    fn msg_type() -> WlMsgType
    where
        Self: Sized;
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

    // dyn-available methods
    fn self_opcode(&self) -> u16;
    fn self_object_type(&self) -> WlObjectType;
    fn self_msg_type(&self) -> WlMsgType;
}

/// A version of [WlParsedMessage] that supports downcasting. By implementing this
/// trait, you promise that the (object_type, msg_type, opcode) triple is unique, i.e.
/// it does not overlap with any other implementation of this trait.
pub unsafe trait AnyWlParsedMessage<'a>: WlParsedMessage<'a> {}

impl<'a> dyn AnyWlParsedMessage<'a> + 'a {
    pub fn downcast_ref<T: AnyWlParsedMessage<'a>>(&self) -> Option<&T> {
        if self.self_opcode() != T::opcode() {
            return None;
        }

        if self.self_object_type() != T::object_type() {
            return None;
        }

        if self.self_msg_type() != T::msg_type() {
            return None;
        }

        Some(unsafe { &*(self as *const dyn AnyWlParsedMessage as *const T) })
    }
}

// TODO: generate these
pub fn decode_event<'obj, 'msg>(
    objects: &'obj WlObjects,
    msg: &'msg WlRawMsg,
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>> {
    if let WaylandProtocolParsingOutcome::Ok(e) = bubble_malformed!(
        <WlRegistryGlobalEvent as WlParsedMessage>::try_from_msg(objects, msg)
    ) {
        return WaylandProtocolParsingOutcome::Ok(Box::new(e));
    }

    WaylandProtocolParsingOutcome::Unknown
}

pub fn decode_request<'obj, 'msg>(
    objects: &'obj WlObjects,
    msg: &'msg WlRawMsg,
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>> {
    if let WaylandProtocolParsingOutcome::Ok(e) = bubble_malformed!(
        <WlDisplayGetRegistryRequest as WlParsedMessage>::try_from_msg(objects, msg)
    ) {
        return WaylandProtocolParsingOutcome::Ok(Box::new(e));
    }

    if let WaylandProtocolParsingOutcome::Ok(e) = bubble_malformed!(
        <WlRegistryBindRequest as WlParsedMessage>::try_from_msg(objects, msg)
    ) {
        return WaylandProtocolParsingOutcome::Ok(Box::new(e));
    }

    WaylandProtocolParsingOutcome::Unknown
}

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;

wayland_proto_gen!("proto/wayland.xml");
