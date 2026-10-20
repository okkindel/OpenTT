//! Saves of earlier versions, read once by the migration (and written by
//! tests that need such files). Everything about the old formats is here:
//!
//! * Integers and floats bare, strings quoted with escapes ([`parse`],
//!   [`export`]).
//! * The names the values had there ([`KEYS`], [`key`], [`old_name`]): the
//!   settings, the player list, the high scores and the level-select
//!   progress.
//! * The old ban list files.
//!
//! Nothing else in the workspace knows the old names: the game keys its
//! settings `section.field` ([`crate::settings`]).

use crate::banlist::{ip_of, Ban, BanList};
use crate::vars::{collapse_escape, expand_escape, VarFile, VarValue};
use tt_game::fmt::format_g;

/// The old saves, relative to the data directory.
pub const LEGACY_PLAYERS: &str = "game/client/players.cs";
pub const LEGACY_HISCORES: &str = "game/client/hiscores.cs";
pub const LEGACY_PROGRESS: &str = "game/client/progress.cs";
pub const LEGACY_CLIENT_PREFS: &str = "game/client/prefs.cs";
pub const LEGACY_SERVER_PREFS: &str = "game/server/prefs.cs";
pub const LEGACY_CONFIG: &str = "game/client/config.cs";
pub const LEGACY_BANLISTS: [&str; 2] = ["game/server/banlist.cs", "common/server/banlist.cs"];

