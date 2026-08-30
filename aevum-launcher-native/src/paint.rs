//! Painting helpers: rounded rects, soft glows, vertical gradients, arcs.

use eframe::egui::{
    self, pos2, Align2, Color32, Pos2, Rect, Rounding, Shape, Stroke,
};

pub fn cr(px: f32) -> Rounding {
    Rounding::same(px)
}

pub fn zero() -> Rounding {
    Rounding::ZERO
}

/// Rounded rect with fill + stroke.
pub fn rect(painter: &egui::Painter, r: Rect, rounding: Rounding, fill: Color32, stroke: Stroke) {
    painter.rect(r, rounding, fill, stroke);
}

/// Soft outer glow around a rounded rect.
pub fn glow(painter: &egui::Painter, r: Rect, rounding: Rounding, color: Color32, spread: f32, strength: f32) {
    const LAYERS: usize = 7;
    for i in (0..LAYERS).rev() {
        let t = i as f32 / LAYERS as f32;
        let alpha = strength * (1.0 - t) * (1.0 - t);
        painter.rect(
            r.expand(spread * t),
            rounding,
            color.gamma_multiply(alpha),
            Stroke::NONE,
        );
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A rotating arc (dash arc) used for orbit rings and spinners.
pub fn arc(painter: &egui::Painter, center: Pos2, radius: f32, width: f32, color: Color32, start: f32, span: f32) {
    const N: usize = 40;
    let mut pts = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let a = start + span * (i as f32 / N as f32);
        pts.push(pos2(center.x + radius * a.cos(), center.y + radius * a.sin()));
    }
    painter.add(Shape::line(pts, Stroke::new(width, color)));
}

/// Centered text helper.
pub fn text(painter: &egui::Painter, center: Pos2, txt: &str, font: egui::FontId, color: Color32) {
    painter.text(center, Align2::CENTER_CENTER, txt, font, color);
}

/// Left-aligned text anchored at its top-left corner.
pub fn text_left(painter: &egui::Painter, at: Pos2, txt: &str, font: egui::FontId, color: Color32) {
    painter.text(at, Align2::LEFT_TOP, txt, font, color);
}

/// Right-aligned text anchored at its top-right corner.
pub fn text_right(painter: &egui::Painter, at: Pos2, txt: &str, font: egui::FontId, color: Color32) {
    painter.text(at, Align2::RIGHT_TOP, txt, font, color);
}

/// Draw a grass block cube (isometric-ish) rotating around Y.
pub fn cube(painter: &egui::Painter, center: Pos2, size: f32, rot: f32) {
    let tilt: f32 = -0.42;

    let proj = |p: [f32; 3]| -> Pos2 {
        let (s, c) = rot.sin_cos();
        let x = p[0] * c + p[2] * s;
        let z = -p[0] * s + p[2] * c;
        let (ts, tc) = tilt.sin_cos();
        let y = p[1] * tc - z * ts;
        let zz = p[1] * ts + z * tc;
        // orthographic projection; zz adds a little depth scale for realism
        let s = size * 0.5 / 1.5;
        pos2(center.x + x * s, center.y - y * s - zz * 0.1)
    };

    let face = |corners: &[[f32; 3]], fill: Color32| {
        let pts: Vec<Pos2> = corners.iter().map(|&c| proj(c)).collect();
        let edge = fill.gamma_multiply(0.55);
        painter.add(Shape::convex_polygon(pts, fill, Stroke::new(1.2_f32, edge)));
    };

    let top = [1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0];
    let top_c: Vec<[f32; 3]> = top.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    let front = [1.0, 1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0];
    let front_c: Vec<[f32; 3]> = front.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    let right = [1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0];
    let right_c: Vec<[f32; 3]> = right.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();

    face(&top_c, Color32::from_rgb(52, 211, 153));
    face(&front_c, Color32::from_rgb(96, 160, 92));
    face(&right_c, Color32::from_rgb(141, 77, 47));
}

pub fn chevron(painter: &egui::Painter, at: Pos2, color: Color32, down: bool) {
    let w = 5.0;
    let h = 3.2;
    let sign = if down { 1.0 } else { -1.0 };
    let p1 = pos2(at.x - w * 0.5, at.y - sign * h * 0.5);
    let p2 = pos2(at.x, at.y + sign * h * 0.5);
    let p3 = pos2(at.x + w * 0.5, at.y - sign * h * 0.5);
    let stroke = Stroke::new(1.7_f32, color);
    painter.line_segment([p1, p2], stroke);
    painter.line_segment([p2, p3], stroke);
}
