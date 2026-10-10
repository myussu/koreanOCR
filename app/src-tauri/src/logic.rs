//! Decisions the overlay makes, kept free of Tauri so they can be unit tested:
//! where the panel goes, when a hover dwell fires, and when an idle panel hides.

use std::time::{Duration, Instant};

use maru_core::config::Placement;
use maru_core::{Point, Rect};

const GAP: f32 = 10.0;
const MARGIN: f32 = 8.0;

/// Top-left corner for a panel of `size` next to `anchor` (the selected text), inside `monitor`.
///
/// Prefers below the selection so the original stays readable; goes above when there is
/// no room below. Never leaves the monitor.
pub fn place_panel(anchor: Rect, size: (f32, f32), monitor: Rect, placement: Placement) -> Point {
    let (w, h) = size;
    let below = anchor.bottom() + GAP;
    let above = anchor.y - GAP - h;
    let fits_below = below + h <= monitor.bottom() - MARGIN;
    let fits_above = above >= monitor.y + MARGIN;
    let y = match placement {
        Placement::Below if fits_below || !fits_above => below,
        Placement::Above if fits_above || !fits_below => above,
        Placement::Auto if fits_below => below,
        Placement::Auto if fits_above => above,
        // Fits nowhere (tall panel): keep it on screen, as close to the anchor as possible.
        _ => below.min(monitor.bottom() - h - MARGIN),
    };
    let max_x = (monitor.right() - w - MARGIN).max(monitor.x + MARGIN);
    let x = anchor.x.clamp(monitor.x + MARGIN, max_x);
    let y = y.clamp(monitor.y + MARGIN, (monitor.bottom() - h - MARGIN).max(monitor.y + MARGIN));
    Point { x, y }
}

/// Keep a panel the user dragged fully on the monitor, with a margin. A panel bigger than the monitor
/// is pinned to its top-left corner.
pub fn keep_on_screen(pos: Point, size: (f32, f32), monitor: Rect) -> Point {
    let max_x = (monitor.right() - size.0 - MARGIN).max(monitor.x + MARGIN);
    let max_y = (monitor.bottom() - size.1 - MARGIN).max(monitor.y + MARGIN);
    Point { x: pos.x.clamp(monitor.x + MARGIN, max_x), y: pos.y.clamp(monitor.y + MARGIN, max_y) }
}

/// A rectangle around a point, used when the capture has no position (clipboard copy).
pub fn around_point(p: Point) -> Rect {
    Rect::new(p.x - 4.0, p.y - 10.0, 8.0, 20.0)
}

/// Is the pointer over one of the scan overlay's labels?
///
/// `cursor` is in physical screen pixels; the labels were reported in the overlay page's CSS pixels,
/// which are physical pixels divided by `dpr`, measured from the window's top-left `origin`.
pub fn over_label(cursor: Point, origin: (f32, f32), dpr: f32, labels: &[Rect]) -> bool {
    let dpr = if dpr > 0.0 { dpr } else { 1.0 };
    let css = Point { x: (cursor.x - origin.0) / dpr, y: (cursor.y - origin.1) / dpr };
    labels.iter().any(|r| r.contains(css))
}

/// Fires once when the pointer has rested in place for `dwell`.
#[derive(Debug)]
pub struct Dwell {
    radius: f32,
    dwell: Duration,
    anchor: Option<(Point, Instant)>,
    fired: bool,
}

impl Dwell {
    pub fn new(radius: f32, dwell: Duration) -> Self {
        Self { radius, dwell, anchor: None, fired: false }
    }

    /// Feed the current pointer position; returns it once when the dwell completes.
    pub fn update(&mut self, now: Instant, pos: Point) -> Option<Point> {
        let moved = self.anchor.is_none_or(|(a, _)| ((a.x - pos.x).powi(2) + (a.y - pos.y).powi(2)).sqrt() > self.radius);
        if moved {
            self.anchor = Some((pos, now));
            self.fired = false;
            return None;
        }
        let (_, since) = self.anchor?;
        if !self.fired && now.duration_since(since) >= self.dwell {
            self.fired = true;
            return Some(pos);
        }
        None
    }