/// Old name -> settings key, for every setting whose key is not the plain
/// rule of [`key`] or that the game reads.
pub const KEYS: &[(&str, &str)] = &[
    ("$pref::Audio::channelVolume2", "audio.sound_volume"),
    ("$pref::Audio::channelVolume4", "audio.music_volume"),
    ("$pref::Audio::driver", "audio.driver"),
    ("$pref::Audio::environmentEnabled", "audio.environment_enabled"),
    ("$pref::Audio::forceMaxDistanceUpdate", "audio.force_max_distance_update"),
    ("$pref::Audio::masterVolume", "audio.master_volume"),
    ("$pref::ChatHudLength", "hud.chat_length"),
    ("$pref::Client::parentalLock", "player.parental_lock"),
    ("$pref::Demo::FinishedTutorial", "player.finished_tutorial"),
    ("$pref::Demo::neverShowMarquee", "demo.never_show_marquee"),
    ("$pref::Graphics::aoStrength", "graphics.ao_strength"),
    ("$pref::Graphics::bloom", "graphics.bloom"),
    ("$pref::Graphics::dynamicLights", "graphics.dynamic_lights"),
    ("$pref::Graphics::dynamicLightStrength", "graphics.dynamic_light_strength"),
    ("$pref::Graphics::glassJar", "graphics.glass_jar"),
    ("$pref::Graphics::hdr", "graphics.hdr"),
    ("$pref::Graphics::normalMaps", "graphics.normal_maps"),
    ("$pref::Graphics::normalStrength", "graphics.normal_strength"),
    ("$pref::Graphics::shadows", "graphics.shadows"),
    ("$pref::Graphics::showFps", "graphics.show_fps"),
    ("$pref::Graphics::ssao", "graphics.ssao"),
    ("$pref::Host::Minutes", "host.minutes"),
    ("$pref::Host::Mission", "host.mission"),
    ("$pref::Host::Mode", "host.mode"),
    ("$pref::Hud::scale", "graphics.hud_scale"),
    ("$pref::HudMessageLogSize", "hud.message_log_size"),
    ("$pref::Input::aimMode", "input.aim_mode"),
    ("$pref::Input::invertMouse", "input.invert_mouse"),
    ("$pref::Input::JoystickEnabled", "input.joystick_enabled"),
    ("$pref::Input::KeyboardEnabled", "input.keyboard_enabled"),
    ("$pref::Input::KeyboardTurnSpeed", "input.keyboard_turn_speed"),
    ("$pref::Input::LinkMouseSensitivity", "input.link_mouse_sensitivity"),
    ("$pref::Input::MouseEnabled", "input.mouse_enabled"),
    ("$pref::Input::mouseScale", "input.mouse_scale"),
    ("$pref::Input::mouseSensitivity", "input.mouse_sensitivity"),
    ("$pref::Input::reverseSteering", "input.reverse_steering"),
    ("$pref::Interior::lockArrays", "interior.lock_arrays"),
    ("$pref::isDemo", "player.is_demo"),
    ("$pref::Master0", "net.master0"),
    ("$Pref::Net::LagThreshold", "net.lag_threshold"),
    ("$pref::Net::MasterServer", "net.master_server"),
    ("$pref::Net::PortMapping", "net.port_mapping"),
    ("$pref::Net::RecentServers", "net.recent_servers"),
    ("$pref::OpenGL::anisotropy", "video.anisotropy"),
    ("$pref::OpenGL::disableARBTextureCompression", "opengl.disable_arb_texture_compression"),
    ("$pref::OpenGL::disableEXTCompiledVertexArray", "opengl.disable_ext_compiled_vertex_array"),
    ("$pref::OpenGL::disableSubImage", "opengl.disable_sub_image"),
    ("$pref::OpenGL::force16BitTexture", "opengl.force_16bit_texture"),
    ("$pref::OpenGL::forcePalettedTexture", "opengl.force_paletted_texture"),
    ("$pref::OpenGL::gammaCorrection", "opengl.gamma_correction"),
    ("$pref::OpenGL::maxHardwareLights", "opengl.max_hardware_lights"),
    ("$pref::OpenGL::maxTextureDim", "opengl.max_texture_dim"),
    ("$pref::OpenGL::mipReduction", "opengl.mip_reduction"),
    ("$pref::OpenGL::noDrawArraysAlpha", "opengl.no_draw_arrays_alpha"),
    ("$pref::OpenGL::noEnvColor", "opengl.no_env_color"),
    ("$pref::OpenGL::skyMipReduction", "opengl.sky_mip_reduction"),
    ("$pref::Particle::detailAdjust", "particle.detail_adjust"),
    ("$pref::playCount", "player.play_count"),
    ("$pref::player::lastServerMessage", "player.last_server_message"),
    ("$pref::Player::Name", "player.name"),
    ("$pref::Player::tankSelection", "player.tank"),
    ("$pref::Player::team", "player.team"),
    ("$pref::Player::unlockBabyBoss", "player.unlock_baby_boss"),
    ("$pref::sceneLighting::cacheLighting", "lighting.cache_lighting"),
    ("$pref::sceneLighting::cacheSize", "lighting.cache_size"),
    ("$pref::sceneLighting::purgeMethod", "lighting.purge_method"),
    ("$pref::sceneLighting::terrainGenerateLevel", "lighting.terrain_generate_level"),
    ("$Pref::Server::AdminPassword", "server.admin_password"),
    ("$Pref::Server::BanTime", "server.ban_time"),
    ("$Pref::Server::ConnectionError", "server.connection_error"),
    ("$Pref::Server::DisplayOnMaster", "server.display_on_master"),
    ("$Pref::Server::FloodProtectionEnabled", "server.flood_protection_enabled"),
    ("$Pref::Server::gameTypeIndex", "server.game_type_index"),
    ("$Pref::Server::KickBanTime", "server.kick_ban_time"),
    ("$Pref::Server::LagCompensation", "server.lag_compensation"),
    ("$pref::Server::MaxBots", "server.max_bots"),
    ("$Pref::Server::MaxChatLen", "server.max_chat_len"),
    ("$Pref::Server::MaxPlayers", "server.max_players"),
    ("$Pref::Server::MinHealthPacks", "server.min_health_packs"),
    ("$Pref::Server::missionIndex", "server.mission_index"),
    ("$pref::Server::missionWorld", "server.mission_world"),
    ("$Pref::Server::Name", "server.name"),
    ("$Pref::Server::Password", "server.password"),
    ("$Pref::Server::PasswordEnabled", "server.password_enabled"),
    ("$Pref::Server::Port", "server.port"),
    ("$Pref::Server::RegionMask", "server.region_mask"),
    ("$pref::Server::ReplaceBots", "server.replace_bots"),
    ("$pref::shell::fadeTime", "shell.fade_time"),
    ("$pref::shell::transDelay", "shell.trans_delay"),
    ("$pref::Tank::autoTurn", "input.auto_turn"),
    ("$pref::Terrain::dynamicLights", "terrain.dynamic_lights"),
    ("$pref::Terrain::screenError", "terrain.screen_error"),
    ("$pref::Terrain::texDetail", "terrain.tex_detail"),
    ("$pref::Terrain::useSmall", "terrain.use_small"),
    ("$pref::TS::detailAdjust", "shapes.detail_adjust"),
    ("$pref::TS::fogTexture", "shapes.fog_texture"),
    ("$pref::TS::ShadowDetail", "shapes.shadow_detail"),
    ("$pref::TS::skipFirstFog", "shapes.skip_first_fog"),
    ("$pref::TS::skipRenderDLs", "shapes.skip_render_dls"),
    ("$pref::Unix::OpenALFrequency", "unix.openal_frequency"),
    ("$pref::Video::allowD3D", "video.allow_d3d"),
    ("$pref::Video::allowOpenGL", "video.allow_opengl"),
    ("$pref::Video::appliedPref", "video.applied_pref"),
    ("$pref::Video::clipHigh", "video.clip_high"),
    ("$pref::Video::deleteContext", "video.delete_context"),
    ("$pref::Video::detailLevel", "video.detail_level"),
    ("$pref::Video::disableVerticalSync", "video.disable_vertical_sync"),
    ("$pref::Video::displayDevice", "video.display_device"),
    ("$pref::Video::fullScreen", "video.full_screen"),
    ("$pref::Video::maxFps", "video.max_fps"),
    ("$pref::Video::monitorNum", "video.monitor_num"),
    ("$pref::Video::only16", "video.only16"),
    ("$pref::Video::preferOpenGL", "video.prefer_opengl"),
    ("$pref::Video::profiledRenderer", "video.profiled_renderer"),
    ("$pref::Video::profiledVendor", "video.profiled_vendor"),
    ("$pref::Video::profileVersion", "video.profile_version"),
    ("$pref::Video::renderScale", "video.render_scale"),
    ("$pref::Video::resolution", "video.resolution"),
    ("$pref::Video::SafeModeOn", "video.safe_mode_on"),
    ("$pref::Video::shellResolution", "video.shell_resolution"),
    ("$pref::Video::simResolution", "video.sim_resolution"),
    ("$pref::Video::windowMode", "video.window_mode"),
    ("$pref::visibleDistanceMod", "video.visible_distance_mod"),
];

