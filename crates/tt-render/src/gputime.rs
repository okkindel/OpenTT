//! Opt-in GPU pass timing (`TT_GPU_TIMING=1`): timestamps between the
//! passes of a 3D view, read back a few frames later (never stalls) and
//! printed as averages every 300 views on stderr, prefixed `[gpu]`.
//! Needs `TIMESTAMP_QUERY | TIMESTAMP_QUERY_INSIDE_ENCODERS` (see
//! [`wanted_features`]); without them it stays off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_MARKS: u32 = 16;
const RING: usize = 4;
const REPORT_EVERY: u32 = 300;

/// The device features the timer needs, when the user asked for timing.
pub fn wanted_features() -> wgpu::Features {
    if enabled() {
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
    } else {
        wgpu::Features::empty()
    }
}

fn enabled() -> bool {
    std::env::var("TT_GPU_TIMING").is_ok_and(|v| v != "0" && !v.is_empty())
}

struct Slot {
    buf: wgpu::Buffer,
    /// Mapped and ready to read.
    ready: Arc<AtomicBool>,
    /// Waiting for the GPU (submitted, map requested).
    busy: bool,
    labels: Vec<&'static str>,
}

pub(crate) struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    period_ns: f32,
    /// Marks of the view being encoded (`None`: not timing this one).
    cur: Option<(usize, Vec<&'static str>)>,
    /// Accumulated ms per stage (between consecutive marks) and view count.
    acc: Vec<(&'static str, f64, f64)>,
    views: u32,
}

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !enabled() || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
            return None;
        }
        let size = MAX_MARKS as u64 * 8;
        Some(GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gpu timer"), ty: wgpu::QueryType::Timestamp, count: MAX_MARKS }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu timer resolve"), size, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            slots: (0..RING)
                .map(|_| Slot {
                    buf: device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu timer read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                    ready: Arc::new(AtomicBool::new(false)),
                    busy: false,
                    labels: Vec::new(),
                })
                .collect(),
            period_ns: queue.get_timestamp_period(),
            cur: None,
            acc: Vec::new(),
            views: 0,
        })
    }

    /// Starts timing a view (if a read-back slot is free) with a first mark.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder) {
        device.poll(wgpu::Maintain::Poll);
        self.collect();
        self.cur = self.slots.iter().position(|s| !s.busy).map(|i| (i, Vec::new()));
        self.mark(enc, "start");
    }

    /// A timestamp after everything encoded so far; the time since the
    /// previous mark is reported as `label`.
    pub(crate) fn mark(&mut self, enc: &mut wgpu::CommandEncoder, label: &'static str) {
        if let Some((_, labels)) = self.cur.as_mut() {
            if (labels.len() as u32) < MAX_MARKS {
                enc.write_timestamp(&self.set, labels.len() as u32);
                labels.push(label);
            }
        }
    }

    /// Resolves the marks into the slot (call before `finish`).
    pub(crate) fn end(&mut self, enc: &mut wgpu::CommandEncoder) {
        let Some((i, labels)) = self.cur.take() else { return };
        let n = labels.len() as u32;
        if n < 2 {
            return;
        }
        enc.resolve_query_set(&self.set, 0..n, &self.resolve, 0);
        enc.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[i].buf, 0, n as u64 * 8);
        let s = &mut self.slots[i];
        s.labels = labels;
        s.busy = true;
        self.cur = Some((i, Vec::new()));
    }

    /// Requests the map of the slot just submitted.
    pub(crate) fn submitted(&mut self) {
        let Some((i, _)) = self.cur.take() else { return };
        let s = &self.slots[i];
        let ready = s.ready.clone();
        s.buf.slice(..s.labels.len() as u64 * 8).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }

    fn collect(&mut self) {
        for k in 0..self.slots.len() {
            let s = &mut self.slots[k];
            if !s.busy || !s.ready.load(Ordering::Acquire) {
                continue;
            }
            let n = s.labels.len();
            let ts: Vec<u64> = {
                let data = s.buf.slice(..n as u64 * 8).get_mapped_range();
                bytemuck::cast_slice::<u8, u64>(&data).to_vec()
            };
            s.buf.unmap();
            s.ready.store(false, Ordering::Release);
            s.busy = false;
            let labels = std::mem::take(&mut s.labels);
            let total = ts[n - 1].saturating_sub(ts[0]) as f64 * self.period_ns as f64 / 1e6;
            for j in 1..n {
                let ms = ts[j].saturating_sub(ts[j - 1]) as f64 * self.period_ns as f64 / 1e6;
                match self.acc.iter_mut().find(|a| a.0 == labels[j]) {
                    Some(a) => {
                        a.1 += ms;
                        a.2 = a.2.max(ms);
                    }
                    None => self.acc.push((labels[j], ms, ms)),
                }
            }
            match self.acc.iter_mut().find(|a| a.0 == "total") {
                Some(a) => {
                    a.1 += total;
                    a.2 = a.2.max(total);
                }
                None => self.acc.push(("total", total, total)),
            }
            self.views += 1;
            if self.views >= REPORT_EVERY {
                let v = self.views as f64;
                let parts: Vec<String> = self.acc.iter().map(|(l, sum, max)| format!("{l} {:.2} (max {:.2})", sum / v, max)).collect();
                eprintln!("[gpu] {} views, avg ms: {}", self.views, parts.join(", "));
                self.acc.clear();
                self.views = 0;
            }
        }
    }
}
