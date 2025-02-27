use proc_macro2::Span;
use quick_xml::events::Event;
use quote::{format_ident, quote};
use syn::{Ident, LitStr, parse_macro_input};

#[proc_macro]
pub fn wayland_proto_gen(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: LitStr = parse_macro_input!(item);
    let xml_str = std::fs::read_to_string(input.value()).expect("Unable to read from file");
    let mut reader = quick_xml::Reader::from_str(&xml_str);
    reader.config_mut().trim_text(true);

    let mut ret = proc_macro2::TokenStream::new();

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => break,
            Event::Start(e) => {
                let name =
                    str::from_utf8(e.local_name().into_inner()).expect("utf8 encoding error");

                match name {
                    "interface" => {
                        let str = handle_interface(&mut reader, e);
                        ret = quote! {
                            #ret
                            #str
                        }
                    }
                    _ => {}
                }
            }
            _ => continue,
        }
    }

    ret.into()
}

fn handle_interface(
    reader: &mut quick_xml::Reader<&[u8]>,
    start: quick_xml::events::BytesStart<'_>,
) -> proc_macro2::TokenStream {
    let name_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "name"
        })
        .expect("No name attr found for interface");

    let interface_name_snake = std::str::from_utf8(&name_attr.value).expect("utf8 encoding error");

    // Generate the implementation of the Wayland object type ID, consisting of a private struct
    // to act as a trait object, a public const that wraps the struct in `WlObjectType`, and a impl
    // of `WlObjectTypeId`.
    // Example:
    //    struct WlDisplayTypeId;
    //    pub const WL_DISPLAY: WlObjectType = WlObjectType::new(&WlDisplayTypeId);
    //    impl WlObjectTypeId for WlDisplayTypeId { ... }
    let interface_type_id_name = format_ident!("{}TypeId", to_camel_case(interface_name_snake));
    let interface_name_literal = LitStr::new(interface_name_snake, Span::call_site());
    let interface_name_snake_upper =
        Ident::new(&interface_name_snake.to_uppercase(), Span::call_site());
    let mut ret: proc_macro2::TokenStream = quote! {
        struct #interface_type_id_name;

        pub const #interface_name_snake_upper: WlObjectType = WlObjectType::new(&#interface_type_id_name);

        impl WlObjectTypeId for #interface_type_id_name {
            fn interface(&self) -> &'static str {
                #interface_name_literal
            }
        }
    };

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => panic!("Unexpected EOF"),
            Event::End(e) if e.local_name() == start.local_name() => break,
            _ => continue,
        }
    }

    ret
}

fn to_camel_case(s: &str) -> String {
    s.split("_")
        .map(|item| {
            item.char_indices()
                .map(|(idx, c)| {
                    if idx == 0 {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .collect::<String>()
        })
        .collect()
}
