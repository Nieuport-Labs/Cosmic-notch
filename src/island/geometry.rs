// SPDX-License-Identifier: GPL-3.0-only

//! Geometry of the island inside its popup.
//!
//! The popup is as large as the island will become (the popup's input region
//! cannot change after it is created), and the visible pill is drawn against
//! the edge of the panel the island hangs from, growing and shrinking inside
//! it. Only the pill is blurred.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::iced::{Point, Rectangle, Size};

/// The largest the island may grow to.
pub const MAX: Size = Size::new(760.0, 540.0);

static REDUCED_MOTION: AtomicBool = AtomicBool::new(false);

/// With reduced motion, the island jumps to its size and nothing fades or slides.
pub fn set_reduced_motion(reduced: bool) {
    REDUCED_MOTION.store(reduced, Ordering::Relaxed);
}

pub fn reduced_motion() -> bool {
    REDUCED_MOTION.load(Ordering::Relaxed)
}

/// Bounds of a pill of `size` inside a popup of size `surface`, against the
/// panel's edge and centered along it.
pub fn pill_bounds(size: Size, surface: Size, anchor: PanelAnchor) -> Rectangle {
    let width = size.width.clamp(0.0, surface.width);
    let height = size.height.clamp(0.0, surface.height);
    let centered_x = ((surface.width - width) / 2.0).round();
    let centered_y = ((surface.height - height) / 2.0).round();
    let origin = match anchor {
        PanelAnchor::Top => Point::new(centered_x, 0.0),
        PanelAnchor::Bottom => Point::new(centered_x, surface.height - height),
        PanelAnchor::Left => Point::new(0.0, centered_y),
        PanelAnchor::Right => Point::new(surface.width - width, centered_y),
    };
    Rectangle::new(origin, Size::new(width, height))
}

/// Corner radius of the pill: the theme radius, but never more than half its height.
pub fn pill_radius(theme_radius: f32, size: Size) -> f32 {
    theme_radius.min(size.height / 2.0).min(size.width / 2.0).max(0.0)
}

/// Rounded rectangle split into the strips the background-effect protocol takes as blur region.
pub fn blur_region(size: Size, radius: f32, surface: Size, anchor: PanelAnchor) -> Vec<Rectangle> {
    use cosmic::iced::platform_specific::runtime::wayland::CornerRadius;
    let r = pill_radius(radius, size).round() as u32;
    cosmic::surface::corner_radius::rounded_rect_strips(
        pill_bounds(size, surface, anchor),
        CornerRadius {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        },
    )
}

/// A damped spring, parameterized like SwiftUI's `spring(response:dampingFraction:)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// Period of the undamped oscillation, in seconds.
    pub response: f32,
    /// 1.0 settles without overshoot; less overshoots slightly.
    pub damping: f32,
}

impl Spring {
    /// Opening, as in Boring Notch: barely overshoots.
    pub const OPEN: Self = Self {
        response: 0.42,
        damping: 0.8,
    };
    /// Closing, as in Boring Notch: no overshoot.
    pub const CLOSE: Self = Self {
        response: 0.45,
        damping: 1.0,
    };

    /// Displacement and velocity `t` seconds after starting at `d0` with velocity `v0`.
    fn solve(self, d0: f32, v0: f32, t: f32) -> (f32, f32) {
        let w0 = std::f32::consts::TAU / self.response;
        let z = self.damping;
        if z < 1.0 {
            let wd = w0 * (1.0 - z * z).sqrt();
            let a = d0;
            let b = (v0 + z * w0 * d0) / wd;
            let e = (-z * w0 * t).exp();
            let (sin, cos) = (wd * t).sin_cos();
            let d = e * (a * cos + b * sin);
            let v = e * ((b * wd - z * w0 * a) * cos - (a * wd + z * w0 * b) * sin);
            (d, v)
        } else {
            let b = v0 + w0 * d0;
            let e = (-w0 * t).exp();
            let d = e * (d0 + b * t);
            let v = e * (b - w0 * (d0 + b * t));
            (d, v)
        }
    }
}

/// Size of the pill, moving towards its target on a spring.
#[derive(Debug, Clone, Copy)]
pub struct SizeAnimation {
    from: Size,
    velocity: (f32, f32),
    to: Size,
    start: Instant,
    spring: Spring,
}

impl SizeAnimation {
    pub fn settled(size: Size) -> Self {
        Self {
            from: size,
            velocity: (0.0, 0.0),
            to: size,
            start: Instant::now(),
            spring: Spring::CLOSE,
        }
    }

    pub fn target(&self) -> Size {
        self.to
    }

    /// Retargets the animation from wherever it currently is, keeping its momentum.
    pub fn go(&mut self, to: Size, now: Instant) {
        if to == self.to {
            return;
        }
        if reduced_motion() {
            *self = Self::settled(to);
            return;
        }
        let (from, velocity) = self.state(now);
        let grows = to.width * to.height >= from.width * from.height;
        *self = Self {
            from,
            velocity,
            to,
            start: now,
            spring: if grows { Spring::OPEN } else { Spring::CLOSE },
        };
    }

    /// Springs back to `to` after a push with `velocity`: a short dip and return.
    pub fn bounce(&mut self, to: Size, velocity: (f32, f32), now: Instant) {
        if reduced_motion() {
            *self = Self::settled(to);
            return;
        }
        let (from, _) = self.state(now);
        *self = Self {
            from,
            velocity,
            to,
            start: now,
            spring: Spring::OPEN,
        };
    }

