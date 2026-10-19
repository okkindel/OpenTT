//! 14) And the spelling of the command tag (`centerPrintAll` sends
//!     `'centerPrint'`, `centerPrint` sends `'CenterPrint'`), because the golden
//!     traces and the network protocol show them.
//!
//! The golden recorder's decoder (`tt-engine` `gameplay/golden/record.rs`)
//! names the fields: [`MsgKind::ClientJoin`] is its `ClientJoin id= name=
//! team= guid= score= cum= ai= admin= super= text=` and so on.

/// A script value in a message: a tagged string (`'text'`, sent by its
/// text) or a plain string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    Str(String),
    Tag(String),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn tag(s: impl Into<String>) -> Value {
        Value::Tag(s.into())
    }

    /// The empty plain string (a missing argument).
    pub fn empty() -> Value {
        Value::Str(String::new())
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Str(s) if s.is_empty())
    }

    /// The text (of the tag, or the string).
    pub fn text(&self) -> &str {
        match self {
            Value::Str(s) | Value::Tag(s) => s,
        }
    }

}

/// A message or command in the script's form: the command name and the
/// arguments after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub args: Vec<Value>,
}

/// The kind of a `ServerMessage` (`messageClient(client, 'MsgKind', text, …)`)
/// with its arguments `%a1…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgKind {
    /// `MsgClientJoin`: a player list row (`name` is the tagged name).
    ClientJoin { name: Value, id: String, team: String, guid: String, score: String, cum: String, ai: String, admin: String, super_admin: String },
    /// `MsgClientDrop`.
    ClientDrop { name: Value, id: String },
    /// `MsgClientKilled` (`killer` empty for a death without a killer).
    ClientKilled { victim: Value, killer: Value },
    /// `MsgClientScoreChanged`.
    ScoreChanged { score: String, cum: String, id: String },
    /// `MsgTeamScoreChanged`.
    TeamScoreChanged { score: String, cum: String, team: String },
    /// `MsgAddTeam`.
    AddTeam { team: String, name: Value, score: String, cum: String },
    /// `MsgLoadInfo`: the mission name.
    LoadInfo { name: Value },
    /// `MsgLoadInfoDone`.
    LoadInfoDone,
    ConnectionError { text: Value },
    /// `MsgAdminForce`: who became admin.
    AdminForce { who: Value },
    /// Any other message type (`kind` may be a plain string, the flood
    /// warning's is `""`), its arguments without trailing empty ones.
    Other { kind: Value, args: Vec<Value> },
}

/// A message from the server to one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Message { kind: MsgKind, text: Value },
    Chat { from: String, voice: Value, pitch: String, text: Value, args: Vec<Value> },
    /// `lc`: sent as `'centerPrint'` (by `centerPrintAll`).
    CenterPrint { text: Value, secs: String, lines: String, lc: bool },
    /// `lc`: sent as `'bottomPrint'` (by `bottomPrintAll`).
    BottomPrint { text: Value, secs: String, lines: String, lc: bool },
    ClearCenterPrint,
    ClearBottomPrint,
    /// `lc`: sent as `'syncClock'` with the time only.
    SyncClock { secs: String, dir: String, lc: bool },
    MissionStart { seq: String },
    MissionEnd { seq: String },
    MissionPhase1 { seq: String, file: String, music: String },
    MissionPhase2 { seq: String, file: String },
    MissionPhase3 { seq: String, file: String },
    GameStart,
    /// The obfuscated stats (`strfrip`).
    GameEnd { stats: String },
    /// A client-side effect of the single-player code (`'TTClient'`).
    Hud(HudMsg),
}

