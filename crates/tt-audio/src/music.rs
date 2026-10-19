//! Each mission's `MissionGroup.music_track` names one of them, the server
//! sends it with `MissionStartPhase1` and the client calls
//! `MusicPlayer.setTrack(track)`; `MusicPlayer.stop()` runs on disconnect.
//!
//! A track is a set of short Ogg segments (`mood<M>[0..2]`) and up to three
//! sequences per mood (`sequences<M>[i]`, a space-separated list of 1-based
//! segment numbers, `0` = the `urgent_mood` segment when the "urgent" flag is
//! set, otherwise skipped). The player picks a mood from its smoothed
//! "intensity", picks a random sequence of that mood and plays its segments
//! back to back on one streaming `AudioMusic` source: whenever the Ogg stream
//! reaches its end, the engine asks the player for the next segment and
//! switches the stream to it (or rewinds when it is the same file). At the
//! end of a sequence a new random sequence is chosen, so the music never
//! stops.
//!
//! The mood inputs (three levels and an urgent flag, smoothed with
//! `smooth1..3`, mixed with `k1..3`, smoothed again with `mood_smooth` and
//! scaled by `mood_scale`) are accumulated in globals that nothing in the
//! shipped executable ever sets (they are only reset), and every shipped
//! track has `num_moods = 1`. In practice the game therefore always plays
//! mood 1 with random sequences. The mood machinery is reproduced anyway
//! ([`MusicPlayer::feed_mood`] / [`MusicPlayer::update`]).

use crate::db::MusicTrack;

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn from_time() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Rng::new(t)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    pub fn rand_i(&mut self, i: i32, j: i32) -> i32 {
        if j < i {
            return i;
        }
        i + (self.next_u32() % (j - i + 1) as u32) as i32
    }
}

#[derive(Debug, Clone, Default)]
struct MoodInput {
    levels: [f32; 3],
    urgent: bool,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub struct MusicPlayer {
    track: Option<MusicTrack>,
    levels: [f32; 3],
    urgent: bool,
    cur_mood: i32,
    intensity: f32,
    seq: i32,
    step: i32,
    current: String,
    input: MoodInput,
    rng: Rng,
}

impl Default for MusicPlayer {
    fn default() -> Self {
        Self::with_rng(Rng::from_time())
    }
}

impl MusicPlayer {
    pub fn with_rng(rng: Rng) -> Self {
        MusicPlayer {
            track: None,
            levels: [0.0; 3],
            urgent: false,
            cur_mood: 0,
            intensity: 0.0,
            seq: 0,
            step: 0,
            current: String::new(),
            input: MoodInput::default(),
            rng,
        }
    }

    pub fn track(&self) -> Option<&MusicTrack> {
        self.track.as_ref()
    }

    /// The segment file currently selected (asset path, `sound/music/...`).
    pub fn current_file(&self) -> &str {
        &self.current
    }

    pub fn current_mood(&self) -> i32 {
        self.cur_mood
    }

    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// Minus the audio side: selects `track` and returns its first
    /// segment file. The caller plays it on a streaming `AudioMusic`
    /// source.
    pub fn set_track(&mut self, track: MusicTrack) -> String {
        self.stop();
        self.track = Some(track);
        self.next_segment(true);
        self.current.clone()
    }

    /// `MusicPlayer::stop`: forgets the track (the caller stops the source).
    pub fn stop(&mut self) {
        self.track = None;
        self.current.clear();
    }

    /// Engine callback when the music stream reaches its end: advances to the
    /// next segment and returns it if it differs from the one that just
    /// ended (`None` = replay the same file).
    pub fn on_segment_end(&mut self) -> Option<String> {
        self.track.as_ref()?;
        let old = std::mem::take(&mut self.current);
        self.current = old.clone();
        self.next_segment(false);
        (self.current != old).then(|| self.current.clone())
    }

    /// Accumulates mood inputs for the next [`update`](Self::update).
    pub fn feed_mood(&mut self, levels: [f32; 3], urgent: bool) {
        for (acc, v) in self.input.levels.iter_mut().zip(levels) {
            *acc += v;
        }
        self.input.urgent |= urgent;
        self.input.dirty = true;
    }

    /// Per-frame mood smoothing. No-op unless inputs were fed since the
    /// last call.
    pub fn update(&mut self) {
        if !self.input.dirty {
            return;
        }
        let g = self.input.levels;
        match &self.track {
            Some(t) => {
                for ((level, smooth), input) in self.levels.iter_mut().zip(t.smooth).zip(g) {
                    *level = *level * smooth + (1.0 - smooth) * input;
                }
                let mix = (t.k[0] * self.levels[0] + t.k[1] * self.levels[1] + t.k[2] * self.levels[2]).clamp(0.0, 1.0);
                self.intensity = self.intensity * t.mood_smooth + (1.0 - t.mood_smooth) * t.mood_scale * mix;
            }
            None => {
                self.levels = g;
                self.intensity = (g[0] + g[1] + g[2]) * (1.0 / 3.0);
            }
        }
        self.urgent = self.input.urgent;
        self.input = MoodInput::default();
    }

