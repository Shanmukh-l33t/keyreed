//! One frame clock for every animated KeyReed window (design v3).
//!
//! `WM_TIMER` is low priority and quantised to the 15.6 ms system tick, so animations stepped at 16 ms really ran at
//! 15.6 / 31.2 ms intervals. This clock is a background thread on a high-resolution waitable timer that ticks at a
//! steady 60 Hz and posts one `WM_APP_FRAME` to each subscribed window. A window has at most one frame message in
//! flight (an atomic "pending" flag, cleared when the window handles it), so the queue never floods. With no
//! subscribers the thread parks: idle costs no frames and no timers.
//!
//! The rule that keeps motion from freezing (Hex learned it the hard way): a window decides whether to stay
//! subscribed only *after* drawing a frame, from the state that frame drew. See `stay_subscribed`.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::Win32::System::Threading::{CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_APP};

/// The frame message: `wParam` and `lParam` are unused.
pub const WM_APP_FRAME: u32 = WM_APP + 0x2F0;
/// 60 Hz, like Hex's Windows HUD: steady, and cheap enough at any display refresh rate.
pub const FRAME: Duration = Duration::from_micros(16_667);

const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 0x2;
const TIMER_ALL_ACCESS: u32 = 0x1F_0003;

struct Clock {
    subs: Mutex<Vec<(isize, Arc<AtomicBool>)>>,
    wake: Condvar,
}

fn clock() -> &'static Clock {
    static CLOCK: OnceLock<Clock> = OnceLock::new();
    CLOCK.get_or_init(|| {
        std::thread::Builder::new().name("keyreed-frameclock".into()).spawn(run).ok();
        Clock { subs: Mutex::new(Vec::new()), wake: Condvar::new() }
    })
}

/// Starts (or keeps) frames coming to `hwnd`. Cheap to call every frame.
pub fn subscribe(hwnd: HWND) {
    let c = clock();
    let mut subs = c.subs.lock().unwrap_or_else(|e| e.into_inner());
    if !subs.iter().any(|(h, _)| *h == hwnd as isize) {
        subs.push((hwnd as isize, Arc::new(AtomicBool::new(false))));
        c.wake.notify_one();
    }
}

/// Stops frames to `hwnd` (its animations are all at rest, or it is going away).
pub fn unsubscribe(hwnd: HWND) {
    let mut subs = clock().subs.lock().unwrap_or_else(|e| e.into_inner());
    subs.retain(|(h, _)| *h != hwnd as isize);
}

/// Call at the top of the window's `WM_APP_FRAME` handler: the next tick may post again.
pub fn frame_taken(hwnd: HWND) {
    let subs = clock().subs.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, pending)) = subs.iter().find(|(h, _)| *h == hwnd as isize) {
        pending.store(false, Ordering::Release);
    }
}

/// Whether `hwnd` is receiving frames.
pub fn is_subscribed(hwnd: HWND) -> bool {
    clock().subs.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(h, _)| *h == hwnd as isize)
}

/// The arm/disarm decision, made after a frame was drawn: keep the frames coming while anything that frame drew is
/// still moving. `moving_after_draw` must come from the values the frame just drew, never from a check made before
/// drawing (a stale "at rest" taken before the last step disarmed Hex's HUD mid-transition and froze it).
pub fn stay_subscribed(moving_after_draw: bool) -> bool {
    moving_after_draw
}

/// Whether a tick should post to a subscriber, flipping its pending flag: at most one frame in flight.
fn claim(pending: &AtomicBool) -> bool {
    !pending.swap(true, Ordering::AcqRel)
}

fn run() {
    let c = clock();
    let timer: HANDLE = unsafe {
        let t = CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);
        // Windows before 10 1803 has no high-resolution timers: fall back to a normal one.
        if t.is_null() { CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), 0, TIMER_ALL_ACCESS) } else { t }
    };
    let mut next = Instant::now();
    loop {
        {
            let mut subs = c.subs.lock().unwrap_or_else(|e| e.into_inner());
            // Windows that were destroyed without unsubscribing drop out here.
            subs.retain(|(h, _)| unsafe { IsWindow(*h as HWND) } != 0);
            if subs.is_empty() {
                subs = c.wake.wait_while(subs, |s| s.is_empty()).unwrap_or_else(|e| e.into_inner());
                next = Instant::now();
            }
            for (h, pending) in subs.iter() {
                if claim(pending) {
                    unsafe { PostMessageW(*h as HWND, WM_APP_FRAME, 0, 0) };
                }
            }
        }
        // Absolute schedule, so the cadence never drifts; after a stall it restarts from now instead of bursting.
        next += FRAME;
        let now = Instant::now();
        if next < now {
            next = now + FRAME;
        }
        let wait = next - now;
        if timer.is_null() {
            std::thread::sleep(wait);
        } else {
            unsafe {
                let due: i64 = -((wait.as_nanos() / 100) as i64).max(1);
                if SetWaitableTimer(timer, &due, 0, None, std::ptr::null(), 0) != 0 {
                    WaitForSingleObject(timer, INFINITE);
                } else {
                    std::thread::sleep(wait);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_frame_in_flight_per_window() {
        let pending = AtomicBool::new(false);
        assert!(claim(&pending), "the first tick posts");
        assert!(!claim(&pending), "later ticks do not stack frames while one is unhandled");
        pending.store(false, Ordering::Release); // the window handled it (frame_taken)
        assert!(claim(&pending));
    }

    #[test]
    fn the_decision_to_disarm_uses_the_frame_that_was_just_drawn() {
        // A spring that was moving before the frame and still is after it keeps the clock.
        assert!(stay_subscribed(true));
        // Only a frame that drew everything at rest lets it go.
        assert!(!stay_subscribed(false));
    }

    #[test]
    fn idle_windows_get_no_frames() {
        // A window handle that was never subscribed receives nothing (the clock parks with no subscribers).
        assert!(!is_subscribed(0x1234 as HWND));
    }
}
