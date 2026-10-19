//! The message lines over the game: a message is cut into lines of at most
//! 50 characters (a word that would be cut moves to the next line), the
//! lines appear 100 ms apart at the bottom, push the older ones up a line,
//! fade out after 8 s over 4 s and go 100 ms later.
//!
//! The numbers are those of `PlayGui.gui`'s `messagesHud` (extent height
//! 200, line height 20, fade delay 8000, fade duration 4000; direction
//! "up") and the arithmetic is the scripts' and `GuiControl`'s
//! (`animateAlpha` in steps of the GUI tick, values stored as `%g` text),
//! so the lines' alpha and slot are what the HUD showed before.

use super::text::{f32r, fs, lf, strip_chars, strlen, strrchr, strstr, sub_str};

/// `messagesHud.extent` height.
const EXTENT_Y: f64 = 200.0;
/// `messagesHud.lineHeight`.
const LINE_HEIGHT: f64 = 20.0;
/// `messagesHud.fadeDelay` / `fadeDuration` (ms).
pub const FADE_DELAY: u32 = 8000;
pub const FADE_DURATION: u32 = 4000;
/// A line leaves `messagesHud` this long after it appeared (ms).
pub const LINE_LIFETIME: u32 = FADE_DELAY + FADE_DURATION + 100;
/// Line length of the wrap.
const LINE: usize = 50;

/// Characters `messageScroll_addMessage1` strips (colour and tag marks).
const STRIP: &str = "\u{10}\u{11}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{b}\u{c}\u{e}";

/// One line on screen (a `GuiTextCtrl` of `messagesHud` before).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub id: u64,
    pub text: String,
    /// A chat line (`ttChatHudProfile`) rather than a server message.
    pub chat: bool,
    /// The control's `alpha` field (`%g` text).
    alpha: String,
    /// The control's `slot` (lines above the bottom).
    slot: f64,
    /// Its `position` y.
    y: f64,
    /// `animateAlpha` time so far (ms), `None` once the fade is complete.
    fade_elapsed: Option<u32>,
}

impl Line {
    /// The fade, 0..1, as the HUD reads the field.
    pub fn alpha(&self) -> f32 {
        super::text::atof(&self.alpha) as f32
    }

    /// Lines above the bottom of the message area (as the HUD computed it
    /// from the position).
    pub fn slot(&self) -> f32 {
        let ext_y = EXTENT_Y as f32;
        (ext_y - 10.0 - self.y as f32) / LINE_HEIGHT as f32
    }
}

/// `messagesHud`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageHud {
    /// `messagesHud.visible` (`hide()` was only used by the parental filter).
    pub visible: bool,
    /// In `messagesHud`'s order (oldest first).
    pub lines: Vec<Line>,
    next_id: u64,
}

impl Default for MessageHud {
    fn default() -> Self {
        MessageHud { visible: true, lines: Vec::new(), next_id: 1 }
    }
}

/// `messageScroll_addMessage1`'s cut: the lines of `message` in order.
pub fn wrap(message: &str) -> Vec<String> {
    #[derive(Clone)]
    enum V {
        S(String),
        I(u32),
        F(f32),
    }
    impl V {
        fn f(&self) -> f64 {
            match self {
                V::S(s) => lf(s),
                V::I(i) => *i as f32 as f64,
                V::F(f) => *f as f64,
            }
        }
        fn s(&self) -> String {
            match self {
                V::S(s) => s.clone(),
                V::I(i) => (*i as i32).to_string(),
                V::F(f) => fs(*f as f64),
            }
        }
    }
    let mut message = strip_chars(message, STRIP);
    let mut length = V::S(strlen(&message).to_string());
    let line = V::I(LINE as u32);
    let mut out = Vec::new();
    while length.f() > 0.0 {
        let mut part = sub_str(&message, "0", &line.s());
        let cur;
        if length.f() < line.f() {
            cur = V::S(length.s());
            length = V::I(0);
        } else {
            cur = V::S(line.s());
            length = V::F((length.f() - line.f()) as f32);
        }
        message = sub_str(&message, &cur.s(), &length.s());
        if length.f() > 0.0 {
            let last_word = strrchr(&part, ' ');
            let word_len = strlen(&last_word).to_string();
            if lf(&word_len) > 1.0 {
                let w = sub_str(&last_word, "1", &fs(lf(&word_len) - 1.0));
                message = format!("{w}{message}");
                length = V::F((length.f() + lf(&word_len)) as f32);
                part = sub_str(&part, "0", &fs(cur.f() - lf(&word_len)));
            }
        }
        out.push(part);
    }
    out
}

/// The delay of the `i`-th line of a message (ms): `schedule(%i * 100, ...)`.
pub fn line_delay(i: usize) -> u32 {
    super::text::float_to_u32(super::text::atof(&fs(f32r(i as f64) * 100.0)))
}

