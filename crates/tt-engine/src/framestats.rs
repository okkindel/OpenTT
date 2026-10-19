//! Opt-in frame-time instrumentation of the window loop (`--frame-stats` or
//! `TT_FRAME_STATS=1`).
//!
//! Every frame is split into stages (milliseconds of wall time):
//!
//! * `events` – window/device event handlers since the previous frame
//!   (keyboard, mouse motion, egui input), with the number of events;
//! * `idle` – the rest of the gap between frames (the event loop waiting);
//! * `step` – sim ticks (with `script`/`sim` sub-times and the tick count)
//!   plus GUI animation and scene rebuilds;
//! * `acquire` – `get_current_texture` (blocks while the swapchain is full:
//!   vsync / GPU back-pressure);
//! * `draw` – CPU side of drawing (3D encode, effects, GUI, menu, submit);
//! * `present` – `SurfaceTexture::present`;
//! * `other` – the rest (video mode, capture, title, audio listener).
//!
//! Frames longer than the spike threshold (default 20 ms, `TT_FRAME_SPIKE_MS`)
//! are printed at once with their breakdown; every 5 s a summary prints the
//! average / p50 / p99 / max frame time, the tick count distribution and the
//! jitter of the real time between ticks. Output goes to stderr, prefixed
//! `[frame]`.

use crate::frontend::Perf;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// Stage times of one frame (ms).
#[derive(Clone, Copy, Default, Debug)]
pub struct Sample {
    /// Wall time from the previous frame's start to this one's.
    pub frame: f64,
    pub events: f64,
    pub n_events: u32,
    pub idle: f64,
    pub step: f64,
    pub ticks: u32,
    pub script: f64,
    pub sim: f64,
    pub acquire: f64,
    pub draw: f64,
    pub present: f64,
    pub other: f64,
}

/// Smoothness of one moving value across frames: its per-frame velocity
/// `v` (units per second) and how much `v` changes from frame to frame.
/// Motion drawn only at tick boundaries (stepping) gives `Σ|Δv| / Σ|v| ≈ 2`;
/// interpolated motion gives a small ratio.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Probe {
    last: Option<(glam::Vec3, f64)>,
    last_v: Option<glam::Vec3>,
    sum_v: f64,
    sum_dv: f64,
    frames: u32,
}

impl Probe {
    /// `Σ|Δv| / Σ|v|` so far (the "judder" of the summary).
    pub(crate) fn judder(&self) -> f64 {
        if self.sum_v > 1e-9 { self.sum_dv / self.sum_v } else { 0.0 }
    }

    pub(crate) fn add(&mut self, x: glam::Vec3, t_ms: f64) {
        if let Some((px, pt)) = self.last {
            let dt = (t_ms - pt) / 1000.0;
            if dt > 0.0 {
                let v = (x - px) / dt as f32;
                // A teleport (respawn) is not motion.
                if v.length() < 500.0 {
                    if let Some(pv) = self.last_v {
                        self.sum_dv += (v - pv).length() as f64;
                        self.sum_v += v.length() as f64;
                        self.frames += 1;
                    }
                    self.last_v = Some(v);
                } else {
                    self.last_v = None;
                }
            }
        }
        self.last = Some((x, t_ms));
    }

    fn describe(&self, name: &str) -> String {
        if self.frames == 0 {
            return String::new();
        }
        format!(" {name}: speed {:.2}/s judder {:.2}", self.sum_v / self.frames as f64, self.judder())
    }
}

/// Timer for one frame and the running statistics.
pub struct FrameStats {
    pub enabled: bool,
    spike_ms: f64,
    /// Event-handler time and count since the last frame (shared with
    /// [`EventGuard`]s so a handler can time itself while borrowing `self`).
    events: Rc<Cell<(f64, u32)>>,
    frame_start: Option<Instant>,
    prev_frame_end: Option<Instant>,
    mark: Instant,
    cur: Sample,
    window: Vec<Sample>,
    /// Real time (ms since start) at which each tick of the window ran.
    tick_times: Vec<f64>,
    start: Instant,
    last_report: Instant,
    frames_total: u64,
    probes: Vec<(&'static str, Probe)>,
    /// Sample counts per [`Perf`] stage at the last frame, and this frame's
    /// new stage times (shown with spikes).
    perf_seen: Vec<(&'static str, u64)>,
    perf_frame: Vec<(&'static str, f64)>,
}

/// Adds the time it lives to the frame's event time.
pub struct EventGuard {
    cell: Option<Rc<Cell<(f64, u32)>>>,
    t: Instant,
}

impl Drop for EventGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.cell {
            let (ms, n) = c.get();
            c.set((ms + self.t.elapsed().as_secs_f64() * 1000.0, n + 1));
        }
    }
}

const REPORT_EVERY_S: f64 = 5.0;

impl FrameStats {
    /// Enabled by `flag` or the `TT_FRAME_STATS` environment variable
    /// (anything but empty/`0`).
    pub fn new(flag: bool) -> FrameStats {
        let env = std::env::var("TT_FRAME_STATS").map(|v| !v.is_empty() && v != "0").unwrap_or(false);
        let spike_ms = std::env::var("TT_FRAME_SPIKE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(20.0);
        let now = Instant::now();
        FrameStats {
            enabled: flag || env,
            spike_ms,
            events: Rc::new(Cell::new((0.0, 0))),
            frame_start: None,
            prev_frame_end: None,
            mark: now,
            cur: Sample::default(),
            window: Vec::with_capacity(4096),
            tick_times: Vec::with_capacity(512),
            start: now,
            last_report: now,
            frames_total: 0,
            probes: Vec::new(),
            perf_seen: Vec::new(),
            perf_frame: Vec::new(),
        }
    }

    /// Times an event handler (a no-op when disabled).
    pub fn event(&self) -> EventGuard {
        EventGuard { cell: self.enabled.then(|| self.events.clone()), t: Instant::now() }
    }

    pub fn begin_frame(&mut self) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let (ev, n) = self.events.replace((0.0, 0));
        self.cur = Sample { events: ev, n_events: n, ..Default::default() };
        if let Some(e) = self.prev_frame_end {
            self.cur.idle = ((now - e).as_secs_f64() * 1000.0 - ev).max(0.0);
        }
        self.frame_start = Some(now);
        self.mark = now;
    }

