//! * `cfg(tt_original)` when `TT_ORIGINAL_DIR` is set at build time: the
//!   import verification test (tests/import.rs) then runs by default instead
//!   of being ignored.
//! * `$OUT_DIR/ours.rs`: our own asset files (the `ours` list of
//!   `mapping.ron`) built into the importer from the repository's
//!   `assets/`, so the game can import without a copy of that directory
//!   (`tt_import::OURS`; a file missing at build time is `None`).

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(tt_original)");
    println!("cargo::rerun-if-env-changed=TT_ORIGINAL_DIR");
    if std::env::var_os("TT_ORIGINAL_DIR").is_some_and(|v| !v.is_empty()) {
        println!("cargo::rustc-cfg=tt_original");
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mapping = Path::new(&manifest).join("mapping.ron");
    println!("cargo::rerun-if-changed={}", mapping.display());
    let text = std::fs::read_to_string(&mapping).expect("mapping.ron");
    let start = text.find("\n  ours: [").expect("mapping.ron: no `ours: [` list") + 10;
    let end = start + text[start..].find(']').expect("mapping.ron: unterminated `ours` list");
    let assets = Path::new(&manifest).join("../../assets");
    let mut code = String::from("/// Our own asset files: (asset path, contents when built in).\npub static OURS: &[(&str, Option<&[u8]>)] = &[\n");
    for asset in text[start..end].split('"').skip(1).step_by(2) {
        let file = assets.join(asset);
        println!("cargo::rerun-if-changed={}", file.display());
        match file.canonicalize() {
            Ok(abs) if abs.is_file() => code.push_str(&format!("    ({asset:?}, Some(include_bytes!({:?}))),\n", abs.display().to_string())),
            _ => code.push_str(&format!("    ({asset:?}, None),\n")),
        }
    }
    code.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("ours.rs");
    std::fs::write(out, code).expect("ours.rs");
}
