//! Lines are read on a thread and executed between ticks ([`Console::poll`]),
//! so the server never blocks on input. The console is a typed command
//! table ([`Command`], plan D6), no script interpreter:
//!
//! * `help`, `status`, `say TEXT`, `kick PLAYER`, `ban PLAYER`,
//!   `map MISSION`, `quit` (PLAYER is a connection id or a name, MISSION a
//!   file or a name like `tt2_3`);
//! * `set KEY VALUE` / `get KEY` for the settings, by their keys
//!   (`set server.max_players 12`; `get` with a trailing `*` lists a
//!   group, `get server.*`), and the assignment `server.max_players = 12`
//!   as an alias of `set`;
//! * `simLag LOSS LAG` (also `simLag(LOSS, LAG);`): simulated packet loss
//!   (%) and latency (ms) for network tests; `echo TEXT`.
//!
//! The commands call the game server. Output goes to stdout; console log
//! lines are already echoed there unless `--quiet`, in which case the lines a
//! command logged are printed as well.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::Engine;
use crate::gameplay::server_host as sh;
use crate::net::Conn;
use tt_gameplay::server::ClientId;

/// Reads lines from a source (stdin, or a channel in tests) and runs them.
pub struct Console {
    rx: Receiver<String>,
    eof: bool,
}

const HELP: &str = "commands: status | say TEXT | kick PLAYER | ban PLAYER | map MISSION | quit | help\n\
                    settings: set KEY VALUE | get KEY (e.g. set server.max_players 12; get server.*) | KEY = VALUE\n\
                    network tests: simLag LOSS% LAGMS | echo TEXT";

/// A console line, parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    Status,
    Quit,
    Say(String),
    Kick(String),
    Ban(String),
    Map(Option<String>),
    /// `set KEY VALUE` / `KEY = VALUE`.
    Set { name: String, value: tt_shell::VarValue },
    /// `get NAME` (a trailing `*` matches a group).
    Get(String),
    SimLag { loss: String, lag: String },
    Echo(String),
    /// What the line was not, with the reason.
    Invalid(String),
}

/// A settings key as typed (`section.field`, a trailing `*` for `get`).
fn pref_name(n: &str) -> Result<String, String> {
    let n = n.trim();
    let base = n.strip_suffix('*').unwrap_or(n);
    let section = base.split('.').next().unwrap_or("");
    if tt_shell::settings::SECTIONS.iter().any(|s| s.eq_ignore_ascii_case(section)) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '*') {
        Ok(n.to_string())
    } else {
        Err(format!("{n:?} is not a setting (settings are section.field, sections: {}; `get video.*` lists a section)", tt_shell::settings::SECTIONS.join(", ")))
    }
}

/// A typed value: `"text"`, an integer, a number, else the word as
/// text.
fn value_of(v: &str) -> tt_shell::VarValue {
    let v = v.trim();
    if let Some(q) = v.strip_prefix('"').and_then(|q| q.strip_suffix('"')) {
        return tt_shell::VarValue::Str(tt_shell::vars::collapse_escape(q));
    }
    if let Ok(i) = v.parse::<i32>() {
        return tt_shell::VarValue::Int(i);
    }
    if v.parse::<f64>().is_ok() {
        return tt_shell::VarValue::Float(tt_shell::vars::atof(v) as f32);
    }
    tt_shell::VarValue::Str(v.to_string())
}

