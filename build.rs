fn main() {
    let out_dir = std::env::var_os("OUT_DIR").unwrap();

    protogen::generate_from_dir(&out_dir, "proto");

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=proto");
    println!("cargo::rerun-if-changed=protogen");
}
