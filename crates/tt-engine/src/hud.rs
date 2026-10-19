//! The native HUD's content ([`tt_menu::hud::HudView`]) for a frame, from the
//! client state ([`tt_gameplay::ClientState`], `EngineHost::client`: message
//! lines, prints, the round clock, the solo widgets, the player list, the
//! chat input, the lag icon) and the simulation (radar blips, name labels
//! over tanks). Also the engine's hooks of the client state: its timers and
//! fades ([`client_tick`], [`client_animate`]) and the chat input's keys
//! ([`chat_key`]).

use crate::engine::Engine;
use crate::host::EngineHost;
use glam::{Vec3, Vec4};
use tt_gameplay::client::{text::clean, ChatKey, TeamIcon};
use tt_gui::hud::{shape_name_height, shape_name_opacity, RadarBlip, RadarParams};
use tt_menu::hud::{Blip, ChatInput, ChatLine, HudView, ListRow, Lives, NameTag, PlayerList, Radar, TeamDot};

type Color32 = egui::Color32;

/// Team colours of the radar blips (team index from `setTeam`).
pub fn team_color(team: i32) -> Color32 {
    match team {
        1 => Color32::from_rgb(0, 255, 0),
        2 => Color32::from_rgb(0, 128, 255),
        3 => Color32::from_rgb(192, 0, 255),
        _ => Color32::from_rgb(255, 0, 0),
    }
}

/// Team colours of names (labels over tanks, message lines): the menu's.
pub fn name_color(team: i32) -> Color32 {
    match team {
        1 => tt_menu::theme::TEAM_GREEN,
        2 => Color32::from_rgb(0x5c, 0x9a, 0xff),
        3 => Color32::from_rgb(0xc8, 0x6c, 0xff),
        _ => Color32::from_rgb(0xff, 0x5a, 0x48),
    }
}

/// `RadarHud` of `PlayGui.gui`: radius, exponent, inner circle, dot offset and size.
fn radar_params() -> RadarParams {
    let f = |s: &str| tt_game::fmt::atof(s) as f32;
    RadarParams {
        radius: f("260"),
        exponent: f("0.67"),
        inner_circle: f("0.8"),
        dot_offset: tt_gui::geom::Point::new(-2, -1),
        dot_size: tt_gui::geom::Point::new(8, 8),
    }
}

/// `GuiShapeNameHud` of `PlayGui.gui`: (min, max vertical offset, distance fade).
fn name_hud_params() -> (f32, f32, f32) {
    let f = |s: &str| tt_game::fmt::atof(s) as f32;
    (f("1.0"), f("8.0"), f("0.2"))
}

fn team_dot(t: TeamIcon) -> TeamDot {
    match t {
        TeamIcon::Red => TeamDot::Red,
        TeamIcon::Green => TeamDot::Green,
        TeamIcon::Blue => TeamDot::Blue,
    }
}

/// The client's timers due by the simulation clock (after the engine's
/// timed events of the tick).
pub fn client_tick(h: &mut EngineHost) {
    h.client.advance(h.sim_time() as u64);
}

/// One GUI tick of the message fades (with the sim, or in real time).
pub fn client_animate(h: &mut EngineHost, ms: u32) {
    h.client.animate(ms);
}

/// A key while the chat input is open (what `MessageHud_Edit` did as the
/// first responder): typing, Backspace, Return sends, Escape closes.
/// Returns true if the input took the key.
pub fn chat_key(h: &mut EngineHost, key: &str, pressed: bool, typed: Option<&str>) -> bool {
    use crate::gameplay::client::hud::{close_chat, send_chat};
    match h.client.chat_key(key, pressed, typed) {
        ChatKey::Pass => false,
        ChatKey::Used => true,
        ChatKey::Send(text) => {
            send_chat(h, &text);
            true
        }
        ChatKey::Close => {
            close_chat(h);
            true
        }
    }
}