/// A message sound (`playMessageSound`): the wave after `~w` in the text.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageSound {
    /// The file: `sound/sfx/<wav>[.wav]`, or `sound/voice/<voice>/<wav>[.wav]`
    /// with a voice (the game's `~/data/sound/[voice/<voice>/]`).
    pub file: String,
    /// `%pitch`: the given text, or `None` for 1.0 (out of 0.5..2).
    pub pitch: Option<String>,
}

/// `playMessageSound(message, voice, pitch)` without the playing: the
/// sound to play and the text without the `~w...` part.
pub fn message_sound(message: &str, voice: &str, pitch: &str) -> (Option<MessageSound>, String) {
    let start = strstr(message, "~w");
    if start == -1 {
        return (None, message.to_string());
    }
    let wav = sub_str(message, &fs(start as f64 + 2.0), "1000");
    let mut file = if voice.is_empty() { format!("sound/sfx/{wav}") } else { format!("sound/voice/{voice}/{wav}") };
    if strstr(&file, ".wav") as f64 != strlen(&file) as f64 - 4.0 {
        file += ".wav";
    }
    let p = lf(pitch);
    let pitch = (!(p < 0.5 || p > 2.0)).then(|| pitch.to_string());
    let text = sub_str(message, "0", &start.to_string());
    (Some(MessageSound { file, pitch }), text)
}

impl MessageHud {
    /// `messageScroll_addMessage2`: a line appears at the bottom; every
    /// line moves up one slot. Returns the new line's id (its removal is
    /// due [`LINE_LIFETIME`] later).
    pub fn add_line(&mut self, text: &str, chat: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.lines.push(Line { id, text: text.to_string(), chat, alpha: "1.0".into(), slot: 0.0, y: EXTENT_Y, fade_elapsed: Some(0) });
        for l in &mut self.lines {
            l.slot = lf(&fs(l.slot + 1.0));
            let y = f32r(EXTENT_Y - LINE_HEIGHT * l.slot - 10.0);
            // `animatePosition(x, y, 0, 0, rate)`: immediate.
            l.y = super::text::atof(&fs(super::text::atof(&super::text::f32s(y))));
        }
        id
    }

    /// The scheduled `messagesHud.remove(line)`.
    pub fn remove_line(&mut self, id: u64) {
        self.lines.retain(|l| l.id != id);
    }

    /// Advances the fades by `ms` (a GUI tick).
    pub fn animate(&mut self, ms: u32) {
        for l in &mut self.lines {
            let Some(e) = l.fade_elapsed.as_mut() else { continue };
            *e += ms;
            if *e < FADE_DELAY {
                continue;
            }
            let t = ((*e - FADE_DELAY) as f64 / FADE_DURATION as f64).min(1.0);
            let from = 1.0;
            l.alpha = fs(from + (0.0 - from) * t);
            if t >= 1.0 {
                l.fade_elapsed = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_words() {
        let long = "this is a rather long chat line that has to be wrapped over several lines of the messages hud";
        let lines = wrap(long);
        assert_eq!(lines, vec!["this is a rather long chat line that has to be", "wrapped over several lines of the messages hud"]);
        assert_eq!(wrap("short"), vec!["short"]);
        assert!(wrap("").is_empty());
        let unbroken = "x".repeat(120);
        assert_eq!(wrap(&unbroken).iter().map(|l| l.len()).collect::<Vec<_>>(), vec![50, 50, 20]);
        assert_eq!(line_delay(3), 300);
    }

    #[test]
    fn lines_slide_and_fade() {
        let mut m = MessageHud::default();
        let a = m.add_line("one", false);
        m.add_line("two", true);
        assert_eq!(m.lines.iter().map(|l| l.slot()).collect::<Vec<_>>(), vec![2.0, 1.0]);
        m.animate(8032);
        assert_eq!(m.lines[0].alpha(), 0.992);
        m.animate(4000);
        assert_eq!(m.lines[0].alpha(), 0.0);
        m.remove_line(a);
        assert_eq!(m.lines.len(), 1);
    }

    #[test]
    fn message_sounds() {
        let (s, t) = message_sound("server says hi~wdryfire.wav", "", "");
        assert_eq!(t, "server says hi");
        assert_eq!(s.unwrap().file, "sound/sfx/dryfire.wav");
        let (s, _) = message_sound("beep~wdryfire", "default", "1.5");
        let s = s.unwrap();
        assert_eq!((s.file.as_str(), s.pitch.as_deref()), ("sound/voice/default/dryfire.wav", Some("1.5")));
        assert_eq!(message_sound("x~wy", "", "3").0.unwrap().pitch, None);
        assert_eq!(message_sound("plain", "", ""), (None, "plain".to_string()));
    }
}
