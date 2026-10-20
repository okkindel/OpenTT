//! The Online screen: **Join** a server from the LAN list or by address, or
//! **Host** a listen server and play on it.

use egui::{Align2, Color32, Key, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::{bot_table, edit_text, frame, pal, pressed, Gfx, Typed};
use crate::backend::*;
use crate::{Field, MenuAction, MenuState, OnlineTab};

/// Server list sort keys as the header shows them (the first three in the
/// first column, whose rows show name, map and mode; the last: the lock
/// icons). The column widths follow the content ([`Gfx::columns`]).
const COLS: [&str; 6] = ["Server", "Map", "Mode", "Players", "Ping", ""];
/// Server list rows (two lines) and their text size.
const SERVER_ROW: f32 = 46.0;
const SERVER_TEXT: f32 = 23.0;

const MAX_ADDRESS: usize = 63;
const MAX_NAME: usize = 24;
const MAX_PASSWORD: usize = 16;

/// A padlock, `h` high.
fn lock_icon(g: &Gfx, center: Pos2, h: f32, color: Color32) {
    let body = Rect::from_center_size(center + Vec2::new(0.0, h * 0.18), Vec2::new(h * 0.62, h * 0.46));
    let shackle = Rect::from_center_size(center + Vec2::new(0.0, -h * 0.16), Vec2::new(h * 0.42, h * 0.5));
    g.painter.rect_stroke(shackle, h * 0.2, Stroke::new(h * 0.1, color), StrokeKind::Inside);
    g.painter.rect_filled(body, 1.5, color);
}

/// A text field; focused: accent outline and caret. `mask` shows `*`s.
fn field(g: &mut Gfx, id: &str, rect: Rect, text: &str, placeholder: &str, focused: bool, mask: bool) -> bool {
    let resp = g.interact_button(rect, ("field", id), true);
    let edge = if focused || resp.hovered() { pal::BRASS } else { super::steel::shade(pal::LINE, 0.1) };
    g.inset(rect, super::steel::shade(pal::WELL, -0.25), pal::WELL, edge, 4.0);
    let shown: String = if mask { "*".repeat(text.chars().count()) } else { text.to_string() };
    let pos = rect.left_center() + Vec2::new(12.0, 0.0);
    let end = if shown.is_empty() {
        if !focused {
            g.text(pos, placeholder, 22.0, pal::DISABLED, Align2::LEFT_CENTER);
        }
        Rect::from_min_size(pos, Vec2::ZERO)
    } else {
        g.text(pos, &shown, 24.0, pal::SAND, Align2::LEFT_CENTER)
    };
    if focused {
        let t = g.ctx().input(|i| i.time);
        if (t * 2.0) as i64 % 2 == 0 {
            let x = if shown.is_empty() { rect.left() + 13.0 } else { end.right() + 3.0 };
            g.painter.line_segment([Pos2::new(x, rect.top() + 8.0), Pos2::new(x, rect.bottom() - 8.0)], Stroke::new(2.0_f32, pal::BRASS));
        }
        g.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    resp.clicked()
}

/// Typing into the focused field. Returns true on Enter.
fn type_into(ctx: &egui::Context, st: &mut MenuState, host: Option<&mut HostGame>) -> bool {
    let Some(f) = st.field else { return false };
    let (text, max) = match (f, host) {
        (Field::Address, _) => (&mut st.address, MAX_ADDRESS),
        (Field::ServerName, Some(host)) => (&mut host.name, MAX_NAME),
        (Field::ServerPassword, Some(host)) => (&mut host.password, MAX_PASSWORD),
        // The host fields are not on the Join screen.
        (Field::ServerName | Field::ServerPassword, None) => return false,
        (Field::Master, _) => (st.master.get_or_insert_with(String::new), MAX_ADDRESS),
    };
    let k = edit_text(ctx, text, max);
    if k.escape || k.tab {
        st.field = None;
        st.esc_eaten = true;
    }
    k.enter
}

fn default_host(b: &dyn MenuBackend) -> HostGame {
    let missions = b.missions();
    let names = b.bot_names();
    let tanks = b.tanks().len().clamp(1, 3);
    HostGame {
        game: CustomGame {
            mission: missions.first().map(|m| m.file.clone()).unwrap_or_default(),
            mode: 0,
            bots: names.iter().take(3).enumerate().map(|(i, n)| BotSetup { name: n.clone(), skill: BotSkill::ALL[i % 3], tank: i % tanks, team: Team::Auto }).collect(),
            player_tank: b.selected_tank(),
            player_team: Team::Auto,
            minutes: 15,
            replace_bots: b.replace_bots(),
        },
        name: match b.current_player() {
            Some(p) => format!("{p}'s game"),
            None => format!("{} server", tt_assets::product::NAME),
        },
        max_players: 16,
        password: String::new(),
        port: 28000,
        port_mapping: false,
        public: true,
    }
}

/// A check box with a label after it; returns true when toggled.
fn tick_box(g: &mut Gfx, id: &str, rect: Rect, label: &str, value: &mut bool, enabled: bool) -> bool {
    let resp = g.interact_button(rect, ("tick", id), enabled);
    let b = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 12.0), Vec2::splat(24.0));
    g.check_box(b, *value && enabled, resp.hovered() && enabled);
    g.text_fit(Pos2::new(b.right() + 10.0, b.center().y), label, 20.0, if enabled { pal::SAND } else { pal::DISABLED }, rect.width() - 34.0);
    let clicked = resp.clicked() && enabled;
    if clicked {
        *value = !*value;
    }
    clicked
}