    /// Ends the current stage: the time since the previous mark.
    fn lap(&mut self) -> f64 {
        let now = Instant::now();
        let ms = (now - self.mark).as_secs_f64() * 1000.0;
        self.mark = now;
        ms
    }

    pub fn step_done(&mut self, ticks: u32, script_ms: f64, sim_ms: f64) {
        if !self.enabled {
            return;
        }
        self.cur.step = self.lap();
        self.cur.ticks = ticks;
        self.cur.script = script_ms;
        self.cur.sim = sim_ms;
        let t = (self.mark - self.start).as_secs_f64() * 1000.0;
        for _ in 0..ticks {
            self.tick_times.push(t);
        }
    }

    pub fn other_done(&mut self) {
        if self.enabled {
            self.cur.other += self.lap();
        }
    }

    pub fn acquire_done(&mut self) {
        if self.enabled {
            self.cur.acquire = self.lap();
        }
    }

    pub fn draw_done(&mut self) {
        if self.enabled {
            self.cur.draw = self.lap();
        }
    }

    pub fn present_done(&mut self) {
        if self.enabled {
            self.cur.present = self.lap();
        }
    }

    pub fn end_frame(&mut self) {
        if !self.enabled {
            return;
        }
        self.cur.other += self.lap();
        let now = self.mark;
        // This frame's work plus the gap (events, idle) before it.
        if let Some(e) = self.prev_frame_end {
            self.cur.frame = (now - e).as_secs_f64() * 1000.0;
        }
        self.prev_frame_end = Some(now);
        self.frames_total += 1;
        let s = self.cur;
        // The first frame has no previous one to measure from.
        if s.frame > 0.0 {
            if s.frame > self.spike_ms {
                let stages: String = self.perf_frame.iter().map(|(k, v)| format!(" {k}={v:.1}")).collect();
                eprintln!("[frame] spike t={:.1}s {}{}", (now - self.start).as_secs_f64(), describe(&s), if stages.is_empty() { String::new() } else { format!(" | stages ≥0.5 ms:{stages}") });
            }
            self.window.push(s);
        }
        if (now - self.last_report).as_secs_f64() >= REPORT_EVERY_S && !self.window.is_empty() {
            eprintln!("[frame] {}", self.summary());
            self.window.clear();
            self.tick_times.clear();
            for (_, p) in &mut self.probes {
                *p = Probe { last: p.last, last_v: p.last_v, ..Default::default() };
            }
            self.last_report = now;
        }
    }

    /// Picks up the [`Perf`] stage times added during this frame.
    pub fn perf(&mut self, perf: &Perf) {
        if !self.enabled {
            return;
        }
        self.perf_frame.clear();
        for (k, n, last) in perf.latest() {
            let seen = match self.perf_seen.iter_mut().find(|(s, _)| *s == k) {
                Some((_, c)) => c,
                None => {
                    self.perf_seen.push((k, 0));
                    &mut self.perf_seen.last_mut().unwrap().1
                }
            };
            if n > *seen && last >= 0.5 {
                self.perf_frame.push((k, last));
            }
            *seen = n;
        }
    }

