use std::path::Path;

use quick_xml::events::Event;
use quote::{format_ident, quote};
use syn::Ident;
use types::{WlArgType, WlInterface, WlMsg, WlMsgType};

mod types;

pub fn generate_from_dir(p: impl AsRef<Path>) -> String {
    let (gen_code, (add_parsers_fn, add_object_types_fn)): (Vec<_>, (Vec<_>, Vec<_>)) =
        std::fs::read_dir(p)
            .expect("cannot open directory")
            .filter_map(|f| f.ok())
            .filter(|f| {
                f.file_name()
                    .to_str()
                    .expect("utf8 encoding error")
                    .ends_with(".xml")
            })
            .map(|f| generate_from_xml_file(f.path()))
            .unzip();

    quote! {
        #( #gen_code )*

        fn wl_init_parsers(event_parsers: &mut Vec<&'static dyn WlMsgParserFn>, request_parsers: &mut Vec<&'static dyn WlMsgParserFn>) {
            #( #add_parsers_fn(event_parsers, request_parsers); )*
        }

        fn wl_init_known_types(object_types: &mut HashMap<&'static str, WlObjectType>) {
            #( #add_object_types_fn(object_types); )*
        }
    }
    .to_string()
}

fn generate_from_xml_file(p: impl AsRef<Path>) -> (proc_macro2::TokenStream, (Ident, Ident)) {
    let file_name = p.as_ref().file_stem().expect("No file name provided");
    let xml_str = std::fs::read_to_string(&p).expect("Unable to read from file");
    let mut reader = quick_xml::Reader::from_str(&xml_str);
    reader.config_mut().trim_text(true);

    let mut interfaces: Vec<WlInterface> = vec![];

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => break,
            Event::Start(e) => {
                let name =
                    str::from_utf8(e.local_name().into_inner()).expect("utf8 encoding error");

                match name {
                    "interface" => {
                        // An <interface> section
                        interfaces.push(handle_interface(&mut reader, e));
                    }
                    _ => {}
                }
            }
            _ => continue,
        }
    }

    let mut code: Vec<proc_macro2::TokenStream> = vec![];
    let mut event_parsers: Vec<Ident> = vec![];
    let mut request_parsers: Vec<Ident> = vec![];
    let (mut known_interface_names, mut known_interface_consts): (Vec<String>, Vec<Ident>) =
        (vec![], vec![]);

    for i in interfaces.iter() {
        known_interface_names.push(i.name_snake.clone());
        known_interface_consts.push(format_ident!("{}", i.type_const_name()));

        code.push(i.generate());

        for m in i.msgs.iter() {
            let parser_name = format_ident!("{}", m.parser_fn_name());

            match m.msg_type {
                WlMsgType::Event => {
                    event_parsers.push(parser_name);
                }
                WlMsgType::Request => {
                    request_parsers.push(parser_name);
                }
            }
        }
    }

    let file_name_snake = file_name.to_str().unwrap().replace("-", "_");

    // A function to add all event/request parsers to WL_EVENT_PARSERS and WL_REQUEST_PARSERS
    let add_parsers_fn = format_ident!("wl_init_parsers_{}", file_name_snake);

    // A function to add all known interfaces to the WL_KNOWN_OBJECT_TYPES map from name -> Rust type
    let add_object_types_fn = format_ident!("wl_init_known_types_{}", file_name_snake);

    let ret_code = quote! {
        #( #code )*

        fn #add_parsers_fn(event_parsers: &mut Vec<&'static dyn WlMsgParserFn>, request_parsers: &mut Vec<&'static dyn WlMsgParserFn>) {
            #( event_parsers.push(&#event_parsers); )*
            #( request_parsers.push(&#request_parsers); )*
        }

        fn #add_object_types_fn(object_types: &mut HashMap<&'static str, WlObjectType>) {
            #( object_types.insert(#known_interface_names, #known_interface_consts); )*
        }
    };

    (ret_code, (add_parsers_fn, add_object_types_fn))
}

fn handle_interface(
    reader: &mut quick_xml::Reader<&[u8]>,
    start: quick_xml::events::BytesStart<'_>,
) -> WlInterface {
    let name_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "name"
        })
        .expect("No name attr found for interface");

    let interface_name_snake = std::str::from_utf8(&name_attr.value).expect("utf8 encoding error");

    let mut msgs: Vec<WlMsg> = vec![];

    // Opcodes are tracked separately, in order, for each type (event or request)
    let mut event_opcode = 0;
    let mut request_opcode = 0;

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => panic!("Unexpected EOF"),
            Event::Start(e) => {
                let start_tag =
                    str::from_utf8(e.local_name().into_inner()).expect("Unable to parse start tag");
                if start_tag == "event" {
                    // An event! Increment our opcode tracker for it!
                    event_opcode += 1;
                    msgs.push(handle_request_or_event(
                        reader,
                        event_opcode - 1,
                        WlMsgType::Event,
                        interface_name_snake,
                        e,
                    ));
                } else if start_tag == "request" {
                    // A request! Increment our opcode tracker for it!
                    request_opcode += 1;
                    msgs.push(handle_request_or_event(
                        reader,
                        request_opcode - 1,
                        WlMsgType::Request,
                        interface_name_snake,
                        e,
                    ));
                };
            }
            Event::End(e) if e.local_name() == start.local_name() => break,
            _ => continue,
        }
    }

    WlInterface {
        name_snake: interface_name_snake.to_string(),
        msgs,
    }
}

fn handle_request_or_event(
    reader: &mut quick_xml::Reader<&[u8]>,
    opcode: u16,
    msg_type: WlMsgType,
    interface_name_snake: &str,
    start: quick_xml::events::BytesStart<'_>,
) -> WlMsg {
    let name_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "name"
        })
        .expect("No name attr found for request/event");
    // Load arguments and their types from XML
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
                let mut interface_name: Option<String> = None;

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
                    } else if attr_name == "interface" {
                        interface_name = Some(
                            str::from_utf8(&attr.value)
                                .expect("utf8 encoding error")
                                .to_string(),
                        );
                    }
                }

                if let Some(WlArgType::NewId(_)) = tt {
                    if let Some(interface_name) = interface_name {
                        tt.as_mut().unwrap().set_interface_name(interface_name);
                    } else {
                        // Unspecified interface for new_id; special serialization format!
                        args.push((
                            format!(
                                "{}_interface_name",
                                name.as_ref().expect("needs an arg name!")
                            ),
                            WlArgType::String,
                        ));
                        args.push((
                            format!(
                                "{}_interface_version",
                                name.as_ref().expect("needs an arg name!")
                            ),
                            WlArgType::Uint,
                        ))
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

    WlMsg {
        interface_name_snake: interface_name_snake.to_string(),
        name_snake: str::from_utf8(&name_attr.value)
            .expect("utf8 encoding error")
            .to_string(),
        msg_type,
        opcode,
        args,
    }
}

pub(crate) fn to_camel_case(s: &str) -> String {
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