    fn state(&self, now: Instant) -> (Size, (f32, f32)) {
        let t = now.saturating_duration_since(self.start).as_secs_f32();
        let (dw, vw) = self.spring.solve(self.from.width - self.to.width, self.velocity.0, t);
        let (dh, vh) = self.spring.solve(self.from.height - self.to.height, self.velocity.1, t);
        (
            Size::new((self.to.width + dw).max(0.0), (self.to.height + dh).max(0.0)),
            (vw, vh),
        )
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        let (size, (vw, vh)) = self.state(now);
        (size.width - self.to.width).abs() > 0.5
            || (size.height - self.to.height).abs() > 0.5
            || vw.abs() > 2.0
            || vh.abs() > 2.0
    }

    pub fn at(&self, now: Instant) -> Size {
        if !self.is_animating(now) {
            return self.to;
        }
        let (size, _) = self.state(now);
        Size::new(size.width.round(), size.height.round())
    }
}

/// Fading content in: opacity and scale, `since` the content appeared.
pub fn content_transition(since: Instant, now: Instant) -> (f32, f32) {
    const DURATION: f32 = 0.35;
    if reduced_motion() {
        return (1.0, 1.0);
    }
    let t = (now.saturating_duration_since(since).as_secs_f32() / DURATION).clamp(0.0, 1.0);
    // Smooth ease-in-out, like SwiftUI's `.smooth`.
    let k = t * t * (3.0 - 2.0 * t);
    (k, 0.8 + 0.2 * k)
}

pub fn content_transition_done(since: Instant, now: Instant) -> bool {
    reduced_motion()
        || now.saturating_duration_since(since).as_secs_f32() >= 0.35
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const SURFACE: Size = Size::new(760.0, 540.0);

    #[test]
    fn pill_hangs_from_the_panel_edge() {
        let size = Size::new(100.0, 32.0);
        let top = pill_bounds(size, SURFACE, PanelAnchor::Top);
        assert_eq!(top.y, 0.0);
        assert_eq!(top.x + top.width / 2.0, SURFACE.width / 2.0);
        let bottom = pill_bounds(size, SURFACE, PanelAnchor::Bottom);
        assert_eq!(bottom.y + bottom.height, SURFACE.height);
        let left = pill_bounds(size, SURFACE, PanelAnchor::Left);
        assert_eq!(left.x, 0.0);
        assert_eq!(left.y + left.height / 2.0, SURFACE.height / 2.0);
        let right = pill_bounds(size, SURFACE, PanelAnchor::Right);
        assert_eq!(right.x + right.width, SURFACE.width);
    }

    #[test]
    fn pill_never_exceeds_surface() {
        let b = pill_bounds(Size::new(9999.0, 9999.0), SURFACE, PanelAnchor::Top);
        assert_eq!(b.size(), SURFACE);
        assert_eq!(b.x, 0.0);
    }

    #[test]
    fn radius_is_capped_by_height() {
        assert_eq!(pill_radius(160.0, Size::new(100.0, 32.0)), 16.0);
        assert_eq!(pill_radius(8.0, Size::new(100.0, 32.0)), 8.0);
    }

    #[test]
    fn blur_region_stays_inside_pill() {
        let size = Size::new(120.0, 32.0);
        let bounds = pill_bounds(size, SURFACE, PanelAnchor::Top);
        let strips = blur_region(size, 8.0, SURFACE, PanelAnchor::Top);
        assert!(!strips.is_empty());
        for s in strips {
            assert!(s.x >= bounds.x && s.x + s.width <= bounds.x + bounds.width);
            assert!(s.y >= bounds.y && s.y + s.height <= bounds.y + bounds.height + 1.0);
        }
    }

    #[test]
    fn animation_reaches_target() {
        let now = Instant::now();
        let mut anim = SizeAnimation::settled(Size::new(100.0, 32.0));
        anim.go(Size::new(500.0, 290.0), now);
        assert!(anim.is_animating(now));
        let end = now + Duration::from_secs(2);
        assert!(!anim.is_animating(end));
        assert_eq!(anim.at(end), Size::new(500.0, 290.0));
    }

    #[test]
    fn opening_overshoots_barely() {
        let now = Instant::now();
        let mut anim = SizeAnimation::settled(Size::new(100.0, 32.0));
        anim.go(Size::new(500.0, 290.0), now);
        let peak = (1..400)
            .map(|i| anim.at(now + Duration::from_millis(i * 5)).width)
            .fold(0.0, f32::max);
        // Damping 0.8 overshoots by about 1.5 % of the distance.
        assert!(peak > 500.0 && peak < 515.0, "peak {peak}");
    }

    #[test]
    fn closing_never_undershoots() {
        let now = Instant::now();
        let mut anim = SizeAnimation::settled(Size::new(500.0, 290.0));
        anim.go(Size::new(100.0, 32.0), now);
        let low = (1..400)
            .map(|i| anim.at(now + Duration::from_millis(i * 5)).width)
            .fold(f32::MAX, f32::min);
        assert!(low >= 100.0, "low {low}");
    }

    #[test]
    fn retargeting_is_continuous() {
        let now = Instant::now();
        let mut anim = SizeAnimation::settled(Size::new(100.0, 32.0));
        anim.go(Size::new(500.0, 290.0), now);
        let mid = now + Duration::from_millis(100);
        let before = anim.at(mid);
        anim.go(Size::new(100.0, 32.0), mid);
        assert!((anim.at(mid).width - before.width).abs() <= 1.0);
    }

    #[test]
    fn content_fades_and_scales_in() {
        let now = Instant::now();
        assert_eq!(content_transition(now, now), (0.0, 0.8));
        assert_eq!(content_transition(now, now + Duration::from_secs(1)), (1.0, 1.0));
    }
}
