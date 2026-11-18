//! Motion core (design v3): springs specified as duration + bounce, cubic-bezier tweens, and retargetable animated
//! values. Everything is a pure function of time, so any frame rate draws the same picture, a dropped frame never
//! slows an animation down, and the gallery can render any instant exactly (`set_time_override`).
//!
//! Springs use the SwiftUI mapping (as Hex by Anomaly does): omega = 2*pi / duration, zeta = 1 - bounce, and a
//! negative bounce is over-damped (zeta = 1 / (1 + bounce)). Springs move geometry only; opacity and colour use a
//! `Tween` with `STANDARD` over the same duration and never overshoot.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// A spring as the designer specifies it: how long it takes, and how much it bounces (0 = none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub duration: f32,
    pub bounce: f32,
}

/// The one spring of the listener family: bloom, width changes, collapse, pop, Review panel entry.
pub const MORPH: Spring = Spring::new(0.30, 0.15);
/// Indicators, navigation selection, hover-intent expansion, scrolling, snap to an edge.
pub const SNAPPY: Spring = Spring::new(0.20, 0.10);
/// Small hero accents only: the toggle thumb, the radio dot, the success pop.
pub const ACCENT: Spring = Spring::new(0.28, 0.25);
/// Returning to rest, progress fills.
pub const SETTLE: Spring = Spring::new(0.36, 0.0);

/// Durations in milliseconds.
pub const PRESS_MS: f32 = 70.0;
pub const FADE_MS: f32 = 140.0;
pub const PAGE_MS: f32 = 220.0;
/// Ceiling for any motion that is not tied to ongoing work.
pub const HERO_MAX_MS: f32 = 450.0;
/// What every motion becomes under Reduce Motion: an opacity crossfade of this length.
pub const REDUCED_FADE_MS: f32 = 160.0;
/// Stagger between items, and the most items that stagger (the rest start with the last).
pub const STAGGER_MS: f32 = 14.0;
pub const STAGGER_MAX: usize = 6;

/// `cubic-bezier(0.2, 0.8, 0.2, 1)`: the website's `--ease`. Fades, colour, small moves.
pub const STANDARD: CubicBezier = CubicBezier(0.2, 0.8, 0.2, 1.0);
/// `cubic-bezier(0.4, 0, 1, 1)`: things leaving.
pub const EXIT: CubicBezier = CubicBezier(0.4, 0.0, 1.0, 1.0);

impl Spring {
    pub const fn new(duration: f32, bounce: f32) -> Self {
        Self { duration, bounce }
    }

    fn omega(self) -> f32 {
        std::f32::consts::TAU / self.duration.max(0.01)
    }

    fn zeta(self) -> f32 {
        if self.bounce >= 0.0 { (1.0 - self.bounce).max(0.0) } else { 1.0 / (1.0 + self.bounce).max(0.01) }
    }

