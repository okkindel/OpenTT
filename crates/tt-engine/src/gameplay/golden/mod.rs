//! One text line per effect, prefixed by the tick (`{tick:6} `). Identities
//! are stable names, never interpreter object ids:
//! * `C<n>` – a client (human, bot or test connection), numbered from 1 in
//!   order of first appearance; on a server `ClientGroup` then
//!   `BotClientGroup` are scanned (member order = join order) before every
//!   record, so the numbers follow the join order there; on a remote client
//!   the server's ids get numbers as the messages bring them;
//! * `T<n>` / `P<n>` / `F<n>` – tt-game's tank / trigger / flag ids;
//! * teams by object name (`BlueTeam`), defs and profiles by name.
//!
//! Lines ([`record`]):
//! * `hud <to> <ClientMsg>` – client-side effects applied without a remote
//!   command (the local client);
//! * `snd`/`src`/`music` – every sound and music change the audio layer gets
//!   ([`record::GoldenAudio`]);
//! * `death`/`gone`/`pup`/`pad`/`flag`/`volcano` – world events (authority),
//!   `spawn T<n> C<n> <def> <pos bits>` – new tanks;
//! * `score C<n> …` / `team <name> …` – the gameplay records (score,
//!   cumScore, kills, deaths, solo counters, …) and K/D/A, on change;
//! * `round …`, `solo …`, `menu …`, `pred …` – round / mission state, the
//!   solo run, the menu screen and canvas content, client prediction stats;
//! * `view <field>=<value>` – changed fields of the native HUD's model
//!   (`HudView`, sampled every tick; radar without blips, no name tags);
//! * `W srng=<script rng> wrng=<world rng> t=<world ms> h=<tank digest>` each
//!   tick it changes, and every 32 ticks `tanks …` with the full tank state
//!   bits;
//! * `save <file> len=<n> fnv=<hash>` when a file in the data directory
//!   changes (the RON saves, the ban lists), and at the end `file <path>`
//!   followed by its lines (`| …`).

pub mod action;
pub mod drive;
pub mod record;
pub mod run;
pub mod scenarios;

pub use action::{Cheat, ScenarioAction};
pub use drive::first_difference;
pub use record::Golden;
pub use run::{run, run_netplay, GoldenScenario};
