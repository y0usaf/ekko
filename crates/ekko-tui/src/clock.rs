//! Universal animation clock — the single source of truth for all animation
//! timing across phi.
//!
//! Modeled after claude-code's `createClock` (see
//! `ref/claude-code/ink/components/ClockContext.tsx`). Every animated surface
//! in phi — the mux sidebar/statusbar shimmer, the session-TUI equalizer,
//! thinking expressions, spinner frames, tool header shimmer, gradient text
//! — derives its animation phase from a single `now_ms()` value produced by
//! this clock.
//!
//! ## How it works
//!
//! The main loop creates one `AnimationClock` and calls [`tick`](Self::tick)
//! on each timer wake-up. [`now_ms`](Self::now_ms) returns the last tick
//! snapshot (so all animation functions in the same render frame see the same
//! value) or real-time when paused. Consumers never access `Instant` directly
//! — they call `clock.now_ms()` and pass the result to [`spinner_frame_at`],
//! [`shimmer_triangle_intensity`], [`equalizer_glyphs`], etc.
//!
//! ## keepAlive
//!
//! When the last keep-alive interest is removed, [`is_ticking`](Self::is_ticking)
//! returns `false` and the main loop stops arming the timer — zero wake-ups
//! while idle. This mirrors claude-code's subscriber model where the clock
//! is fully torn down when no visible animation needs it.

use std::time::Instant;

/// Animation frame duration (ms) when the terminal has focus (~60 fps).
/// Matches claude-code's `FRAME_INTERVAL_MS = 16`.
pub const ANIMATION_FRAME_MS: u64 = 16;

/// Multiplier applied to the tick interval when the terminal loses focus.
/// When blurred, the clock ticks at `ANIMATION_FRAME_MS * 2 = 32 ms` (~31 fps),
/// halving CPU usage for invisible animations.
const BLURRED_TICK_MULTIPLIER: u64 = 2;

/// Universal animation clock, modeled after claude-code's `createClock`.
///
/// One instance per main loop (mux or session-TUI). All animated surfaces
/// within that loop derive their phase from [`now_ms`](Self::now_ms).
#[derive(Debug)]
pub struct AnimationClock {
    /// Monotonic epoch. All `now_ms` values are `start.elapsed().as_millis()`.
    start: Instant,
    /// Last synchronized snapshot. Updated by [`tick`](Self::tick), read by
    /// [`now_ms`](Self::now_ms). `0` means `tick` has never been called.
    tick_time: u64,
    /// Count of active keep-alive interests. When > 0 the clock is ticking
    /// and the main loop arms the timer.
    keep_alive: usize,
    /// Whether the terminal currently has focus. When `false` the tick
    /// interval doubles.
    focused: bool,
}

