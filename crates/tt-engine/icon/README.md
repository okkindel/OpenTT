# Application icon

A cartoon tank driven by a brain in a jar, firing, on a desert battlefield.

* Source: `opentt-source.png` (1254x1254, second version), supplied by the project author
  (okkindel); no game art. Same licence as the code.
* `opentt-<size>.png` (16, 32, 48, 64, 128, 256), `opentt.ico` (16, 32,
  48, 256 px) and the macOS `opentt.icns` (16 to 1024 px, with the Retina
  variants) are generated from it: 64 px and up the whole picture
  (Lanczos), 48 px and below a tighter square around the tank and its jar,
  lightly sharpened; rounded corners.

## Where it is used

* Window icon: `src/icon.rs` embeds `opentt-256.png` and sets it on the game
  window (X11 and Windows; Wayland compositors take the icon from the
  desktop entry, matched by the app id `io.github.okkindel.OpenTT`).
* Windows: `build.rs` puts `opentt.ico` into the executable's resources
  (with the version information).
* Linux: the desktop entry and AppStream metadata are in `packaging/linux/`
  (`io.github.okkindel.OpenTT.desktop`, `Icon=io.github.okkindel.OpenTT`);
  the packages install the PNGs as
  `share/icons/hicolor/<s>x<s>/apps/io.github.okkindel.OpenTT.png`. To
  install them for the current user from a checkout:

  ```sh
  for s in 16 32 48 64 128 256; do
    install -Dm644 crates/tt-engine/icon/opentt-${s}.png ~/.local/share/icons/hicolor/${s}x${s}/apps/io.github.okkindel.OpenTT.png
  done
  install -Dm644 packaging/linux/io.github.okkindel.OpenTT.desktop ~/.local/share/applications/io.github.okkindel.OpenTT.desktop
  ```

  `Exec=opentt` names the installed binary: edit it to the binary's path
  when it is not on `PATH`.
* macOS: `opentt.icns` is the app bundle's icon (`packaging/package-macos.sh`).
