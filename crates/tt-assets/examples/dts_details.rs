//! Prints the detail levels of `.dts` shapes (name, size, polys, geometry).
//!
//! ```sh
//! cargo run -p tt-assets --example dts_details -- assets/models/props/green/*.dts
//! ```

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("{path}: unreadable");
            continue;
        };
        match tt_assets::dts::Shape::parse(&data) {
            Ok(s) => {
                println!("{path}: radius {:.2}, smallest visible size {} dl {}", s.radius, s.smallest_visible_size, s.smallest_visible_dl);
                for m in &s.materials {
                    println!("  material {:<20} flags {:#x} reflect {:?} {:.2}", m.name, m.flags, m.reflectance_map, m.reflection_amount);
                }
                for (i, d) in s.detail_levels.iter().enumerate() {
                    let tris: usize = s.detail_geometry(i).iter().map(|m| m.triangles.len()).sum();
                    println!("  {i}: {:<16} size {:>7.1} sub {:>2} od {:>3} polys {:>5} tris {tris}", d.name, d.size, d.sub_shape, d.object_detail, d.poly_count);
                }
            }
            Err(e) => eprintln!("{path}: {e:?}"),
        }
    }
}
