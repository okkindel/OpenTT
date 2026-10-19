//! The application icon (`icon/` of this crate, see its `README.md`): the
//! window icon and the application id that desktop files match.

use winit::window::{Icon, WindowAttributes};

/// The application id: the Wayland app id and the X11 `WM_CLASS`, which
/// the desktop entry `io.github.okkindel.OpenTT.desktop` (its file name and
/// `StartupWMClass`, `packaging/linux/`) and the AppStream id match.
pub const APP_ID: &str = "io.github.okkindel.OpenTT";

/// The window icon, 256 px (window managers scale it down to the title bar,
/// task bar and dock sizes; a 64 px one looked blurry when a panel scaled it up).
const PNG: &[u8] = include_bytes!("../icon/opentt-256.png");

/// Edge length of [`PNG`].
pub const SIZE: u32 = 256;

/// The icon as RGBA pixels: (pixels, width, height).
pub fn rgba() -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png).map_err(|e| format!("window icon: {e}"))?.into_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

/// The window icon; `None` (with a warning) if it cannot be made.
pub fn window_icon() -> Option<Icon> {
    match rgba().and_then(|(px, w, h)| Icon::from_rgba(px, w, h).map_err(|e| format!("window icon: {e}"))) {
        Ok(icon) => Some(icon),
        Err(e) => {
            eprintln!("warning: {e}");
            None
        }
    }
}

/// `attrs` with the window icon and the application id.
pub fn with_icon(attrs: WindowAttributes) -> WindowAttributes {
    let attrs = attrs.with_window_icon(window_icon());
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, APP_ID, APP_ID);
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_decodes_to_rgba() {
        let (px, w, h) = rgba().unwrap();
        assert_eq!((w, h), (SIZE, SIZE));
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        // Transparent corners (rounded tile), an opaque centre.
        assert_eq!(px[3], 0);
        let centre = ((SIZE / 2 * SIZE + SIZE / 2) * 4 + 3) as usize;
        assert_eq!(px[centre], 255);
        assert!(Icon::from_rgba(px, w, h).is_ok());
    }

    #[test]
    fn ico_lists_the_sizes() {
        let ico = include_bytes!("../icon/opentt.ico");
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let sizes: Vec<u8> = (0..n).map(|i| ico[6 + 16 * i]).collect();
        assert_eq!(sizes, [16, 32, 48, 0]); // 0 = 256
    }

    #[test]
    fn desktop_entry_matches_the_app_id() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/linux");
        let desktop = std::fs::read_to_string(dir.join(format!("{APP_ID}.desktop"))).unwrap();
        assert!(desktop.lines().any(|l| l == format!("StartupWMClass={APP_ID}")));
        assert!(desktop.lines().any(|l| l == format!("Icon={APP_ID}")));
        let metainfo = std::fs::read_to_string(dir.join(format!("{APP_ID}.metainfo.xml"))).unwrap();
        assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
    }
}