impl Engine {
    /// The HUD for a frame: `None` unless the game (`PlayGui`) is on the
    /// canvas. `target` is the window size in pixels (the 3D view fills
    /// it), `time_ms` real time for blinking, `camera` the game view's
    /// camera and horizontal field of view (name labels). Also applies the
    /// clock's `showTime` visibility and plays its tick (`AudioTickToc`) on
    /// each new second, as `GuiHudClock` did while drawn.
    pub fn hud_view(&mut self, target: (u32, u32), time_ms: u64, camera: Option<(&tt_render::Camera, f32)>) -> Option<HudView> {
        use crate::canvas::Ctrl;
        if self.host.canvas.content() != Some(Ctrl::PlayGui) {
            return None;
        }
        let (chat_on, list_on) = (self.host.canvas.is_awake(Ctrl::MessageHud), self.host.canvas.is_awake(Ctrl::PlayerListGui));
        let mut v = HudView { time_ms, ..Default::default() };
        let now = self.host.sim_time() as u64;
        if let Some((secs, tick)) = self.host.client.clock.sample(now, tt_gui::hud::hud_clock_visible) {
            if tick {
                self.host.audio.play_profile(tt_gameplay::client::hud::CLOCK_TICK_SOUND, None);
            }
            use tt_gameplay::client::hud::{CLOCK_SHOW_HOURS, CLOCK_SHOW_MINUTES};
            v.clock = Some(tt_gui::hud::hud_clock_text(secs, CLOCK_SHOW_MINUTES, CLOCK_SHOW_HOURS));
        }
        let c = &self.host.client;
        if c.messages.visible {
            v.chat = c.messages.lines.iter().map(|l| ChatLine { text: clean(&l.text), chat: l.chat, alpha: l.alpha(), slot: l.slot() }).collect();
        }
        if c.chat.open && chat_on {
            v.chat_input = Some(ChatInput { team: c.chat.team, text: c.chat.text.clone() });
        }
        v.center_print = c.center.active.then(|| clean(&c.center.text));
        v.bottom_print = c.bottom.active.then(|| clean(&c.bottom.text));
        if c.solo.score_visible {
            v.score = c.solo.score_text();
        }
        if c.solo.lives_visible {
            let icons = c.solo.life_icons.iter().filter(|&&on| on).count() as u32;
            v.lives = Some(Lives { icons, count: c.solo.lives_count.0.then(|| c.solo.lives_count.1.clone()) });
        }
        if c.solo.brains_visible {
            v.brains = Some(clean(&c.solo.brains_text));
        }
        v.lag = c.lag;
        let player_obj = self.control_object();
        v.radar = self.radar(player_obj, time_ms);
        if let Some((cam, hfov)) = camera {
            v.names = self.shape_names(player_obj, cam, hfov, target);
        }
        if list_on {
            v.player_list = Some(self.player_list());
        }
        // Names in message lines: team colours in team games, the local
        // player in green otherwise.
        if let Some(list) = &v.player_list {
            let teams = !list.teams.is_empty();
            for r in &list.players {
                let name = r.name.split(" [").next().unwrap_or("").trim().to_string();
                let c = if teams { r.team.color() } else if r.me { tt_menu::theme::TEAM_GREEN } else { tt_menu::theme::GOLD };
                if !name.is_empty() {
                    v.name_colors.push((name, c));
                }
            }
        }
        Some(v)
    }

    fn player_list(&self) -> PlayerList {
        let list = &self.host.client.players;
        let me = self.host.shell.prefs.get("player.name");
        let (players, teams) = list.rows();
        let row = |r: tt_gameplay::client::RowView, me: &str| {
            let is_me = !me.is_empty() && r.name.split(" [").next().is_some_and(|n| n.trim() == me);
            ListRow { team: team_dot(r.team), name: r.name, score: r.score, kda: r.kda, ping: r.ping, me: is_me }
        };
        PlayerList { teams: teams.into_iter().map(|r| row(r, "")).collect(), players: players.into_iter().map(|r| row(r, &me)).collect(), kda: true, ping: list.ping_column }
    }

