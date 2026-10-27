//! The Selora orbital planet brand mark.
//!
//! Rendered in code from the construction in `assets/icons/selora-ide-mono.svg`
//! (DESIGN_TOKENS.md §9). The mark is monochrome white on transparent, so it
//! reads on every ink surface. Reference geometry is the 32px construction:
//!
//! - White core orb, 8px diameter.
//! - Tilted ring, a 16x4 ellipse rotated 20 degrees.
//! - Satellite, a 2px dot at the ring's far end.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

/// The reference construction size (DESIGN_TOKENS.md §9).
pub const MARK_CANVAS: f32 = 32.0;
/// Core orb radius at the reference size (8px diameter).
pub const CORE_RADIUS: f32 = 4.0;
/// Ring semi-major axis at the reference size (16px wide).
pub const RING_SEMI_MAJOR: f32 = 8.0;
/// Ring semi-minor axis at the reference size (4px tall).
pub const RING_SEMI_MINOR: f32 = 2.0;
/// Ring tilt, clockwise, in degrees.
pub const RING_TILT_DEG: f32 = 20.0;
/// Satellite radius at the reference size (2px dot).
pub const SATELLITE_RADIUS: f32 = 1.0;

const RING_SEGMENTS: usize = 64;

/// Paint the brand mark into `rect`.
///
/// `rect` should be square; the mark scales uniformly to fit (all reference
/// geometry multiplies by `rect.width() / 32`). The mark is centered in the
/// rect, so callers can allocate a taller hit target without skewing the art.
pub fn paint_mark(painter: &Painter, rect: Rect) {
    paint_mark_orbiting(painter, rect, 0.0);
}

/// Paint the brand mark with the satellite at `theta` radians along its
/// elliptical path (0 = the ring's far end, the static mark's pose). The
/// splash's `orbit` animation (DESIGN_TOKENS.md §7.3, UI_SPEC.md §7) drives
/// theta; every other surface keeps the static mark.
pub fn paint_mark_orbiting(painter: &Painter, rect: Rect, theta: f32) {
    let scale = rect.width().max(rect.height()) / MARK_CANVAS;
    let center = rect.center();

    // Tilted ring: an ellipse sampled as a closed polyline.
    let (sin, cos) = RING_TILT_DEG.to_radians().sin_cos();
    let a = RING_SEMI_MAJOR * scale;
    let b = RING_SEMI_MINOR * scale;
    let ring: Vec<Pos2> = (0..=RING_SEGMENTS)
        .map(|i| {
            let t = (i as f32 / RING_SEGMENTS as f32) * std::f32::consts::TAU;
            let x = a * t.cos();
            let y = b * t.sin();
            Pos2::new(center.x + x * cos - y * sin, center.y + x * sin + y * cos)
        })
        .collect();
    // 1px ring stroke at the reference size, scaled with the mark.
    painter.add(Shape::line(ring, Stroke::new(scale, Color32::WHITE)));

    // Core orb.
    painter.circle_filled(center, CORE_RADIUS * scale, Color32::WHITE);

    // Satellite on the ring's path: the local point (a·cosθ, b·sinθ)
    // rotated by the tilt.
    let x = a * theta.cos();
    let y = b * theta.sin();
    let satellite = Pos2::new(center.x + x * cos - y * sin, center.y + x * sin + y * cos);
    painter.circle_filled(satellite, SATELLITE_RADIUS * scale, Color32::WHITE);
}
