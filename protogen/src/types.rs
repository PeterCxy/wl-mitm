use quote::quote;

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
}
