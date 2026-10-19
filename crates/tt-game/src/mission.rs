//! Mission object helpers: transforms and shapes of mission objects.

use std::path::Path;

use glam::Affine3A;
use tt_assets::dts::Shape;
use tt_assets::mission::SceneObject;

use crate::geom::mission_transform;

/// Object transform from its mission fields.
pub fn object_transform(o: &SceneObject) -> Affine3A {
    mission_transform(&o.floats("position"), &o.floats("rotation"), &o.floats("scale"))
}

pub fn load_shape(game_root: &Path, mission_dir: &Path, path: &str) -> Option<Shape> {
    let p = tt_assets::find_ci(&tt_assets::resolve(game_root, mission_dir, path))?;
    Shape::parse(&std::fs::read(p).ok()?).ok()
}
