//! Protocol definitions necessary for this MITM proxy

use std::{any::TypeId, collections::HashMap, os::fd::OwnedFd, sync::LazyLock};

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjects},
};

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
    pub(super) trait WlParsedMessagePrivate: Sized {}
    pub(super) struct WlParsedMessagePrivateToken;
}

#[allow(private_bounds, private_interfaces)]
pub trait WlParsedMessage<'a>: __private::WlParsedMessagePrivate {
    fn opcode() -> u16;
    fn object_type() -> WlObjectType;
    fn msg_name() -> &'static str;
    /// Is this request / event a destructor? That is, does it destroy [Self::obj_id()]?
    fn is_destructor() -> bool;
    /// How many fds have been consumed in parsing this message?
    /// This is used to return any unused fds to the decoder.
    fn num_consumed_fds() -> usize;

    fn try_from_msg<'obj>(
        objects: &'obj WlObjects,
        msg: &'a WlRawMsg,
    ) -> WaylandProtocolParsingOutcome<Self>
    where
        Self: 'a,
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
        Self: 'a;

    /// The object ID which this message acts upon
    fn _obj_id(&self) -> u32;

    /// List of (object id, object type) pairs created by this message
    /// Note that this only includes objects created with a fixed, known interface
    /// type. Wayland requests with `new_id` but without a fixed interface are
    /// serialized differently, and are not included here. However, the only
    /// widely-used message with that capability is [WlRegistryBindRequest],
    /// which is already handled separately on its own.
    fn _known_objects_created(&self) -> Option<Vec<(u32, WlObjectType)>>;

    /// Serialize this message into a JSON string, for use with ask scripts
    fn _to_json(&self) -> String;
}

/// A version of [WlParsedMessage] that supports downcasting. By implementing this
/// trait, you assert that the [DowncastableWlParsedMessage::Static] MUST correspond
/// to a version of the implementor type with a static lifetime.
///
/// Any implementor also asserts that the type implementing this trait
/// does not contain any lifetime other than 'a, and that the implenetor struct is
/// _covariant_ with respect to lifetime 'a.
///
/// This is required for the soundness of the downcast_ref implementation.
pub unsafe trait DowncastableWlParsedMessage<'a>: Send + WlParsedMessage<'a> {
    type Static: 'static;
}

/// The implementation of dyn-available methods and downcasting for
/// [DowncastableWlParsedMessage]
pub trait AnyWlParsedMessage: Send {
    fn static_type_id(&self) -> TypeId;
    fn opcode(&self) -> u16;
    fn object_type(&self) -> WlObjectType;
    fn msg_name(&self) -> &'static str;
    fn is_destructor(&self) -> bool;
    fn obj_id(&self) -> u32;
    fn known_objects_created(&self) -> Option<Vec<(u32, WlObjectType)>>;
    fn to_json(&self) -> String;
    fn num_consumed_fds(&self) -> usize;
}

impl<'a, T> AnyWlParsedMessage for T
where
    T: DowncastableWlParsedMessage<'a> + 'a,
{
    fn static_type_id(&self) -> TypeId {
        TypeId::of::<T::Static>()
    }

    fn opcode(&self) -> u16 {
        T::opcode()
    }

    fn object_type(&self) -> WlObjectType {
        T::object_type()
    }

    fn msg_name(&self) -> &'static str {
        T::msg_name()
    }

    fn is_destructor(&self) -> bool {
        T::is_destructor()
    }

    fn obj_id(&self) -> u32 {
        T::_obj_id(self)
    }

    fn known_objects_created(&self) -> Option<Vec<(u32, WlObjectType)>> {
        T::_known_objects_created(self)
    }

    fn to_json(&self) -> String {
        T::_to_json(self)
    }

    fn num_consumed_fds(&self) -> usize {
        T::num_consumed_fds()
    }
}

impl<'out, 'data: 'out> dyn AnyWlParsedMessage + 'data {
    /// Downcast the type-erased, borrowed Wayland message to a concrete type. Note that the
    /// safety of this relies on a few invariants:
    ///
    /// 1. 'data outlives 'out (guaranteed by the trait bound above)
    /// 2. The type implementing [DowncastableWlParsedMessage] does not contain any lifetime other than
    ///    'data (or 'a in the trait's definition).
    pub fn downcast_ref<
        T: AnyWlParsedMessage + DowncastableWlParsedMessage<'data> + 'data,
    >(
        &'data self,
    ) -> Option<&'out T> {
        if self.static_type_id() != TypeId::of::<T::Static>() {
            return None;
        }

        // SAFETY: We have verified the type IDs match up.
        //
        // In addition, because [DowncastableWlParsedMessage]'s contract requires that no
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
    ) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage + 'msg>>;
}

/// Messages that can be converted back to [WlRawMsg]
pub trait WlConstructableMessage<'a>: Sized + WlParsedMessage<'a> {
    fn build(&self) -> WlRawMsg {
        WlRawMsg::build(self._obj_id(), Self::opcode(), |buf, fds| {
            self.build_inner(buf, fds)
        })
    }

    fn build_inner(&self, buf: &mut BytesMut, fds: &mut Vec<OwnedFd>);
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
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage + 'msg>> {
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
) -> WaylandProtocolParsingOutcome<Box<dyn AnyWlParsedMessage + 'msg>> {
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
use bytes::BytesMut;
pub use proto_generated::*;