/// A client-side effect of the gameplay code (formerly a direct poke at the
/// local client's objects). Values are formatted as the script formatted them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HudMsg {
    /// `HudClock.showTime = v`.
    ClockShowTime(String),
    /// `HudClock.setTime(v)`.
    ClockSetTime(String),
    /// `HudClock.setVisible(v)`.
    ClockVisible(String),
    /// `SPBrainCounter.maxBrains = v`.
    BrainsMax(String),
    /// `SPBrainCounter.setCount(v)`.
    BrainsCount(String),
    /// `SPBrainCounter.setVisible(v)`.
    BrainsVisible(String),
    /// `SPScoreGui.setScore(v)`.
    Score(String),
    /// `SPLivesGui.showLives(v)`.
    Lives(String),
    /// `alxPlay(profile)`.
    Sound(String),
    TargetPracticeDone,
    /// Solo game over: `soloPlayGui.schedule(0, endGame)`.
    SoloGameOver,
}

impl HudMsg {
    /// (kind, arguments) in the script form (`'TTClient'`, kind, args…).
    pub fn encode(&self) -> (&'static str, Vec<String>) {
        use HudMsg::*;
        match self {
            ClockShowTime(v) => ("clockShowTime", vec![v.clone()]),
            ClockSetTime(v) => ("clockSetTime", vec![v.clone()]),
            ClockVisible(v) => ("clockVisible", vec![v.clone()]),
            BrainsMax(v) => ("brainsMax", vec![v.clone()]),
            BrainsCount(v) => ("brainsCount", vec![v.clone()]),
            BrainsVisible(v) => ("brainsVisible", vec![v.clone()]),
            Score(v) => ("score", vec![v.clone()]),
            Lives(v) => ("lives", vec![v.clone()]),
            Sound(v) => ("sound", vec![v.clone()]),
            TargetPracticeDone => ("targetPracticeDone", vec![]),
            SoloGameOver => ("soloGameOver", vec![]),
        }
    }

    pub fn decode(kind: &str, args: &[String]) -> Option<HudMsg> {
        use HudMsg::*;
        let a = || args.first().cloned().unwrap_or_default();
        Some(match kind {
            "clockShowTime" => ClockShowTime(a()),
            "clockSetTime" => ClockSetTime(a()),
            "clockVisible" => ClockVisible(a()),
            "brainsMax" => BrainsMax(a()),
            "brainsCount" => BrainsCount(a()),
            "brainsVisible" => BrainsVisible(a()),
            "score" => Score(a()),
            "lives" => Lives(a()),
            "sound" => Sound(a()),
            "targetPracticeDone" => TargetPracticeDone,
            "soloGameOver" => SoloGameOver,
            _ => return None,
        })
    }
}

/// `messageClient`'s argument count after the command (type, text, %a1..%a13).
pub const MESSAGE_ARGS: usize = 15;
/// `chatMessageClient`'s (sender, voice tag, pitch, text, %a1..%a10).
pub const CHAT_ARGS: usize = 14;

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

/// Drops trailing empty plain strings.
pub fn trim(args: &[Value]) -> &[Value] {
    let mut n = args.len();
    while n > 0 && args[n - 1].is_empty() {
        n -= 1;
    }
    &args[..n]
}

fn pad(mut v: Vec<Value>, n: usize) -> Vec<Value> {
    while v.len() < n {
        v.push(Value::empty());
    }
    v
}

impl MsgKind {
    /// The message type argument (`'MsgClientJoin'`, …).
    pub fn kind_value(&self) -> Value {
        use MsgKind::*;
        let t = match self {
            ClientJoin { .. } => "MsgClientJoin",
            ClientDrop { .. } => "MsgClientDrop",
            ClientKilled { .. } => "MsgClientKilled",
            ScoreChanged { .. } => "MsgClientScoreChanged",
            TeamScoreChanged { .. } => "MsgTeamScoreChanged",
            AddTeam { .. } => "MsgAddTeam",
            LoadInfo { .. } => "MsgLoadInfo",
            LoadInfoDone => "MsgLoadInfoDone",
            ConnectionError { .. } => "MsgConnectionError",
            AdminForce { .. } => "MsgAdminForce",
            Other { kind, .. } => return kind.clone(),
        };
        Value::tag(t)
    }

