//! Protocol definitions necessary for this MITM proxy

use std::{collections::HashMap, sync::LazyLock};

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjects},
};

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
    pub(super) trait WlParsedMessagePrivate: Send {}
    pub(super) struct WlParsedMessagePrivateToken;
}

#[allow(private_bounds, private_interfaces)]
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

        Self::try_from_msg_impl(msg, __private::WlParsedMessagePrivateToken)
    }

    fn try_from_msg_impl(
        msg: &'a WlRawMsg,
        _token: __private::WlParsedMessagePrivateToken,
    ) -> WaylandProtocolParsingOutcome<Self>
    where
        Self: Sized + 'a;

    // dyn-available methods
    fn self_opcode(&self) -> u16;
    fn self_object_type(&self) -> WlObjectType;
    fn self_msg_type(&self) -> WlMsgType;
    fn self_msg_name(&self) -> &'static str;

    /// The object ID which this message acts upon
    fn obj_id(&self) -> u32;

    /// Is this request / event a destructor? That is, does it destroy [Self::obj_id()]?
    fn is_destructor(&self) -> bool;

    /// List of (object id, object type) pairs created by this message
    /// Note that this only includes objects created with a fixed, known interface
    /// type. Wayland requests with `new_id` but without a fixed interface are
    /// serialized differently, and are not included here. However, the only
    /// widely-used message with that capability is [WlRegistryBindRequest],
    /// which is already handled separately on its own.
    fn known_objects_created(&self) -> Option<Vec<(u32, WlObjectType)>>;

    /// Serialize this message into a JSON string, for use with ask scripts
    fn to_json(&self) -> String;

    /// How many fds have been consumed in parsing this message?
    /// This is used to return any unused fds to the decoder.
    fn num_consumed_fds(&self) -> usize;
}

/// A version of [WlParsedMessage] that supports downcasting. By implementing this
/// trait, you promise that the (object_type, msg_type, opcode) triple is unique, i.e.
/// it does not overlap with any other implementation of this trait.
///
/// In addition, any implementor also asserts that the type implementing this trait
/// does not contain any lifetime other than 'a, and that the implenetor struct is
/// _covariant_ with respect to lifetime 'a.
///
/// This is required for the soundness of the downcast_ref implementation.
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
static WL_KNOWN_OBJECT_TYPES: LazyLock<HashMap<&'static str, WlObjectType>> = LazyLock::new(|| {
    let mut ret = HashMap::new();
    wl_init_known_types(&mut ret);
    ret
});
/// Parsers for all known events / requests
static WL_EVENT_REQUEST_PARSERS: LazyLock<(
    HashMap<(WlObjectType, u16), &'static dyn WlMsgParserFn>,
    HashMap<(WlObjectType, u16), &'static dyn WlMsgParserFn>,
)> = LazyLock::new(|| {
    let mut event_parsers = Default::default();
    let mut request_parsers = Default::default();
    wl_init_parsers(&mut event_parsers, &mut request_parsers);
    (event_parsers, request_parsers)
});

/// Decode a Wayland event from a [WlRawMsg], returning the type-erased result, or
/// [WaylandProtocolParsingOutcome::Unknown] for unknown messages, [WaylandProtocolParsingOutcome::MalformedMessage]
/// for  malformed messages.
///
/// To downcast the parse result to a concrete message type, use [<dyn AnyWlParsedMessage>::downcast_ref]
pub fn decode_event<'obj, 'msg>(
    objects: &'obj WlObjects,
    msg: &'msg WlRawMsg,
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage<'msg> + 'msg>> {
    let Some(obj_type) = objects.lookup_object(msg.obj_id) else {
        return WaylandProtocolParsingOutcome::Unknown;
    };

    let Some(msg_parser_fn) = WL_EVENT_REQUEST_PARSERS.0.get(&(obj_type, msg.opcode)) else {
        return WaylandProtocolParsingOutcome::Unknown;
    };

    msg_parser_fn.try_from_msg(objects, msg)
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
    let Some(obj_type) = objects.lookup_object(msg.obj_id) else {
        return WaylandProtocolParsingOutcome::Unknown;
    };

    let Some(msg_parser_fn) = WL_EVENT_REQUEST_PARSERS.1.get(&(obj_type, msg.opcode)) else {
        return WaylandProtocolParsingOutcome::Unknown;
    };

    msg_parser_fn.try_from_msg(objects, msg)
}

/// Look up a known object type from its name to its Rust [WlObjectType] representation
pub fn lookup_known_object_type(name: &str) -> Option<WlObjectType> {
    WL_KNOWN_OBJECT_TYPES.get(name).copied()
}

/// The default object ID of wl_display
pub const WL_DISPLAY_OBJECT_ID: u32 = 1;

// Include code generated by protogen
// Note: to generate this, run generate.sh at the project root.
#[rustfmt::skip]
#[path = "../generated/proto_generated.rs"]
mod proto_generated;
pub use proto_generated::*;
