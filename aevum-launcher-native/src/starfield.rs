//! Ambient starfield painted behind every panel.

use eframe::egui::{Color32, Painter, Pos2, Rect};

use crate::theme;

pub struct Star {
    x: f32,
    y: f32,
    size: f32,
    base: f32,
    amp: f32,
    speed: f32,
    phase: f32,
    color: Color32,
}

/// Deterministic RNG (LCG) so the layout is stable between runs.
fn rng_state() -> u32 {
    0x9E37_79B9
}

fn rand(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state >> 8) as f32 / 16777216.0
}

pub fn spawn(count: usize) -> Vec<Star> {
    let mut s = rng_state();
    let mut stars = Vec::with_capacity(count);
    for _ in 0..count {
        let roll = rand(&mut s);
        let size = if roll > 0.88 {
            2.4
        } else if roll > 0.55 {
            1.6
        } else {
            1.0
        };
        let hue = if rand(&mut s) > 0.55 {
            Color32::from_rgb(207, 232, 255)
        } else {
            Color32::from_rgb(227, 217, 255)
        };
        stars.push(Star {
            x: rand(&mut s),
            y: rand(&mut s),
            size,
            base: 0.15 + rand(&mut s) * 0.2,
            amp: 0.3 + rand(&mut s) * 0.5,
            speed: 0.8 + rand(&mut s) * 1.6,
            phase: rand(&mut s) * 6.2832,
            color: hue,
        });
    }
    stars
}

/// Paint the deep-space background and twinkling stars.
///
/// Painted into the caller's painter so the background stays in the same
/// deterministic paint layer as the panels (a separate `Order::Background`
/// layer can be reordered behind the UI by egui's hash-based layer merge).
pub fn paint(painter: &Painter, stars: &[Star], time: f32, sr: Rect) {
    painter.rect_filled(sr, eframe::egui::Rounding::ZERO, theme::BG_DEEP);

    for st in stars {
        let a = (st.base + st.amp * (time * st.speed + st.phase).sin()).clamp(0.0, 1.0);
        let pos = Pos2::new(sr.min.x + st.x * sr.width(), sr.min.y + st.y * sr.height());
        painter.circle_filled(pos, st.size, st.color.gamma_multiply(a));
    }
}
