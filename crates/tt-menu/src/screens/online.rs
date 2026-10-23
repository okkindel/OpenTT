//! The Online screen: **Join** a server from the LAN list or by address, or
//! **Host** a listen server and play on it.

use egui::{Align2, Color32, Key, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::selectors::team_selector;
use super::steel::Button;
use super::{edit_text, frame, pal, pressed, Gfx, Typed};
use crate::backend::*;
use crate::{Field, MastersEdit, MenuAction, MenuState, OnlineTab};

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
pub(super) fn lock_icon(g: &Gfx, center: Pos2, h: f32, color: Color32) {
    let body = Rect::from_center_size(center + Vec2::new(0.0, h * 0.18), Vec2::new(h * 0.62, h * 0.46));
    let shackle = Rect::from_center_size(center + Vec2::new(0.0, -h * 0.16), Vec2::new(h * 0.42, h * 0.5));
    g.painter.rect_stroke(shackle, h * 0.2, Stroke::new(h * 0.1, color), StrokeKind::Inside);
    g.painter.rect_filled(body, 1.5, color);
}

/// A text field; focused: accent outline and caret. `mask` shows `*`s.
pub(super) fn field(g: &mut Gfx, id: &str, rect: Rect, text: &str, placeholder: &str, focused: bool, mask: bool) -> bool {
    let resp = g.interact_button(rect, ("field", id), true);
    let edge = if focused || resp.hovered() { pal::BRASS } else { super::steel::shade(pal::LINE, 0.1) };
    g.inset(rect, super::steel::shade(pal::WELL, -0.25), pal::WELL, edge, 4.0);
    let mut shown: String = if mask { "*".repeat(text.chars().count()) } else { text.to_string() };
    // Too long: "..." where it is cut (the start while typing, so the caret shows).
    let max_w = rect.width() - 30.0;
    if g.text_width(&shown, 24.0) > max_w {
        while !shown.is_empty() && g.text_width(&format!("{shown}..."), 24.0) > max_w {
            if focused {
                shown.remove(0);
            } else {
                shown.pop();
            }
        }
        shown = if focused { format!("...{shown}") } else { format!("{shown}...") };
    }
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
pub(super) fn type_into(ctx: &egui::Context, st: &mut MenuState, host: Option<&mut HostGame>) -> bool {
    let Some(f) = st.field else { return false };
    let (text, max) = match (f, host) {
        (Field::Address, _) => (&mut st.address, MAX_ADDRESS),
        (Field::ServerName, Some(host)) => (&mut host.name, MAX_NAME),
        (Field::ServerPassword, Some(host)) => (&mut host.password, MAX_PASSWORD),
        // The host fields are not on the Join screen.
        (Field::ServerName | Field::ServerPassword, None) => return false,
    };
    let k = edit_text(ctx, text, max);
    if k.escape || k.tab {
        st.field = None;
        st.esc_eaten = true;
    }
    k.enter
}

pub(super) fn default_host(b: &dyn MenuBackend) -> HostGame {
    let missions = b.missions();
    let names = b.bot_names();
    let tanks = b.tanks().len().clamp(1, 3);
    let mission = missions.first().map(|m| m.file.clone()).unwrap_or_default();
    HostGame {
        rotation: vec![mission.clone()],
        game: CustomGame {
            mission,
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
        port_mapping: true,
        public: true,
        rotation_random: false,
    }
}

/// A check box with a label after it; returns true when toggled.
pub(super) fn tick_box(g: &mut Gfx, id: &str, rect: Rect, label: &str, value: &mut bool, enabled: bool) -> bool {
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

/// The master servers of the pref (comma-separated), one a row.
fn master_rows(pref: &str) -> Vec<String> {
    pref.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// A light check of a master address: `host[:port]`, `IP:a.b.c.d:port`
/// or `[v6]:port`, no spaces or commas.
fn valid_master(s: &str) -> bool {
    let s = s.strip_prefix("IP:").or_else(|| s.strip_prefix("ip:")).unwrap_or(s);
    if s.is_empty() || s.contains(|c: char| c.is_whitespace() || c == ',') {
        return false;
    }
    if s.starts_with('[') {
        return s.parse::<std::net::SocketAddr>().is_ok();
    }
    match s.split_once(':') {
        Some((host, port)) => !host.is_empty() && port.parse::<u16>().is_ok(),
        None => true,
    }
}

/// Rows the master servers dialog shows at most.
const MAX_MASTER_ROWS: usize = 6;

/// The master servers dialog: one address a row (× removes it), "+ Add",
/// Save (Enter) stores them comma-separated and lists their servers,
/// Cancel (Escape) keeps the old ones.
pub(super) fn masters_dialog(ctx: &egui::Context, st: &mut MenuState, b: &mut dyn MenuBackend) {
    let Some(mut m) = st.masters.take() else { return };
    m.focus = m.focus.min(m.rows.len().saturating_sub(1));
    let Typed { enter: mut save, escape: mut cancel, tab } = match m.rows.get_mut(m.focus) {
        Some(row) => edit_text(ctx, row, MAX_ADDRESS),
        None => Typed { escape: pressed(ctx, Key::Escape), enter: pressed(ctx, Key::Enter), ..Default::default() },
    };
    if tab && !m.rows.is_empty() {
        m.focus = (m.focus + 1) % m.rows.len();
    }
    let (mut add, mut remove) = (false, None);
    frame(ctx, st, egui::Order::Foreground, false, |g, _| {
        let row_h = 56.0;
        let (inner, y) = g.dialog(Vec2::new(720.0, 250.0 + row_h * MAX_MASTER_ROWS as f32), "Master servers");
        g.text(inner.min, "Each lists internet games; servers you host register with all of them.", 20.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        for (i, row) in m.rows.iter().enumerate() {
            let top = inner.top() + 40.0 + i as f32 * row_h;
            let r = Rect::from_min_size(Pos2::new(inner.left(), top), Vec2::new(inner.width() - 62.0, row_h - 8.0));
            if field(g, &format!("master{i}"), r, row, "host or host:port", i == m.focus, false) {
                m.focus = i;
            }
            let x = Rect::from_min_size(Pos2::new(r.right() + 14.0, top), Vec2::splat(row_h - 8.0));
            let resp = g.button(("master-x", i), x, "", false, false, true);
            if resp.clicked() {
                remove = Some(i);
            }
            let c = x.center();
            let color = if resp.hovered() { pal::BRASS } else { pal::SAND_MUTED };
            for d in [Vec2::new(8.0, 8.0), Vec2::new(8.0, -8.0)] {
                g.painter.line_segment([c - d, c + d], Stroke::new(3.0_f32, color));
            }
        }
        let add_top = inner.top() + 40.0 + m.rows.len() as f32 * row_h;
        add = m.rows.len() < MAX_MASTER_ROWS && g.button("master-add", Rect::from_min_size(Pos2::new(inner.left(), add_top), Vec2::new(160.0, 44.0)), "+ Add", false, false, true).clicked();
        if m.invalid {
            g.text_fit(Pos2::new(inner.left() + 180.0, add_top + 22.0), "Use host or host:port (port 28100 if left out)", 20.0, pal::RED, inner.width() - 180.0);
        }
        let right = inner.right();
        cancel |= g.button("master-cancel", Rect::from_min_size(Pos2::new(right - 330.0, y - 25.0), Vec2::new(170.0, 50.0)), "Cancel", false, false, true).clicked();
        save |= g.button("master-save", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Save", true, false, true).clicked();
    });
    if cancel {
        return;
    }
    if let Some(i) = remove {
        m.rows.remove(i);
        m.invalid = false;
    } else if add {
        m.rows.push(String::new());
        m.focus = m.rows.len() - 1;
    } else if save {
        let rows: Vec<&str> = m.rows.iter().map(|r| r.trim()).filter(|r| !r.is_empty()).collect();
        m.invalid = !rows.iter().all(|r| valid_master(r));
        if !m.invalid {
            let pref = rows.join(", ");
            if pref != b.master_server() {
                b.set_master_server(&pref);
                refresh(st, b);
            }
            return;
        }
    }
    st.masters = Some(m);
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
        OnlineTab::Host => super::host::host(g, st, b, inner),
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
    if !st.queried {
        st.queried = true;
        b.set_server_source(st.server_source);
        refresh(st, b);
    }
    let typed_enter = if g.blocked { false } else { type_into(&ctx, st, None) };
    // Two columns: the list (with the source above it and joining by
    // address below it) and the selected server's details.
    let left = Rect::from_min_max(inner.min, Pos2::new(inner.right() - DETAILS_W - 20.0, inner.bottom()));
    let right = Rect::from_min_max(Pos2::new(inner.right() - DETAILS_W, inner.top()), inner.max);
    let top_row = |r: Rect, x: f32, w: f32| Rect::from_min_size(Pos2::new(r.left() + x, r.top()), Vec2::new(w, TOP_ROW));
    // Source: LAN / Internet.
    let internet = st.server_source == ServerSource::Internet;
    if let Some(i) = g.segmented("source", top_row(left, 0.0, 240.0), &["Internet", "LAN"], !internet as usize) {
        st.server_source = if i == 0 { ServerSource::Internet } else { ServerSource::Lan };
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
    // Beside the source: what the list shows (LAN), or the master field's
    // hint (Internet); the search status at the right.
    let status = if b.querying() { "Searching..." } else { "" };
    let mid = left.top() + TOP_ROW / 2.0;
    g.text(Pos2::new(left.right(), mid), status, 20.0, pal::RUST, Align2::RIGHT_CENTER);
    // The master servers: a plate in the tab row, right of the Join / Host
    // tabs, above the details; it opens the master servers dialog.
    if internet {
        let tab_row = g.tab_panel_rect().top();
        let plate = Rect::from_min_max(Pos2::new(right.left() - 100.0, tab_row - 36.0), Pos2::new(inner.right(), tab_row - 4.0));
        let list = master_rows(&master);
        let text = match list.len() {
            0 => "Master servers: none".to_string(),
            1 => format!("Master servers: {}", list[0]),
            n => format!("Master servers: {} (+{})", list[0], n - 1),
        };
        if g.button("masters", plate, "", false, false, true).clicked() {
            st.masters = Some(MastersEdit { rows: if list.is_empty() { vec![String::new()] } else { list }, ..Default::default() });
        }
        g.text_fit(Pos2::new(plate.left() + 14.0, plate.center().y), &text, 20.0, pal::SAND, plate.width() - 28.0);
        let hint = if no_master { "No master server set" } else { "Games listed by the master servers" };
        let hint_w = left.right() - g.text_width("Searching...", 20.0) - 20.0 - (left.left() + 260.0);
        g.text_fit(Pos2::new(left.left() + 260.0, mid), hint, 18.0, pal::DISABLED, hint_w);
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
    // Players / slots, and the human players among them when bots play.
    let players_of = |s: &ServerEntry| match s.bots {
        0 => format!("{}/{}", s.players, s.max_players),
        b => format!("{}/{} ({})", s.players, s.max_players, s.players.saturating_sub(b)),
    };
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
            "No master server set: add one at the top right to list internet games."
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
    if g.plate_button("find", find_r, "Find", Button::enabled(has_addr), 20.0).clicked() && has_addr {
        b.query_address(st.address.trim());
    }
    let go = g.plate_button("join-addr", go_r, "Join", Button::enabled(has_addr), 20.0).clicked() || (typed_enter && focused);
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
    if st.field.is_some() && ctx.input(|i| i.pointer.any_click()) && !ctx.pointer_interact_pos().is_some_and(|p| fr.contains(p)) {
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
        // The Internet/LAN switch's size; with its label centred between Back and Refresh.
        let (back, refresh) = (g.action_left(0.0, 200.0), g.action(240.0, 200.0));
        let (w, label_w) = (300.0, g.text_width("Team", 22.0) + 12.0);
        let left = (back.right() + refresh.left() - label_w - w) / 2.0 + label_w;
        let r = Rect::from_min_size(Pos2::new(left, back.center().y - TOP_ROW / 2.0), Vec2::new(w, TOP_ROW));
        g.text(Pos2::new(r.left() - 12.0, r.center().y), "Team", 22.0, pal::SAND_MUTED, Align2::RIGHT_CENTER);
        team_selector(g, "jteam", r, &mut st.join_team);
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
        (true, "") => ("Same as yours".to_string(), pal::SAND),
        (true, v) => (v.to_string(), pal::SAND),
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