    /// Position and velocity `t` seconds after leaving `x0` with velocity `v0` toward `target` (closed form).
    pub fn sample(self, x0: f32, v0: f32, target: f32, t: f32) -> (f32, f32) {
        if t <= 0.0 || !t.is_finite() {
            return (x0, v0);
        }
        let (w, z) = (self.omega(), self.zeta());
        let a = x0 - target;
        if (z - 1.0).abs() < 1e-4 {
            // Critically damped.
            let c = v0 + w * a;
            let e = (-w * t).exp();
            (target + (a + c * t) * e, (v0 - w * c * t) * e)
        } else if z < 1.0 {
            let wd = w * (1.0 - z * z).sqrt();
            let b = (v0 + z * w * a) / wd;
            let (s, c) = (wd * t).sin_cos();
            let e = (-z * w * t).exp();
            let x = a * c + b * s;
            (target + e * x, e * (-z * w * x + wd * (b * c - a * s)))
        } else {
            let root = (z * z - 1.0).sqrt();
            let (r1, r2) = (-w * (z - root), -w * (z + root));
            let c2 = (v0 - r1 * a) / (r2 - r1);
            let c1 = a - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (target + c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}

/// A CSS `cubic-bezier(x1, y1, x2, y2)` easing curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier(pub f32, pub f32, pub f32, pub f32);

impl CubicBezier {
    fn axis(p1: f32, p2: f32, t: f32) -> f32 {
        // B(t) for control points 0, p1, p2, 1.
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    }

    fn slope(p1: f32, p2: f32, t: f32) -> f32 {
        let u = 1.0 - t;
        3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    }

    /// Eased progress for linear progress `x` in 0..=1.
    pub fn eval(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        if x == 0.0 || x == 1.0 {
            return x;
        }
        // Newton-Raphson on x(t) = x, then bisection if the slope is too flat to trust.
        let mut t = x;
        for _ in 0..8 {
            let err = Self::axis(self.0, self.2, t) - x;
            if err.abs() < 1e-6 {
                return Self::axis(self.1, self.3, t);
            }
            let d = Self::slope(self.0, self.2, t);
            if d.abs() < 1e-6 {
                break;
            }
            t = (t - err / d).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        t = x;
        for _ in 0..40 {
            if Self::axis(self.0, self.2, t) < x { lo = t } else { hi = t }
            t = 0.5 * (lo + hi);
        }
        Self::axis(self.1, self.3, t)
    }
}

/// A fixed-length eased change from `from` to `to`. Opacity and colour only: it never overshoots.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: f64,
    pub ms: f32,
    pub ease: CubicBezier,
}

impl Tween {
    /// Under Reduce Motion a longer tween becomes the short crossfade.
    fn ms(&self) -> f32 {
        if reduced_motion() { self.ms.min(REDUCED_FADE_MS) } else { self.ms }
    }

    pub fn at(&self, now: f64) -> f32 {
        let ms = self.ms();
        let p = if ms <= 0.0 { 1.0 } else { ((now - self.start) as f32 * 1000.0 / ms).clamp(0.0, 1.0) };
        self.from + (self.to - self.from) * self.ease.eval(p)
    }

    pub fn done(&self, now: f64) -> bool {
        (now - self.start) as f32 * 1000.0 >= self.ms()
    }
}

/// A geometric value that moves on a spring and can be retargeted at any moment without a jump: the new spring
/// starts from where the value is and how fast it is moving.
#[derive(Clone, Copy, Debug)]
pub struct Animated {
    spring: Spring,
    x0: f32,
    v0: f32,
    target: f32,
    start: f64,
    /// At rest when within this distance of the target (and slower than 16 * eps per second).
    eps: f32,
}

impl Animated {
    /// A value at rest at `x`. `eps` is the rest threshold in the value's own units (0.25 for pixels).
    pub fn new(x: f32, spring: Spring, eps: f32) -> Self {
        Self { spring, x0: x, v0: 0.0, target: x, start: 0.0, eps }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Position and velocity at `now`. Under Reduce Motion the value is already at its target.
    pub fn state(&self, now: f64) -> (f32, f32) {
        if reduced_motion() {
            return (self.target, 0.0);
        }
        self.spring.sample(self.x0, self.v0, self.target, (now - self.start) as f32)
    }

    pub fn value(&self, now: f64) -> f32 {
        self.state(now).0
    }

    /// Heads for `target` from the current position and velocity; a no-op when the target is unchanged.
    pub fn retarget(&mut self, target: f32, now: f64) {
        if target == self.target {
            return;
        }
        let (x, v) = self.state(now);
        *self = Self { x0: x, v0: v, target, start: now, ..*self };
    }

    /// Same as `retarget`, with another spring for this move (e.g. MORPH out, SETTLE back).
    pub fn retarget_with(&mut self, target: f32, spring: Spring, now: f64) {
        let (x, v) = self.state(now);
        *self = Self { spring, x0: x, v0: v, target, start: now, ..*self };
    }

    /// Jumps to `x` and stops (first paint, or a change that must not animate).
    pub fn jump(&mut self, x: f32) {
        *self = Self { x0: x, v0: 0.0, target: x, ..*self };
    }

    pub fn at_rest(&self, now: f64) -> bool {
        let (x, v) = self.state(now);
        (x - self.target).abs() < self.eps && v.abs() < 16.0 * self.eps
    }
}

static REDUCED: AtomicBool = AtomicBool::new(false);
/// Milliseconds since the clock's epoch as f64 bits; 0 = live time.
static TIME_OVERRIDE: AtomicU64 = AtomicU64::new(0);

/// Windows' "Animation effects" setting, mirrored here by the window code (`SPI_GETCLIENTAREAANIMATION`).
pub fn set_reduced_motion(on: bool) {
    REDUCED.store(on, Ordering::Relaxed);
}

pub fn reduced_motion() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Gallery hook: freeze the motion clock at `ms` (None returns to live time).
pub fn set_time_override(ms: Option<f64>) {
    TIME_OVERRIDE.store(ms.map_or(0, |m| m.max(1e-9).to_bits()), Ordering::Relaxed);
}

/// Seconds on the motion clock (monotonic).
pub fn now() -> f64 {
    let o = TIME_OVERRIDE.load(Ordering::Relaxed);
    if o != 0 {
        return f64::from_bits(o) / 1000.0;
    }
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// When item `i` of a staggered group starts, in milliseconds after the first.
pub fn stagger_ms(i: usize) -> f32 {
    STAGGER_MS * i.min(STAGGER_MAX - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reduced motion and the time override are process-wide; tests that touch them take turns.
    static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn run(s: Spring, from: f32, to: f32) -> Vec<(f32, f32)> {
        (0..=1500).map(|ms| s.sample(from, 0.0, to, ms as f32 / 1000.0)).collect()
    }

    #[test]
    fn presets_settle_and_overshoot_within_their_budget() {
        for (s, max_overshoot) in [(MORPH, 0.02), (SNAPPY, 0.01), (ACCENT, 0.06), (SETTLE, 0.0005)] {
            let path = run(s, 0.0, 100.0);
            let peak = path.iter().map(|p| p.0).fold(f32::MIN, f32::max);
            assert!(peak - 100.0 <= max_overshoot * 100.0, "{s:?} overshoots {}", peak - 100.0);
            // At rest (0.25 px, 4 px/s on a 100 px move) well inside the hero ceiling plus its own duration.
            let rest = path.iter().position(|p| (p.0 - 100.0).abs() < 0.25 && p.1.abs() < 4.0).expect("settles");
            assert!((rest as f32) < s.duration * 1000.0 + HERO_MAX_MS, "{s:?} rests at {rest} ms");
        }
        // The accent spring really does bounce (it is the reward); the settle spring never does.
        assert!(run(ACCENT, 0.0, 100.0).iter().any(|p| p.0 > 100.5));
    }

    #[test]
    fn every_damping_regime_starts_where_it_was_and_is_continuous() {
        for s in [Spring::new(0.3, 0.3), Spring::new(0.3, 0.0), Spring::new(0.3, -0.4)] {
            assert_eq!(s.sample(5.0, 2.0, 10.0, 0.0), (5.0, 2.0));
            let (x, v) = s.sample(5.0, 2.0, 10.0, 1e-4);
            assert!((x - 5.0).abs() < 1e-2 && (v - 2.0).abs() < 2.0, "{s:?}: {x} {v}");
            // Stepping in two pieces equals one piece: the solution is exact, not integrated.
            let (xa, va) = s.sample(5.0, 2.0, 10.0, 0.05);
            let two = s.sample(xa, va, 10.0, 0.07);
            let one = s.sample(5.0, 2.0, 10.0, 0.12);
            assert!((two.0 - one.0).abs() < 1e-3 && (two.1 - one.1).abs() < 1e-2, "{s:?} {two:?} {one:?}");
        }
    }

    #[test]
    fn retargeting_keeps_position_and_velocity() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let mut a = Animated::new(0.0, MORPH, 0.25);
        a.retarget(100.0, 0.0);
        let (x, v) = a.state(0.08);
        a.retarget(20.0, 0.08);
        let (x2, v2) = a.state(0.08);
        assert!((x - x2).abs() < 1e-4 && (v - v2).abs() < 1e-3, "no jump: {x} {v} vs {x2} {v2}");
        assert!(a.state(0.081).0 > x, "momentum carries it forward for a moment");
        assert!(a.at_rest(2.0) && (a.value(2.0) - 20.0).abs() < 0.25);
    }

    #[test]
    fn reduced_motion_lands_on_the_target_at_once() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        let mut a = Animated::new(0.0, ACCENT, 0.25);
        a.retarget(50.0, 0.0);
        set_reduced_motion(true);
        let snap = (a.value(0.0), a.at_rest(0.0));
        let fade = Tween { from: 0.0, to: 1.0, start: 0.0, ms: 400.0, ease: STANDARD };
        let (fade_mid, fade_done) = (fade.at(0.08), fade.done(0.17));
        set_reduced_motion(false);
        assert_eq!(snap, (50.0, true));
        assert!(fade_mid > 0.0 && fade_mid < 1.0 && fade_done, "a long fade becomes the short crossfade");
    }

    #[test]
    fn bezier_matches_css_reference_points() {
        assert_eq!((STANDARD.eval(0.0), STANDARD.eval(1.0)), (0.0, 1.0));
        // cubic-bezier(.2,.8,.2,1) is a strong ease-out. Reference values from a 60-step bisection of the same curve.
        assert!((STANDARD.eval(0.25) - 0.7673).abs() < 0.002, "{}", STANDARD.eval(0.25));
        assert!((STANDARD.eval(0.5) - 0.9461).abs() < 0.002, "{}", STANDARD.eval(0.5));
        // Linear stays linear; the exit curve starts slow.
        assert!((CubicBezier(0.0, 0.0, 1.0, 1.0).eval(0.3) - 0.3).abs() < 1e-3);
        assert!(EXIT.eval(0.25) < 0.1);
        let mut last = 0.0;
        for i in 0..=100 {
            let y = STANDARD.eval(i as f32 / 100.0);
            assert!(y >= last - 1e-5 && y <= 1.0 + 1e-5, "monotonic, never past 1");
            last = y;
        }
    }

    #[test]
    fn time_override_freezes_the_clock_and_stagger_is_capped() {
        let _g = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        set_time_override(Some(1234.0));
        let t = now();
        set_time_override(None);
        assert!((t - 1.234).abs() < 1e-9);
        assert_eq!((stagger_ms(0), stagger_ms(2), stagger_ms(40)), (0.0, 28.0, 70.0));
    }
}