impl Command {
    pub fn parse(line: &str) -> Option<Command> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        // `KEY = VALUE` (a dotted word before the `=`).
        if let Some((n, v)) = line.trim_end_matches(';').split_once('=') {
            let n = n.trim();
            if n.contains('.') && !n.contains(char::is_whitespace) {
                return Some(match pref_name(n) {
                    Ok(name) => Command::Set { name, value: value_of(v) },
                    Err(e) => Command::Invalid(e),
                });
            }
        }
        let (cmd, rest) = line.split_once(char::is_whitespace).map(|(c, r)| (c, r.trim())).unwrap_or((line, ""));
        let cmd = cmd.to_ascii_lowercase();
        if cmd.starts_with("simlag(") {
            let inner = line[line.find('(').unwrap_or(0) + 1..].trim_end_matches(';').trim_end_matches(')');
            let p: Vec<&str> = inner.split(',').map(str::trim).collect();
            return Some(match p[..] {
                [loss, lag] => Command::SimLag { loss: loss.into(), lag: lag.into() },
                _ => Command::Invalid("usage: simLag LOSS LAG".into()),
            });
        }
        let need = |usage: &str, f: fn(String) -> Command| if rest.is_empty() { Command::Invalid(format!("usage: {usage}")) } else { f(rest.to_string()) };
        Some(match cmd.as_str() {
            "help" | "?" => Command::Help,
            "status" | "players" => Command::Status,
            "quit" | "exit" if rest.is_empty() => Command::Quit,
            "say" => need("say TEXT", Command::Say),
            "kick" => need("kick PLAYER", Command::Kick),
            "ban" => need("ban PLAYER", Command::Ban),
            "map" => Command::Map((!rest.is_empty()).then(|| rest.to_string())),
            "echo" => Command::Echo(rest.to_string()),
            "set" => {
                let w = split_words(rest);
                match (w.first(), rest.split_once(char::is_whitespace)) {
                    (Some(n), Some((_, v))) => match pref_name(n) {
                        Ok(name) => Command::Set { name, value: value_of(v) },
                        Err(e) => Command::Invalid(e),
                    },
                    _ => Command::Invalid("usage: set KEY VALUE".into()),
                }
            }
            "get" => match pref_name(rest) {
                Ok(n) => Command::Get(n),
                Err(e) => Command::Invalid(if rest.is_empty() { "usage: get KEY".into() } else { e }),
            },
            "simlag" => match split_words(rest)[..] {
                [ref loss, ref lag] => Command::SimLag { loss: loss.clone(), lag: lag.clone() },
                _ => Command::Invalid("usage: simLag LOSS LAG".into()),
            },
            _ => Command::Invalid(format!("unknown command {cmd:?} (help lists them)")),
        })
    }
}

