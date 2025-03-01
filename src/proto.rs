//! Protocol definitions necessary for this MITM proxy

use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, DefaultHasher},
    sync::RwLock,
};

use byteorder::ByteOrder;

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

impl<T> WaylandProtocolParsingOutcome<T> {
    pub fn map<U>(self, f: impl Fn(T) -> U) -> WaylandProtocolParsingOutcome<U> {
        match self {
            WaylandProtocolParsingOutcome::Ok(t) => WaylandProtocolParsingOutcome::Ok(f(t)),
            WaylandProtocolParsingOutcome::MalformedMessage => {
                WaylandProtocolParsingOutcome::MalformedMessage
            }
            WaylandProtocolParsingOutcome::IncorrectObject => {
                WaylandProtocolParsingOutcome::IncorrectObject
            }
            WaylandProtocolParsingOutcome::IncorrectOpcode => {
                WaylandProtocolParsingOutcome::IncorrectOpcode
            }
            WaylandProtocolParsingOutcome::Unknown => WaylandProtocolParsingOutcome::Unknown,
        }
    }
}

/// Internal module used to seal the [WlParsedMessage] trait
mod __private {
    pub(super) trait WlParsedMessagePrivate {}
}

#[allow(private_bounds)]
pub trait WlParsedMessage<'a>: __private::WlParsedMessagePrivate {
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
        Self: Sized + 'a,
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
        Self: Sized + 'a;

    // dyn-available methods
    fn self_opcode(&self) -> u16;
    fn self_object_type(&self) -> WlObjectType;
    fn self_msg_type(&self) -> WlMsgType;
}

/// A version of [WlParsedMessage] that supports downcasting. By implementing this
/// trait, you promise that the (object_type, msg_type, opcode) triple is unique, i.e.
/// it does not overlap with any other implementation of this trait.
///
/// In addition, any implementor also asserts that the type implementing this trait
/// does not contain any lifetime other than 'a. This is required for the soundness of
/// the downcast_ref implementation.
pub unsafe trait AnyWlParsedMessage<'a>: WlParsedMessage<'a> {}

impl<'out, 'data: 'out> dyn AnyWlParsedMessage<'data> + 'data {
    /// Downcast the type-erased, borrowed Wayland message to a concrete type. Note that the
    /// safety of this relies on a few invariants:
    ///
    /// 1. The (object_type, msg_type, opcode) triple is unique (guaranteed by unsafe trait)
    /// 2. 'data outlives 'out (guaranteed by the trait bound above)
    /// 3. The type implementing [AnyWlParsedMessage] does not contain any lifetime other than
    ///    'data (or 'a in the trait's definition).
    /// 4. No type other than those contained in this mod can implement [AnyWlParsedMessage]
    ///    (enforced by the private trait bound [__private::WlParsedMessagePrivate])
    pub fn downcast_ref<T: AnyWlParsedMessage<'data> + 'data>(&'out self) -> Option<&'out T> {
        if self.self_opcode() != T::opcode() {
            return None;
        }

        if self.self_object_type() != T::object_type() {
            return None;
        }

        if self.self_msg_type() != T::msg_type() {
            return None;
        }

        // SAFETY: We have verified the opcode, type, and msg type all match up
        // As per safety guarantee of [AnyWlParsedMessage], we've now narrowed
        // [self] down to one concrete type.
        // In addition, because [AnyWlParsedMessage]'s contract requires that no
        // lifetime other than 'a ('data) is contained in the implemetor, the
        // output type T cannot contain another lifetime that may be transmuted
        // by this unsafe block.
        Some(unsafe { &*(self as *const dyn AnyWlParsedMessage as *const T) })
    }
}

/// A dyn-compatible wrapper over a specific [WlParsedMessage] type's static methods.
/// The only exposed method, [try_from_msg], attempts to parse the message
/// to the given type. This is used as members of [WL_EVENT_PARSERS]
/// and [WL_REQUEST_PARSERS] to facilitate automatic parsing of all
/// known message types.
pub trait WlMsgParserFn: Send + Sync {
    fn try_from_msg<'obj, 'msg>(
        &self,
        objects: &'obj WlObjects,
        msg: &'msg WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>>;
}

/// A map from known interface names to their object types in Rust representation
static WL_KNOWN_OBJECT_TYPES: RwLock<
    HashMap<&'static str, WlObjectType, BuildHasherDefault<DefaultHasher>>,
> = RwLock::new(HashMap::with_hasher(BuildHasherDefault::new()));
/// Parsers for all known events
static WL_EVENT_PARSERS: RwLock<Vec<&'static dyn WlMsgParserFn>> = RwLock::new(Vec::new());
/// Parsers for all known requests
static WL_REQUEST_PARSERS: RwLock<Vec<&'static dyn WlMsgParserFn>> = RwLock::new(Vec::new());

/// Decode a Wayland event from a [WlRawMsg], returning the type-erased result, or
/// [WaylandProtocolParsingOutcome::Unknown] for unknown messages, [WaylandProtocolParsingOutcome::MalformedMessage]
/// for  malformed messages.
///
/// To downcast the parse result to a concrete message type, use [<dyn AnyWlParsedMessage>::downcast_ref]
pub fn decode_event<'obj, 'msg>(
    objects: &'obj WlObjects,
    msg: &'msg WlRawMsg,
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>> {
    for p in WL_EVENT_PARSERS.read().unwrap().iter() {
        if let WaylandProtocolParsingOutcome::Ok(e) =
            bubble_malformed!(p.try_from_msg(objects, msg))
        {
            return WaylandProtocolParsingOutcome::Ok(e);
        }
    }

    WaylandProtocolParsingOutcome::Unknown
}

/// Decode a Wayland request from a [WlRawMsg], returning the type-erased result, or
/// [WaylandProtocolParsingOutcome::Unknown] for unknown messages, [WaylandProtocolParsingOutcome::MalformedMessage]
/// for  malformed messages.
///
/// To downcast the parse result to a concrete message type, use [<dyn AnyWlParsedMessage>::downcast_ref]
pub fn decode_request<'obj, 'msg>(
    objects: &'obj WlObjects,
    msg: &'msg WlRawMsg,
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>> {
    for p in WL_REQUEST_PARSERS.read().unwrap().iter() {
        if let WaylandProtocolParsingOutcome::Ok(e) =
            bubble_malformed!(p.try_from_msg(objects, msg))
        {
            return WaylandProtocolParsingOutcome::Ok(e);
        }
    }

    WaylandProtocolParsingOutcome::Unknown
}

/// Look up a known object type from its name to its Rust [WlObjectType] representation
pub fn lookup_known_object_type(name: &str) -> Option<WlObjectType> {
    WL_KNOWN_OBJECT_TYPES
        .read()
        .ok()
        .and_then(|t| t.get(name).copied())
}

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;

// Include code generated by build.rs
include!(concat!(env!("OUT_DIR"), "/proto_generated.rs"));
