use std::{env, path::Path};

use quick_xml::events::Event;
use quote::{format_ident, quote};
use syn::Ident;
use types::{WlArgType, WlInterface, WlMsg, WlMsgType};

mod types;

pub fn main() {
    let proto_path = env::current_dir()
        .expect("current dir undefined")
        .join("proto");
    let generated_path = env::current_dir()
        .expect("current dir undefined")
        .join("generated");
    std::fs::create_dir(&generated_path).ok();
    generate_from_dir(&generated_path, &proto_path);
}

pub fn generate_from_dir(out_dir: impl AsRef<Path>, p: impl AsRef<Path>) {
    let proto_mods_dir = out_dir.as_ref().join("proto_generated");
    std::fs::remove_dir_all(&proto_mods_dir).ok();
    std::fs::create_dir(&proto_mods_dir).expect("Unable to create proto_generated");

    let ((file_names, gen_code), (add_parsers_fn, add_object_types_fn)): (
        (Vec<_>, Vec<_>),
        (Vec<_>, Vec<_>),
    ) = std::fs::read_dir(p)
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

    let file_name_idents = file_names.iter().map(|name| format_ident!("{name}"));
    let file_relative_paths = file_names
        .iter()
        .map(|name| format!("../generated/proto_generated/{name}.rs"));

    for (i, file_name) in file_names.iter().enumerate() {
        let rs_file = proto_mods_dir.join(format!("{}.rs", file_name));
        std::fs::write(&rs_file, gen_code[i].to_string()).expect("unable to write generated file");
        std::process::Command::new("rustfmt")
            .arg(rs_file.to_str().expect("utf8 error"))
            .output()
            .ok();
    }

    let main_gen = quote! {
        #( #[path = #file_relative_paths] mod #file_name_idents; pub use #file_name_idents::*; )*

        pub(super) fn wl_init_parsers(
            event_parsers: &mut std::collections::HashMap<(crate::objects::WlObjectType, u16), &'static dyn crate::proto::WlMsgParserFn>,
            request_parsers: &mut std::collections::HashMap<(crate::objects::WlObjectType, u16), &'static dyn crate::proto::WlMsgParserFn>
        ) {
            #( #add_parsers_fn(event_parsers, request_parsers); )*
        }

        pub(super) fn wl_init_known_types(object_types: &mut std::collections::HashMap<&'static str, crate::objects::WlObjectType>) {
            #( #add_object_types_fn(object_types); )*
        }
    }.to_string();

    let main_gen_file = out_dir.as_ref().join("proto_generated.rs");
    std::fs::write(&main_gen_file, main_gen).expect("unable to write proto_generated.rs");

    std::process::Command::new("rustfmt")
        .arg(main_gen_file.to_str().expect("utf8 error"))
        .output()
        .ok();
}

fn generate_from_xml_file(
    p: impl AsRef<Path>,
) -> ((String, proc_macro2::TokenStream), (Ident, Ident)) {
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
    let (mut event_interface_types, mut event_opcodes, mut event_parsers): (
        Vec<Ident>,
        Vec<u16>,
        Vec<Ident>,
    ) = Default::default();
    let (mut request_interface_types, mut request_opcodes, mut request_parsers): (
        Vec<Ident>,
        Vec<u16>,
        Vec<Ident>,
    ) = Default::default();
    let (mut known_interface_names, mut known_interface_consts): (Vec<String>, Vec<Ident>) =
        (vec![], vec![]);

    for i in interfaces.iter() {
        known_interface_names.push(i.name_snake.clone());
        known_interface_consts.push(format_ident!("{}", i.type_const_name()));

        code.push(i.generate());

        let interface_type = format_ident!("{}", i.name_snake.to_uppercase());

        for m in i.msgs.iter() {
            let parser_name = format_ident!("{}", m.parser_fn_name());
            let opcode = m.opcode;

            match m.msg_type {
                WlMsgType::Event => {
                    event_interface_types.push(interface_type.clone());
                    event_opcodes.push(opcode);
                    event_parsers.push(parser_name);
                }
                WlMsgType::Request => {
                    request_interface_types.push(interface_type.clone());
                    request_opcodes.push(opcode);
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
        #[allow(unused)]
        use crate::proto::WlParsedMessage;
        #[allow(unused)]
        use byteorder::ByteOrder;
        #[allow(unused)]
        use serde_derive::Serialize;

        #( #code )*

        #[allow(unused)]
        pub(super) fn #add_parsers_fn(
            event_parsers: &mut std::collections::HashMap<(crate::objects::WlObjectType, u16), &'static dyn crate::proto::WlMsgParserFn>,
            request_parsers: &mut std::collections::HashMap<(crate::objects::WlObjectType, u16), &'static dyn crate::proto::WlMsgParserFn>
        ) {
            #( event_parsers.insert((#event_interface_types, #event_opcodes), &#event_parsers); )*
            #( request_parsers.insert((#request_interface_types, #request_opcodes), &#request_parsers); )*
        }

        #[allow(unused)]
        pub(super) fn #add_object_types_fn(object_types: &mut std::collections::HashMap<&'static str, crate::objects::WlObjectType>) {
            #( object_types.insert(#known_interface_names, #known_interface_consts); )*
        }
    };

    (
        (file_name_snake, ret_code),
        (add_parsers_fn, add_object_types_fn),
    )
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

    let mut add_msg = |reader: &mut quick_xml::Reader<&[u8]>,
                       e: quick_xml::events::BytesStart<'_>,
                       is_empty: bool| {
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
                is_empty,
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
                is_empty,
            ));
        };
    };

    loop {
        match reader.read_event().expect("Unable to parse XML file") {
            Event::Eof => panic!("Unexpected EOF"),
            Event::Start(e) => {
                add_msg(reader, e, false);
            }
            Event::Empty(e) => {
                add_msg(reader, e, true);
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
    is_empty: bool,
) -> WlMsg {
    let name_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "name"
        })
        .expect("No name attr found for request/event");
    let type_attr = start
        .attributes()
        .map(|a| a.expect("attr parsing error"))
        .find(|a| {
            std::str::from_utf8(a.key.local_name().into_inner()).expect("utf8 encoding error")
                == "type"
        })
        .map(|a| {
            str::from_utf8(&a.value)
                .expect("utf8 encoding error")
                .to_string()
        });

    let is_destructor = type_attr.map(|a| a == "destructor").unwrap_or(false);

    // Load arguments and their types from XML
    let mut args: Vec<(String, WlArgType)> = Vec::new();

    if !is_empty {
        loop {
            match reader.read_event().expect("Unable to parse XML file") {
                Event::Eof => panic!("Unexpected EOF"),
                Event::Empty(e)
                    if str::from_utf8(e.local_name().into_inner())
                        .expect("utf8 encoding error")
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

                    if let Some(ref mut name) = name {
                        if name == "type" {
                            *name = "_type".to_string();
                        } else if name == "msg" {
                            *name = "_msg".to_string();
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
    }

    WlMsg {
        interface_name_snake: interface_name_snake.to_string(),
        name_snake: str::from_utf8(&name_attr.value)
            .expect("utf8 encoding error")
            .to_string(),
        msg_type,
        opcode,
        is_destructor,
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