    /// `%a1…` (without trailing padding).
    pub fn args(&self) -> Vec<Value> {
        use MsgKind::*;
        match self {
            ClientJoin { name, id, team, guid, score, cum, ai, admin, super_admin } => vec![name.clone(), s(id), s(team), s(guid), s(score), s(cum), s(ai), s(admin), s(super_admin)],
            ClientDrop { name, id } => vec![name.clone(), s(id)],
            ClientKilled { victim, killer } => vec![victim.clone(), killer.clone()],
            ScoreChanged { score, cum, id } => vec![s(score), s(cum), s(id)],
            TeamScoreChanged { score, cum, team } => vec![s(score), s(cum), s(team)],
            AddTeam { team, name, score, cum } => vec![s(team), name.clone(), s(score), s(cum)],
            LoadInfo { name } => vec![name.clone()],
            LoadInfoDone => vec![],
            ConnectionError { text } => vec![text.clone()],
            AdminForce { who } => vec![who.clone()],
            Other { args, .. } => args.clone(),
        }
    }

    /// The typed kind for `kind` and `%a1…` (`Other` when the type is not
    /// one of the typed ones).
    fn from_args(kind: &Value, a: &[Value]) -> MsgKind {
        let v = |i: usize| a.get(i).cloned().unwrap_or_else(Value::empty);
        let t = |i: usize| v(i).text().to_string();
        let typed = match kind {
            Value::Tag(k) => match k.as_str() {
                "MsgClientJoin" => Some(MsgKind::ClientJoin { name: v(0), id: t(1), team: t(2), guid: t(3), score: t(4), cum: t(5), ai: t(6), admin: t(7), super_admin: t(8) }),
                "MsgClientDrop" => Some(MsgKind::ClientDrop { name: v(0), id: t(1) }),
                "MsgClientKilled" => Some(MsgKind::ClientKilled { victim: v(0), killer: v(1) }),
                "MsgClientScoreChanged" => Some(MsgKind::ScoreChanged { score: t(0), cum: t(1), id: t(2) }),
                "MsgTeamScoreChanged" => Some(MsgKind::TeamScoreChanged { score: t(0), cum: t(1), team: t(2) }),
                "MsgAddTeam" => Some(MsgKind::AddTeam { team: t(0), name: v(1), score: t(2), cum: t(3) }),
                "MsgLoadInfo" => Some(MsgKind::LoadInfo { name: v(0) }),
                "MsgLoadInfoDone" => Some(MsgKind::LoadInfoDone),
                "MsgConnectionError" => Some(MsgKind::ConnectionError { text: v(0) }),
                "MsgAdminForce" => Some(MsgKind::AdminForce { who: v(0) }),
                _ => None,
            },
            Value::Str(_) => None,
        };
        // Typed only if that loses nothing (a tag where a plain text is
        // expected, more arguments than the type has).
        match typed {
            Some(k) if trim(&k.args()) == trim(a) => k,
            _ => MsgKind::Other { kind: kind.clone(), args: trim(a).to_vec() },
        }
    }
}

