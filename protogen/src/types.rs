use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Ident, LitStr};

pub(crate) struct WlInterface {
    pub name_snake: String,
    pub msgs: Vec<WlMsg>,
}

impl WlInterface {
    pub fn generate(&self) -> proc_macro2::TokenStream {
        // Generate struct and parser impls for all messages belonging to this interface
        let msg_impl = self
            .msgs
            .iter()
            .map(|msg| msg.generate_struct_and_impl(&self.name_snake));

        // Also generate a struct representing the type of this interface
        // This is used to keep track of all objects in [objects]
        // Example:
        //    struct WlDisplayTypeId;
        //    pub const WL_DISPLAY: WlObjectType = WlObjectType::new(&WlDisplayTypeId);
        //    impl WlObjectTypeId for WlDisplayTypeId { ... }
        let interface_type_id_name =
            format_ident!("{}TypeId", crate::to_camel_case(&self.name_snake));
        let interface_name_literal = LitStr::new(&self.name_snake, Span::call_site());
        let interface_name_snake_upper =
            Ident::new(&self.name_snake.to_uppercase(), Span::call_site());

        quote! {
            struct #interface_type_id_name;

            pub const #interface_name_snake_upper: WlObjectType = WlObjectType::new(&#interface_type_id_name);

            impl WlObjectTypeId for #interface_type_id_name {
                fn interface(&self) -> &'static str {
                    #interface_name_literal
                }
            }

            #( #msg_impl )*
        }
    }
}

pub(crate) enum WlMsgType {
    Request,
    Event,
}

impl WlMsgType {
    fn as_str(&self) -> &'static str {
        match self {
            WlMsgType::Request => "Request",
            WlMsgType::Event => "Event",
        }
    }
}

pub(crate) struct WlMsg {
    pub name_snake: String,
    pub msg_type: WlMsgType,
    pub opcode: u16,
    pub args: Vec<(String, WlArgType)>,
}

impl WlMsg {
    /// Generates a struct corresponding to the message type and a impl for [WlParsedMessage]
    /// that includes a parser
    pub fn generate_struct_and_impl(&self, interface_name_snake: &str) -> proc_macro2::TokenStream {
        let opcode = self.opcode;
        let interface_name_snake_upper = format_ident!("{}", interface_name_snake.to_uppercase());
        let msg_type = format_ident!("{}", self.msg_type.as_str());

        // e.g. WlRegistryBindRequest
        let struct_name = format_ident!(
            "{}{}{}",
            crate::to_camel_case(interface_name_snake),
            crate::to_camel_case(&self.name_snake),
            self.msg_type.as_str()
        );

        // Build all field names and their corresponding Rust type identifiers
        let (field_names, field_types): (Vec<_>, Vec<_>) = self
            .args
            .iter()
            .map(|(name, tt)| (format_ident!("{name}"), tt.to_rust_type()))
            .unzip();

        // Generate code to include in the parser for every field
        let parser_code: Vec<_> = self
            .args
            .iter()
            .map(|(arg_name, arg_type)| {
                let arg_name_ident = format_ident!("{arg_name}");
                arg_type.generate_parser_code(arg_name_ident)
            })
            .collect();

        quote! {
            pub struct #struct_name<'a> {
                _phantom: std::marker::PhantomData<&'a ()>,
                #( pub #field_names: #field_types, )*
            }

            impl<'a> __private::WlParsedMessagePrivate for #struct_name<'a> {}

            impl<'a> WlParsedMessage<'a> for #struct_name<'a> {
                fn opcode() -> u16 {
                    #opcode
                }

                fn self_opcode(&self) -> u16 {
                    #opcode
                }

                fn object_type() -> WlObjectType {
                    #interface_name_snake_upper
                }

                fn self_object_type(&self) -> WlObjectType {
                    #interface_name_snake_upper
                }

                fn msg_type() -> WlMsgType {
                    WlMsgType::#msg_type
                }

                fn self_msg_type(&self) -> WlMsgType {
                    WlMsgType::#msg_type
                }

                fn try_from_msg_impl(msg: &crate::codec::WlRawMsg) -> WaylandProtocolParsingOutcome<#struct_name> {
                    let payload = msg.payload();
                    let mut pos = 0usize;
                    #( #parser_code )*
                    WaylandProtocolParsingOutcome::Ok(#struct_name {
                        _phantom: std::marker::PhantomData,
                        #( #field_names, )*
                    })
                }
            }

            unsafe impl<'a> AnyWlParsedMessage<'a> for #struct_name<'a> {}
        }
    }
}

