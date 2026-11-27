//! Geometry and drag math for the thin side scrollbars drawn over scroll views.

use gpui::ScrollHandle;

/// Width of the hit strip. The thumb itself is inset inside this strip.
pub const WIDTH: f32 = 10.0;
pub const THUMB_WIDTH: f32 = 6.0;
const INSET: f32 = 4.0;
const MIN_THUMB: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub height: f32,
    pub top: f32,
    pub travel: f32,
}

/// Where the thumb sits for one viewport, range, and offset.
#[must_use]
pub fn thumb(viewport: f32, max_offset: f32, offset: f32) -> Option<Thumb> {
    if viewport <= 0.0 || max_offset <= 1.0 {
        return None;
    }
    let track = viewport - 2.0 * INSET;
    if track <= 0.0 {
        return None;
    }
    let height = (track * viewport / (viewport + max_offset))
        .clamp(MIN_THUMB.min(track), track);
    let travel = track - height;
    let ratio = (offset / max_offset).clamp(0.0, 1.0);
    Some(Thumb {
        height,
        top: INSET + travel * ratio,
        travel,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub left: f32,
    pub track_top: f32,
    pub track_height: f32,
    pub thumb_top: f32,
    pub thumb_height: f32,
    pub travel: f32,
    pub max_offset: f32,
}

/// Reads the current geometry from a tracked GPUI scroll container.
#[must_use]
pub fn geometry(handle: &ScrollHandle) -> Option<Geometry> {
    let bounds = handle.bounds();
    let viewport = f32::from(bounds.size.height);
    let max_offset = f32::from(handle.max_offset().height);
    let offset = -f32::from(handle.offset().y);
    let thumb = thumb(viewport, max_offset, offset)?;
    Some(Geometry {
        left: f32::from(bounds.right()) - WIDTH,
        track_top: f32::from(bounds.top()) + INSET,
        track_height: thumb.travel + thumb.height,
        thumb_top: f32::from(bounds.top()) + thumb.top,
        thumb_height: thumb.height,
        travel: thumb.travel,
        max_offset,
    })
}

/// Offset produced by pressing the track at a window-space `y`.
#[must_use]
pub fn offset_for_click(geometry: &Geometry, y: f32) -> f32 {
    let center = geometry.thumb_height * 0.5;
    let distance =
        (y - geometry.track_top - center).clamp(0.0, geometry.travel);
    distance / geometry.travel.max(1.0) * geometry.max_offset
}

#[derive(Clone, Debug)]
pub struct ScrollbarDrag {
    pub id: &'static str,
    pub handle: ScrollHandle,
    pub start_y: f32,
    pub start_offset: f32,
    pub travel: f32,
    pub max_offset: f32,
}

impl ScrollbarDrag {
    pub fn new(
        id: &'static str,
        handle: &ScrollHandle,
        start_y: f32,
        start_offset: f32,
        geometry: &Geometry,
    ) -> Self {
        Self {
            id,
            handle: handle.clone(),
            start_y,
            start_offset,
            travel: geometry.travel,
            max_offset: geometry.max_offset,
        }
    }

    /// Offset after dragging the pointer to a window-space `y`.
    #[must_use]
    pub fn offset_at(&self, y: f32) -> f32 {
        let distance = y - self.start_y;
        let change = distance / self.travel.max(1.0) * self.max_offset;
        (self.start_offset + change).clamp(0.0, self.max_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::{Geometry, ScrollbarDrag, offset_for_click, thumb};

    fn geometry(travel: f32, max_offset: f32) -> Geometry {
        Geometry {
            left: 100.0,
            track_top: 10.0,
            track_height: travel + 40.0,
            thumb_top: 10.0,
            thumb_height: 40.0,
            travel,
            max_offset,
        }
    }

    #[test]
    fn thumb_shrinks_as_content_grows() {
        let short = thumb(800.0, 200.0, 0.0).expect("short content");
        let long = thumb(800.0, 3200.0, 0.0).expect("long content");
        assert!(long.height < short.height);
        assert_eq!(short.top, 4.0);
    }

    #[test]
    fn thumb_tracks_the_scroll_range() {
        let geometry = geometry(160.0, 1000.0);
        assert_eq!(offset_for_click(&geometry, 10.0 + 20.0), 0.0);
        assert_eq!(offset_for_click(&geometry, 10.0 + 20.0 + 80.0), 500.0);
        assert_eq!(offset_for_click(&geometry, 10.0 + 20.0 + 160.0), 1000.0);
    }

    #[test]
    fn drag_keeps_the_grab_point_under_the_pointer() {
        let drag = ScrollbarDrag {
            id: "test",
            handle: gpui::ScrollHandle::new(),
            start_y: 100.0,
            start_offset: 250.0,
            travel: 160.0,
            max_offset: 1000.0,
        };
        assert_eq!(drag.offset_at(100.0), 250.0);
        assert_eq!(drag.offset_at(116.0), 350.0);
        assert_eq!(drag.offset_at(-1000.0), 0.0);
        assert_eq!(drag.offset_at(1000.0), 1000.0);
    }
}