    /// Engine: mood (1-based) for the current intensity.
    fn choose_mood(&self) -> i32 {
        let Some(t) = &self.track else { return -1 };
        let n = t.num_moods;
        if n == 0 {
            return -1;
        }
        // Reproduces the executable: `ftol((1.0 / n) * intensity)`.
        let m = ((1.0 / n as f32) * self.intensity) as i32;
        m.min(n - 1).max(0) + 1
    }

    /// Engine: random 1-based sequence of the current mood.
    fn choose_sequence(&mut self) -> i32 {
        let Some(t) = &self.track else { return -1 };
        let count = t.sequence_count(self.cur_mood.clamp(1, 3) as usize) as i32;
        self.rng.rand_i(1, count.max(1))
    }

    /// Engine (`MusicDef`): file for token `step` of sequence `seq` of mood
    /// `mood`; empty at the end of the sequence.
    fn get_file(t: &MusicTrack, mood: i32, seq: i32, mut step: usize, urgent: bool) -> String {
        let m = match mood {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        let Some(seq_str) = usize::try_from(seq - 1).ok().and_then(|i| t.sequences[m].get(i)) else {
            return String::new();
        };
        loop {
            let Some(tok) = seq_str.split(' ').filter(|s| !s.is_empty()).nth(step) else { return String::new() };
            let v: i32 = tok.trim().parse().unwrap_or(0);
            if v == 0 {
                if urgent {
                    return t.urgent_mood.clone();
                }
                step += 1;
                continue;
            }
            return usize::try_from(v - 1).ok().and_then(|i| t.moods[m].get(i)).cloned().unwrap_or_default();
        }
    }

    fn next_segment(&mut self, reset: bool) {
        if reset {
            self.cur_mood = -1;
        }
        let mood = self.choose_mood();
        if mood == self.cur_mood {
            self.step += 1;
            if let Some(t) = &self.track {
                let f = Self::get_file(t, mood, self.seq, self.step as usize, self.urgent);
                if !f.is_empty() {
                    self.current = f;
                    return;
                }
            }
        }
        self.cur_mood = mood;
        self.seq = self.choose_sequence();
        self.step = 0;
        self.current = match &self.track {
            Some(t) => Self::get_file(t, mood, self.seq, 0, self.urgent),
            None => String::new(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lush() -> MusicTrack {
        let mut m = MusicTrack { name: "Lush".into(), num_moods: 1, mood_scale: 1.0, mood_smooth: 0.5, ..Default::default() };
        m.moods[0] = ["g1".into(), "g2".into(), "g3".into()];
        m.sequences[0][0] = "1 1 2 3 1 1 2 3".into();
        m.sequences[0][1] = "1 2 2 3 1 2 2 3".into();
        m.urgent_mood = "urgent".into();
        m.k = [0.33; 3];
        m.smooth = [0.5; 3];
        m
    }

    #[test]
    fn plays_whole_sequences_back_to_back() {
        let mut p = MusicPlayer::with_rng(Rng::new(7));
        let mut files = vec![p.set_track(lush())];
        for _ in 0..63 {
            let next = p.on_segment_end();
            files.push(next.unwrap_or_else(|| files.last().unwrap().clone()));
        }
        // 64 segments = 8 complete 8-step sequences, each one of the two.
        for chunk in files.chunks(8) {
            let s: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
            assert!(
                s == ["g1", "g1", "g2", "g3", "g1", "g1", "g2", "g3"] || s == ["g1", "g2", "g2", "g3", "g1", "g2", "g2", "g3"],
                "{s:?}"
            );
        }
        assert_eq!(p.current_mood(), 1);
    }

    #[test]
    fn same_file_means_replay() {
        let mut p = MusicPlayer::with_rng(Rng::new(1));
        let mut t = lush();
        t.sequences[0] = ["1 1".into(), String::new(), String::new()];
        assert_eq!(p.set_track(t), "g1");
        assert_eq!(p.on_segment_end(), None);
        assert_eq!(p.current_file(), "g1");
    }

    #[test]
    fn zero_token_is_urgent_slot() {
        let mut t = lush();
        t.sequences[0] = ["0 2".into(), String::new(), String::new()];
        assert_eq!(MusicPlayer::get_file(&t, 1, 1, 0, false), "g2");
        assert_eq!(MusicPlayer::get_file(&t, 1, 1, 0, true), "urgent");
        assert_eq!(MusicPlayer::get_file(&t, 1, 1, 2, false), "");
    }

    #[test]
    fn mood_smoothing() {
        let mut p = MusicPlayer::with_rng(Rng::new(3));
        p.set_track(lush());
        p.update();
        assert_eq!(p.intensity(), 0.0);
        p.feed_mood([1.0, 1.0, 1.0], false);
        p.update();
        // levels 0.5 each, mix 0.495, intensity 0.5 * 0.495
        assert!((p.intensity() - 0.2475).abs() < 1e-5, "{}", p.intensity());
        assert_eq!(p.current_mood(), 1);
    }

    #[test]
    fn stop_clears() {
        let mut p = MusicPlayer::with_rng(Rng::new(3));
        p.set_track(lush());
        p.stop();
        assert!(p.track().is_none());
        assert_eq!(p.on_segment_end(), None);
    }
}