impl Console {
    /// Reads stdin on a thread.
    pub fn stdin() -> Console {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("console".into())
            .spawn(move || {
                for line in std::io::stdin().lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .expect("console thread");
        Console { rx, eof: false }
    }

    /// Runs every line that arrived; prints the output. Returns the lines run.
    pub fn poll(&mut self, engine: &mut Engine) -> usize {
        let mut n = 0;
        loop {
            match self.rx.try_recv() {
                Ok(line) => {
                    n += 1;
                    let log_before = engine.host.log.history.len();
                    let out = execute(engine, &line);
                    if !engine.host.log.echo {
                        for (_, l) in &engine.host.log.history[log_before.min(engine.host.log.history.len())..] {
                            println!("{l}");
                        }
                    }
                    for l in out {
                        println!("{l}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    break;
                }
            }
        }
        n
    }
}

/// Splits a command line into words; `"…"` keeps spaces (`\"` inside).
pub fn split_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted && it.peek() == Some(&'"') => cur.push(it.next().unwrap()),
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The connected players: the local connection's, then every remote
/// client's (connection, client).
fn clients(e: &Engine) -> Vec<(Conn, ClientId)> {
    let mut conns: Vec<Conn> = e.host.net.server.into_iter().collect();
    conns.extend(crate::netgame::remote_conns(&e.host));
    conns.into_iter().filter_map(|c| sh::client_of_conn(&e.host, c).map(|cl| (c, cl))).collect()
}

/// A client's player name (without the colour/control characters).
pub fn client_name(e: &Engine, c: ClientId) -> String {
    sh::record(&e.host, c).map(|r| r.name().to_string()).unwrap_or_default().chars().filter(|ch| !ch.is_control()).collect()
}

/// The client a `kick`/`ban` argument names: its id, else its name
/// (case-insensitive; a unique prefix also works).
fn find_client(e: &mut Engine, who: &str) -> Result<ClientId, String> {
    let all: Vec<ClientId> = clients(e).into_iter().map(|(_, c)| c).collect();
    if let Ok(id) = who.parse::<u32>() {
        if all.contains(&ClientId(id)) {
            return Ok(ClientId(id));
        }
    }
    let named: Vec<(ClientId, String)> = all.iter().map(|&c| (c, client_name(e, c))).collect();
    if let Some((c, _)) = named.iter().find(|(_, n)| n.eq_ignore_ascii_case(who)) {
        return Ok(*c);
    }
    let lw = who.to_ascii_lowercase();
    let prefix: Vec<_> = named.iter().filter(|(_, n)| n.to_ascii_lowercase().starts_with(&lw)).collect();
    match prefix.as_slice() {
        [(c, _)] => Ok(*c),
        [] => Err(format!("no player {who:?} (status lists them)")),
        _ => Err(format!("{who:?} matches several players")),
    }
}

pub fn mission_path(e: &Engine, name: &str) -> Option<String> {
    let file = if name.contains('/') { tt_assets::layout::migrate_mission(name).unwrap_or_else(|| name.to_string()) } else { tt_assets::layout::mission(&tt_assets::layout::mission_id(name)) };
    crate::mission::mission_exists(&e.host, &file).then_some(file)
}

fn status(e: &mut Engine) -> Vec<String> {
    let mut out = Vec::new();
    let addr = crate::netgame::server_addr(&e.host).map(|a| a.to_string()).unwrap_or_else(|| "not listening".into());
    let info = crate::netgame::info::server_info(&e.host);
    out.push(format!("server {:?} on {addr}{}", info.name, if info.password { " (password)" } else { "" }));
    out.push(format!("mission {} {:?}, {}; {}/{} players, {} bots", info.mission_file, info.map(), info.mission_type, info.player_count, info.max_players, info.bot_count));
    let cl = clients(e);
    if cl.is_empty() {
        out.push("no players connected".into());
    } else {
        out.push(format!("{:>6}  {:<20} {:<24} {:>6} {:>6}", "id", "name", "address", "ping", "loss%"));
        for (conn, c) in cl {
            let name = client_name(e, c);
            let addr = crate::netgame::address(&e.host, conn).unwrap_or_else(|| "local".into());
            let (ping, loss) = crate::netgame::ping_loss(&mut e.host, conn).map(|(p, l)| (format!("{p:.0}"), format!("{l:.0}"))).unwrap_or(("-".into(), "-".into()));
            out.push(format!("{:>6}  {name:<20} {addr:<24} {ping:>6} {loss:>6}", c.0));
        }
    }
    out
}

/// A typed call into the server.
fn server(e: &mut Engine, f: impl FnOnce(&mut tt_gameplay::server::Server, &mut crate::gameplay::server_host::Bridge)) {
    crate::gameplay::server_host::with_server(&mut e.host, f);
}

/// Runs one console line; returns what to print (besides the log lines).
pub fn execute(e: &mut Engine, line: &str) -> Vec<String> {
    let Some(cmd) = Command::parse(line) else { return Vec::new() };
    match cmd {
        Command::Help => HELP.lines().map(|l| l.trim().to_string()).collect(),
        Command::Status => status(e),
        Command::Quit => {
            e.host.log.info("Console: quit");
            e.host.quit_requested = true;
            vec!["shutting down".into()]
        }
        Command::Say(text) => {
            let msg = format!("\u{3}Server: {text}");
            let (kind, text_) = tt_gameplay::server::Server::message_of("MsgAdminForce", &msg, &[]);
            server(e, |s, b| s.message_all(b, &kind, &text_));
            e.host.log.info(&format!("Server: {text}"));
            Vec::new()
        }
        c @ (Command::Kick(_) | Command::Ban(_)) => {
            let (verb, who) = match &c {
                Command::Kick(w) => ("kick", w.as_str()),
                Command::Ban(w) => ("ban", w.as_str()),
                _ => unreachable!(),
            };
            match find_client(e, who) {
                Ok(c) => {
                    let name = client_name(e, c);
                    server(e, |s, b| if verb == "kick" { s.kick(b, c) } else { s.ban(b, c) });
                    vec![format!("{} {name} ({})", if verb == "kick" { "kicked" } else { "banned" }, c.0)]
                }
                Err(m) => vec![m],
            }
        }
        Command::Map(None) => vec![format!("usage: map MISSION (now {})", e.server().session.mission_file)],
        Command::Map(Some(m)) => match mission_path(e, &m) {
            Some(p) => {
                server(e, |s, b| s.load_mission(b, &p, false));
                vec![format!("loading {p}")]
            }
            None => vec![format!("no mission {m:?}")],
        },
        Command::Set { name, value } => {
            let shown = value.as_string();
            e.host.shell.prefs.set_value(&name, value);
            vec![format!("{name} = {shown}")]
        }
        Command::Get(name) => {
            let p = &e.host.shell.prefs;
            match name.strip_suffix('*') {
                Some(prefix) => {
                    let lp = prefix.to_ascii_lowercase();
                    let mut v: Vec<String> = p.vars().names().filter(|n| n.to_ascii_lowercase().starts_with(&lp)).map(|n| format!("{n} = {}", p.get(n))).collect();
                    v.sort_by_key(|l| l.to_ascii_lowercase());
                    if v.is_empty() {
                        vec![format!("no setting matches {name}")]
                    } else {
                        v
                    }
                }
                None if p.is_set(&name) => vec![format!("{name} = {}", p.get(&name))],
                None => vec![format!("{name} is not set")],
            }
        }
        Command::SimLag { loss, lag } => {
            crate::gameplay::server_host::sim_lag(&mut e.host, &loss, &lag);
            vec![format!("simulated loss {loss}%, lag {lag} ms")]
        }
        Command::Echo(text) => {
            e.host.log.info(&text);
            Vec::new()
        }
        Command::Invalid(m) => vec![m],
    }
}

/// Set by SIGINT/SIGTERM/SIGHUP (Ctrl+C, `docker stop`; Ctrl+C/Break on
/// Windows, where closing the console still ends the process unsaved).
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes SIGINT/SIGTERM request the console's `quit` (polled with
/// [`take_stop`], so the settings and the ban list are saved); a second
/// one while shutting down exits at once.
pub fn install_stop_handler() {
    let _ = ctrlc::set_handler(|| {
        if STOP.swap(true, std::sync::atomic::Ordering::SeqCst) {
            std::process::exit(130);
        }
    });
}

/// Turns a received stop signal into `quit_requested`.
pub fn take_stop(h: &mut crate::host::EngineHost) {
    if STOP.load(std::sync::atomic::Ordering::SeqCst) && !h.quit_requested {
        h.log.info("Signal: quit");
        h.quit_requested = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{split_words, Command};
    use tt_shell::VarValue;

    #[test]
    fn commands() {
        let set = |n: &str, v: VarValue| Some(Command::Set { name: n.into(), value: v });
        assert_eq!(Command::parse("server.max_players = 12;"), set("server.max_players", VarValue::Int(12)));
        assert_eq!(Command::parse("set server.name \"My \\\"game\\\"\""), set("server.name", VarValue::Str("My \"game\"".into())));
        assert_eq!(Command::parse("set audio.master_volume 0.5"), set("audio.master_volume", VarValue::Float(0.5)));
        assert_eq!(Command::parse("get server.*"), Some(Command::Get("server.*".into())));
        assert!(matches!(Command::parse("session.status = 1;"), Some(Command::Invalid(_))));
        assert_eq!(Command::parse("simLag(10, 200);"), Some(Command::SimLag { loss: "10".into(), lag: "200".into() }));
        assert_eq!(Command::parse("simlag 5 100"), Some(Command::SimLag { loss: "5".into(), lag: "100".into() }));
        assert_eq!(Command::parse("kick  Tester "), Some(Command::Kick("Tester".into())));
        assert!(matches!(Command::parse("echo(\"x\");"), Some(Command::Invalid(_))));
        assert_eq!(Command::parse("  "), None);
    }

    #[test]
    fn words() {
        assert_eq!(split_words("say  hello there "), vec!["say", "hello", "there"]);
        assert_eq!(split_words(r#"f "a b" "" c"#), vec!["f", "a b", "", "c"]);
        assert_eq!(split_words(r#"f "say \"hi\"""#), vec!["f", "say \"hi\""]);
    }
}
