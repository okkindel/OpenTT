//! Rewrites data files (`data/*.ron`, missions) written with the names of
//! earlier versions to the current names, in place (only the names change;
//! the game reads the old names too, with a deprecation note):
//!
//! ```text
//! cargo run -p tt-assets --example migrate_data -- mods/my-mod/data/weapons.ron ...
//! ```

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: migrate_data FILE.ron...");
        std::process::exit(2);
    }
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        let (new, notes) = tt_assets::data::legacy::migrate(&text);
        if notes.is_empty() {
            println!("{f}: up to date");
            continue;
        }
        std::fs::write(&f, new.as_bytes()).unwrap_or_else(|e| panic!("{f}: {e}"));
        println!("{f}: {} names rewritten", notes.len());
        for n in notes {
            println!("  {n}");
        }
    }
}
