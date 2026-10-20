# Packaging

Scripts that build the release packages into `target/dist/`. The GitHub
release workflow (`.github/workflows/release.yml`) runs the same scripts on
a tag `v<version>`; locally they need the tools listed below.

| Package | Script | Runs on |
|---|---|---|
| `opentt-<version>-linux-x86_64.tar.gz` | `package-linux.sh tar` | Linux |
| `opentt_<version>-1_amd64.deb` | `package-linux.sh deb` | Linux |
| `opentt-<version>-linux-x86_64.AppImage` | `package-linux.sh appimage` | Linux |
| `opentt-<version>-windows-x86_64.zip` | `package-windows.sh` (Windows, Git Bash) or `cross-windows.sh` (Linux) | Windows / Linux |
| `opentt-<version>-macos-universal.dmg` | `package-macos.sh --universal` | macOS |
| `opentt-master-<version>-<os>-<arch>.tar.gz` / `.zip` (Windows): the master server alone | `package-master.sh` | Linux, macOS, Windows |

`<version>` is the workspace version of `Cargo.toml` (or `$VERSION`). Every
package carries `LICENSE` (GPL-3.0), `NOTICE`, `README.md` and the licence
of each embedded font (`fonts/<font>/OFL.txt`, SIL Open Font License 1.1).
`--no-build` reuses the release build in `target/`.

## Linux

```sh
packaging/package-linux.sh            # tar.gz, .deb and AppImage
packaging/package-linux.sh tar deb    # some of them
```

* Build: a Rust toolchain, `pkg-config` and the ALSA headers
  (`libasound2-dev` on Debian/Ubuntu).
* `.deb`: `cargo install cargo-deb`. The package is described in
  `[package.metadata.deb]` of `crates/tt-engine/Cargo.toml`: `/usr/bin/opentt`,
  the desktop entry, the AppStream metadata, the icons (hicolor) and the
  documents in `/usr/share/doc/opentt/`. Its dependencies are the libraries
  the binary links plus those it loads at run time (X11/Wayland,
  xkbcommon, Vulkan or EGL).
* AppImage: `appimagetool` on `PATH` or `APPIMAGETOOL=/path/to/appimagetool`
  (<https://github.com/AppImage/appimagetool/releases>). The AppImage holds
  the binary only, no libraries: it needs the system's ALSA, window system
  and graphics drivers, like the other packages. Build it on the oldest
  distribution it should run on (glibc).
* `tar.gz`: a portable folder: `opentt`, the documents, and `share/` with
  the desktop entry and icons (to copy to `~/.local/share/` if wanted).

`packaging/linux/` has the desktop entry and AppStream metadata
(`io.github.okkindel.OpenTT.*`, the id the game's window reports). Check
them with `desktop-file-validate` and `appstreamcli validate`.

An installed game (`/usr/bin`, an AppImage) cannot write next to its
executable, so it imports the game data into the user data folder
(`~/.local/share/opentt/gamedata`); the portable `tar.gz` folder keeps it
next to the executable when that folder is writable.

## Windows

On Windows (Git Bash, with Rust for MSVC): `packaging/package-windows.sh`.

From Linux: `packaging/cross-windows.sh` builds for `x86_64-pc-windows-gnu`.
Prerequisites (the script checks them and says what is missing):

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64     # x86_64-w64-mingw32-gcc and -windres
```

`windres` puts the icon and the version information (product name,
publisher, copyright, version) into the executable; without it the
executable is built without them. Zipping needs `zip`, `7z` or `python3`.

A release build is a GUI program (`windows_subsystem = "windows"`,
`crates/tt-engine/src/main.rs`): double-clicking it opens no console
window. Started from cmd or PowerShell it attaches to that console, and
`--dedicated` / `--master` open a console of their own when there is none.
A fatal error or panic shows a message box and goes to
`%APPDATA%\opentt\logs\crash.log` (next to `console.log`;
`tt_engine::crash`). Debug builds keep the console. To test a package:
start the game by double-click on a machine without the game data (the
first-run screen must open), then from cmd with `--help` (the usage must
appear in the console).

### Troubleshooting a player's report

* Ask for `%APPDATA%\opentt\logs\console.log` and `crash.log`.
* "It opens and closes": release 0.1.0 was a console program and
  panicked at once (winit refused the event loop off the main thread); the
  message vanished with the console window. Now it is in `crash.log` and a
  message box.
* Graphics: `WGPU_BACKEND=dx12|vulkan|gl` picks the backend (wgpu's
  `InstanceDescriptor::from_env_or_default`, `tt_engine::gfx::instance`);
  without an adapter that can draw, the error lists the adapters found.

## macOS

On a Mac with Rust and both targets
(`rustup target add aarch64-apple-darwin x86_64-apple-darwin`):

```sh
packaging/package-macos.sh --universal
```

It builds `OpenTT.app` (`packaging/macos/Info.plist`, bundle id
`io.github.okkindel.OpenTT`, icon `crates/tt-engine/icon/opentt.icns`),
signs it ad hoc (no developer certificate) and puts it with the documents
into a disk image (`hdiutil`).

The app is not notarized, so macOS blocks it on the first start. Open it
once from Finder with right-click → Open, or allow it in System Settings →
Privacy & Security → Open Anyway; or remove the quarantine flag:
`xattr -dr com.apple.quarantine /Applications/OpenTT.app`.
