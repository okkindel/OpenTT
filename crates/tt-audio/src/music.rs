//! Each mission's `MissionGroup.music_track` names one of them, the server
//! sends it with `MissionStartPhase1` and the client calls
//! `MusicPlayer.setTrack(track)`; `MusicPlayer.stop()` runs on disconnect.
//!
//! A track is a set of short Ogg segments (`mood<M>[0..2]`) and up to three
//! sequences per mood (`sequences<M>[i]`, a space-separated list of 1-based
//! segment numbers; `0` is the `urgent_mood` slot, skipped since the urgent
//! flag is never set). The player picks a random sequence and plays its
//! segments back to back on one streaming `AudioMusic` source: whenever the
//! Ogg stream reaches its end, the engine asks the player for the next
//! segment and switches the stream to it (or rewinds when it is the same
//! file). At the end of a sequence a new random sequence is chosen, so the
//! music never stops.
//!
//! The executable also has a mood machinery (three smoothed levels and an
//! urgent flag choosing among `num_moods` moods), but its inputs are globals
//! that nothing in the shipped executable ever sets, and every shipped track
//! has `num_moods = 1`: the game always plays mood 1 (the intensity stays
//! 0), which is all that is reproduced here.

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

#[derive(Debug, Clone)]
pub struct MusicPlayer {
    track: Option<MusicTrack>,
    cur_mood: i32,
    seq: i32,
    step: i32,
    current: String,
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
            cur_mood: 0,
            seq: 0,
            step: 0,
            current: String::new(),
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

    /// Engine at intensity 0: mood 1 (-1 without moods).
    fn choose_mood(&self) -> i32 {
        match &self.track {
            Some(t) if t.num_moods != 0 => 1,
            _ => -1,
        }
    }

    /// Engine: random 1-based sequence of the current mood.
    fn choose_sequence(&mut self) -> i32 {
        let Some(t) = &self.track else { return -1 };
        let count = t.sequence_count(self.cur_mood.clamp(1, 3) as usize) as i32;
        self.rng.rand_i(1, count.max(1))
    }

    /// Engine (`MusicDef`): file for token `step` of sequence `seq` of mood
    /// `mood`; empty at the end of the sequence.
    fn get_file(t: &MusicTrack, mood: i32, seq: i32, mut step: usize) -> String {
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
                let f = Self::get_file(t, mood, self.seq, self.step as usize);
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
            Some(t) => Self::get_file(t, mood, self.seq, 0),
            None => String::new(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lush() -> MusicTrack {
        let mut m = MusicTrack { name: "Lush".into(), num_moods: 1, ..Default::default() };
        m.moods[0] = ["g1".into(), "g2".into(), "g3".into()];
        m.sequences[0][0] = "1 1 2 3 1 1 2 3".into();
        m.sequences[0][1] = "1 2 2 3 1 2 2 3".into();
        m.urgent_mood = "urgent".into();
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
    fn zero_token_is_skipped() {
        let mut t = lush();
        t.sequences[0] = ["0 2".into(), String::new(), String::new()];
        assert_eq!(MusicPlayer::get_file(&t, 1, 1, 0), "g2");
        assert_eq!(MusicPlayer::get_file(&t, 1, 1, 2), "");
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
