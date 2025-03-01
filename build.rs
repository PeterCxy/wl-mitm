use std::{env, path::Path};

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_file = Path::new(&out_dir).join("proto_generated.rs");

    std::fs::write(&out_file, protogen::generate_from_dir("proto"))
        .expect("unable to write to file");

    // If rustfmt is present, run rustfmt so that the generated file is somewhat human-readable
    std::process::Command::new("rustfmt")
        .arg(out_file.to_str().expect("utf8 error"))
        .output()
        .ok();

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=proto");
    println!("cargo::rerun-if-changed=protogen");
}
