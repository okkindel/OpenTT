//! `cfg(tt_original)` when `TT_ORIGINAL_DIR` is set at build time: the
//! import-flow test (tests/game_data.rs) then runs instead of being ignored.
//!
//! For a macOS target the binary's main thread gets a 32 MB stack (the
//! game runs on it there; `src/main.rs`).
//!
//! For a Windows target the application icon (`icon/opentt.ico`) and the
//! version information (company, product name from `TT_PRODUCT_NAME`,
//! copyright, the package version) go into the executable's resources
//! (build dependency `winresource`: the MSVC resource compiler on Windows,
//! `x86_64-w64-mingw32-windres` in a cross build from Linux; without one
//! the executable is built without them, with a warning).

fn main() {
    println!("cargo::rustc-check-cfg=cfg(tt_original)");
    println!("cargo::rerun-if-env-changed=TT_ORIGINAL_DIR");
    if std::env::var_os("TT_ORIGINAL_DIR").is_some_and(|v| !v.is_empty()) {
        println!("cargo::rustc-cfg=tt_original");
    }
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => windows_resources(),
        // The game runs on the main thread there (src/main.rs,
        // ENGINE_STACK): its stack is the size linked into the binary
        // (default 8 MB; Apple's ld takes at most 64 MB, in hex, a multiple
        // of the page size). tests/startup.rs checks the binary's header.
        Ok("macos") => println!("cargo::rustc-link-arg-bins=-Wl,-stack_size,{MACOS_MAIN_STACK}"),
        _ => {}
    }
}

/// The main thread's stack of the macOS binary: 32 MB.
const MACOS_MAIN_STACK: &str = "0x2000000";

/// The publisher shown in the executable's properties.
const COMPANY: &str = "okkindel";
/// The copyright line shown in the executable's properties.
const COPYRIGHT: &str = "Copyright (C) 2026 okkindel";

fn windows_resources() {
    println!("cargo::rerun-if-changed=icon/opentt.ico");
    println!("cargo::rerun-if-env-changed=TT_PRODUCT_NAME");
    let product = std::env::var("TT_PRODUCT_NAME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "OpenTT".into());
    let mut res = winresource::WindowsResource::new();
    res.set_icon("icon/opentt.ico")
        .set("CompanyName", COMPANY)
        .set("ProductName", &product)
        .set("FileDescription", &product)
        .set("InternalName", &product)
        .set("LegalCopyright", COPYRIGHT);
    if let Err(e) = res.compile() {
        println!("cargo::warning=no icon or version information in the executable: {e}");
    }
}
