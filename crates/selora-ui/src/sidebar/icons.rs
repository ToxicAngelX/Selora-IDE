//! 16×16 monochrome stroke icons (DESIGN_TOKENS.md §8), painted in code.
//!
//! No icon font, no external assets: every glyph is a handful of
//! [`egui::Painter`] line segments / circles with a 1.5px stroke and no
//! fill, colored by the caller (`currentColor` semantics — the icon
//! inherits the text color of the row it lives in). Each painter takes
//! the 16×16 slot it should draw into, so callers control placement.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};

/// DESIGN_TOKENS.md §8: 1.5px stroke, no fill.
const STROKE_WIDTH: f32 = 1.5;

fn stroke(color: Color32) -> Stroke {
    Stroke::new(STROKE_WIDTH, color)
}

/// A 16×16 slot centered inside `slot`'s larger rect (callers may pass
/// taller rows); keeps the glyph optically centered.
fn square(slot: Rect) -> Rect {
    let size = slot.width().min(slot.height()).min(16.0);
    let center = slot.center();
    Rect::from_center_size(center, Vec2::splat(size))
}

fn circle_slot(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    painter.circle(center, radius, Color32::TRANSPARENT, stroke(color));
}

fn line(painter: &Painter, from: (f32, f32), to: (f32, f32), color: Color32) {
    painter.line_segment(
        [Pos2::new(from.0, from.1), Pos2::new(to.0, to.1)],
        stroke(color),
    );
}

/// `chevron-right` / `chevron-down` — the tree row disclosure glyph.
/// `down` selects the rotated (expanded) form.
pub fn paint_chevron(painter: &Painter, slot: Rect, color: Color32, down: bool) {
    let r = square(slot);
    let (left, mid, right) = (r.left() + 4.0, r.center().x, r.right() - 4.0);
    let (top, bottom) = (r.top() + 5.0, r.bottom() - 5.0);
    if down {
        line(painter, (left, top), (mid, bottom), color);
        line(painter, (mid, bottom), (right, top), color);
    } else {
        line(painter, (left, bottom), (mid, top), color);
        line(painter, (mid, top), (right, bottom), color);
    }
}

/// `file` — a sheet with a folded corner.
pub fn paint_sheet(painter: &Painter, slot: Rect, color: Color32) {
    let r = square(slot);
    let (l, t) = (r.left() + 3.5, r.top() + 1.5);
    let (rt, b) = (r.right() - 3.5, r.bottom() - 1.5);
    let fold = 3.0;
    // Body outline: left, bottom, right, folded top-right corner, top.
    line(painter, (l, t), (l, b), color);
    line(painter, (l, b), (rt, b), color);
    line(painter, (rt, b), (rt, t + fold), color);
    line(painter, (rt, t + fold), (rt - fold, t), color);
    line(painter, (rt - fold, t), (l, t), color);
    // The fold's inner edge.
    line(painter, (rt - fold, t), (rt - fold, t + fold), color);
    line(painter, (rt - fold, t + fold), (rt, t + fold), color);
}

/// `folder` / `folder-open` — the tree's directory glyph.
pub fn paint_folder(painter: &Painter, slot: Rect, color: Color32, open: bool) {
    let r = square(slot);
    if open {
        // Tab + top edge, then a front panel that flares out downward.
        let (l, t) = (r.left() + 2.0, r.top() + 3.5);
        let tab_end = l + 3.5;
        let notch_y = t + 1.5;
        let top_right = r.right() - 2.0;
        let bottom = r.bottom() - 3.0;
        let flare_l = l - 1.5;
        let flare_r = r.right() - 0.5;
        line(painter, (l, t), (tab_end, t), color);
        line(painter, (tab_end, t), (tab_end + 1.0, notch_y), color);
        line(
            painter,
            (tab_end + 1.0, notch_y),
            (top_right, notch_y),
            color,
        );
        line(painter, (top_right, notch_y), (flare_r, bottom), color);
        line(painter, (flare_r, bottom), (flare_l, bottom), color);
        line(painter, (flare_l, bottom), (l, t), color);
    } else {
        let (l, t) = (r.left() + 2.0, r.top() + 4.0);
        let tab_end = l + 3.5;
        let notch_y = t + 1.5;
        let (rt, b) = (r.right() - 2.0, r.bottom() - 3.0);
        line(painter, (l, t), (tab_end, t), color);
        line(painter, (tab_end, t), (tab_end + 1.0, notch_y), color);
        line(painter, (tab_end + 1.0, notch_y), (rt, notch_y), color);
        line(painter, (rt, notch_y), (rt, b), color);
        line(painter, (rt, b), (l, b), color);
        line(painter, (l, b), (l, t), color);
    }
}

/// `search` — a magnifier.
pub fn paint_search(painter: &Painter, slot: Rect, color: Color32) {
    let r = square(slot);
    let center = Pos2::new(r.left() + 6.5, r.top() + 6.5);
    circle_slot(painter, center, 3.5, color);
    line(
        painter,
        (center.x + 2.8, center.y + 2.8),
        (r.right() - 3.0, r.bottom() - 3.0),
        color,
    );
}

/// `git-branch` — a trunk with two commits and a branch head.
pub fn paint_git_branch(painter: &Painter, slot: Rect, color: Color32) {
    let r = square(slot);
    let dot_radius = 1.75;
    let trunk_x = r.left() + 4.5;
    let top_y = r.top() + 4.0;
    let bottom_y = r.bottom() - 4.0;
    // Commits at both ends of the trunk.
    circle_slot(painter, Pos2::new(trunk_x, top_y), dot_radius, color);
    circle_slot(painter, Pos2::new(trunk_x, bottom_y), dot_radius, color);
    line(
        painter,
        (trunk_x, top_y + 2.0),
        (trunk_x, bottom_y - 2.0),
        color,
    );
    // Branch: out from the trunk's middle, up to the head.
    let head = Pos2::new(r.right() - 4.0, top_y);
    circle_slot(painter, head, dot_radius, color);
    let mid_y = (top_y + bottom_y) / 2.0;
    line(painter, (trunk_x, mid_y), (trunk_x + 4.5, mid_y), color);
    line(
        painter,
        (trunk_x + 4.5, mid_y),
        (head.x - 1.0, head.y + 2.0),
        color,
    );
}

/// `chat` — a speech bubble.
pub fn paint_chat(painter: &Painter, slot: Rect, color: Color32) {
    let r = square(slot);
    let body = Rect::from_min_max(
        Pos2::new(r.left() + 1.5, r.top() + 2.5),
        Pos2::new(r.right() - 1.5, r.top() + 11.0),
    );
    painter.rect(
        body,
        2.5,
        Color32::TRANSPARENT,
        stroke(color),
        egui::StrokeKind::Inside,
    );
    // Tail, bottom-left.
    line(
        painter,
        (body.left() + 3.0, body.bottom()),
        (body.left() + 2.0, r.bottom() - 1.0),
        color,
    );
    line(
        painter,
        (body.left() + 2.0, r.bottom() - 1.0),
        (body.left() + 5.5, body.bottom()),
        color,
    );
}

/// `plus` — the new-file affordance.
pub fn paint_plus(painter: &Painter, slot: Rect, color: Color32) {
    let r = square(slot);
    let c = r.center();
    let arm = 4.0;
    line(painter, (c.x - arm, c.y), (c.x + arm, c.y), color);
    line(painter, (c.x, c.y - arm), (c.x, c.y + arm), color);
}