    /// Records values the player sees move this frame (see [`Probe`]), at
    /// the frame's start time (when the sim state it shows was sampled).
    pub fn probe(&mut self, values: &[(&'static str, glam::Vec3)]) {
        let Some(start) = self.frame_start.filter(|_| self.enabled) else { return };
        let t = (start - self.start).as_secs_f64() * 1000.0;
        for &(name, v) in values {
            match self.probes.iter_mut().find(|(n, _)| *n == name) {
                Some((_, p)) => p.add(v, t),
                None => {
                    let mut p = Probe::default();
                    p.add(v, t);
                    self.probes.push((name, p));
                }
            }
        }
    }

    /// Statistics of the frames since the last summary.
    pub fn summary(&self) -> String {
        let mut s = summarize(&self.window, &self.tick_times, (self.mark - self.start).as_secs_f64());
        let probes: String = self.probes.iter().map(|(n, p)| p.describe(n)).collect();
        if !probes.is_empty() {
            s += " | motion";
            s += &probes;
        }
        s
    }
}

fn describe(s: &Sample) -> String {
    format!(
        "{:.1} ms: events {:.2} ({} ev) idle {:.2} step {:.2} ({} ticks, script {:.2} sim {:.2}) acquire {:.2} draw {:.2} present {:.2} other {:.2}",
        s.frame, s.events, s.n_events, s.idle, s.step, s.ticks, s.script, s.sim, s.acquire, s.draw, s.present, s.other
    )
}

/// avg / p50 / p99 / max frame time, average per stage, ticks per frame and
/// the jitter of the real-time spacing of ticks (ideal: 32 ms each).
pub fn summarize(frames: &[Sample], tick_times: &[f64], t_s: f64) -> String {
    if frames.is_empty() {
        return "no frames".into();
    }
    let n = frames.len() as f64;
    let mut ft: Vec<f64> = frames.iter().map(|s| s.frame).collect();
    ft.sort_by(|a, b| a.total_cmp(b));
    let pct = |q: f64| ft[((ft.len() as f64 * q) as usize).min(ft.len() - 1)];
    let avg = |f: fn(&Sample) -> f64| frames.iter().map(f).sum::<f64>() / n;
    let mut tick_hist = [0u32; 4];
    for s in frames {
        tick_hist[(s.ticks as usize).min(3)] += 1;
    }
    // Real-time spacing of consecutive ticks; several ticks in one frame
    // count as one gap at the frame and zero gaps inside it.
    let gaps: Vec<f64> = tick_times.windows(2).map(|w| w[1] - w[0]).collect();
    let tick_jitter = if gaps.is_empty() {
        String::from("-")
    } else {
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        let sd = (gaps.iter().map(|g| (g - mean) * (g - mean)).sum::<f64>() / gaps.len() as f64).sqrt();
        let max = gaps.iter().copied().fold(0.0, f64::max);
        format!("mean {mean:.2} sd {sd:.2} max {max:.1}")
    };
    format!(
        "t={t_s:.0}s {} frames ({:.0} fps): frame avg {:.2} p50 {:.2} p99 {:.2} max {:.2} ms, >20ms {}, >33ms {} | avg events {:.2} ({:.1} ev) idle {:.2} step {:.2} acquire {:.2} draw {:.2} present {:.2} other {:.2} | ticks/frame 0:{} 1:{} 2:{} 3+:{} | tick gap ms {tick_jitter}",
        frames.len(),
        1000.0 / avg(|s| s.frame),
        avg(|s| s.frame),
        pct(0.5),
        pct(0.99),
        ft[ft.len() - 1],
        ft.iter().filter(|f| **f > 20.0).count(),
        ft.iter().filter(|f| **f > 33.3).count(),
        avg(|s| s.events),
        frames.iter().map(|s| s.n_events as f64).sum::<f64>() / n,
        avg(|s| s.idle),
        avg(|s| s.step),
        avg(|s| s.acquire),
        avg(|s| s.draw),
        avg(|s| s.present),
        avg(|s| s.other),
        tick_hist[0],
        tick_hist[1],
        tick_hist[2],
        tick_hist[3],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_ticks_and_jitter() {
        let frames: Vec<Sample> = (0..10).map(|i| Sample { frame: 16.0 + i as f64, ticks: (i % 2) as u32, ..Default::default() }).collect();
        let s = summarize(&frames, &[0.0, 32.0, 64.0, 96.0], 1.0);
        assert!(s.contains("ticks/frame 0:5 1:5 2:0 3+:0"), "{s}");
        assert!(s.contains("tick gap ms mean 32.00 sd 0.00 max 32.0"), "{s}");
        assert!(s.contains("max 25.00 ms"), "{s}");
    }

    #[test]
    fn probe_tells_stepped_from_smooth_motion() {
        let (mut smooth, mut stepped) = (Probe::default(), Probe::default());
        for i in 0..400 {
            let t = i as f64 * 2.0;
            // 10 units/s.
            smooth.add(glam::Vec3::X * (t * 0.01) as f32, t);
            // The same, moving only every 32 ms (16 frames).
            stepped.add(glam::Vec3::X * ((i / 16) as f32 * 0.32), t);
        }
        let j = |p: &Probe| p.sum_dv / p.sum_v;
        assert!(j(&smooth) < 0.01, "{}", j(&smooth));
        assert!(j(&stepped) > 1.8, "{}", j(&stepped));
    }

    #[test]
    fn disabled_is_inert() {
        let mut f = FrameStats { enabled: false, ..FrameStats::new(false) };
        f.begin_frame();
        drop(f.event());
        f.step_done(2, 0.0, 0.0);
        f.end_frame();
        assert_eq!(f.frames_total, 0);
        assert_eq!(f.events.get().1, 0);
    }
}
