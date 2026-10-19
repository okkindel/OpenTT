# Contributing to OpenTT

Thanks for helping. A few rules keep the project healthy and legal:

* **No game content.** Never commit files from a *Think Tanks* install or
  anything derived from them (models, textures, sounds, music, missions,
  scripts, extracted tables, screenshots of the game's art). The game data
  comes from `tt-import` on each user's machine. New assets must be your
  own work (or under a compatible free licence, recorded in `NOTICE`).
* **Code** is GPL-3.0 (`LICENSE`); by contributing you license your
  changes under it.
* **Build and test** before sending a change: `cargo build`,
  `cargo test --workspace` (with the game data imported if you can:
  `OPENTT_ASSETS=require cargo test --workspace`), `cargo doc --no-deps`.
* Keep changes focused, and describe what you changed and why in the
  pull request. Behaviour changes come with a test where possible.