    pub fn reset(&mut self) {
        self.anchor = None;
        self.fired = false;
    }
}

/// Tracks how long the pointer has been far from the panel.
#[derive(Debug, Default)]
pub struct AutoHide {
    away_since: Option<Instant>,
}

impl AutoHide {
    /// `true` when an unpinned panel has been ignored long enough to hide.
    pub fn update(&mut self, now: Instant, cursor: Point, panel: Rect, pinned: bool, far: f32, after: Duration) -> bool {
        let near = Rect::new(panel.x - far, panel.y - far, panel.w + 2.0 * far, panel.h + 2.0 * far);
        if pinned || near.contains(cursor) {
            self.away_since = None;
            return false;
        }
        let since = *self.away_since.get_or_insert(now);
        now.duration_since(since) >= after
    }

    pub fn reset(&mut self) {
        self.away_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    const SIZE: (f32, f32) = (600.0, 300.0);

    #[test]
    fn goes_below_the_selection_when_there_is_room() {
        let p = place_panel(Rect::new(400.0, 200.0, 300.0, 24.0), SIZE, MON, Placement::Auto);
        assert_eq!((p.x, p.y), (400.0, 234.0));
    }

    #[test]
    fn flips_above_near_the_bottom_edge() {
        let p = place_panel(Rect::new(400.0, 900.0, 300.0, 24.0), SIZE, MON, Placement::Auto);
        assert_eq!(p.y, 900.0 - 10.0 - 300.0);
    }

    #[test]
    fn explicit_placement_is_honoured_when_possible() {
        let anchor = Rect::new(400.0, 500.0, 300.0, 24.0);
        assert_eq!(place_panel(anchor, SIZE, MON, Placement::Above).y, 500.0 - 10.0 - 300.0);
        assert_eq!(place_panel(anchor, SIZE, MON, Placement::Below).y, 534.0);
        // asked for Below but it doesn't fit and above does → flips rather than going off-screen
        let low = Rect::new(400.0, 900.0, 300.0, 24.0);
        assert!(place_panel(low, SIZE, MON, Placement::Below).y + SIZE.1 <= 1080.0);
    }

    #[test]
    fn stays_inside_the_monitor_horizontally() {
        let p = place_panel(Rect::new(1800.0, 200.0, 100.0, 24.0), SIZE, MON, Placement::Auto);
        assert_eq!(p.x, 1920.0 - 600.0 - 8.0);
        let p = place_panel(Rect::new(-50.0, 200.0, 100.0, 24.0), SIZE, MON, Placement::Auto);
        assert_eq!(p.x, 8.0);
    }

    #[test]
    fn works_on_a_second_monitor_with_an_offset_origin() {
        let second = Rect::new(1920.0, 0.0, 2560.0, 1440.0);
        let p = place_panel(Rect::new(2000.0, 100.0, 200.0, 24.0), SIZE, second, Placement::Auto);
        assert!(p.x >= 1928.0 && p.x + SIZE.0 <= 4480.0 - 8.0);
    }

    #[test]
    fn a_panel_taller_than_the_screen_is_pinned_to_the_top() {
        let p = place_panel(Rect::new(100.0, 500.0, 100.0, 20.0), (600.0, 2000.0), MON, Placement::Auto);
        assert_eq!(p.y, 8.0);
    }

    #[test]
    fn a_dragged_panel_cannot_leave_the_screen() {
        let size = (600.0, 300.0);
        let at = |x: f32, y: f32| keep_on_screen(Point { x, y }, size, MON);
        assert_eq!((at(500.0, 400.0).x, at(500.0, 400.0).y), (500.0, 400.0), "inside: untouched");
        assert_eq!((at(-300.0, -50.0).x, at(-300.0, -50.0).y), (8.0, 8.0));
        assert_eq!((at(1800.0, 1000.0).x, at(1800.0, 1000.0).y), (1920.0 - 600.0 - 8.0, 1080.0 - 300.0 - 8.0));
        let huge = keep_on_screen(Point { x: 900.0, y: 900.0 }, (3000.0, 2000.0), MON);
        assert_eq!((huge.x, huge.y), (8.0, 8.0), "bigger than the monitor: top-left corner");
        let second = Rect::new(1920.0, 0.0, 1280.0, 1024.0);
        assert!(keep_on_screen(Point { x: 1000.0, y: 10.0 }, size, second).x >= 1928.0, "a second monitor keeps its own bounds");
    }

    #[test]
    fn dwell_fires_once_after_the_pointer_rests() {
        let t0 = Instant::now();
        let mut d = Dwell::new(4.0, Duration::from_millis(300));
        let p = Point { x: 100.0, y: 100.0 };
        assert_eq!(d.update(t0, p), None);
        assert_eq!(d.update(t0 + Duration::from_millis(200), Point { x: 102.0, y: 101.0 }), None);
        assert_eq!(d.update(t0 + Duration::from_millis(320), Point { x: 101.0, y: 100.0 }), Some(Point { x: 101.0, y: 100.0 }));
        assert_eq!(d.update(t0 + Duration::from_millis(900), p), None, "does not fire again while resting");
        // moving away re-arms it
        assert_eq!(d.update(t0 + Duration::from_millis(950), Point { x: 300.0, y: 300.0 }), None);
        assert!(d.update(t0 + Duration::from_millis(1300), Point { x: 300.0, y: 300.0 }).is_some());
    }

    #[test]
    fn a_pointer_that_keeps_moving_never_fires() {
        let t0 = Instant::now();
        let mut d = Dwell::new(4.0, Duration::from_millis(300));
        for i in 0..50 {
            assert_eq!(d.update(t0 + Duration::from_millis(i * 100), Point { x: i as f32 * 20.0, y: 50.0 }), None);
        }
    }

    #[test]
    fn idle_panels_hide_but_pinned_or_near_ones_do_not() {
        let t0 = Instant::now();
        let panel = Rect::new(500.0, 500.0, 600.0, 300.0);
        let mut a = AutoHide::default();
        let far = Point { x: 10.0, y: 10.0 };
        let after = Duration::from_millis(1500);
        assert!(!a.update(t0, far, panel, false, 300.0, after));
        assert!(!a.update(t0 + Duration::from_millis(1000), far, panel, false, 300.0, after));
        assert!(a.update(t0 + Duration::from_millis(1600), far, panel, false, 300.0, after));

        // coming back resets the timer
        assert!(!a.update(t0 + Duration::from_millis(1700), Point { x: 600.0, y: 600.0 }, panel, false, 300.0, after));
        assert!(!a.update(t0 + Duration::from_millis(1800), far, panel, false, 300.0, after));
        // pinned never hides
        assert!(!a.update(t0 + Duration::from_secs(60), far, panel, true, 300.0, after));
    }

    #[test]
    fn the_overlay_is_clickable_only_over_a_label() {
        let labels = [Rect::new(100.0, 50.0, 80.0, 18.0), Rect::new(300.0, 200.0, 60.0, 18.0)];
        let over = |x: f32, y: f32, origin: (f32, f32), dpr: f32| over_label(Point { x, y }, origin, dpr, &labels);
        assert!(over(120.0, 60.0, (0.0, 0.0), 1.0));
        assert!(!over(120.0, 100.0, (0.0, 0.0), 1.0), "empty space is click-through");
        // a second monitor to the right (origin 1920) at 150% scaling: CSS = (physical - origin) / 1.5
        assert!(over(1920.0 + 150.0 * 1.5 + 3.0, 60.0 * 1.5, (1920.0, 0.0), 1.5));
        assert!(!over(150.0 * 1.5 + 3.0, 60.0 * 1.5, (1920.0, 0.0), 1.5), "a pointer on the other monitor is not over it");
        // a monitor to the left has a negative origin
        assert!(over(-1000.0 + 320.0, 210.0, (-1000.0, 0.0), 1.0));
        assert!(!over(10.0, 10.0, (0.0, 0.0), 1.0));
        assert!(!over_label(Point::default(), (0.0, 0.0), 1.0, &[]), "no labels, nothing to click");
        assert!(over_label(Point { x: 120.0, y: 60.0 }, (0.0, 0.0), 0.0, &labels), "a missing scale factor counts as 1");
    }
}