/// Team colours of the player list.
fn team_color(t: Team) -> Color32 {
    match t {
        Team::Green => Color32::from_rgb(0x5c, 0xc8, 0x46),
        Team::Blue => Color32::from_rgb(0x4a, 0x8c, 0xe0),
        Team::Auto => pal::SAND_MUTED,
    }
}

/// Starts a query of the current source (none for Internet without a master).
fn refresh(st: &MenuState, b: &mut dyn MenuBackend) {
    if st.server_source == ServerSource::Internet && b.master_server().trim().is_empty() {
        return;
    }
    b.refresh_servers();
}

/// Saves an edited master address (when it changed) and lists its servers.
fn commit_master(st: &mut MenuState, b: &mut dyn MenuBackend) {
    let Some(m) = st.master.as_ref().map(|m| m.trim().to_string()) else { return };
    if m != b.master_server() {
        b.set_master_server(&m);
        refresh(st, b);
    }
}

pub(super) fn online(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let tabs = [OnlineTab::Join, OnlineTab::Host];
    let cur = tabs.iter().position(|t| *t == st.online_tab).unwrap_or(0);
    // One panel under the Join / Host tabs holds the whole page.
    let outer = g.tab_panel_rect();
    let inner = g.panel(outer);
    if let Some(i) = g.subtabs("otab", &["Join", "Host"], cur, outer) {
        st.online_tab = tabs[i];
        st.field = None;
    }
    if let Some(a) = b.take_password_request() {
        st.password = Some((a, String::new()));
    }
    match st.online_tab {
        OnlineTab::Join => join(g, st, b, inner),
        OnlineTab::Host => host(g, st, b, inner),
    }
}

fn sorted(mut v: Vec<ServerEntry>, (col, asc): (usize, bool)) -> Vec<ServerEntry> {
    v.sort_by(|a, b| {
        let o = match col {
            0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            1 => a.map.cmp(&b.map),
            2 => a.mode.cmp(&b.mode),
            3 => a.players.cmp(&b.players),
            4 => a.ping_ms.cmp(&b.ping_ms),
            _ => a.password.cmp(&b.password),
        };
        if asc {
            o
        } else {
            o.reverse()
        }
    });
    v
}

/// Height of the top row of the Join page (source, master server; the
/// details' Info / Players switch) and the gap below it.
const TOP_ROW: f32 = 32.0;
const ROW_GAP: f32 = 12.0;
/// Width of the details column on the right of the Join page.
const DETAILS_W: f32 = 372.0;

/// An address as typed (without the `IP:` of the list's addresses).
fn plain_address(a: &str) -> &str {
    a.strip_prefix("IP:").unwrap_or(a)
}

