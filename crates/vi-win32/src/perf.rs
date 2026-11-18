//! Opt-in UI frame timing (`KEYREED_UI_PERF=1`): how long each animated frame took to render and present, and the
//! gap between consecutive frames of one surface. Numbers only; no text is ever recorded.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// A gap longer than this starts a new burst of motion; it is not a frame interval.
const BURST_GAP_MS: f32 = 100.0;

#[derive(Default)]
struct Surface {
    frame_ms: Vec<f32>,
    interval_ms: Vec<f32>,
    last: Option<Instant>,
    frames: u64,
}

fn store() -> Option<&'static Mutex<HashMap<&'static str, Surface>>> {
    static ON: OnceLock<bool> = OnceLock::new();
    static STORE: OnceLock<Mutex<HashMap<&'static str, Surface>>> = OnceLock::new();
    ON.get_or_init(|| std::env::var("KEYREED_UI_PERF").is_ok_and(|v| v == "1"))
        .then(|| STORE.get_or_init(Default::default))
}

/// Starts timing one frame of `surface`; the frame is recorded when the guard drops.
pub fn frame(surface: &'static str) -> Option<FrameGuard> {
    store().map(|_| FrameGuard { surface, start: Instant::now() })
}

pub struct FrameGuard {
    surface: &'static str,
    start: Instant,
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        let Some(s) = store() else { return };
        let mut m = s.lock().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(self.surface).or_default();
        e.frame_ms.push(self.start.elapsed().as_secs_f32() * 1000.0);
        if let Some(prev) = e.last {
            let gap = self.start.duration_since(prev).as_secs_f32() * 1000.0;
            if gap < BURST_GAP_MS {
                e.interval_ms.push(gap);
            }
        }
        e.last = Some(self.start);
        e.frames += 1;
    }
}

fn pct(v: &[f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[((s.len() - 1) as f32 * p).round() as usize]
}

/// Frames painted so far by `surface` (0 when timing is off).
pub fn frames(surface: &str) -> u64 {
    store().map_or(0, |s| s.lock().unwrap_or_else(|e| e.into_inner()).get(surface).map_or(0, |e| e.frames))
}

/// Forgets every sample (the gallery measures one scene at a time).
pub fn reset() {
    if let Some(s) = store() {
        s.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// One surface's timing, in milliseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub frames: u64,
    pub frame_p50: f32,
    pub frame_p95: f32,
    pub frame_max: f32,
    pub interval_p50: f32,
    pub interval_p95: f32,
}

/// Every surface that painted, with its timing (empty when timing is off).
pub fn summary() -> Vec<(&'static str, Stats)> {
    let Some(s) = store() else { return Vec::new() };
    let m = s.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<_> = m
        .iter()
        .map(|(k, e)| {
            let stats = Stats {
                frames: e.frames,
                frame_p50: pct(&e.frame_ms, 0.5),
                frame_p95: pct(&e.frame_ms, 0.95),
                frame_max: e.frame_ms.iter().copied().fold(0.0, f32::max),
                interval_p50: pct(&e.interval_ms, 0.5),
                interval_p95: pct(&e.interval_ms, 0.95),
            };
            (*k, stats)
        })
        .collect();
    out.sort_by_key(|(k, _)| *k);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn percentiles_pick_the_nearest_rank() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(super::pct(&v, 0.5), 3.0);
        assert_eq!(super::pct(&v, 0.95), 5.0);
        assert_eq!(super::pct(&[], 0.95), 0.0);
    }
}