/// Indexed settings: old prefix -> key prefix (the index follows).
const INDEXED: &[(&str, &str)] = &[
    ("$pref::Server::BotName", "server.bot_name."),
    ("$pref::Server::BotTeam", "server.bot_team."),
    ("$pref::Server::Bot", "server.bot."),
    ("$pref::Audio::channelVolume", "audio.channel_volume."),
];

/// Old groups -> key prefix (the field follows as written).
const GROUPS: &[(&str, &str)] = &[("$playerList::", "players."), ("$solohi::", "hiscores."), ("$ttProgress::", "progress.")];

/// Old section names -> settings section.
const SECTIONS: &[(&str, &str)] = &[
    ("player", "player"),
    ("video", "video"),
    ("graphics", "graphics"),
    ("audio", "audio"),
    ("input", "input"),
    ("host", "host"),
    ("net", "net"),
    ("server", "server"),
    ("opengl", "opengl"),
    ("ts", "shapes"),
    ("terrain", "terrain"),
    ("scenelighting", "lighting"),
    ("shell", "shell"),
    ("demo", "demo"),
    ("unix", "unix"),
    ("tank", "tank"),
    ("particle", "particle"),
    ("interior", "interior"),
    ("hud", "hud"),
    ("client", "client"),
];

fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix)).then(|| &s[prefix.len()..])
}

/// `camelCase` / `PascalCase` -> `snake_case` (`maxFps` -> `max_fps`,
/// `disableARBTextureCompression` -> `disable_arb_texture_compression`).
pub fn snake(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &ch) in c.iter().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            let p = c[i - 1];
            let next_lower = c.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if p.is_ascii_lowercase() || p.is_ascii_digit() || (p.is_ascii_uppercase() && next_lower) {
                out.push('_');
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// The key of an old name (None: not a setting, player, high-score or
/// progress value). Settings: the table, else `section.snake_field`
/// (unknown sections and section-less names go to `misc`).
pub fn key(old: &str) -> Option<String> {
    if let Some((_, k)) = KEYS.iter().find(|(o, _)| o.eq_ignore_ascii_case(old)) {
        return Some(k.to_string());
    }
    for (o, k) in GROUPS {
        if let Some(field) = strip_ci(old, o).filter(|f| !f.is_empty()) {
            return Some(format!("{k}{field}"));
        }
    }
    for (o, k) in INDEXED {
        if let Some(i) = strip_ci(old, o).filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit())) {
            return Some(format!("{k}{i}"));
        }
    }
    let rest = strip_ci(old, "$pref::")?;
    let parts: Vec<&str> = rest.split("::").collect();
    if parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return None;
    }
    Some(match parts[..] {
        [field] => format!("misc.{}", snake(field)),
        [section, ref fields @ ..] => {
            let field = fields.iter().map(|f| snake(f)).collect::<Vec<_>>().join("_");
            match SECTIONS.iter().find(|(s, _)| s.eq_ignore_ascii_case(section)) {
                Some((_, s)) => format!("{s}.{field}"),
                None => format!("misc.{}_{field}", snake(section)),
            }
        }
        [] => return None,
    })
}