fn join(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, inner: Rect) -> Option<MenuAction> {
    let ctx = g.ctx();
    if st.master.is_none() {
        st.master = Some(b.master_server());
    }
    if !st.queried {
        st.queried = true;
        b.set_server_source(st.server_source);
        refresh(st, b);
    }
    let was_field = st.field;
    let typed_enter = if g.blocked { false } else { type_into(&ctx, st, None) };
    // The master field: Enter, or leaving it, saves the address.
    if was_field == Some(Field::Master) && (st.field.is_none() || typed_enter) {
        st.field = None;
        commit_master(st, b);
    }
    // Two columns: the list (with the source above it and joining by
    // address below it) and the selected server's details.
    let left = Rect::from_min_max(inner.min, Pos2::new(inner.right() - DETAILS_W - 20.0, inner.bottom()));
    let right = Rect::from_min_max(Pos2::new(inner.right() - DETAILS_W, inner.top()), inner.max);
    let top_row = |r: Rect, x: f32, w: f32| Rect::from_min_size(Pos2::new(r.left() + x, r.top()), Vec2::new(w, TOP_ROW));
    // Source: LAN / Internet.
    let internet = st.server_source == ServerSource::Internet;
    if let Some(i) = g.segmented("source", top_row(left, 0.0, 240.0), &["LAN", "Internet"], internet as usize) {
        st.server_source = if i == 1 { ServerSource::Internet } else { ServerSource::Lan };
        st.join_sel = None;
        b.set_server_source(st.server_source);
        refresh(st, b);
    }
    let internet = st.server_source == ServerSource::Internet;
    let master = b.master_server();
    let no_master = internet && master.trim().is_empty();
    let servers = sorted(b.lan_servers(), st.sort);
    if st.join_sel.as_ref().is_some_and(|a| !servers.iter().any(|s| &s.address == a)) {
        st.join_sel = None;
    }
    if st.join_sel.is_none() {
        st.join_sel = servers.iter().find(|s| s.compatible).map(|s| s.address.clone());
    }
    let mut out = None;
    // Beside the source: what the list shows (LAN), or the master server
    // address (Internet); the search status at the right.
    let status = if b.querying() { "Searching..." } else { "" };
    let mid = left.top() + TOP_ROW / 2.0;
    g.text(Pos2::new(left.right(), mid), status, 20.0, pal::RUST, Align2::RIGHT_CENTER);
    let master_r = top_row(left, 380.0, 280.0);
    if internet {
        g.text(Pos2::new(left.left() + 260.0, mid), "Master server", 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        let focused = st.field == Some(Field::Master);
        let text = st.master.clone().unwrap_or_default();
        if field(g, "master", master_r, &text, "host:port (empty: none)", focused, false) {
            st.field = Some(Field::Master);
        }
        let hint = if focused { "Enter saves" } else if no_master { "not set" } else { "" };
        g.text(Pos2::new(master_r.right() + 12.0, mid), hint, 18.0, pal::DISABLED, Align2::LEFT_CENTER);
    } else {
        g.label(Pos2::new(left.left() + 260.0, mid), "Servers on your network", 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    }
    // The list.
    let addr_h = 40.0;
    let well = g.well(Rect::from_min_max(Pos2::new(left.left(), left.top() + TOP_ROW + ROW_GAP), Pos2::new(left.right(), left.bottom() - addr_h - ROW_GAP)));
    let head = Rect::from_min_size(well.min, Vec2::new(well.width(), 40.0));
    // Two lines a server: its name, then map and mode (all of it is in the
    // details too); players, ping and the lock in columns as wide as they
    // need, the name gets the rest.
    let name_of = |s: &ServerEntry| format!("{}{}\n{}  \u{b7}  {}", s.name, if s.modded { "  [modded]" } else { "" }, s.map, s.mode);
    let players_of = |s: &ServerEntry| format!("{}/{}", s.players, s.max_players);
    let xs = g.columns(
        well.width(),
        16.0,
        0,
        &[
            ("Server", servers.iter().map(name_of).collect()),
            ("Players", servers.iter().map(players_of).chain(["64/64".to_string()]).collect()),
            ("Ping", servers.iter().map(|s| s.ping_ms.to_string()).chain(["999".to_string()]).collect()),
            ("", vec!["   ".to_string()]),
        ],
        SERVER_TEXT,
    );
    // The header: the first column sorts by name, map or mode; the others by themselves.
    let mut sorts: Vec<(usize, f32, &str)> = Vec::new();
    let mut x = xs[0];
    for (i, h) in COLS[..3].iter().enumerate() {
        sorts.push((i, x, h));
        x += g.label_width(h, 20.0) + 48.0;
    }
    sorts.push((3, xs[1], COLS[3]));
    sorts.push((4, xs[2], COLS[4]));
    sorts.push((5, xs[3], COLS[5]));
    let body = g.header(well, &sorts.iter().map(|(_, x, h)| (*x, *h)).collect::<Vec<_>>());
    // Dots between Server, Map and Mode, like in the rows' second line.
    for s in &sorts[1..3] {
        let x = head.left() + s.1 - 22.0;
        g.text(Pos2::new(x, head.center().y), "\u{b7}", 20.0, pal::DISABLED, Align2::CENTER_CENTER);
    }
    // Header cells sort (again: reverse).
    for (k, (i, x, h)) in sorts.iter().enumerate() {
        let next = sorts.get(k + 1).map_or(head.width(), |n| n.1);
        let cell = Rect::from_min_max(Pos2::new(head.left() + x - 8.0, head.top()), Pos2::new(head.left() + next - 8.0, head.bottom()));
        let resp = g.interact_button(cell, ("sort", *i), true);
        if st.sort.0 == *i {
            // A small triangle after the header text: up = ascending.
            let x = cell.left() + 8.0 + g.label_width(h, 20.0) + 10.0;
            let c = Pos2::new(x, cell.center().y);
            let pts = if st.sort.1 { vec![c + Vec2::new(-5.0, 3.0), c + Vec2::new(5.0, 3.0), c + Vec2::new(0.0, -4.0)] } else { vec![c + Vec2::new(-5.0, -3.0), c + Vec2::new(5.0, -3.0), c + Vec2::new(0.0, 4.0)] };
            g.painter.add(egui::Shape::convex_polygon(pts, pal::RUST, Stroke::NONE));
        }
        if resp.hovered() {
            g.painter.rect_stroke(cell.shrink(2.0), 2.0, Stroke::new(1.0_f32, pal::LINE), StrokeKind::Inside);
        }
        if resp.clicked() {
            st.sort = if st.sort.0 == *i { (*i, !st.sort.1) } else { (*i, true) };
        }
    }
    let rows: Vec<Vec<(f32, String, Color32)>> = servers
        .iter()
        .map(|s| {
            let c = if s.compatible { pal::SAND } else { pal::DISABLED };
            let ping = if s.ping_ms < 80 { pal::BRASS } else if s.ping_ms < 200 { pal::SAND } else { pal::RED };
            vec![(xs[0], name_of(s), c), (xs[1], players_of(s), c), (xs[2], format!("{}", s.ping_ms), if s.compatible { ping } else { pal::DISABLED })]
        })
        .collect();
    let sel = st.join_sel.as_ref().and_then(|a| servers.iter().position(|s| &s.address == a));
    let (clicked, double) = g.table_sized("servers", body, &rows, sel, SERVER_ROW, SERVER_TEXT);
    // Lock icons in the last column (the table draws text only).
    let visible = ((body.height() + 1.0) / SERVER_ROW).floor() as usize;
    let off = g.table_offset("servers");
    for (k, s) in servers.iter().enumerate().skip(off).take(visible) {
        if s.password {
            let y = body.top() + (k - off) as f32 * SERVER_ROW + SERVER_ROW / 2.0;
            lock_icon(g, Pos2::new(body.left() + xs[3] + 10.0, y), 20.0, pal::RUST);
        }
    }
    if servers.is_empty() {
        let msg = if no_master {
            "No master server set: enter one above to list internet games."
        } else if b.querying() {
            if internet {
                "Asking the master server..."
            } else {
                "Looking for servers on your network..."
            }
        } else if internet {
            "No internet servers listed. Refresh, or join one by its address."
        } else {
            "No servers found. Refresh, or join one by its address."
        };
        g.text(body.center(), msg, 24.0, pal::DISABLED, Align2::CENTER_CENTER);
        if no_master {
            g.text(body.center() + Vec2::new(0.0, 34.0), "A master server lists games on the internet (run your own: tt-master).", 18.0, pal::DISABLED, Align2::CENTER_CENTER);
        }
    }
    if let Some(i) = clicked {
        st.join_sel = Some(servers[i].address.clone());
        st.field = None;
    }
    let selected = st.join_sel.as_ref().and_then(|a| servers.iter().find(|s| &s.address == a)).cloned();
    let join_selected = |st: &mut MenuState, s: &ServerEntry| -> Option<MenuAction> {
        if !s.compatible {
            return None;
        }
        if s.password {
            st.password = Some((s.address.clone(), String::new()));
            None
        } else {
            Some(MenuAction::Join { address: s.address.clone(), password: String::new(), team: if s.team_game() { st.join_team } else { Team::Auto } })
        }
    };
    if double {
        if let Some(s) = &selected {
            out = join_selected(st, s);
        }
    }
    // Join by address: one row under the list (field, Find, Join, the
    // last two addresses).
    let ay = left.bottom() - addr_h;
    g.label(Pos2::new(left.left(), ay + addr_h / 2.0), "Address", 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    let fr = Rect::from_min_size(Pos2::new(left.left() + 96.0, ay), Vec2::new(260.0, addr_h));
    let focused = st.field == Some(Field::Address);
    let addr = st.address.clone();
    if field(g, "addr", fr, &addr, "host or host:port", focused, false) {
        st.field = Some(Field::Address);
    }
    let find_r = Rect::from_min_size(Pos2::new(fr.right() + 10.0, ay + 1.0), Vec2::new(96.0, addr_h - 2.0));
    let go_r = Rect::from_min_size(Pos2::new(find_r.right() + 8.0, ay + 1.0), Vec2::new(96.0, addr_h - 2.0));
    let has_addr = !st.address.trim().is_empty();
    if g.plate_button("find", find_r, "Find", false, false, has_addr, 20.0).clicked() && has_addr {
        b.query_address(st.address.trim());
    }
    let go = g.plate_button("join-addr", go_r, "Join", false, false, has_addr, 20.0).clicked() || (typed_enter && focused);
    if go && has_addr {
        st.field = None;
        out = Some(MenuAction::Join { address: st.address.trim().to_string(), password: String::new(), team: st.join_team });
    }
    let recent = b.recent_addresses();
    let rx = go_r.right() + 16.0;
    for (i, r) in recent.iter().take(2).enumerate() {
        let rr = Rect::from_min_size(Pos2::new(rx, ay + i as f32 * addr_h / 2.0), Vec2::new(left.right() - rx, addr_h / 2.0));
        let resp = g.interact_button(rr, ("recent", i), true);
        g.text_fit(Pos2::new(rr.left(), rr.center().y), r, 18.0, if resp.hovered() { pal::BRASS } else { pal::SAND_MUTED }, rr.width());
        if resp.clicked() {
            st.address = r.clone();
            st.field = Some(Field::Address);
        }
    }
    // A click anywhere else leaves the text field.
    if st.field.is_some() && ctx.input(|i| i.pointer.any_click()) && !ctx.pointer_interact_pos().is_some_and(|p| fr.contains(p) || (internet && master_r.contains(p))) {
        if st.field == Some(Field::Master) {
            st.field = None;
            commit_master(st, b);
        }
        st.field = None;
    }
    // The divider between the columns.
    let dx = (left.right() + right.left()) / 2.0;
    g.painter.line_segment([Pos2::new(dx, inner.top()), Pos2::new(dx, inner.bottom())], Stroke::new(1.0_f32, Color32::from_black_alpha(120)));
    g.painter.line_segment([Pos2::new(dx + 1.0, inner.top()), Pos2::new(dx + 1.0, inner.bottom())], Stroke::new(1.0_f32, super::steel::shade(pal::PANEL, 0.14)));
    // Details.
    g.label(Pos2::new(right.left(), mid), "Server", 22.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    if let Some(s) = &selected {
        let n = s.player_list.len().max(s.player_names.len());
        let players = format!("Players {n}");
        if let Some(i) = g.segmented("details", top_row(right, right.width() - 256.0, 256.0), &["Info", &players], st.show_players as usize) {
            st.show_players = i == 1;
        }
    }
    let dwell = g.well(Rect::from_min_max(Pos2::new(right.left(), right.top() + TOP_ROW + ROW_GAP), right.max));
    let view = dwell.shrink2(Vec2::new(12.0, 10.0));
    let key = egui::Id::new(("tt-menu-details-h", st.show_players));
    let content_h: f32 = ctx.data(|d| d.get_temp(key)).unwrap_or(0.0);
    let off = g.scroll_offset(if st.show_players { "details-players" } else { "details-info" }, dwell, content_h + 20.0);
    let top = view.top() - off;
    let used = g.clipped(dwell.shrink(1.0), |g| match &selected {
        Some(s) if st.show_players => {
            // The info reply's player list: team colour, name, score; best first.
            let mut list: Vec<ServerPlayer> = if s.player_list.is_empty() { s.player_names.iter().map(|n| ServerPlayer::new("", n, 0)).collect() } else { s.player_list.clone() };
            list.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            let rh = 26.0;
            for (i, p) in list.iter().enumerate() {
                let y = top + i as f32 * rh + rh / 2.0;
                if p.team != Team::Auto {
                    g.painter.circle_filled(Pos2::new(view.left() + 6.0, y), 5.0, team_color(p.team));
                }
                g.text_fit(Pos2::new(view.left() + 20.0, y), &p.name, 20.0, pal::SAND, view.width() - 80.0);
                g.text(Pos2::new(view.right() - 6.0, y), &p.score.to_string(), 20.0, pal::SAND_MUTED, Align2::RIGHT_CENTER);
            }
            if list.is_empty() {
                g.text(view.min, "Nobody is playing", 22.0, pal::DISABLED, Align2::LEFT_TOP);
            }
            list.len() as f32 * rh
        }
        Some(s) => server_info(g, s, Rect::from_min_max(Pos2::new(view.left(), top), Pos2::new(view.right() - 6.0, top + 10_000.0))),
        None => {
            g.text(view.min, "No server selected", 24.0, pal::DISABLED, Align2::LEFT_TOP);
            0.0
        }
    });
    ctx.data_mut(|d| d.insert_temp(key, used));
    // Team choice for a team game (sent in the connect args).
    if selected.as_ref().is_some_and(|s| s.team_game()) {
        let r = g.action(460.0, 330.0);
        g.text(Pos2::new(r.left() - 12.0, r.center().y), "Team", 22.0, pal::SAND_MUTED, Align2::RIGHT_CENTER);
        super::team_selector(g, "jteam", r, &mut st.join_team);
    }
    // Actions.
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let can_refresh = !b.querying() && !no_master;
    if g.button("refresh", g.action(240.0, 200.0), "Refresh", false, false, can_refresh).clicked() && can_refresh {
        refresh(st, b);
    }
    let can_join = selected.as_ref().is_some_and(|s| s.compatible);
    let enter = !g.blocked && st.field.is_none() && pressed(&ctx, Key::Enter);
    if (g.button("join", g.action(0.0, 220.0), "Join", true, false, can_join).clicked() || enter) && can_join {
        if let Some(s) = &selected {
            out = join_selected(st, s);
        }
    }
    if back {
        st.back();
    }
    if g.blocked {
        return None;
    }
    out
}

/// The selected server's details from `area.min` down: its name (wrapped),
/// then a key / value list (long values wrap). Returns the height used.
fn server_info(g: &mut Gfx, s: &ServerEntry, area: Rect) -> f32 {
    let mut y = area.top();
    y += g.paragraph(Rect::from_min_max(Pos2::new(area.left(), y), area.max), &s.name, 26.0, pal::SAND) + 6.0;
    let key_w = 92.0;
    let mut line = |g: &mut Gfx, k: &str, v: &str, c: Color32| {
        g.text(Pos2::new(area.left(), y + 1.0), k, 18.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        y += g.paragraph(Rect::from_min_max(Pos2::new(area.left() + key_w, y), area.max), v, 20.0, c) + 2.0;
    };
    line(g, "Address", plain_address(&s.address), pal::SAND);
    line(g, "Map", &s.map, pal::SAND);
    line(g, "Mode", &s.mode, pal::SAND);
    let bots = match s.bots {
        0 => String::new(),
        1 => ", 1 bot".into(),
        n => format!(", {n} bots"),
    };
    line(g, "Players", &format!("{}/{}{bots}", s.players, s.max_players), pal::SAND);
    line(g, "Ping", &format!("{} ms", s.ping_ms), pal::SAND);
    line(g, "Host", if s.dedicated { "Dedicated server" } else { "Hosted by a player" }, pal::SAND);
    line(g, "Password", if s.password { "Needed" } else { "None" }, if s.password { pal::RUST } else { pal::SAND });
    let (v, c) = match (s.compatible, s.version.as_str()) {
        (true, _) => ("Same as yours".to_string(), pal::SAND),
        (false, "") => ("Other game version: cannot join".to_string(), pal::RED),
        (false, v) => (format!("{v}: cannot join"), pal::RED),
    };
    line(g, "Version", &v, c);
    if s.modded {
        let mods = if s.mods.is_empty() { "Modded (not listed)".to_string() } else { s.mods.join(", ") };
        line(g, "Mods", &mods, pal::RUST);
    } else {
        line(g, "Mods", "None", pal::SAND);
    }
    y - area.top()
}

fn host(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, inner: Rect) -> Option<MenuAction> {
    let ctx = g.ctx();
    let missions = b.missions();
    let modes = b.game_modes();
    let tanks = b.tanks();
    let bot_tanks: Vec<TankInfo> = tanks.iter().take(3).cloned().collect();
    let names = b.bot_names();
    let max_bots = b.max_bots();
    let mut hg = st.host.take().unwrap_or_else(|| b.host_settings().unwrap_or_else(|| default_host(b)));
    if !g.blocked {
        type_into(&ctx, st, Some(&mut hg));
    }
    // Three columns: maps, server options, bots; captions above them, a
    // note at the bottom.
    let (map_w, bots_w, gap) = (380.0, 380.0, 16.0);
    let cap_y = inner.top();
    let top = inner.top() + 32.0;
    let bottom = inner.bottom();
    // The map list leaves room below it for a note on how friends join.
    let note_h = 64.0;
    // Maps.
    g.caption(Pos2::new(inner.left(), cap_y), "Map");
    if let Some(i) = super::map_table(g, "hmaps", Rect::from_min_max(Pos2::new(inner.left(), top), Pos2::new(inner.left() + map_w, bottom - note_h - 8.0)), &missions, &hg.game.mission) {
        hg.game.mission = missions[i].file.clone();
    }
    // Server options (rows on the panel's steel).
    let opts = Rect::from_min_max(Pos2::new(inner.left() + map_w + gap, top), Pos2::new(inner.right() - bots_w - gap, bottom));
    g.caption(Pos2::new(opts.left(), cap_y), "Server");
    let rowh = opts.height() / 9.0;
    let ctl_h = 38.0;
    let mid = |i: usize| opts.top() + i as f32 * rowh + rowh / 2.0;
    let row = |i: usize| Rect::from_min_size(Pos2::new(opts.left() + 100.0, mid(i) - ctl_h / 2.0), Vec2::new(opts.width() - 100.0, ctl_h));
    let label = |g: &Gfx, i: usize, t: &str| {
        g.text(Pos2::new(opts.left(), mid(i)), t, 24.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    };
    label(g, 0, "Name");
    let name = hg.name.clone();
    let name_r = row(0);
    if field(g, "sname", name_r, &name, "server name", st.field == Some(Field::ServerName), false) {
        st.field = Some(Field::ServerName);
    }
    let mode_i = modes.iter().position(|m| m.index == hg.game.mode).unwrap_or(0);
    let teams = modes.get(mode_i).is_some_and(|m| m.teams);
    label(g, 1, "Mode");
    let d = g.selector("hmode", row(1), modes.get(mode_i).map(|m| m.label.as_str()).unwrap_or("-"), false);
    if d != 0 && !modes.is_empty() {
        hg.game.mode = modes[(mode_i as i32 + d).rem_euclid(modes.len() as i32) as usize].index;
    }
    label(g, 2, "Round");
    let d = g.selector("hminutes", row(2), &format!("{} min", hg.game.minutes), false);
    hg.game.minutes = (hg.game.minutes as i32 + d * if hg.game.minutes >= 10 { 5 } else { 1 }).clamp(1, 60) as u32;
    label(g, 3, "Players");
    let d = g.selector("hmax", row(3), &format!("{} max", hg.max_players), false);
    hg.max_players = (hg.max_players as i32 + d).clamp(2, 64) as u32;
    label(g, 4, "Password");
    let pw = hg.password.clone();
    let pw_r = row(4);
    if field(g, "spw", pw_r, &pw, "none", st.field == Some(Field::ServerPassword), false) {
        st.field = Some(Field::ServerPassword);
    }
    label(g, 5, "Port");
    let d = g.selector("hport", row(5), &format!("{}", hg.port), false);
    hg.port = (hg.port as i32 + d).clamp(1024, 65535) as u16;
    label(g, 6, "Tank");
    let tank_name = tanks.get(hg.game.player_tank).map(|t| t.name.clone()).unwrap_or_default();
    let d = g.selector("htank", row(6), &tank_name, false);
    if d != 0 && !tanks.is_empty() {
        let mut i = hg.game.player_tank as i32;
        for _ in 0..tanks.len() {
            i = (i + d).rem_euclid(tanks.len() as i32);
            if !tanks[i as usize].locked {
                break;
            }
        }
        hg.game.player_tank = i as usize;
    }
    label(g, 7, "Health");
    super::health_packs_selector(g, b, "hhealth", row(7));
    if teams {
        label(g, 8, "Team");
        super::team_selector(g, "hteam", row(8), &mut hg.game.player_team);
    }
    if st.field.is_some() && ctx.input(|i| i.pointer.any_click()) && !ctx.pointer_interact_pos().is_some_and(|p| name_r.contains(p) || pw_r.contains(p)) {
        st.field = None;
    }
    // Bots.
    let bots = Rect::from_min_max(Pos2::new(inner.right() - bots_w, top), Pos2::new(inner.right(), bottom));
    bot_table(g, &mut hg.game, teams, &bot_tanks, &names, max_bots, true, bots);
    g.paragraph(Rect::from_min_max(Pos2::new(inner.left(), bottom - note_h), Pos2::new(inner.left() + map_w, bottom)), "Friends join with your address and this port; over the internet, forward the UDP port to this computer (or let the router do it).", 18.0, pal::SAND_MUTED);
    // Internet options: the router port mapping and the master server listing.
    // (In the action row, between Back and Host.)
    tick_box(g, "portmap", g.action_left(230.0, 340.0), "Forward the port on the router", &mut hg.port_mapping, true);
    let has_master = !b.master_server().trim().is_empty();
    let label = if has_master { "List on the master server" } else { "List online (set a master on Join)" };
    tick_box(g, "public", g.action_left(590.0, 340.0), label, &mut hg.public, has_master);
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let start = g.button("host", g.action(0.0, 220.0), "Host", true, false, !hg.name.trim().is_empty()).clicked() && !hg.name.trim().is_empty();
    let out = (start && !g.blocked).then(|| MenuAction::Host(hg.clone()));
    st.host = Some(hg);
    if back {
        st.back();
    }
    out
}

/// The password prompt (`joinPasswordDlg`).
pub(super) fn password_dialog(ctx: &egui::Context, st: &mut MenuState) -> Option<MenuAction> {
    let (address, mut pw) = st.password.take()?;
    let Typed { enter: mut ok, escape: mut cancel, .. } = edit_text(ctx, &mut pw, MAX_PASSWORD);
    frame(ctx, st, egui::Order::Foreground, false, |g, _| {
        let (inner, y) = g.dialog(Vec2::new(660.0, 340.0), "Password");
        g.text(inner.min, "This server needs a password.", 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        g.text_fit(inner.min + Vec2::new(0.0, 28.0), &address, 20.0, pal::DISABLED, inner.width());
        let r = Rect::from_min_size(inner.min + Vec2::new(0.0, 64.0), Vec2::new(inner.width(), 50.0));
        field(g, "pw", r, &pw, "", true, true);
        let right = inner.right();
        cancel |= g.button("pw-cancel", Rect::from_min_size(Pos2::new(right - 330.0, y - 25.0), Vec2::new(170.0, 50.0)), "Cancel", false, false, true).clicked();
        ok |= g.button("pw-ok", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Join", true, false, true).clicked();
    });
    if cancel {
        return None;
    }
    if ok {
        return Some(MenuAction::Join { address, password: pw, team: st.join_team });
    }
    st.password = Some((address, pw));
    None
}