impl ServerMsg {
    /// The command in the script's form, with the argument count the
    /// scripts used.
    pub fn lower(&self) -> Command {
        use ServerMsg::*;
        let (name, args): (&'static str, Vec<Value>) = match self {
            Message { kind, text } => {
                let mut v = vec![kind.kind_value(), text.clone()];
                v.extend(kind.args());
                ("ServerMessage", pad(v, MESSAGE_ARGS))
            }
            Chat { from, voice, pitch, text, args } => {
                let mut v = vec![s(from), voice.clone(), s(pitch), text.clone()];
                v.extend(args.iter().cloned());
                ("ChatMessage", pad(v, CHAT_ARGS))
            }
            CenterPrint { text, secs, lines, lc } => (if *lc { "centerPrint" } else { "CenterPrint" }, vec![text.clone(), s(secs), s(lines)]),
            BottomPrint { text, secs, lines, lc } => (if *lc { "bottomPrint" } else { "BottomPrint" }, vec![text.clone(), s(secs), s(lines)]),
            ClearCenterPrint => ("ClearCenterPrint", vec![]),
            ClearBottomPrint => ("ClearBottomPrint", vec![]),
            SyncClock { secs, dir, lc: false } => ("SyncClock", vec![s(secs), s(dir)]),
            SyncClock { secs, dir, lc: true } => ("syncClock", if dir.is_empty() { vec![s(secs)] } else { vec![s(secs), s(dir)] }),
            MissionStart { seq } => ("MissionStart", vec![s(seq)]),
            MissionEnd { seq } => ("MissionEnd", vec![s(seq)]),
            MissionPhase1 { seq, file, music } => ("MissionStartPhase1", vec![s(seq), s(file), s(music)]),
            MissionPhase2 { seq, file } => ("MissionStartPhase2", vec![s(seq), s(file)]),
            MissionPhase3 { seq, file } => ("MissionStartPhase3", vec![s(seq), s(file)]),
            GameStart => ("GameStart", vec![]),
            GameEnd { stats } => ("GameEnd", vec![s(stats)]),
            Hud(m) => {
                let (kind, a) = m.encode();
                let mut v = vec![s(kind)];
                v.extend(a.iter().map(|x| s(x)));
                ("TTClient", v)
            }
        };
        Command { name, args }
    }

    /// The arguments as the client's handler takes them: trailing empty
    /// arguments dropped, every tagged text expanded with the arguments
    /// after it (`%1`..`%9`, from the last argument to the first, so an
    /// argument that is itself a template is expanded before it is used),
    /// plain strings as they are (never with a tag marker).
    pub fn received(&self) -> Vec<Received> {
        let c = self.lower();
        let args = trim(&c.args);
        let mut out: Vec<Received> = args
            .iter()
            .map(|v| match v {
                Value::Tag(t) => Received { tag: true, text: t.clone() },
                Value::Str(s) => Received { tag: false, text: s.replace('\u{1}', "") },
            })
            .collect();
        for i in (0..out.len()).rev() {
            if out[i].tag {
                let expanded = expand(&out[i].text, &out[i + 1..]);
                out[i].text = expanded;
            }
        }
        out
    }

    /// `name` is matched like a function name (case-insensitive).
    pub fn parse(name: &str, args: &[Value]) -> Option<ServerMsg> {
        use ServerMsg::*;
        let v = |i: usize| args.get(i).cloned().unwrap_or_else(Value::empty);
        let t = |i: usize| v(i).text().to_string();
        let m = match name.to_ascii_lowercase().as_str() {
            "servermessage" => Message { kind: MsgKind::from_args(&v(0), args.get(2..).unwrap_or(&[])), text: v(1) },
            "chatmessage" => Chat { from: t(0), voice: v(1), pitch: t(2), text: v(3), args: trim(args.get(4..).unwrap_or(&[])).to_vec() },
            "centerprint" => CenterPrint { text: v(0), secs: t(1), lines: t(2), lc: name == "centerPrint" },
            "bottomprint" => BottomPrint { text: v(0), secs: t(1), lines: t(2), lc: name == "bottomPrint" },
            "clearcenterprint" => ClearCenterPrint,
            "clearbottomprint" => ClearBottomPrint,
            "syncclock" => SyncClock { secs: t(0), dir: t(1), lc: name == "syncClock" },
            "missionstart" => MissionStart { seq: t(0) },
            "missionend" => MissionEnd { seq: t(0) },
            "missionstartphase1" => MissionPhase1 { seq: t(0), file: t(1), music: t(2) },
            "missionstartphase2" => MissionPhase2 { seq: t(0), file: t(1) },
            "missionstartphase3" => MissionPhase3 { seq: t(0), file: t(1) },
            "gamestart" => GameStart,
            "gameend" => GameEnd { stats: t(0) },
            "ttclient" => {
                let a: Vec<String> = args.get(1..).unwrap_or(&[]).iter().map(|x| x.text().to_string()).collect();
                Hud(HudMsg::decode(&t(0), &a)?)
            }
            _ => return None,
        };
        (trim(&m.lower().args) == trim(args)).then_some(m)
    }
}

