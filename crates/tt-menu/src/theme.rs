//! Look of the new menus: the "field garage" palette (`screens::pal`:
//! gunmetal, olive-tinted steel, olive drab, brass, sand), the bundled fonts
//! (Chakra Petch for text, Russo One for headings and labels).

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

use crate::screens::pal;

/// Brass, the logo lettering (highlights outside the menus, e.g. the HUD).
pub const GOLD: Color32 = pal::BRASS;
/// Team colours as the game uses them.
pub const TEAM_GREEN: Color32 = Color32::from_rgb(0x3c, 0xdc, 0x3c);
pub const TEAM_BLUE: Color32 = Color32::from_rgb(0x3c, 0x78, 0xff);

/// Font family for headings and big menu items.
pub fn heading_family() -> FontFamily {
    FontFamily::Name("tt-heading".into())
}

/// Font family of the game lettering (the regular weight of the UI font).
pub fn game_family() -> FontFamily {
    FontFamily::Name("tt-game".into())
}

/// Font family of the display face (Russo One: headings, tab and button labels).
pub fn display_family() -> FontFamily {
    FontFamily::Name("tt-display".into())
}

/// The display face for a line box of `size` points, with the same cap
/// height as [`game_font`] of that size.
pub fn display_font(size: f32) -> FontId {
    FontId::new(size * 0.62 / tt_gui::uifont::heading_face().cap_per_em(), display_family())
}

/// The game lettering for a line box of `size` points: cap height 0.62 of
/// `size` (egui font sizes are em sizes).
pub fn game_font(size: f32) -> FontId {
    FontId::new(size * 0.62 / tt_gui::uifont::ui_font().regular.cap_per_em(), game_family())
}

/// Install the UI font (`tt_gui::uifont`, bundled): the regular weight for
/// body text and the game lettering, the bold weight for headings; egui's
/// built-in fonts stay behind them for missing glyphs.
pub fn install_fonts(ctx: &egui::Context) {
    let ui = tt_gui::uifont::ui_font();
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("tt-regular".into(), FontData::from_static(ui.regular.data).into());
    defs.font_data.insert("tt-bold".into(), FontData::from_static(ui.bold.data).into());
    defs.font_data.insert("tt-display".into(), FontData::from_static(tt_gui::uifont::HEADING_TTF).into());
    let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| std::iter::once(first.to_string()).chain(fallback.iter().cloned()).collect::<Vec<_>>();
    defs.families.insert(FontFamily::Proportional, with("tt-regular"));
    defs.families.insert(game_family(), with("tt-regular"));
    defs.families.insert(heading_family(), with("tt-bold"));
    defs.families.insert(display_family(), with("tt-display"));
    ctx.set_fonts(defs);
}

/// Installs the fonts unless they are installed; true when it did (they
/// take effect on the next frame).
pub fn ensure(ctx: &egui::Context) -> bool {
    if ctx.fonts(|f| f.families().contains(&heading_family())) {
        return false;
    }
    install_fonts(ctx);
    ctx.request_repaint();
    true
}

/// Points per pixel so the 1280x720 layout scales with the window
/// (resolution independence: the menus are laid out in a 1280-wide space).
pub fn pixels_per_point(screen_px: [f32; 2]) -> f32 {
    (screen_px[0] / 1280.0).min(screen_px[1] / 720.0).max(0.5)
}