/// The old name of a key (the reverse of [`key`] for the table, the
/// indexed settings and the groups; others: None).
pub fn old_name(key: &str) -> Option<String> {
    if let Some((o, _)) = KEYS.iter().find(|(_, k)| k.eq_ignore_ascii_case(key)) {
        return Some(o.to_string());
    }
    for (o, k) in GROUPS.iter().chain(INDEXED) {
        if let Some(rest) = strip_ci(key, k).filter(|r| !r.is_empty()) {
            return Some(format!("{o}{rest}"));
        }
    }
    None
}

/// Parses an old save file into keys.
pub fn parse(src: &str) -> VarFile {
    let mut f = VarFile::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('$') else { continue };
        let Some(eq) = rest.find('=') else { continue };
        let Some(name) = key(&format!("${}", rest[..eq].trim())) else { continue };
        let v = rest[eq + 1..].trim();
        let value = if let Some(q) = v.strip_prefix('"') {
            // Up to the closing unescaped quote.
            let mut end = None;
            let mut esc = false;
            for (i, c) in q.char_indices() {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    end = Some(i);
                    break;
                }
            }
            VarValue::Str(collapse_escape(&q[..end.unwrap_or(q.len())]))
        } else {
            let tok = v.trim_end_matches(';').trim();
            if let Ok(i) = tok.parse::<i32>() {
                VarValue::Int(i)
            } else if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) {
                VarValue::Float(tt_game::fmt::atof(tok) as f32)
            } else {
                VarValue::Str(tok.to_string())
            }
        };
        f.set_value(&name, value);
    }
    f
}

/// An old save file as the old builds wrote it: every value whose key
/// starts with `prefix` (case-insensitive, "" = all) and has an old name.
pub fn export(vars: &VarFile, prefix: &str) -> String {
    let p = prefix.to_ascii_lowercase();
    let mut v: Vec<(String, &VarValue)> = vars.names().filter(|n| n.to_ascii_lowercase().starts_with(&p)).filter_map(|n| Some((old_name(n)?, vars.value(n)?))).collect();
    v.sort_by_key(|(n, _)| n.to_ascii_lowercase());
    let mut out = String::new();
    for (name, value) in v {
        match value {
            VarValue::Int(i) => out.push_str(&format!("{name} = {i};")),
            VarValue::Float(f) => out.push_str(&format!("{name} = {};", format_g(*f as f64))),
            VarValue::Str(s) => out.push_str(&format!("{name} = \"{}\";", expand_escape(s))),
        }
        out.push_str("\r\n");
    }
    out
}

/// The ban-list line of the old files.
const BAN_LINE: &str = "BanList::addAbsolute(";

/// Adds the bans of an old ban list file (lines it does not know are skipped).
pub fn import_banlist(list: &mut BanList, src: &str) {
    for line in src.lines() {
        let Some(args) = line.trim().strip_prefix(BAN_LINE).and_then(|r| r.strip_suffix(");")) else { continue };
        let parts: Vec<&str> = args.split(',').map(|p| p.trim()).collect();
        if let [guid, addr, until] = parts[..] {
            if let Ok(until) = until.parse::<i64>() {
                list.add(Ban { guid: guid.trim_matches('"').to_string(), ip: ip_of(addr), until });
            }
        }
    }
}

/// An old ban list file holding `bans` (for tests of the import).
pub fn banlist_file(bans: &[Ban]) -> String {
    bans.iter().map(|b| format!("{BAN_LINE}{}, \"IP:{}:0\", {});\n", b.guid, b.ip, b.until)).collect()
}

