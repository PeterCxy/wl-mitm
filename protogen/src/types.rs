use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Ident, LitStr};

pub(crate) struct WlInterface {
    pub name_snake: String,
    pub msgs: Vec<WlMsg>,
}

impl WlInterface {
    /// Name of the interface type's const representation, e.g. WL_WAYLAND
    /// This can be used as a discriminant for interface types in Rust
    pub fn type_const_name(&self) -> String {
        self.name_snake.to_uppercase()
    }

    pub fn generate(&self) -> proc_macro2::TokenStream {
        // Generate struct and parser impls for all messages belonging to this interface
        let msg_impl = self.msgs.iter().map(|msg| msg.generate_struct_and_impl());

        // Also generate a struct representing the type of this interface
        // This is used to keep track of all objects in [objects]
        // Example:
        //    struct WlDisplayTypeId;
        //    pub const WL_DISPLAY: WlObjectType = WlObjectType::new(&WlDisplayTypeId);
        //    impl WlObjectTypeId for WlDisplayTypeId { ... }
        let interface_type_id_name =
            format_ident!("{}TypeId", crate::to_camel_case(&self.name_snake));
        let interface_name_literal = LitStr::new(&self.name_snake, Span::call_site());
        let type_const_name = format_ident!("{}", self.type_const_name());

        quote! {
            struct #interface_type_id_name;

            pub const #type_const_name: crate::objects::WlObjectType = crate::objects::WlObjectType::new(&#interface_type_id_name);

            impl crate::objects::WlObjectTypeId for #interface_type_id_name {
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
    pub interface_name_snake: String,
    pub name_snake: String,
    pub msg_type: WlMsgType,
    pub opcode: u16,
    pub is_destructor: bool,
    pub args: Vec<(String, WlArgType)>,
}

impl WlMsg {
    /// Get the name of the structure generated for this message
    /// e.g. WlRegistryBindRequest
    pub fn struct_name(&self) -> String {
        format!(
            "{}{}{}",
            crate::to_camel_case(&self.interface_name_snake),
            crate::to_camel_case(&self.name_snake),
            self.msg_type.as_str()
        )
    }

    pub fn parser_fn_name(&self) -> String {
        format!("{}ParserFn", self.struct_name())
    }

    /// Generates a struct corresponding to the message type and a impl for [WlParsedMessage]
    /// that includes a parser
    pub fn generate_struct_and_impl(&self) -> proc_macro2::TokenStream {
        let opcode = self.opcode;
        let interface_name_snake_upper =
            format_ident!("{}", self.interface_name_snake.to_uppercase());
        let msg_name_snake = &self.name_snake;

        let struct_name = format_ident!("{}", self.struct_name());

        let parser_fn_name = format_ident!("{}", self.parser_fn_name());

        // Build all field names and their corresponding Rust type identifiers
        let (field_names, (field_types, field_attrs)): (Vec<_>, (Vec<_>, Vec<_>)) = self
            .args
            .iter()
            .map(|(name, tt)| {
                (
                    format_ident!("{name}"),
                    (
                        tt.to_rust_type(),
                        match tt {
                            // Can't serialize fds!
                            WlArgType::Fd => quote! { #[serde(skip)] },
                            _ => quote! {},
                        },
                    ),
                )
            })
            .unzip();

        let num_consumed_fds = self
            .args
            .iter()
            .filter(|(_, tt)| matches!(tt, WlArgType::Fd))
            .count();

        // Generate code to include in the parser / builder for every field
        let (parser_code, builder_code): (Vec<_>, Vec<_>) = self
            .args
            .iter()
            .map(|(arg_name, arg_type)| {
                let arg_name_ident = format_ident!("{arg_name}");
                (
                    arg_type.generate_parser_code(&arg_name_ident),
                    arg_type.generate_builder_code(&arg_name_ident),
                )
            })
            .unzip();

        // Collect new objects created in this msg with a known object type (interface)
        let (new_id_name, new_id_type): (Vec<_>, Vec<_>) = self
            .args
            .iter()
            .filter_map(|it| match it.1 {
                WlArgType::NewId(Some(ref interface)) => Some((
                    format_ident!("{}", it.0),
                    format_ident!("{}", interface.to_uppercase()),
                )),
                _ => None,
            })
            .unzip();

        let known_objects_created = if new_id_name.len() > 0 {
            quote! {
                Some(vec![
                    #( (self.#new_id_name, crate::proto::#new_id_type), )*
                ])
            }
        } else {
            quote! {
                None
            }
        };

        let is_destructor = self.is_destructor;

        quote! {
            #[allow(unused, non_snake_case)]
            #[derive(Serialize)]
            pub struct #struct_name<'a> {
                #[serde(skip)]
                _phantom: std::marker::PhantomData<&'a ()>,
                obj_id: u32,
                #( #field_attrs pub #field_names: #field_types, )*
            }

            impl<'a> #struct_name<'a> {
                #[allow(unused, non_snake_case)]
                pub fn new(
                    obj_id: u32,
                    #( #field_names: #field_types, )*
                ) -> Self {
                    Self {
                        _phantom: std::marker::PhantomData,
                        obj_id,
                        #( #field_names, )*
                    }
                }
            }

            impl<'a> crate::proto::__private::WlParsedMessagePrivate for #struct_name<'a> {}

            impl<'a> crate::proto::WlParsedMessage<'a> for #struct_name<'a> {
                fn opcode() -> u16 {
                    #opcode
                }

                fn self_opcode(&self) -> u16 {
                    #opcode
                }

                fn object_type() -> crate::objects::WlObjectType {
                    crate::proto::#interface_name_snake_upper
                }

                fn self_object_type(&self) -> crate::objects::WlObjectType {
                    crate::proto::#interface_name_snake_upper
                }

                fn self_msg_name(&self) -> &'static str {
                    #msg_name_snake
                }

                fn static_type_id() -> std::any::TypeId {
                    std::any::TypeId::of::<#struct_name<'static>>()
                }

                fn self_static_type_id(&self) -> std::any::TypeId {
                    std::any::TypeId::of::<#struct_name<'static>>()
                }

                #[allow(unused, private_interfaces, non_snake_case)]
                fn try_from_msg_impl(
                    msg: &crate::codec::WlRawMsg, _token: crate::proto::__private::WlParsedMessagePrivateToken
                ) -> crate::proto::WaylandProtocolParsingOutcome<#struct_name> {
                    let payload = msg.payload();
                    let mut pos = 0usize;
                    let mut pos_fds = 0usize;
                    #( #parser_code )*
                    crate::proto::WaylandProtocolParsingOutcome::Ok(#struct_name {
                        _phantom: std::marker::PhantomData,
                        obj_id: msg.obj_id,
                        #( #field_names, )*
                    })
                }

                fn obj_id(&self) -> u32 {
                    self.obj_id
                }

                fn is_destructor(&self) -> bool {
                    #is_destructor
                }

                fn known_objects_created(&self) -> Option<Vec<(u32, crate::objects::WlObjectType)>> {
                    #known_objects_created
                }

                fn to_json(&self) -> String {
                    serde_json::to_string(self).unwrap()
                }

                fn num_consumed_fds(&self) -> usize {
                    #num_consumed_fds
                }
            }