impl Default for AnimationClock {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationClock {
    /// Create a new clock: focused, no keep-alive interests.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            tick_time: 0,
            keep_alive: 0,
            focused: true,
        }
    }

    /// Create a clock with a baseline keep-alive interest of 1.
    ///
    /// Used by the mux, whose statusbar shimmer is always visible and always
    /// animated, so the clock should never pause. The session-TUI starts
    /// with zero (animations only run during an active turn).
    pub fn with_baseline_keep_alive() -> Self {
        Self {
            keep_alive: 1,
            ..Self::new()
        }
    }

    /// Snapshot the current elapsed time.
    ///
    /// Called by the main loop's timer arm on each wake-up. Returns the
    /// snapshot so the caller can thread it directly if desired.
    pub fn tick(&mut self) -> u64 {
        self.tick_time = self.start.elapsed().as_millis() as u64;
        self.tick_time
    }

    /// Current animation time in milliseconds.
    ///
    /// When the clock is ticking (keep-alive > 0) and has been ticked at
    /// least once, returns the synchronized `tick_time` so all animation
    /// functions called during the same render frame see the same value.
    ///
    /// When paused (keep-alive == 0) or before the first `tick`, returns
    /// real-time `start.elapsed()` so animations don't freeze at a stale
    /// snapshot from before the pause.
    pub fn now_ms(&self) -> u64 {
        if self.is_ticking() && self.tick_time != 0 {
            self.tick_time
        } else {
            self.start.elapsed().as_millis() as u64
        }
    }

    /// Register a keep-alive interest. While at least one is active the
    /// clock is ticking and the main loop arms the timer.
    pub fn add_keep_alive(&mut self) {
        self.keep_alive = self.keep_alive.saturating_add(1);
    }

    /// Revoke a keep-alive interest previously added via
    /// [`add_keep_alive`](Self::add_keep_alive). When the count drops to
    /// zero the clock pauses — the main loop stops arming the timer.
    pub fn remove_keep_alive(&mut self) {
        self.keep_alive = self.keep_alive.saturating_sub(1);
    }

    /// Whether the clock should be ticking. True when at least one
    /// keep-alive interest is active.
    pub fn is_ticking(&self) -> bool {
        self.keep_alive > 0
    }

    /// Set the terminal focus state. When blurred the tick interval doubles.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Whether the terminal currently has focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Current tick interval in milliseconds.
    ///
    /// [`ANIMATION_FRAME_MS`] (16) when focused, doubled to 32 when blurred —
    /// matching claude-code's `FRAME_INTERVAL_MS` and
    /// `BLURRED_TICK_INTERVAL_MS`.
    pub fn tick_interval_ms(&self) -> u64 {
        if self.focused {
            ANIMATION_FRAME_MS
        } else {
            ANIMATION_FRAME_MS * BLURRED_TICK_MULTIPLIER
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn starts_paused_and_focused() {
        let clock = AnimationClock::new();
        assert!(!clock.is_ticking());
        assert!(clock.is_focused());
    }

    #[test]
    fn now_ms_returns_realtime_when_paused() {
        let clock = AnimationClock::new();
        let t1 = clock.now_ms();
        thread::sleep(Duration::from_millis(10));
        let t2 = clock.now_ms();
        assert!(t2 > t1, "real-time should advance: {t1} -> {t2}");
    }

    #[test]
    fn now_ms_returns_frozen_tick_when_ticking() {
        let mut clock = AnimationClock::new();
        clock.add_keep_alive();
        assert!(clock.is_ticking());
        clock.tick();
        thread::sleep(Duration::from_millis(5));
        // Tick again so we have a non-zero snapshot (on very fast machines
        // the first tick() can land at elapsed=0).
        clock.tick();
        let t = clock.now_ms();
        assert!(t > 0, "tick_time should be non-zero after sleep");
        thread::sleep(Duration::from_millis(10));
        assert_eq!(clock.now_ms(), t, "should return frozen tick_time");
    }

    #[test]
    fn tick_advances_time() {
        let mut clock = AnimationClock::new();
        clock.add_keep_alive();
        clock.tick();
        let t1 = clock.now_ms();
        thread::sleep(Duration::from_millis(10));
        clock.tick();
        let t2 = clock.now_ms();
        assert!(t2 > t1);
    }

    #[test]
    fn keep_alive_add_remove() {
        let mut clock = AnimationClock::new();
        clock.add_keep_alive();
        assert!(clock.is_ticking());
        clock.add_keep_alive();
        clock.remove_keep_alive();
        assert!(clock.is_ticking(), "still 1 remaining");
        clock.remove_keep_alive();
        assert!(!clock.is_ticking());
    }

    #[test]
    fn baseline_keep_alive() {
        let clock = AnimationClock::with_baseline_keep_alive();
        assert!(clock.is_ticking());
    }

    #[test]
    fn tick_interval_focused() {
        let clock = AnimationClock::new();
        assert_eq!(clock.tick_interval_ms(), ANIMATION_FRAME_MS);
    }

    #[test]
    fn tick_interval_blurred() {
        let mut clock = AnimationClock::new();
        clock.set_focused(false);
        assert_eq!(clock.tick_interval_ms(), ANIMATION_FRAME_MS * 2);
    }

    #[test]
    fn paused_returns_realtime_after_resume() {
        let mut clock = AnimationClock::new();
        clock.add_keep_alive();
        clock.tick();
        clock.remove_keep_alive();
        // Paused: now_ms should return real-time, not the frozen tick_time
        let t1 = clock.now_ms();
        thread::sleep(Duration::from_millis(10));
        let t2 = clock.now_ms();
        assert!(t2 > t1, "paused clock should return real-time");
    }
}