/// An old prefs file holding `settings` (key, value; integers bare, other
/// values quoted), e.g. for tests of the import.
pub fn prefs_file(settings: &[(&str, &str)]) -> String {
    let mut v = VarFile::new();
    for (k, val) in settings {
        match val.parse::<i32>() {
            Ok(i) => v.set_value(k, VarValue::Int(i)),
            Err(_) => v.set(k, val),
        }
    }
    export(&v, "")
}

/// The old prefs files (client, then server: later values win) as keys,
/// with the old mouse slider turned into the new one.
pub fn import_prefs(client: &str, server: &str) -> VarFile {
    let mut v = parse(client);
    v.merge(&parse(server));
    const LINK: &str = "input.link_mouse_sensitivity";
    if v.is_set(LINK) && !v.is_set(crate::prefs::MOUSE_SENSITIVITY) {
        let m = crate::prefs::migrate_mouse_sensitivity(v.get_f64(LINK));
        v.set(crate::prefs::MOUSE_SENSITIVITY, &crate::vars::num(m));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_of_old_names() {
        assert_eq!(key("$pref::Video::fullScreen").as_deref(), Some("video.full_screen"));
        assert_eq!(key("$PREF::VIDEO::FULLSCREEN").as_deref(), Some("video.full_screen"));
        assert_eq!(key("$Pref::Server::MaxPlayers").as_deref(), Some("server.max_players"));
        assert_eq!(key("$pref::Server::BotName3").as_deref(), Some("server.bot_name.3"));
        assert_eq!(key("$pref::Audio::channelVolume4").as_deref(), Some("audio.music_volume"));
        assert_eq!(key("$pref::Audio::channelVolume7").as_deref(), Some("audio.channel_volume.7"));
        assert_eq!(key("$pref::OpenGL::disableARBTextureCompression").as_deref(), Some("opengl.disable_arb_texture_compression"));
        assert_eq!(key("$pref::Foo::barBaz").as_deref(), Some("misc.foo_bar_baz"));
        assert_eq!(key("$pref::someThing").as_deref(), Some("misc.some_thing"));
        assert_eq!(key("$playerList::name0").as_deref(), Some("players.name0"));
        assert_eq!(key("$Game::Running"), None);
        assert_eq!(old_name("server.bot.2").as_deref(), Some("$pref::Server::Bot2"));
        assert_eq!(old_name("player.name").as_deref(), Some("$pref::Player::Name"));
        // Every table entry is its own reverse.
        for (o, k) in KEYS {
            assert_eq!(key(o).as_deref(), Some(*k), "{o}");
            assert_eq!(old_name(k).as_deref(), Some(*o), "{k}");
        }
    }

    #[test]
    fn parse_and_export_round_trip() {
        let src = "$playerList::name0 = \"Tester\";\r\n$playerList::count = 1;\r\n$pref::Audio::channelVolume4 = 0.8;\r\n$pref::Player::Name = \"a\\\"b\\\\c\\n\";\r\n$Game::Unrelated = 1;\r\n";
        let f = parse(src);
        assert_eq!(f.get("PLAYERS.NAME0"), "Tester");
        assert_eq!(f.value("players.count"), Some(&VarValue::Int(1)));
        assert_eq!(f.get("audio.music_volume"), "0.8");
        assert_eq!(f.get("player.name"), "a\"b\\c\n");
        assert_eq!(f.names().count(), 4);
        assert_eq!(export(&f, "players."), "$playerList::count = 1;\r\n$playerList::name0 = \"Tester\";\r\n");
        assert_eq!(export(&parse(&export(&f, "")), ""), export(&f, ""));
    }

    #[test]
    fn old_ban_lists() {
        let mut b = BanList::default();
        import_banlist(&mut b, "BanList::addAbsolute(0, \"IP:local:0\", 1800001825);\nBanList::addAbsolute(1234, \"IP:10.0.0.7:0\", 1800000100);\ngarbage\n");
        import_banlist(&mut b, "BanList::addAbsolute(1234, \"IP:10.0.0.7:0\", 1800009999);\n");
        assert_eq!(b.bans, vec![Ban { guid: "0".into(), ip: "local".into(), until: 1800001825 }, Ban { guid: "1234".into(), ip: "10.0.0.7".into(), until: 1800009999 }]);
        let mut c = BanList::default();
        import_banlist(&mut c, &banlist_file(&b.bans));
        assert_eq!(c, b);
    }
}
