//! [`EngineHost`]: the engine's state — files, log, window and video,
//! canvas, audio, input, the local and network connections, the game
//! server and client, the simulation of the loaded mission, the shell —
//! and the clock and timed events every part shares.

use crate::audio::Audio;
use crate::canvas::Canvas;
use crate::fs::GameFs;
use crate::input::Input;
use crate::log::Logger;
use std::collections::HashMap;
use std::time::Instant;

/// Build answers for the distribution queries. The defaults describe the
/// full, registered, non-demo game (see `Platform::default`).
#[derive(Debug, Clone)]
pub struct Platform {
    pub release: bool,
    pub demo: bool,
    pub world_limited: bool,
    pub platform: String,
}

impl Default for Platform {
    fn default() -> Self {
        Platform {
            release: true,
            demo: false,
            world_limited: false,
            platform: "windows".into(),
        }
    }
}

/// Window/video state the game asks for. The window follows it.
#[derive(Debug, Clone)]
pub struct Video {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub fullscreen: bool,
    /// Set when the game changed the mode; the window code clears it.
    pub changed: bool,
    pub desktop: (u32, u32),
    pub force_windowed: bool,
    /// The monitor's display modes (w, h), sorted, from the window system.
    pub modes: Vec<(u32, u32)>,
    /// Present with vsync (Fifo/Mailbox) or not (Immediate).
    pub vsync: bool,
}

impl Default for Video {
    fn default() -> Self {
        Video {
            title: tt_assets::product::NAME.into(),
            width: 800,
            height: 600,
            bpp: 32,
            fullscreen: false,
            changed: false,
            desktop: (1920, 1080),
            force_windowed: false,
            modes: Vec::new(),
            vsync: true,
        }
    }
}

#[derive(Default)]
pub struct Stats {
    /// Engine features that exist but do nothing here (counted by name).
    pub stubs: HashMap<String, usize>,
}

pub struct EngineHost {
    pub fs: GameFs,
    pub log: Logger,
    pub platform: Platform,
    pub video: Video,
    /// The screens and dialogs on the canvas, the cursor.
    pub canvas: Canvas,
    pub audio: Box<dyn Audio>,
    pub input: Input,
    pub stats: Stats,
    start: Instant,
    pub quit_requested: bool,
    /// Settings → Game data → Re-import: the window shuts the game down
    /// and shows the import screen ([`crate::launcher`]).
    pub reimport_requested: bool,
    /// Options → Mods → Restart: the window shuts the game down and boots
    /// it again on the same data (the enabled mods apply; [`crate::launcher`]).
    pub restart_requested: bool,
    /// The static mission scene (terrain, sky, shapes) changed.
    pub scene_dirty: bool,
    /// Local client/server connection and deferred "packets".
    pub net: crate::net::Net,
    /// Network play over tt-net (remote clients / our remote server).
    pub netgame: crate::netgame::NetGame,
    /// Gameplay simulation of the loaded mission.
    pub game: Option<crate::game::GameSim>,
    /// The defs the data files declared ([`crate::gameplay::defs`]).
    pub defs: tt_assets::data::defs::DefSet,
    /// The same, typed (rebuilt as the defs change:
    /// [`crate::gameplay::defs::refresh`]).
    pub data: tt_gameplay::data::GameData,
    /// The power-up weapons of the server's data.
    pub weapons: Vec<tt_gameplay::data::WeaponEntry>,
    /// The loaded mission's objects as its RON file declared them
    /// ([`crate::mission::exec_mission`]; what the world and the scene are built from).
    pub mission_load: Option<crate::game::MissionLoad>,
    /// Mission loads so far (each load's id, [`crate::game::MissionLoad::group`]).
    pub mission_loads: u32,
    /// `setControlCameraFov` (horizontal degrees), if set.
    pub camera_fov: Option<f32>,
    /// The game wants relative mouse motion (cursor hidden, `lockMouse(1)`).
    pub mouse_locked: bool,
    /// The next presented frame is saved under this game path (data dir),
    /// see [`crate::engine::Engine::save_screenshot`].
    pub screenshot_request: Option<String>,
    /// Show debug state (canvas stack) in the window title.
    pub debug: bool,
    /// The client's state (HUD model, player list, chat, the player's move).
    pub client: tt_gameplay::ClientState,
    /// The client's connection flow and settings ([`crate::gameplay::client`]).
    pub cl: crate::gameplay::client::ClientGlue,
    /// The native shell (tt-shell) behind the menu.
    pub shell: crate::shell::Shell,
    /// The game server (`tt_gameplay::server`, with its typed state) and the engine's view of it.
    pub srv: crate::gameplay::server_host::ServerGlue,
    /// Golden behaviour trace (`crate::gameplay::golden`): observable
    /// effects, recorded by the golden fixtures' harness.
    pub golden: Option<crate::gameplay::golden::Golden>,
    /// Stands in for the real-time clock (deterministic runs advance it
    /// with the sim).
    pub virtual_clock_ms: Option<f64>,
    /// Simulation time and the timed events ([`crate::sched`]).
    pub sched: crate::sched::Scheduler,
    /// The game logic's random numbers; the server draws from it.
    pub rng: tt_game::rng::EngineRng,
}

impl EngineHost {
    pub fn new(fs: GameFs, log: Logger, audio: Box<dyn Audio>) -> EngineHost {
        EngineHost {
            fs,
            log,
            platform: Platform::default(),
            video: Video::default(),
            canvas: Canvas::new(),
            audio,
            input: Input::default(),
            stats: Stats::default(),
            start: Instant::now(),
            quit_requested: false,
            reimport_requested: false,
            restart_requested: false,
            scene_dirty: false,
            net: crate::net::Net::default(),
            netgame: Default::default(),
            game: None,
            defs: Default::default(),
            data: Default::default(),
            weapons: Vec::new(),
            mission_load: None,
            mission_loads: 0,
            camera_fov: None,
            shell: crate::shell::Shell::new(),
            srv: crate::gameplay::server_host::ServerGlue::new(),
            golden: None,
            virtual_clock_ms: None,
            client: tt_gameplay::ClientState::default(),
            cl: Default::default(),
            mouse_locked: false,
            screenshot_request: None,
            debug: false,
            sched: Default::default(),
            rng: Default::default(),
        }
    }

    /// A deterministic run that must not touch the outside world: no ban
    /// file with wall-clock times, no LAN browser or master server query
    /// (the golden harness).
    pub fn offline(&self) -> bool {
        self.golden.is_some()
    }

    /// [`Self::now_ms`] with sub-millisecond precision ([`Self::virtual_clock_ms`] when set).
    pub fn now_ms_f64(&self) -> f64 {
        match self.virtual_clock_ms {
            Some(t) => t,
            None => self.start.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// The simulation time (ms).
    pub fn sim_time(&self) -> u32 {
        self.sched.now()
    }

    pub fn stub(&mut self, name: &str) {
        *self.stats.stubs.entry(name.to_string()).or_default() += 1;
    }
}