/// One argument of a received message ([`ServerMsg::received`]): its text
/// (a template already expanded) and whether it was a tagged string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Received {
    pub tag: bool,
    pub text: String,
}

/// `%1`..`%9` replaced by the texts of the arguments after the template.
fn expand(template: &str, rest: &[Received]) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some(&d) = chars.peek() {
                if ('1'..='9').contains(&d) {
                    chars.next();
                    if let Some(a) = rest.get(d as usize - '1' as usize) {
                        out += &a.text;
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugCmd {
    ToggleCamera,
    DropPlayerAtCamera,
    DropCameraAtPlayer,
    SelectObject,
    CycleTank,
    TogglePlayerAI,
}

const DEBUG_CMDS: &[(DebugCmd, &str)] = &[
    (DebugCmd::ToggleCamera, "ToggleCamera"),
    (DebugCmd::DropPlayerAtCamera, "DropPlayerAtCamera"),
    (DebugCmd::DropCameraAtPlayer, "DropCameraAtPlayer"),
    (DebugCmd::SelectObject, "SelectObject"),
    (DebugCmd::CycleTank, "CycleTank"),
    (DebugCmd::TogglePlayerAI, "TogglePlayerAI"),
];

/// A command from a client to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCmd {
    /// `MissionStartPhase<phase>Ack(seq)`, phase 1–3.
    PhaseAck { phase: u8, seq: String },
    SpawnPlayer,
    /// Chat to everyone (`messageSent`).
    MessageSent { text: String },
    /// Chat to the team (`teamMessageSent`).
    TeamMessageSent { text: String },
    /// Admin login (`SAD`).
    Sad { password: String },
    SadSetPassword { password: String },
    /// A development command and its arguments (without trailing empty ones).
    Debug { cmd: DebugCmd, args: Vec<String> },
}

impl ClientCmd {
    pub fn lower(&self) -> (&'static str, Vec<String>) {
        use ClientCmd::*;
        match self {
            PhaseAck { phase: 1, seq } => ("MissionStartPhase1Ack", vec![seq.clone()]),
            PhaseAck { phase: 2, seq } => ("MissionStartPhase2Ack", vec![seq.clone()]),
            PhaseAck { seq, .. } => ("MissionStartPhase3Ack", vec![seq.clone()]),
            SpawnPlayer => ("SpawnPlayer", vec![]),
            MessageSent { text } => ("messageSent", vec![text.clone()]),
            TeamMessageSent { text } => ("teamMessageSent", vec![text.clone()]),
            Sad { password } => ("SAD", vec![password.clone()]),
            SadSetPassword { password } => ("SADSetPassword", vec![password.clone()]),
            Debug { cmd, args } => (DEBUG_CMDS.iter().find(|(c, _)| c == cmd).map(|(_, n)| *n).unwrap_or(""), args.clone()),
        }
    }

    /// A development command (refused from remote clients unless cheats are on in a debug build).
    pub fn is_debug(&self) -> bool {
        matches!(self, ClientCmd::Debug { .. })
    }

    /// Every string the client chose (for the length and tag sanitising on the server).
    pub fn strings_mut(&mut self) -> Vec<&mut String> {
        use ClientCmd::*;
        match self {
            PhaseAck { seq, .. } => vec![seq],
            SpawnPlayer => vec![],
            MessageSent { text } | TeamMessageSent { text } => vec![text],
            Sad { password } | SadSetPassword { password } => vec![password],
            Debug { args, .. } => args.iter_mut().collect(),
        }
    }
}

#[cfg(test)]
pub(crate) use tests::samples;

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(t: &str) -> Value {
        Value::tag(t)
    }

    pub(crate) fn samples() -> Vec<ServerMsg> {
        use ServerMsg::*;
        let k = |kind: MsgKind, text: Value| Message { kind, text };
        vec![
            k(
                MsgKind::ClientJoin { name: tag("Bait"), id: "1210".into(), team: "BlueTeam".into(), guid: "0".into(), score: "0".into(), cum: "0".into(), ai: "0".into(), admin: "1".into(), super_admin: "1".into() },
                tag("\u{2}%1 joined the game."),
            ),
            k(MsgKind::ClientJoin { name: tag("\u{10}\u{c}Ri [Bot]\u{11}"), id: "1300".into(), team: "".into(), guid: "".into(), score: "0".into(), cum: "0".into(), ai: "1".into(), admin: "0".into(), super_admin: "0".into() }, Value::empty()),
            k(MsgKind::ClientDrop { name: tag("Tester.1"), id: "1211".into() }, tag("\u{2}%1 has left the game.")),
            k(MsgKind::ClientKilled { victim: tag("Player0"), killer: Value::empty() }, tag("%1 has been eliminated!")),
            k(MsgKind::ScoreChanged { score: "2".into(), cum: "12".into(), id: "1300".into() }, Value::empty()),
            k(MsgKind::TeamScoreChanged { score: "7".into(), cum: "7".into(), team: "1150".into() }, Value::empty()),
            k(MsgKind::AddTeam { team: "1150".into(), name: Value::str("Blue Team"), score: "0".into(), cum: "0".into() }, Value::empty()),
            k(MsgKind::LoadInfo { name: Value::str("Medulla Mesa") }, Value::empty()),
            k(MsgKind::LoadInfoDone, Value::empty()),
            k(MsgKind::ConnectionError { text: Value::str("You do not have the correct version") }, Value::empty()),
            k(MsgKind::AdminForce { who: Value::str("LocalClientConnection") }, Value::str("\u{3}Tester has become Admin by force.")),
            k(MsgKind::Other { kind: Value::empty(), args: vec![Value::str("10")] }, tag("\u{4}FLOOD PROTECTION:\u{f} You must wait another %1 seconds.")),
            k(MsgKind::Other { kind: tag("MsgTest"), args: vec![] }, Value::str("server says hi~wdryFire.wav")),
            Chat { from: "1210".into(), voice: Value::empty(), pitch: "".into(), text: tag("\u{5}%1: %2"), args: vec![tag("Tester"), Value::str("hello")] },
            CenterPrint { text: Value::str("G A M E  O V E R"), secs: "0".into(), lines: "2".into(), lc: false },
            CenterPrint { text: Value::str("all"), secs: "1".into(), lines: "2".into(), lc: true },
            BottomPrint { text: Value::str("x5 Quick-shot bonus!"), secs: "2".into(), lines: "2".into(), lc: false },
            BottomPrint { text: Value::str("Mal [Bot] scored a goal!"), secs: "4".into(), lines: "2".into(), lc: true },
            ClearCenterPrint,
            ClearBottomPrint,
            SyncClock { secs: "899.936".into(), dir: "0".into(), lc: false },
            SyncClock { secs: "35".into(), dir: "".into(), lc: true },
            MissionStart { seq: "1".into() },
            MissionEnd { seq: "1".into() },
            MissionPhase1 { seq: "1".into(), file: "missions/tt1_1.ron".into(), music: "Lush".into() },
            MissionPhase2 { seq: "2".into(), file: "missions/tt1_1.ron".into() },
            MissionPhase3 { seq: "2".into(), file: "missions/tt1_1.ron".into() },
            GameStart,
            GameEnd { stats: "\u{7}abc 1".into() },
            Hud(HudMsg::Lives("2".into())),
            Hud(HudMsg::SoloGameOver),
        ]
    }

    #[test]
    fn server_messages_lower_and_parse_back() {
        for m in samples() {
            let c = m.lower();
            assert_eq!(ServerMsg::parse(c.name, &c.args), Some(m.clone()), "{c:?}");
            // Trailing empty arguments do not matter.
            assert_eq!(ServerMsg::parse(c.name, trim(&c.args)), Some(m));
        }
        // The script's argument counts.
        assert_eq!(samples()[0].lower().args.len(), MESSAGE_ARGS);
        assert_eq!(samples()[13].lower().args.len(), CHAT_ARGS);
    }

    #[test]
    fn received_form_expands_templates() {
        let m = &samples()[0];
        let r = m.received();
        let texts: Vec<&str> = r.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["MsgClientJoin", "\u{2}Bait joined the game.", "Bait", "1210", "BlueTeam", "0", "0", "0", "0", "1", "1"]);
        assert!(r[0].tag && r[1].tag && r[2].tag && !r[3].tag);
        // A template argument of a template: expanded first.
        let c = ServerMsg::Chat { from: "1".into(), voice: Value::empty(), pitch: "".into(), text: tag("%1!"), args: vec![tag("<%1>"), Value::str("x")] };
        let r = c.received();
        assert_eq!(r[3].text, "<x>!");
        assert_eq!(r.len(), 6);
        // A missing argument expands to nothing; other `%` stay.
        let p = ServerMsg::Message { kind: MsgKind::Other { kind: tag("K"), args: vec![] }, text: tag("%1 100% %x") };
        assert_eq!(p.received()[1].text, " 100% %x");
    }

    #[test]
    fn untyped_or_lossy_commands() {
        // A plain-string message type stays untyped.
        let m = ServerMsg::parse("ServerMessage", &[Value::str("MsgClientDrop"), Value::empty(), tag("A"), Value::str("1")]).unwrap();
        assert!(matches!(m, ServerMsg::Message { kind: MsgKind::Other { .. }, .. }));
        // A tag where the type has a number: untyped, nothing lost.
        let a = [tag("MsgClientScoreChanged"), Value::empty(), tag("2"), Value::str("2"), Value::str("5")];
        let m = ServerMsg::parse("ServerMessage", &a).unwrap();
        assert!(matches!(&m, ServerMsg::Message { kind: MsgKind::Other { .. }, .. }));
        assert_eq!(trim(&m.lower().args), &a[..]);
        // Extra arguments a typed command has no place for.
        assert_eq!(ServerMsg::parse("MissionStart", &[Value::str("1"), Value::str("x")]), None);
        assert_eq!(ServerMsg::parse("NoSuchCommand", &[]), None);
        assert_eq!(ServerMsg::parse("TTClient", &[Value::str("nope")]), None);
        // Function-name matching: any case parses (to the usual spelling).
        assert_eq!(ServerMsg::parse("gamestart", &[]), Some(ServerMsg::GameStart));
    }

    #[test]
    fn client_commands_have_names() {
        let all = [
            ClientCmd::PhaseAck { phase: 1, seq: "3".into() },
            ClientCmd::PhaseAck { phase: 2, seq: "3".into() },
            ClientCmd::PhaseAck { phase: 3, seq: "3".into() },
            ClientCmd::SpawnPlayer,
            ClientCmd::MessageSent { text: "hi".into() },
            ClientCmd::TeamMessageSent { text: "team".into() },
            ClientCmd::Sad { password: "pw".into() },
            ClientCmd::SadSetPassword { password: "new".into() },
            ClientCmd::Debug { cmd: DebugCmd::CycleTank, args: vec![] },
            ClientCmd::Debug { cmd: DebugCmd::SelectObject, args: vec!["1".into(), "2".into()] },
            ClientCmd::Debug { cmd: DebugCmd::TogglePlayerAI, args: vec![] },
        ];
        for c in all {
            assert!(!c.lower().0.is_empty(), "{c:?} has a name");
            assert_eq!(c.is_debug(), matches!(c, ClientCmd::Debug { .. }));
        }
    }

    #[test]
    fn hud_messages_round_trip() {
        use HudMsg::*;
        let all = [ClockShowTime("-1".into()), ClockSetTime("300".into()), ClockVisible("0".into()), BrainsMax("4".into()), BrainsCount("1".into()), BrainsVisible("1".into()), Score("1250".into()), Lives("2".into()), Sound("SPQuick".into()), TargetPracticeDone, SoloGameOver];
        for m in all {
            let (kind, args) = m.encode();
            assert_eq!(HudMsg::decode(kind, &args), Some(m));
        }
        assert_eq!(HudMsg::decode("nope", &[]), None);
    }
}
