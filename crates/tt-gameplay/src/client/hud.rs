//! The smaller HUD parts: their state as the controls held it, changed as
//! the scripts changed it.

use super::text::{atof, atoi, float_to_u32, lf, truthy};

/// A centre or bottom print (`centerPrintDlg` / `bottomPrintDlg`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Print {
    /// The dialog is visible.
    pub active: bool,
    /// `CenterPrintText` / `BottomPrintText` (bottom prints start with
    /// `<just:center>`).
    pub text: String,
    /// The pending clear (`removePrint`).
    pub(super) remove: Option<u64>,
}

/// `schedule(%time * 1000, ...)` of a print: the delay in ms, `None` for
/// no timeout (`%time <= 0`).
pub fn print_timeout(time: &str) -> Option<u32> {
    (lf(time) > 0.0).then(|| float_to_u32((lf(time) * 1000.0).round()))
}

/// `GuiHudClock` (`HudClock`): counts down (`countDown = 1`) from the
/// time last set, in whole seconds; `showTime` makes it visible for the
/// last that many seconds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudClock {
    /// `time` as set (seconds, script text).
    pub time: String,
    /// Sim time (ms) of the last `setTime`.
    pub set_at: u64,
    pub visible: bool,
    /// `showTime` (script text; empty = never set).
    pub show_time: String,
    /// The second shown at the last sample (the tick sound plays on a change).
    pub last_shown: Option<i32>,
}


/// `PlayGui.gui`: `countDown = "1"`, `showHours = "0"`; minutes always
/// shown (mm:ss; the game's `showMinutes = "0"` drew "30" for 0:30).
pub const CLOCK_COUNT_DOWN: bool = true;
pub const CLOCK_SHOW_MINUTES: bool = true;
pub const CLOCK_SHOW_HOURS: bool = false;
/// `ttClockProfile.soundButtonDown`: the tick of each new second.
pub const CLOCK_TICK_SOUND: &str = "AudioTickToc";

impl HudClock {
    /// `setTime(v)` at sim time `now`.
    pub fn set_time(&mut self, v: &str, now: u64) {
        self.time = v.to_string();
        self.set_at = now;
    }

    /// The value in whole seconds at sim time `now`.
    pub fn seconds(&self, now: u64) -> i32 {
        let base = atof(&self.time);
        let at = atof(&self.set_at.to_string());
        let elapsed = (now as f64 - at).max(0.0) / 1000.0;
        let t = if CLOCK_COUNT_DOWN { base - elapsed } else { base + elapsed };
        (t.ceil() as i32).max(0)
    }

    /// One sample of the drawn clock at `now`: the `showTime` rule, then
    /// its text if visible, and whether the tick sound plays (a new second).
    pub fn sample(&mut self, now: u64, visible_rule: impl Fn(i32, bool, i32) -> Option<bool>) -> Option<(i32, bool)> {
        let secs = self.seconds(now);
        if !self.show_time.is_empty() {
            if let Some(v) = visible_rule(secs, CLOCK_COUNT_DOWN, atoi(&self.show_time)) {
                self.visible = v;
            }
        }
        if !self.visible {
            return None;
        }
        let tick = self.last_shown.replace(secs).is_some_and(|old| old != secs);
        Some((secs, tick))
    }
}

/// The solo widgets of `PlayGui`: score digits, lives, brain counter.
#[derive(Debug, Clone, PartialEq)]
pub struct SoloHud {
    /// `SPScoreGui` visible.
    pub score_visible: bool,
    /// The seven digit controls' texts, most significant first.
    pub digits: [String; 7],
    /// `SPLivesGui` visible.
    pub lives_visible: bool,
    /// The four life icons' visibility.
    pub life_icons: [bool; 4],
    /// The " X n" label: visible, text.
    pub lives_count: (bool, String),
    /// `SPBrainCounter` visible.
    pub brains_visible: bool,
    /// `SPBrainCounter.maxBrains` (script text).
    pub max_brains: String,
    /// Its text control ("03 / 10").
    pub brains_text: String,
}

impl Default for SoloHud {
    /// As `PlayGui.gui` declares them.
    fn default() -> Self {
        SoloHud {
            score_visible: true,
            digits: Default::default(),
            lives_visible: true,
            life_icons: [true; 4],
            lives_count: (true, "X 5".into()),
            brains_visible: false,
            max_brains: String::new(),
            brains_text: "0 / 0".into(),
        }
    }
}

impl SoloHud {
    pub fn show_lives(&mut self, lives: &str) {
        let n = lf(lives);
        let on = if n < 5.0 { n } else { 1.0 };
        for (i, icon) in self.life_icons.iter_mut().enumerate() {
            *icon = on > i as f64;
        }
        self.lives_count = (n > 4.0, format!(" X {lives}"));
    }

    /// One digit per control, from the last.
    pub fn set_score(&mut self, score: &str) {
        // `%score` is the argument (a string), then an F32 local.
        let mut int = super::text::lu(score);
        let mut float = lf(score);
        for i in 0..7 {
            self.digits[6 - i] = ((int % 10) as i32).to_string();
            let f = ((float - (int % 10) as f64) / 10.0) as f32;
            float = f as f64;
            int = float_to_u32(f as f64);
        }
    }

    pub fn set_brain_count(&mut self, count: &str) {
        let max = &self.max_brains;
        self.brains_text = if atof(max) > 9.0 && lf(count) < 10.0 { format!("0{count} / {max}") } else { format!("{count} / {max}") };
    }

    /// The score digits shown, if any.
    pub fn score_text(&self) -> Option<String> {
        let d: String = self.digits.concat();
        (!d.trim().is_empty()).then_some(d)
    }
}

/// `setVisible(v)`.
pub fn visible_value(v: &str) -> bool {
    truthy(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_widgets() {
        let mut s = SoloHud::default();
        s.show_lives("2");
        assert_eq!((s.life_icons, s.lives_count.0), ([true, true, false, false], false));
        s.show_lives("7");
        assert_eq!((s.life_icons, s.lives_count.clone()), ([true, false, false, false], (true, " X 7".to_string())));
        assert_eq!(s.score_text(), None);
        s.set_score("1234567");
        assert_eq!(s.score_text().as_deref(), Some("1234567"));
        s.set_score("42");
        assert_eq!(s.score_text().as_deref(), Some("0000042"));
        s.max_brains = "12".into();
        s.set_brain_count("3");
        assert_eq!(s.brains_text, "03 / 12");
        s.set_brain_count("11");
        assert_eq!(s.brains_text, "11 / 12");
    }

    #[test]
    fn clock() {
        let mut c = HudClock::default();
        c.set_time("30", 1000);
        assert_eq!(c.seconds(1000), 30);
        assert_eq!(c.seconds(1500), 30);
        assert_eq!(c.seconds(2000), 29);
        assert_eq!(c.seconds(100_000), 0);
        assert_eq!(print_timeout("1.5"), Some(1500));
        assert_eq!(print_timeout("0"), None);
    }
}
