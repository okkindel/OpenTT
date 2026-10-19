//! Terrain heightfield files (`.ter`, version 4).
//!
//! ```text
//! u8  version (4)
//! u8  block_shift            size = 1 << block_shift (128, or 64 for *_LO)
//! u8  unknown[8]             always 00 00 00 09 00 00 00 00 in the game data
//! u16 heights[size*size]     row-major (x fastest), metres = value / 32
//! u8  material_map[size*size]
//! 8 x { u8 len, char[len] }  material texture paths (empty = unused slot)
//! u8  alpha[size*size]       for each used material slot, in slot order
//! u32 len, char[len]         texture generation script (editor)
//! u32 len, char[len]         heightfield generation script (editor)
//! ```
//!
//! The block is centred on the world origin: grid point (0, 0) is at
//! `-size * square_size / 2` on both axes (the mission's `position` field is
//! not used), and the heightfield repeats infinitely. Verified by fitting the
//! positions of static shapes in the missions against the ground height.

pub const MATERIAL_SLOTS: usize = 8;

pub use crate::bytes::Error;
use crate::bytes::Reader;

#[derive(Debug, Clone)]
pub struct Terrain {
    pub version: u8,
    pub block_shift: u8,
    pub heights: Vec<u16>,
    pub header: [u8; 8],
    pub material_map: Vec<u8>,
    /// Texture path (without extension) per slot; empty for unused slots.
    pub materials: [String; MATERIAL_SLOTS],
    /// Per-slot blend weights (0..255), present for used slots.
    pub alpha: [Option<Vec<u8>>; MATERIAL_SLOTS],
    pub texture_script: String,
    pub heightfield_script: String,
}

impl Terrain {
    pub fn parse(data: &[u8]) -> Result<Terrain, Error> {
        let mut r = Reader { d: data, pos: 0 };
        let version = r.u8()?;
        if version != 4 {
            return Err(Error(format!("unsupported terrain version {version}")));
        }
        let block_shift = r.u8()?;
        if !(1..=12).contains(&block_shift) {
            return Err(Error(format!("implausible block shift {block_shift}")));
        }
        let n = 1usize << (2 * block_shift as usize);
        let header = r.take(8)?.try_into().unwrap();
        let heights = r.take(n * 2)?.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        let material_map = r.take(n)?.to_vec();
        let mut materials: [String; MATERIAL_SLOTS] = Default::default();
        for m in &mut materials {
            let len = r.u8()? as usize;
            *m = crate::latin1(r.take(len)?);
        }
        let mut alpha: [Option<Vec<u8>>; MATERIAL_SLOTS] = Default::default();
        for (a, m) in alpha.iter_mut().zip(&materials) {
            if !m.is_empty() {
                *a = Some(r.take(n)?.to_vec());
            }
        }
        let len = r.u32()? as usize;
        let texture_script = crate::latin1(r.take(len)?);
        let len = r.u32()? as usize;
        let heightfield_script = crate::latin1(r.take(len)?);
        if r.pos != data.len() {
            return Err(Error(format!("{} trailing bytes", data.len() - r.pos)));
        }
        Ok(Terrain {
            version,
            block_shift,
            header,
            heights,
            material_map,
            materials,
            alpha,
            texture_script,
            heightfield_script,
        })
    }

    /// A new terrain of `1 << block_shift` samples per edge from heights in
    /// metres (row-major, x fastest; rounded to the 1/32 m file units) and
    /// up to [`MATERIAL_SLOTS`] materials with their blend weights.
    pub fn new(block_shift: u8, heights_m: &[f32], materials: &[(&str, Vec<u8>)]) -> Terrain {
        let n = 1usize << (2 * block_shift as usize);
        assert_eq!(heights_m.len(), n, "heights");
        assert!(materials.len() <= MATERIAL_SLOTS);
        let mut m: [String; MATERIAL_SLOTS] = Default::default();
        let mut alpha: [Option<Vec<u8>>; MATERIAL_SLOTS] = Default::default();
        for (i, (path, a)) in materials.iter().enumerate() {
            assert_eq!(a.len(), n, "alpha of {path}");
            m[i] = path.to_string();
            alpha[i] = Some(a.clone());
        }
        Terrain {
            version: 4,
            block_shift,
            // As in every shipped terrain.
            header: [0, 0, 0, 9, 0, 0, 0, 0],
            heights: heights_m.iter().map(|h| (h * 32.0).round().clamp(0.0, u16::MAX as f32) as u16).collect(),
            material_map: vec![0; n],
            materials: m,
            alpha,
            texture_script: String::new(),
            heightfield_script: String::new(),
        }
    }