    fn radar(&self, player_obj: Option<tt_gameplay::EntityId>, time_ms: u64) -> Option<Radar> {
        let g = self.host.game.as_ref()?;
        let me = g.world.tank(g.tank_of(player_obj?)?)?;
        let f = me.rot.y_axis;
        let heading_deg = f.x.atan2(f.y).to_degrees();
        let rel = |p: Vec3| (p.x - me.pos.x, p.y - me.pos.y);
        let mut blips = Vec::new();
        for t in &g.world.tanks {
            if t.id == me.id || t.deleted || t.is_corpse {
                continue;
            }
            let (dx, dy) = rel(t.pos);
            blips.push(RadarBlip::tank(dx, dy, tt_gui::geom::Color(team_color(t.team).to_array())));
        }
        for fl in &g.world.flags {
            let (dx, dy) = rel(fl.pos);
            blips.push(RadarBlip::flag(dx, dy));
        }
        let carrying = me.carried_flag.is_some();
        for tr in &g.world.triggers {
            let tt_game::TriggerKind::PowerUp(d) = &tr.kind else { continue };
            let (dx, dy) = rel(tr.transform.translation.into());
            let ty = d.kind.to_ascii_lowercase();
            if ty.starts_with('g') {
                blips.push(RadarBlip::goal(dx, dy, carrying));
            } else if ty == "ammo" {
                blips.push(RadarBlip::ammo(dx, dy, g.ammo_warning));
            } else if ty == "health" && tr.on && me.damage >= (me.data.damage_amount * 0.65) as i32 as f32 {
                // ASSUMPTION: "damage high" uses the health-hint threshold (damage_amount * 0.65).
                blips.push(RadarBlip::health(dx, dy));
            }
        }
        let blink_on = (time_ms / 300) & 1 != 0;
        let blips = blips.into_iter().map(|b| Blip { dx: b.dx, dy: b.dy, color: Color32::from_rgba_unmultiplied(b.color.0[0], b.color.0[1], b.color.0[2], b.color.0[3]), big: b.big, on: !b.blink || blink_on }).collect();
        Some(Radar { heading_deg, params: radar_params(), blips })
    }

    /// `GuiShapeNameHud` labels in window pixels.
    fn shape_names(&self, player_obj: Option<tt_gameplay::EntityId>, camera: &tt_render::Camera, hfov: f32, target: (u32, u32)) -> Vec<NameTag> {
        let Some(g) = self.host.game.as_ref() else { return Vec::new() };
        let me = player_obj.and_then(|o| g.tank_of(o));
        let sky = self.host.mission_load.as_ref().and_then(|l| l.nodes.iter().map(|(_, o)| o).find(|o| o.class.eq_ignore_ascii_case("Sky")));
        let visible = sky.and_then(|s| s.get("visible_distance")).filter(|v| !v.is_empty()).map(|v| tt_game::fmt::atof(v) as f32).unwrap_or(500.0);
        let (min_off, max_off, fade) = name_hud_params();
        let aspect = target.0.max(1) as f32 / target.1.max(1) as f32;
        let vp = camera.view_proj_fov(aspect, visible.max(10.0), tt_render::vertical_fov(hfov, aspect));
        let mut out = Vec::new();
        for t in &g.world.tanks {
            if Some(t.id) == me || t.deleted || t.is_corpse || t.name.is_empty() {
                continue;
            }
            let dist = (t.pos - camera.pos).length();
            if dist > visible {
                continue;
            }
            let b = t.data.geometry.bounds;
            let top = t.pos + Vec3::Z * shape_name_height(b.max.z - b.min.z, dist, visible, min_off, max_off);
            // Line of sight from the camera, over the terrain only.
            if g.world.terrain.as_ref().is_some_and(|h| h.segment_hit(camera.pos, top).is_some()) {
                continue;
            }
            let c = vp * Vec4::new(top.x, top.y, top.z, 1.0);
            if c.w <= 0.0 {
                continue;
            }
            let (x, y) = (c.x / c.w, c.y / c.w);
            if x.abs() > 1.2 || y.abs() > 1.2 {
                continue;
            }
            let pos = egui::Pos2::new((x + 1.0) * 0.5 * target.0 as f32, (1.0 - y) * 0.5 * target.1 as f32);
            out.push(NameTag { pos, name: clean(&t.name), color: name_color(t.team), opacity: shape_name_opacity(dist, visible, fade), dist });
        }
        out
    }
}
