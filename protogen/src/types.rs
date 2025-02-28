use quote::quote;
use syn::Ident;

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