    /// The file bytes (the inverse of [`Terrain::parse`]).
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let n = self.size() * self.size();
        if self.heights.len() != n || self.material_map.len() != n {
            return Err(Error("heights/material map do not match the block size".into()));
        }
        let latin1 = |s: &str| -> Result<Vec<u8>, Error> { s.chars().map(|c| u8::try_from(c as u32).map_err(|_| Error(format!("{s:?} is not latin-1")))).collect() };
        let mut out = Vec::with_capacity(10 + n * 8);
        out.push(self.version);
        out.push(self.block_shift);
        out.extend_from_slice(&self.header);
        for h in &self.heights {
            out.extend_from_slice(&h.to_le_bytes());
        }
        out.extend_from_slice(&self.material_map);
        for m in &self.materials {
            let b = latin1(m)?;
            out.push(u8::try_from(b.len()).map_err(|_| Error(format!("material path {m:?} too long")))?);
            out.extend_from_slice(&b);
        }
        for (a, m) in self.alpha.iter().zip(&self.materials) {
            match (a, m.is_empty()) {
                (Some(a), false) if a.len() == n => out.extend_from_slice(a),
                (_, true) => {}
                _ => return Err(Error(format!("material {m:?} needs {n} alpha values"))),
            }
        }
        for s in [&self.texture_script, &self.heightfield_script] {
            let b = latin1(s)?;
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(&b);
        }
        Ok(out)
    }

    /// Number of samples along one edge.
    pub fn size(&self) -> usize {
        1 << self.block_shift
    }

    /// World coordinate of grid point 0 on both axes.
    pub fn origin(&self, square_size: f32) -> f32 {
        -(self.size() as f32) * square_size / 2.0
    }

    /// Height in metres at grid point (x, y).
    pub fn height(&self, x: i64, y: i64) -> f32 {
        let s = self.size() as i64;
        let (x, y) = (x.rem_euclid(s) as usize, y.rem_euclid(s) as usize);
        self.heights[y * self.size() + x] as f32 / 32.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_parse_round_trip() {
        let shift = 3;
        let n = 64;
        let heights: Vec<f32> = (0..n).map(|i| 100.0 + i as f32 * 0.25).collect();
        let a: Vec<u8> = (0..n).map(|i| (i * 4) as u8).collect();
        let b: Vec<u8> = a.iter().map(|v| 255 - v).collect();
        let mut t = Terrain::new(shift, &heights, &[("terrain/green/fterrain01.png", a.clone()), ("terrain/green/fterrain03.png", b.clone())]);
        t.texture_script = "tex".into();
        t.heightfield_script = "hf".into();
        let bytes = t.to_bytes().unwrap();
        let p = Terrain::parse(&bytes).unwrap();
        assert_eq!(p.size(), 8);
        assert_eq!(p.heights, t.heights);
        assert_eq!(p.height(1, 0), 100.25);
        assert_eq!(p.height(9, 8), 100.25, "wraps");
        assert_eq!(p.materials[1], "terrain/green/fterrain03.png");
        assert!(p.materials[2].is_empty());
        assert_eq!(p.alpha[0].as_deref(), Some(&a[..]));
        assert_eq!(p.alpha[1].as_deref(), Some(&b[..]));
        assert!(p.alpha[2].is_none());
        assert_eq!((p.texture_script.as_str(), p.heightfield_script.as_str()), ("tex", "hf"));
        assert_eq!(p.to_bytes().unwrap(), bytes);
    }

    /// Shipped terrains are written back byte for byte.
    #[test]
    fn rewrites_game_terrain_identically() {
        crate::require_game_data!();
        let p = crate::layout::default_dir().join("missions/tt1_1.ter");
        let data = std::fs::read(p).unwrap();
        assert_eq!(Terrain::parse(&data).unwrap().to_bytes().unwrap(), data);
    }
}