            unsafe impl<'a> crate::proto::AnyWlParsedMessage<'a> for #struct_name<'a> {}

            pub struct #parser_fn_name;

            impl crate::proto::WlMsgParserFn for #parser_fn_name {
                fn try_from_msg<'obj, 'msg>(
                    &self,
                    objects: &'obj crate::proto::WlObjects,
                    msg: &'msg crate::codec::WlRawMsg,
                ) -> crate::proto::WaylandProtocolParsingOutcome<Box<dyn crate::proto::AnyWlParsedMessage<'msg> + 'msg>> {
                    #struct_name::try_from_msg(objects, msg).map(|r| Box::new(r) as Box<_>)
                }
            }

            impl<'a> crate::proto::WlConstructableMessage<'a> for #struct_name<'a> {
                #[allow(unused, non_snake_case)]
                fn build_inner(&self, buf: &mut bytes::BytesMut, fds: &mut Vec<std::os::fd::OwnedFd>) {
                    use bytes::BufMut;
                    use std::os::fd::BorrowedFd;
                    #( #builder_code )*
                }
            }
        }
    }
}

pub(crate) enum WlArgType {
    Int,
    Uint,
    Fixed,
    Object,
    NewId(Option<String>),
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
            "new_id" => WlArgType::NewId(None),
            "string" => WlArgType::String,
            "array" => WlArgType::Array,
            "fd" => WlArgType::Fd,
            "enum" => WlArgType::Enum,
            _ => panic!("Unknown arg type!"),
        }
    }

    /// Attach a known, fixed interface name to `self`, if `self`
    /// is a [WlArgType::NewId].
    ///
    /// If a [WlArgType::NewId] does not come with a known interface
    /// tag, the caller is responsible for generating the additional
    /// args (interface, version) as required by Wayland's special
    /// serailization format for them
    ///
    /// We don't verify whether the interface is actually known here.
    /// Rather, if it isn't known, our emitted code will refer to
    /// an unknown type / const, which will cause a compile-time error.
    pub fn set_interface_name(&mut self, interface: String) {
        match self {
            WlArgType::NewId(_) => *self = WlArgType::NewId(Some(interface)),
            _ => panic!("not a new_id but got interface tag!"),
        }
    }

    /// What's the Rust type corresponding to this WL protocol type?
    /// Returned as a token that can be used directly in quote! {}
    pub fn to_rust_type(&self) -> proc_macro2::TokenStream {
        match self {
            WlArgType::Int => quote! { i32 },
            WlArgType::Uint | WlArgType::Object | WlArgType::NewId(_) | WlArgType::Enum => {
                quote! { u32 }
            }
            WlArgType::Fixed => quote! { fixed::types::I24F8 }, // wl fixed point is 24.8 signed
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
    pub fn generate_parser_code(&self, var_name: &Ident) -> proc_macro2::TokenStream {
        match self {
            WlArgType::Int => quote! {
                if payload.len() < pos + 4 {
                    return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: i32 = byteorder::NativeEndian::read_i32(&payload[pos..pos + 4]);

                pos += 4;
            },
            WlArgType::Uint | WlArgType::Object | WlArgType::NewId(_) | WlArgType::Enum => quote! {
                if payload.len() < pos + 4 {
                    return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: u32 = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]);

                pos += 4;
            },
            WlArgType::Fixed => quote! {
                if payload.len() < pos + 4 {
                    return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name = fixed::types::I24F8::from_bits(byteorder::NativeEndian::read_i32(&payload[pos..pos + 4]));

                pos += 4;
            },
            WlArgType::String => quote! {
                let #var_name: &str = {
                    if payload.len() < pos + 4 {
                        return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let len = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]) as usize;

                    pos += 4;

                    if len == 0 {
                        ""
                    } else {
                        if payload.len() < pos + len {
                            return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                        }

                        let Ok(#var_name) = std::str::from_utf8(&payload[pos..pos + len - 1]) else {
                            return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                        };

                        if len % 4 == 0 {
                            pos += len;
                        } else {
                            pos += len + (4 - len % 4);
                        }

                        #var_name
                    }
                };
            },
            WlArgType::Array => quote! {
                let #var_name: &[u8] = {
                    if payload.len() < pos + 4 {
                        return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                    }

                    let len = byteorder::NativeEndian::read_u32(&payload[pos..pos + 4]) as usize;

                    if len == 0 {
                        &[]
                    } else {
                        pos += 4;

                        if payload.len() < pos + len {
                            return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                        }

                        let #var_name = &payload[pos..pos + len];

                        if len % 4 == 0 {
                            pos += len;
                        } else {
                            pos += len + (4 - len % 4);
                        }

                        #var_name
                    }
                };
            },
            WlArgType::Fd => quote! {
                if msg.fds.len() < pos_fds + 1 {
                    return crate::proto::WaylandProtocolParsingOutcome::MalformedMessage;
                }

                let #var_name: std::os::fd::BorrowedFd<'_> = std::os::fd::AsFd::as_fd(&msg.fds[pos_fds]);
                pos_fds += 1;
            },
        }
    }

    pub fn generate_builder_code(&self, var_name: &Ident) -> proc_macro2::TokenStream {
        match self {
            WlArgType::Int => quote! {
                buf.put_i32_ne(self.#var_name);
            },
            WlArgType::Uint | WlArgType::Object | WlArgType::NewId(_) | WlArgType::Enum => quote! {
                buf.put_u32_ne(self.#var_name);
            },
            WlArgType::Fixed => quote! {
                buf.extend_from_slice(&self.#var_name.to_ne_bytes());
            },
            WlArgType::String => quote! {
                let bytes = self.#var_name.as_bytes();
                let len = bytes.len() + 1;
                buf.put_u32_ne(len as u32);
                buf.extend_from_slice(bytes);
                buf.put_u8(0);

                if len % 4 != 0 {
                    buf.put_bytes(0, (4 - len % 4));
                }
            },
            WlArgType::Array => quote! {
                buf.put_u32_ne(self.#var_name.len() as u32);
                buf.extend_from_slice(self.#var_name);

                if self.#var_name.len() % 4 != 0 {
                    buf.put_bytes(0, (4 - self.#var_name.len() % 4));
                }
            },
            WlArgType::Fd => quote! {
                fds.push(self.#var_name.try_clone_to_owned().unwrap());
            },
        }
    }
}
