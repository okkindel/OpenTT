//! Renderer for ThinkTanks scenes (terrain, sky, fog, DTS shapes), shared by
//! the mission viewer and the game.

pub mod lighting;
pub mod loadstats;
pub mod objectview;
pub mod fx;
pub mod gpu;
pub mod gputime;
pub mod pose;
pub mod post;
pub mod render;
pub mod scene;
pub mod shapes;
pub mod sky;
pub mod sprites;
pub mod textures;

pub use render::{vertical_fov, Camera, DynamicInstance, InstanceOptions, PointLight, RenderSettings, Renderer, Target, View, MAX_POINT_LIGHTS};
pub use post::Extras;
pub use sprites::{Sprite, SpriteBlend, SpriteOrient};
pub use scene::Scene;

/// An sRGB colour channel (0..1, clamped) in linear light.
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
