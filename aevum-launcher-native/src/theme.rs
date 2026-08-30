//! Aevum Launcher — theme: fonts, colors, egui visual style.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};

// ---- Palette ----
pub const BG_DEEP: Color32 = Color32::from_rgb(5, 5, 13);
pub const ACCENT_1: Color32 = Color32::from_rgb(56, 189, 248);
pub const ACCENT_2: Color32 = Color32::from_rgb(129, 140, 248);
pub const ACCENT_3: Color32 = Color32::from_rgb(192, 132, 252);
pub const MAGENTA: Color32 = Color32::from_rgb(232, 121, 249);
pub const TEXT: Color32 = Color32::from_rgb(238, 240, 255);
pub const TEXT_DIM: Color32 = Color32::from_rgb(154, 160, 196);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(92, 98, 136);
#[allow(dead_code)]
pub const OK: Color32 = Color32::from_rgb(52, 211, 153);
#[allow(dead_code)]
pub const WARN: Color32 = Color32::from_rgb(251, 191, 36);
pub const DANGER: Color32 = Color32::from_rgb(248, 113, 113);

// ---- Alpha / glass surfaces ----
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(16, 18, 40, 242);
pub const PANEL_HOVER: Color32 = Color32::from_rgba_premultiplied(32, 36, 66, 245);
pub const PANEL_STRONG: Color32 = Color32::from_rgba_premultiplied(20, 22, 44, 250);
pub const BORDER: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);

// ---- Font families ----
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}
pub fn display_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display_bold".into()))
}
pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Install Orbitron (display) + Space Grotesk (body) fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "space_grotesk".into(),
        FontData::from_static(include_bytes!("../assets/fonts/space-grotesk-v22-latin-regular.ttf")),
    );
    fonts.font_data.insert(
        "space_grotesk_med".into(),
        FontData::from_static(include_bytes!("../assets/fonts/space-grotesk-v22-latin-500.ttf")),
    );
    fonts.font_data.insert(
        "space_grotesk_semibold".into(),
        FontData::from_static(include_bytes!("../assets/fonts/space-grotesk-v22-latin-600.ttf")),
    );
    fonts.font_data.insert(
        "orbitron_700".into(),
        FontData::from_static(include_bytes!("../assets/fonts/orbitron-v35-latin-700.ttf")),
    );
    fonts.font_data.insert(
        "orbitron_900".into(),
        FontData::from_static(include_bytes!("../assets/fonts/orbitron-v35-latin-900.ttf")),
    );

    fonts.families.insert(
        FontFamily::Proportional,
        vec![
            "space_grotesk_med".into(),
            "space_grotesk".into(),
            "NotoEmoji-Regular".into(),
            "emoji-icon-font".into(),
        ],
    );
    fonts.families.insert(
        FontFamily::Monospace,
        vec![
            "space_grotesk".into(),
            "NotoEmoji-Regular".into(),
            "emoji-icon-font".into(),
        ],
    );
    fonts.families.insert(
        FontFamily::Name("display".into()),
        vec!["orbitron_700".into(), "orbitron_900".into()],
    );
    fonts.families.insert(
        FontFamily::Name("display_bold".into()),
        vec!["orbitron_900".into(), "orbitron_700".into()],
    );

    ctx.set_fonts(fonts);
}

/// Apply the dark space visual style.
pub fn install_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = BG_DEEP;
    v.extreme_bg_color = BG_DEEP;
    v.faint_bg_color = Color32::from_white_alpha(8);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_2.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_2);
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.bg_fill = Color32::from_white_alpha(10);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    v.widgets.inactive.bg_fill = Color32::from_white_alpha(14);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(18, 20, 35, 36);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_2.gamma_multiply(0.7));
    v.widgets.active.bg_fill = Color32::from_rgba_premultiplied(28, 31, 54, 56);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(egui::TextStyle::Body, body(15.0));
    style.text_styles.insert(egui::TextStyle::Button, body(14.5));
    style.text_styles.insert(egui::TextStyle::Heading, display(22.0));
    style.text_styles.insert(egui::TextStyle::Small, body(12.5));
    style.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.5));
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    ctx.set_style(style);
}
