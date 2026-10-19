//! What the native HUD, the FPS counter and the menus share:
//!
//! * [`uifont`] – the bundled UI typeface (all text), rasterised at the target pixel size
//!   into a glyph atlas;
//! * [`geom`] – pixel points, rectangles and colours.

pub mod geom;
pub mod hud;
pub mod uifont;

pub use geom::{Color, Point, Rect};
