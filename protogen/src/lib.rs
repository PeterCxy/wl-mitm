use proc_macro2::Span;
use quick_xml::events::Event;
use quote::{format_ident, quote};
use syn::{Ident, LitStr, parse_macro_input};
use types::WlArgType;

mod types;

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
    let interface_name_camel = to_camel_case(interface_name_snake);

    // Generate the implementation of the Wayland object type ID, consisting of a private struct
    // to act as a trait object, a public const that wraps the struct in `WlObjectType`, and a impl
    // of `WlObjectTypeId`.
    // Example:
    //    struct WlDisplayTypeId;
    //    pub const WL_DISPLAY: WlObjectType = WlObjectType::new(&WlDisplayTypeId);
    //    impl WlObjectTypeId for WlDisplayTypeId { ... }
    let interface_type_id_name = format_ident!("{}TypeId", interface_name_camel);
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

    let mut event_opcode = 0;
    let mut request_opcode = 0;

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => panic!("Unexpected EOF"),
            Event::Start(e) => {
                let start_tag =
                    str::from_utf8(e.local_name().into_inner()).expect("Unable to parse start tag");
                let append = if start_tag == "event" {
                    event_opcode += 1;
                    handle_request_or_event(
                        reader,
                        &interface_name_camel,
                        &interface_name_snake_upper,
                        event_opcode - 1,
                        e,
                    )
                } else if start_tag == "request" {
                    request_opcode += 1;
                    handle_request_or_event(
                        reader,
                        &interface_name_camel,
                        &interface_name_snake_upper,
                        request_opcode - 1,
                        e,
                    )
                } else {
                    proc_macro2::TokenStream::new()
                };

                ret = quote! {
                    #ret
                    #append
                }
            }
            Event::End(e) if e.local_name() == start.local_name() => break,
            _ => continue,
        }
    }

    ret
}

fn handle_request_or_event(
    reader: &mut quick_xml::Reader<&[u8]>,
    interface_name_camel: &str,
    interface_name_snake_upper: &Ident,
    opcode: u16,
    start: quick_xml::events::BytesStart<'_>,
) -> proc_macro2::TokenStream {
    let start_tag =
        str::from_utf8(start.local_name().into_inner()).expect("Unable to parse start tag");
    let start_tag_camel = to_camel_case(start_tag);
    let name_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "name"
        })
        .expect("No name attr found for request/event");
    let name_camel = to_camel_case(str::from_utf8(&name_attr.value).expect("utf8 encoding error"));

    let mut args: Vec<(String, WlArgType)> = Vec::new();

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => panic!("Unexpected EOF"),
            Event::Empty(e)
                if str::from_utf8(e.local_name().into_inner()).expect("utf8 encoding error")
                    == "arg" =>
            {
                let mut name: Option<String> = None;
                let mut tt: Option<WlArgType> = None;

                for attr in e.attributes() {
                    let attr = attr.expect("attr parsing error");
                    let attr_name = str::from_utf8(attr.key.local_name().into_inner())
                        .expect("utf8 encoding error");
                    if attr_name == "name" {
                        name = Some(
                            str::from_utf8(&attr.value)
                                .expect("utf8 encoding error")
                                .to_string(),
                        );
                    } else if attr_name == "type" {
                        tt = Some(WlArgType::parse(
                            str::from_utf8(&attr.value).expect("utf8 encoding error"),
                        ));
                    }
                }

                args.push((
                    name.expect("args must have a name"),
                    tt.expect("args must have a type"),
                ));
            }
            Event::End(e) if e.local_name() == start.local_name() => break,
            _ => continue,
        }
    }

    let (field_names, field_types): (Vec<_>, Vec<_>) = args
        .iter()
        .map(|(name, tt)| (format_ident!("{name}"), tt.to_rust_type()))
        .unzip();

    let struct_name = format_ident!("{interface_name_camel}{name_camel}{start_tag_camel}");

    let struct_def = quote! {
        pub struct #struct_name<'a> {
            _phantom: std::marker::PhantomData<&'a ()>,
            #( pub #field_names: #field_types, )*
        }
    };

    let parser_code: Vec<_> = args
        .into_iter()
        .map(|(arg_name, arg_type)| {
            let arg_name_ident = format_ident!("{arg_name}");
            arg_type.generate_parser_code(arg_name_ident)
        })
        .collect();

    let struct_impl = quote! {
        impl<'a> WlParsedMessage<'a> for #struct_name<'a> {
            fn opcode() -> u16 {
                #opcode
            }

            fn object_type() -> WlObjectType {
                #interface_name_snake_upper
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
    };

    quote! {
        #struct_def
        #struct_impl
    }
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
