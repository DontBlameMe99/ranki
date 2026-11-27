//! Animated wheel scrolling for a GPUI scroll container.
//!
//! GPUI applies wheel deltas to a scroll offset immediately. Keeping a target
//! offset here and moving the real offset toward it on animation frames gives
//! the wheel a short, consistent transition without re-reading the store.

use std::time::Instant;

use gpui::{Pixels, ScrollHandle, Window, point, px};

/// Time constant of the ease-out, in seconds. About 95% of a wheel step is
/// covered in three constants, so a step settles in roughly a quarter second.
const RESPONSE_SECONDS: f32 = 0.08;
/// Stop early instead of drawing subpixel movement forever.
const SETTLE_PX: f32 = 0.5;
/// A change larger than this came from the scrollbar or a layout change.
const EXTERNAL_PX: f32 = 1.0;

#[derive(Clone, Debug)]
pub struct SmoothScroll {
    handle: ScrollHandle,
    offset: f32,
    target: f32,
    painted: f32,
    tick: Option<Instant>,
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self::new()
    }
}

impl SmoothScroll {
    #[must_use]
    pub fn new() -> Self {
        Self::with_handle(ScrollHandle::new())
    }

    /// Drives an existing scroll handle, such as one shared with a scrollbar.
    #[must_use]
    pub fn with_handle(handle: ScrollHandle) -> Self {
        Self {
            handle,
            offset: 0.0,
            target: 0.0,
            painted: 0.0,
            tick: None,
        }
    }

    #[must_use]
    pub fn handle(&self) -> &ScrollHandle {
        &self.handle
    }

    #[must_use]
    pub fn offset(&self) -> f32 {
        self.offset
    }

    #[cfg(test)]
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    #[cfg(test)]
    #[must_use]
    pub fn animating(&self) -> bool {
        self.tick.is_some()
    }

    /// Moves the target by a positive distance when scrolling down.
    ///
    /// GPUI's own wheel listener runs before this one and moves the offset
    /// immediately. Put it back so the visible movement happens on frames.
    pub fn wheel(&mut self, distance: f32) {
        self.handle
            .set_offset(point(Pixels::ZERO, px(-self.painted)));
        self.offset = self.painted;
        self.target = (self.target + distance).clamp(0.0, self.max_offset());
        if self.tick.is_none() {
            self.tick = Some(Instant::now());
        }
    }

    /// Keeps the offset inside a new scroll range, such as after a smaller
    /// search result set.
    pub fn clamp(&mut self, max: f32) {
        self.offset = self.offset.clamp(0.0, max);
        self.target = self.target.clamp(0.0, max);
        self.painted = self.offset;
        self.tick = None;
        self.handle
            .set_offset(point(Pixels::ZERO, px(-self.offset)));
    }

    /// Advances the visible offset one frame. With motion disabled the
    /// container's own wheel handler owns the offset, so this only tracks it.
    pub fn advance(&mut self, window: &Window, smooth: bool) {
        self.sync_external();
        let max = self.max_offset();
        self.offset = self.offset.clamp(0.0, max);
        self.target = self.target.clamp(0.0, max);

        if smooth && self.tick.is_some() {
            let now = Instant::now();
            let elapsed = self.tick.map_or(0.0, |tick| {
                now.saturating_duration_since(tick).as_secs_f32()
            });
            self.tick = Some(now);
            self.step(elapsed);
        } else {
            self.tick = None;
            self.offset = self.handle_offset();
            self.target = self.offset;
        }

        if (self.offset - self.painted).abs() > 0.01 {
            self.painted = self.offset;
            self.handle
                .set_offset(point(Pixels::ZERO, px(-self.offset)));
        }
        if self.tick.is_some() {
            window.request_animation_frame();
        }
    }
    fn step(&mut self, elapsed_seconds: f32) {
        let remaining = self.target - self.offset;
        if remaining.abs() <= SETTLE_PX {
            self.offset = self.target;
            self.tick = None;
        } else {
            let factor = 1.0 - (-elapsed_seconds / RESPONSE_SECONDS).exp();
            self.offset += remaining * factor.clamp(0.0, 1.0);
        }
    }

    #[cfg(test)]
    pub fn settle(&mut self) {
        for _ in 0..20 {
            self.step(0.05);
        }
        self.painted = self.offset;
        self.handle
            .set_offset(point(Pixels::ZERO, px(-self.offset)));
    }

    fn max_offset(&self) -> f32 {
        f32::from(self.handle.max_offset().height).max(0.0)
    }

    fn handle_offset(&self) -> f32 {
        -f32::from(self.handle.offset().y)
    }

    fn sync_external(&mut self) {
        let actual = self.handle_offset();
        if (actual - self.painted).abs() > EXTERNAL_PX {
            self.offset = actual;
            self.target = actual;
            self.painted = actual;
            self.tick = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SmoothScroll;

    #[test]
    fn wheel_never_moves_the_offset_past_the_scroll_range() {
        let mut scroll = SmoothScroll::new();
        scroll.wheel(-40.0);
        assert_eq!(scroll.offset(), 0.0);
        assert_eq!(scroll.target(), 0.0);
    }

    #[test]
    fn shrinking_the_scroll_range_brings_the_offset_back_into_view() {
        let mut scroll = SmoothScroll::new();
        scroll.offset = 500.0;
        scroll.target = 500.0;
        scroll.painted = 500.0;
        scroll.tick = Some(std::time::Instant::now());
        scroll.clamp(120.0);
        assert_eq!(scroll.offset(), 120.0);
        assert_eq!(scroll.target(), 120.0);
        assert!(!scroll.animating());
    }

    #[test]
    fn external_scrollbar_movement_becomes_the_new_target() {
        let mut scroll = SmoothScroll::new();
        scroll
            .handle
            .set_offset(gpui::point(gpui::px(0.), gpui::px(-200.0)));
        scroll.wheel(-40.0);
        assert_eq!(scroll.target(), 0.0, "nothing is scrollable yet");

        scroll
            .handle
            .set_offset(gpui::point(gpui::px(0.), gpui::px(-50.0)));
        scroll.sync_external();
        assert_eq!(scroll.target(), 50.0);
        assert_eq!(scroll.offset(), 50.0);
    }
}