pub(crate) enum WlArgType {
    Int,
    Uint,
    Fixed,
    Object,
    NewId,
    String,
    Array,
    Fd,
    Enum,
}

impl WlArgType {
    pub fn parse(s: &str) -> WlArgType {
        match s {
            "int" => WlArgType::Int,
            "uint" => WlArgType::Uint,
            "fixed" => WlArgType::Fixed,
            "object" => WlArgType::Object,
            "new_id" => WlArgType::NewId,
            "string" => WlArgType::String,
            "array" => WlArgType::Array,
            "fd" => WlArgType::Fd,
            "enum" => WlArgType::Enum,
            _ => panic!("Unknown arg type!"),
        }
    }

    /// What's the Rust type corresponding to this WL protocol type?
    /// Returned as a token that can be used directly in quote! {}
    pub fn to_rust_type(&self) -> proc_macro2::TokenStream {
        match self {
            WlArgType::Int => quote! { i32 },
            // TODO: "fixed" is decoded directly as a u32. fix it
            WlArgType::Uint
            | WlArgType::Fixed
            | WlArgType::Object
            | WlArgType::NewId
            | WlArgType::Enum => quote! { u32 },
            WlArgType::String => quote! { &'a str },
            WlArgType::Array => quote! { &'a [u8] },
            WlArgType::Fd => quote! { std::os::fd::BorrowedFd<'a> },
        }
    }

    /// Generate code to be inserted into the parsing function. The parsing function is expected
    /// to set up two variables (with `msg` as the input [WlRawMsg]):
    ///
    ///   let payload: &[u8] = msg.payload();
    ///   let mut pos: usize = 0;
    ///
    /// `pos` records where we last read in the payload.
    ///
    /// Code generated here will set up a variable with `var_name` containing the parsed result
    /// of the current argument. This `var_name` can then be used later to construct the event or
    /// request's struct.
    pub fn generate_parser_code(&self, var_name: Ident) -> proc_macro2::TokenStream {
        match self {
            WlArgType::Int => quote! {
                if payload.len() < pos + 4 {
                    return WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: i32 = byteorder::NativeEndian::read_i32(&payload[pos..pos + 4]);

                pos += 4;
            },
            WlArgType::Uint
            | WlArgType::Fixed
            | WlArgType::Object
            | WlArgType::NewId
            | WlArgType::Enum => quote! {
                if payload.len() < pos + 4 {
                    return WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: u32 = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]);

                pos += 4;
            },
            WlArgType::String => quote! {
                let #var_name: &str = {
                    if payload.len() < pos + 4 {
                        return WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let len = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]) as usize;

                    pos += 4;

                    if payload.len() < pos + len {
                        return WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let Ok(#var_name) = std::str::from_utf8(&payload[pos..pos + len - 1]) else {
                        return WaylandProtocolParsingOutcome::MalformedMessage;
                    };

                    pos += len;

                    #var_name
                };
            },
            WlArgType::Array => quote! {
                let #var_name: &[u8] = {
                    if payload.len() < pos + 4 {
                        return WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let len = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]) as usize;

                    pos += 4;

                    if payload.len() < pos + len {
                        return WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let #var_name = &payload[pos..pos + len];
                    pos += len;

                    #var_name
                };
            },
            WlArgType::Fd => quote! {
                if msg.fds.len() == 0 {
                    return WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: std::os::fd::BorrowedFd<'_> = std::os::fd::AsFd::as_fd(&msg.fds[0]);
            },
        }
    }
}
