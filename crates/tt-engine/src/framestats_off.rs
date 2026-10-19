//! Frame statistics without the `dev` feature: the same calls, doing nothing.

use crate::frontend::Perf;

/// What timing an event would hold.
pub struct EventGuard;

pub struct FrameStats {
    pub enabled: bool,
}

impl FrameStats {
    pub fn new(_flag: bool) -> FrameStats {
        FrameStats { enabled: false }
    }
    pub fn event(&self) -> EventGuard {
        EventGuard
    }
    pub fn begin_frame(&mut self) {}
    pub fn step_done(&mut self, _ticks: u32, _script_ms: f64, _sim_ms: f64) {}
    pub fn other_done(&mut self) {}
    pub fn acquire_done(&mut self) {}
    pub fn draw_done(&mut self) {}
    pub fn present_done(&mut self) {}
    pub fn end_frame(&mut self) {}
    pub fn perf(&mut self, _perf: &Perf) {}
    pub fn probe(&mut self, _values: &[(&'static str, glam::Vec3)]) {}
    pub fn summary(&self) -> String {
        String::new()
    }
}
