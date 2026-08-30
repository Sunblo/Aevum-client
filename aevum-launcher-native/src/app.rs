//! Aevum Launcher — main application: boot, title bar, sidebar, views, overlays.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui::{
    self, pos2, vec2, Color32, Frame, Id, Key, LayerId, Order, Pos2, Rect, RichText, ScrollArea,
    Sense, Stroke, ViewportCommand,
};

use crate::launcher;
use crate::paint;
use crate::starfield;
use crate::theme;

const TITLEBAR_H: f32 = 44.0;
const SIDEBAR_W: f32 = 232.0;

const BOOT_PHASES: &[(&str, u8)] = &[
    ("Initializing core systems", 12),
    ("Mounting Aevum kernel", 28),
    ("Scanning star charts", 46),
    ("Loading interface modules", 62),
    ("Warming up rendering core", 78),
    ("Synchronizing with orbit", 90),
    ("All systems nominal", 100),
];

#[derive(PartialEq, Clone, Copy, Debug)]
enum View {
    Play,
    Instance,
    Settings,
}

struct Boot {
    t: f32,
    step: usize,
    fade: f32,
}

/// Snapshot of the version manifest shared with the fetch thread.
struct ManifestState {
    entries: Vec<launcher::VersionEntry>,
    loading: bool,
    error: Option<String>,
}

impl Default for ManifestState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            loading: true,
            error: None,
        }
    }
}

pub struct AevumApp {
    time: f32,
    dt: f32,
    boot: Boot,
    booted: bool,
    view: View,
    view_epoch: u64,
    stars: Vec<starfield::Star>,

    // Real launcher state
    manifest: Arc<Mutex<ManifestState>>,
    username: String,
    selected_version: String,
    mem_gb: f32,
    launch_report: Arc<Mutex<launcher::LaunchReport>>,
    launch_thread: Option<JoinHandle<()>>,
    last_phase: launcher::Phase,

    // UI state
    account_modal: bool,
    version_menu: bool,
    vol: f32,
    anim_on: bool,
    reduce_motion: bool,
    sound_on: bool,
    toast: Option<(String, f32)>,
}

impl AevumApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_visuals(&cc.egui_ctx);

        let manifest = Arc::new(Mutex::new(ManifestState::default()));
        let manifest_fetch = manifest.clone();
        std::thread::spawn(move || {
            let loaded = launcher::load_cached_manifest()
                .ok()
                .filter(|v| !v.is_empty())
                .map(Ok)
                .unwrap_or_else(|| launcher::fetch_manifest());
            if let Ok(mut m) = manifest_fetch.lock() {
                match loaded {
                    Ok(entries) => {
                        m.entries = entries;
                        m.error = None;
                    }
                    Err(e) => {
                        m.error = Some(e);
                        m.loading = false;
                        return;
                    }
                }
                m.loading = false;
            }
        });

        let selected_version = String::new();
        let username = "Player".to_string();

        Self {
            time: 0.0,
            dt: 0.0,
            boot: Boot {
                t: 0.0,
                step: 0,
                fade: 1.0,
            },
            booted: false,
            view: View::Play,
            view_epoch: 0,
            stars: starfield::spawn(140),

            manifest,
            username,
            selected_version,
            mem_gb: 4.0,
            launch_report: Arc::new(Mutex::new(launcher::LaunchReport::default())),
            launch_thread: None,
            last_phase: launcher::Phase::Idle,

            account_modal: false,
            version_menu: false,
            vol: 70.0,
            anim_on: true,
            reduce_motion: false,
            sound_on: true,
            toast: None,
        }
    }

    fn toast(&mut self, msg: String) {
        self.toast = Some((msg, 0.0));
    }

    fn launching(&self) -> bool {
        let ph = self
            .launch_report
            .lock()
            .map(|r| r.phase)
            .unwrap_or(launcher::Phase::Idle);
        matches!(
            ph,
            launcher::Phase::Fetching
                | launcher::Phase::Downloading
                | launcher::Phase::Extracting
                | launcher::Phase::Launching
        )
    }

    fn game_running(&self) -> bool {
        let ph = self
            .launch_report
            .lock()
            .map(|r| r.phase)
            .unwrap_or(launcher::Phase::Idle);
        ph == launcher::Phase::Running
    }

    fn manifest_snapshot(&self) -> Vec<launcher::VersionEntry> {
        self.manifest
            .lock()
            .map(|m| m.entries.clone())
            .unwrap_or_default()
    }

    fn manifest_loading(&self) -> bool {
        self.manifest.lock().map(|m| m.loading).unwrap_or(false)
    }

    fn manifest_error(&self) -> Option<String> {
        self.manifest.lock().ok().and_then(|m| m.error.clone())
    }

    fn refresh_manifest(&mut self) {
        let manifest = self.manifest.clone();
        if let Ok(mut m) = manifest.lock() {
            m.loading = true;
            m.error = None;
        }
        std::thread::spawn(move || match launcher::fetch_manifest() {
            Ok(entries) => {
                if let Ok(mut m) = manifest.lock() {
                    m.entries = entries;
                    m.loading = false;
                }
            }
            Err(e) => {
                if let Ok(mut m) = manifest.lock() {
                    m.error = Some(e);
                    m.loading = false;
                }
            }
        });
    }

    fn start_launch(&mut self) {
        if self.launching() {
            return;
        }
        if self.selected_version.is_empty() {
            self.toast("No game version selected".into());
            return;
        }
        let report = self.launch_report.clone();
        let profile = launcher::Profile {
            version_id: self.selected_version.clone(),
            username: self.username.clone(),
            ram_mb: (self.mem_gb * 1024.0).max(512.0) as u32,
        };
        self.launch_thread = Some(std::thread::spawn(move || {
            launcher::run_launch(profile, report);
        }));
    }

    fn stop_launch(&self) {
        if let Ok(r) = self.launch_report.lock() {
            if let Some(pid) = r.pid {
                launcher::kill_pid(pid);
            }
        }
    }

    fn open_path(&self, path: &Path) {
        let p = path.to_string_lossy().to_string();
        let cmd = if cfg!(windows) {
            ("explorer", p)
        } else if cfg!(target_os = "macos") {
            ("open", p)
        } else {
            ("xdg-open", p)
        };
        let _ = Command::new(cmd.0).arg(cmd.1).spawn();
    }

    fn mods_folder_jars(&self) -> Vec<(String, u64)> {
        let dir = launcher::game_dir().join("mods");
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "jar").unwrap_or(false) {
                    let name = e.file_name().to_string_lossy().to_string();
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push((name, size));
                }
            }
        }
        out
    }
}

// ---- Shared custom widgets (free functions so they don't conflict with &mut self) ----

fn slider(ui: &mut egui::Ui, track: Rect, value: &mut f32, min: f32, max: f32, id: Id) {
    let resp = ui.interact(track, id, Sense::click_and_drag());
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            *value = min + t * (max - min);
        }
    }
    let painter = ui.painter();
    painter.rect_filled(track, paint::cr(4.0), Color32::from_white_alpha(26));
    let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
    if t > 0.0 {
        let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * t, track.max.y));
        painter.rect_filled(fill, paint::cr(4.0), theme::ACCENT_2);
    }
    let knob_x = paint::lerp(track.left(), track.right(), t);
    painter.circle_filled(pos2(knob_x, track.center().y), 8.0, Color32::WHITE);
    painter.circle_stroke(pos2(knob_x, track.center().y), 8.0, Stroke::new(2.0_f32, theme::ACCENT_2));
}

fn switch(ui: &mut egui::Ui, sw: Rect, on: &mut bool, id: Id) -> bool {
    let resp = ui.interact(sw, Id::new(("switch", id)), Sense::click());
    let painter = ui.painter();
    let k = ui.ctx().animate_bool(Id::new(("swanim", id)), *on);
    let mut changed = false;
    if resp.clicked() {
        *on = !*on;
        changed = true;
    }
    let track = Rect::from_center_size(sw.center(), vec2(sw.width(), 22.0));
    if *on {
        paint::rect(&painter, track, paint::cr(999.0), theme::ACCENT_2, Stroke::NONE);
    } else {
        paint::rect(&painter, track, paint::cr(999.0), Color32::from_white_alpha(28), Stroke::NONE);
    }
    let knob_x = paint::lerp(track.min.x + 3.0 + 9.0, track.max.x - 3.0 - 9.0, k);
    painter.circle_filled(pos2(knob_x, track.center().y), 9.0, Color32::WHITE);
    changed
}

impl eframe::App for AevumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        self.dt = dt;
        self.time += dt;

        let busy = self.launching()
            || self.game_running()
            || self.manifest_loading()
            || self.launch_report.lock().map(|r| r.phase).unwrap_or(launcher::Phase::Idle)
                == launcher::Phase::Error;
        if !self.reduce_motion || busy {
            ctx.request_repaint_after(Duration::from_millis(33));
        }

        // Ambient background is painted inside main_ui so it stays on the
        // same deterministic layer as the panels (a dedicated Background
        // layer can be reordered behind the UI by egui's layer merge).

        // Boot progression
        if !self.booted {
            self.boot.t += dt;
            const PHASE: f32 = 0.45;
            self.boot.step =
                ((self.boot.t / PHASE) as usize).min(BOOT_PHASES.len() - 1);
            let total = BOOT_PHASES.len() as f32 * PHASE;
            if self.boot.t >= total + 0.35 {
                self.boot.fade = (self.boot.fade - dt / 0.8).max(0.0);
                if self.boot.fade <= 0.0 {
                    self.booted = true;
                }
            }
        }

        // Pick a default version once the manifest arrives.
        if self.selected_version.is_empty() {
            let entries = self.manifest_snapshot();
            if let Some(rel) = entries.iter().find(|v| v.kind == "release") {
                self.selected_version = rel.id.clone();
            }
        }

        // Fire one-shot toasts on launch state transitions.
        let current_phase = self.launch_report.lock().map(|r| r.phase).unwrap_or(launcher::Phase::Idle);
        if current_phase != self.last_phase {
            match current_phase {
                launcher::Phase::Running => {
                    let pid = self.launch_report.lock().unwrap().pid.unwrap_or(0);
                    self.toast(format!("Game launched — process {}", pid));
                }
                launcher::Phase::Exited => {
                    let code = self.launch_report.lock().unwrap().exit_code.unwrap_or(-1);
                    self.toast(format!("Game closed (exit code {})", code));
                }
                launcher::Phase::Error => {
                    let msg = self
                        .launch_report
                        .lock()
                        .unwrap()
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string());
                    self.toast(format!("Launch failed: {}", msg));
                }
                _ => {}
            }
            self.last_phase = current_phase;
        }

        // Drop the finished thread handle so a new launch is allowed.
        if self.launching() == false && self.game_running() == false && self.launch_thread.is_some() {
            let _ = self.launch_thread.take();
        }

        self.main_ui(ctx);

        if !self.booted {
            self.boot_ui(ctx);
        }

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.account_modal = false;
            self.version_menu = false;
        }
    }
}

impl AevumApp {
    // =========================================================
    //  Boot screen
    // =========================================================
    fn boot_ui(&mut self, ctx: &egui::Context) {
        let k = self.boot.fade;
        if k <= 0.0 {
            return;
        }
        let sr = ctx.screen_rect();
        let c = sr.center();
        let t = self.boot.t;

        egui::Area::new(Id::new("boot_area"))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                // blocking backdrop so the main UI cannot be interacted with
                ui.interact(sr, Id::new("boot_block"), Sense::click());
                let painter = ui.painter();

                painter.rect_filled(sr, paint::zero(), theme::BG_DEEP.gamma_multiply(k));

                // orbit rings
                let rot = t * 1.2;
                paint::arc(&painter, c, 62.0, 2.0, theme::ACCENT_1.gamma_multiply(k), rot, 3.2);
                paint::arc(
                    &painter,
                    c,
                    62.0,
                    2.0,
                    theme::ACCENT_3.gamma_multiply(k * 0.9),
                    rot + std::f32::consts::PI,
                    3.2,
                );
                let rot2 = -t * 1.9;
                paint::arc(
                    &painter,
                    c,
                    45.0,
                    1.4,
                    theme::MAGENTA.gamma_multiply(k * 0.75),
                    rot2,
                    2.0,
                );

                // satellites
                let sa = pos2(c.x + 62.0 * rot.cos(), c.y + 62.0 * rot.sin());
                painter.circle_filled(sa, 4.0, theme::ACCENT_1.gamma_multiply(k));
                let sb = pos2(c.x + 45.0 * rot2.cos(), c.y + 45.0 * rot2.sin());
                painter.circle_filled(sb, 3.0, theme::MAGENTA.gamma_multiply(k));

                // core logo
                let core = Rect::from_center_size(c, vec2(66.0, 66.0));
                paint::glow(&painter, core, paint::cr(18.0), theme::ACCENT_2, 42.0, 0.5 * k);
                paint::rect(
                    &painter,
                    core,
                    paint::cr(18.0),
                    theme::ACCENT_2,
                    Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.25 * k)),
                );
                paint::rect(
                    &painter,
                    Rect::from_min_max(
                        core.min + vec2(0.0, 2.0),
                        pos2(core.max.x, core.min.y + 27.0),
                    ),
                    paint::cr(18.0),
                    Color32::WHITE.gamma_multiply(0.12 * k),
                    Stroke::NONE,
                );
                paint::text(&painter, c, "A", theme::display_bold(30.0), Color32::WHITE.gamma_multiply(k));

                // title
                paint::text(
                    &painter,
                    pos2(c.x, c.y + 98.0),
                    "AEVUM LAUNCHER",
                    theme::display(19.0),
                    theme::TEXT.gamma_multiply(k),
                );
                paint::text(
                    &painter,
                    pos2(c.x, c.y + 124.0),
                    "BEYOND THE HORIZON",
                    theme::body(10.5),
                    theme::TEXT_FAINT.gamma_multiply(k),
                );

                // progress bar
                let bar = Rect::from_center_size(pos2(c.x, c.y + 176.0), vec2(300.0, 4.0));
                let pct = (t / (BOOT_PHASES.len() as f32 * 0.45)).clamp(0.0, 1.0);
                painter.rect_filled(
                    bar,
                    paint::cr(4.0),
                    Color32::from_white_alpha((18.0 * k) as u8),
                );
                if pct > 0.0 {
                    let fw = bar.width() * pct;
                    painter.rect_filled(
                        Rect::from_min_max(bar.min, pos2(bar.min.x + fw, bar.max.y)),
                        paint::cr(4.0),
                        theme::ACCENT_1,
                    );
                }

                // status
                let phase = BOOT_PHASES[self.boot.step];
                paint::text(
                    &painter,
                    pos2(c.x, bar.max.y + 24.0),
                    phase.0,
                    theme::body(12.5),
                    theme::TEXT_DIM.gamma_multiply(k),
                );

                // dots
                let dots_y = c.y + 216.0;
                for i in 0..3 {
                    let da = ((t * 4.0 + i as f32 * 0.6).sin() * 0.5 + 0.5).min(1.0);
                    painter.circle_filled(
                        pos2(c.x - 12.0 + i as f32 * 12.0, dots_y),
                        2.6,
                        theme::ACCENT_2.gamma_multiply(da * k),
                    );
                }
            });
    }

    // =========================================================
    //  Main window
    // =========================================================
    fn main_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let full = ui.max_rect();

                // Background: solid or starfield, painted first so every panel draws on top.
                let bg = ui.painter();
                if self.reduce_motion {
                    bg.rect_filled(full, paint::zero(), theme::BG_DEEP);
                } else {
                    starfield::paint(bg, &self.stars, self.time, full);
                }

                let tb = Rect::from_min_max(full.min, pos2(full.max.x, full.min.y + TITLEBAR_H));
                let body = Rect::from_min_max(pos2(full.min.x, tb.max.y), full.max);
                let side = Rect::from_min_max(body.min, pos2(body.min.x + SIDEBAR_W, body.max.y));
                let content = Rect::from_min_max(pos2(side.max.x, body.min.y), body.max);

                self.view_area(ui, content);
                self.sidebar_ui(ui, side);
                self.titlebar_ui(ui, tb);
            });

        self.toast_ui(ctx);

        if self.account_modal {
            self.account_modal_ui(ctx);
        }

        self.launch_ui(ctx);
    }

    // =========================================================
    //  Title bar
    // =========================================================
    fn titlebar_ui(&mut self, ui: &mut egui::Ui, tb: Rect) {
        let painter = ui.painter();
        painter.rect_filled(tb, paint::zero(), Color32::from_rgba_premultiplied(12, 12, 28, 248));
        painter.line_segment(
            [pos2(tb.min.x, tb.max.y), pos2(tb.max.x, tb.max.y)],
            Stroke::new(1.0_f32, theme::BORDER),
        );

        // brand
        let cy = tb.center().y;
        let logo_r = Rect::from_min_size(pos2(tb.min.x + 16.0, cy - 12.0), vec2(24.0, 24.0));
        paint::rect(
            &painter,
            logo_r,
            paint::cr(7.0),
            theme::ACCENT_2,
            Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.3)),
        );
        paint::text(
            &painter,
            logo_r.center(),
            "A",
            theme::display_bold(13.0),
            Color32::WHITE,
        );
        paint::text_left(
            &painter,
            pos2(logo_r.max.x + 10.0, cy - 9.0),
            "AEVUM ",
            theme::display(11.0),
            theme::ACCENT_1,
        );
        paint::text_left(
            &painter,
            pos2(logo_r.max.x + 62.0, cy - 9.0),
            "LAUNCHER",
            theme::display(11.0),
            theme::TEXT,
        );

        // window controls
        let bw = 46.0;
        let close_r = Rect::from_min_size(pos2(tb.max.x - bw, tb.min.y), vec2(bw, TITLEBAR_H));
        let max_r = close_r.translate(vec2(-bw, 0.0));
        let min_r = max_r.translate(vec2(-bw, 0.0));

        // drag region between brand and controls
        let drag_r = Rect::from_min_max(
            pos2(tb.min.x + 170.0, tb.min.y),
            pos2(min_r.min.x, tb.max.y),
        );

        let maximized = ui.ctx().input(|i| i.viewport().maximized).unwrap_or(false);

        if ui
            .ctx()
            .input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
            && drag_r.contains(ui.ctx().input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO))
        {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        if ui.ctx().input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()) {
                if drag_r.contains(p) {
                    ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }
        }

        let mut action: Option<&'static str> = None;
        self.win_button(ui, min_r, "min", |_| action = Some("min"));
        self.win_button(ui, max_r, "max", |_| action = Some("max"));
        self.win_button(ui, close_r, "close", |_| action = Some("close"));

        match action {
            Some("min") => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
            Some("max") => {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Maximized(!maximized))
            }
            Some("close") => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
            _ => {}
        }
    }

    fn win_button(&mut self, ui: &mut egui::Ui, r: Rect, kind: &str, on_click: impl FnOnce(&mut Self)) {
        let resp = ui.interact(r, Id::new(("winbtn", kind)), Sense::click());
        let painter = ui.painter();
        let hover = resp.hovered();

        if hover || resp.is_pointer_button_down_on() {
            let fill = if kind == "close" {
                theme::DANGER
            } else {
                Color32::from_white_alpha(22)
            };
            painter.rect_filled(r, paint::zero(), fill);
        }

        let c = r.center();
        let col = if kind == "close" && hover {
            Color32::WHITE
        } else {
            theme::TEXT_DIM
        };

        match kind {
            "min" => {
                painter.line_segment(
                    [pos2(c.x - 4.5, c.y), pos2(c.x + 4.5, c.y)],
                    Stroke::new(1.5_f32, col),
                );
            }
            "max" => {
                let m = if self.window_maximized() {
                    Rect::from_center_size(c + vec2(1.0, -1.0), vec2(9.0, 9.0))
                } else {
                    Rect::from_center_size(c, vec2(9.0, 9.0))
                };
                painter.rect_stroke(m, 1.0, Stroke::new(1.5_f32, col));
            }
            _ => {
                painter.line_segment(
                    [pos2(c.x - 4.5, c.y - 4.5), pos2(c.x + 4.5, c.y + 4.5)],
                    Stroke::new(1.5_f32, col),
                );
                painter.line_segment(
                    [pos2(c.x - 4.5, c.y + 4.5), pos2(c.x + 4.5, c.y - 4.5)],
                    Stroke::new(1.5_f32, col),
                );
            }
        }

        if resp.clicked() {
            on_click(self);
        }
    }

    fn window_maximized(&self) -> bool {
        false // refreshed each frame via ctx where needed
    }

    // =========================================================
    //  Sidebar
    // =========================================================
    fn sidebar_ui(&mut self, ui: &mut egui::Ui, side: Rect) {
        let painter = ui.painter();
        painter.rect_filled(side, paint::zero(), Color32::from_rgba_premultiplied(12, 12, 28, 248));
        painter.line_segment(
            [pos2(side.max.x, side.min.y), pos2(side.max.x, side.max.y)],
            Stroke::new(1.0_f32, theme::BORDER),
        );

        let mut y = side.min.y + 26.0;
        let items: [(&str, &str, View); 3] = [
            ("P", "Play", View::Play),
            ("I", "Instance", View::Instance),
            ("S", "Settings", View::Settings),
        ];

        for (i, (letter, label, v)) in items.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(side.min.x + 14.0, y),
                vec2(side.width() - 28.0, 48.0),
            );
            let active = self.view == *v;
            let resp = ui.interact(r, Id::new(("nav", i)), Sense::click());

            if resp.hovered() || active {
                if !active {
                    painter.rect_filled(r, paint::cr(12.0), theme::PANEL_HOVER);
                } else {
                    painter.rect_filled(
                        r,
                        paint::cr(12.0),
                        Color32::from_rgba_premultiplied(11, 12, 21, 22),
                    );
                }
            }

            if active {
                painter.line_segment(
                    [pos2(r.min.x, r.center().y - 14.0), pos2(r.min.x, r.center().y + 14.0)],
                    Stroke::new(3.0_f32, theme::ACCENT_2),
                );
            }

            // icon box
            let icon = Rect::from_min_size(pos2(r.min.x + 10.0, r.center().y - 17.0), vec2(34.0, 34.0));
            if active {
                paint::glow(&painter, icon, paint::cr(10.0), theme::ACCENT_2, 16.0, 0.5);
                paint::rect(
                    &painter,
                    icon,
                    paint::cr(10.0),
                    theme::ACCENT_2,
                    Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.35)),
                );
            } else {
                painter.rect_filled(icon, paint::cr(10.0), Color32::from_white_alpha(12));
                painter.rect_stroke(icon, paint::cr(10.0), Stroke::new(1.0_f32, theme::BORDER));
            }
            paint::text(
                &painter,
                icon.center(),
                letter,
                theme::display_bold(13.0),
                if active { Color32::WHITE } else { theme::TEXT_DIM },
            );

            paint::text_left(
                &painter,
                pos2(icon.max.x + 12.0, r.center().y - 9.0),
                label,
                theme::body(14.5),
                if active { theme::TEXT } else { theme::TEXT_DIM },
            );

            if resp.clicked() && !active {
                self.view = *v;
                self.view_epoch += 1;
            }

            y += 54.0;
        }

        // account chip
        let chip_h = 60.0;
        let chip = Rect::from_min_size(
            pos2(side.min.x + 12.0, side.max.y - chip_h - 14.0),
            vec2(side.width() - 24.0, chip_h),
        );
        let resp = ui.interact(chip, Id::new("account_chip"), Sense::click());

        let hover = resp.hovered();
        paint::rect(
            &painter,
            chip,
            paint::cr(14.0),
            if hover { theme::PANEL_HOVER } else { theme::PANEL },
            Stroke::new(1.0_f32, if hover { theme::ACCENT_2.gamma_multiply(0.8) } else { theme::BORDER }),
        );

        // avatar
        let av = Rect::from_center_size(pos2(chip.min.x + 30.0, chip.center().y), vec2(38.0, 38.0));
        painter.circle_filled(av.center(), 19.0, theme::ACCENT_2);
        paint::arc(&painter, av.center(), 19.0, 3.0, theme::ACCENT_1, -0.6, std::f32::consts::PI + 1.2);
        let initial = self.username.chars().next().unwrap_or('?');
        let initial_str = initial.to_string();
        paint::text(
            &painter,
            av.center(),
            &initial_str,
            theme::display_bold(15.0),
            Color32::WHITE,
        );

        let uname = if self.username.is_empty() { "Offline" } else { self.username.as_str() };
        let vers = if self.selected_version.is_empty() { "no version" } else { self.selected_version.as_str() };
        paint::text_left(
            &painter,
            pos2(av.max.x + 11.0, chip.min.y + 13.0),
            uname,
            theme::body(13.5),
            theme::TEXT,
        );
        paint::text_left(
            &painter,
            pos2(av.max.x + 11.0, chip.min.y + 33.0),
            &format!("Offline profile · {}", vers),
            theme::body(11.0),
            theme::TEXT_FAINT,
        );

        paint::chevron(
            &painter,
            pos2(chip.max.x - 16.0, chip.center().y),
            theme::TEXT_FAINT,
            true,
        );

        if resp.clicked() {
            self.account_modal = true;
        }
    }

    // =========================================================
    //  Content area
    // =========================================================
    fn view_area(&mut self, ui: &mut egui::Ui, content: Rect) {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content), |sui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(sui, |ssui| {
                    match self.view {
                        View::Play => self.play_view(ssui, content.width()),
                        View::Instance => self.instance_view(ssui, content.width()),
                        View::Settings => self.settings_view(ssui, content.width()),
                    }
                });

            // view transition wipe
            let k = sui
                .ctx()
                .animate_value_with_time(Id::new(("viewin", self.view_epoch)), 1.0, 0.42);
            if k < 1.0 {
                let p = sui.painter();
                p.rect_filled(content, paint::zero(), theme::BG_DEEP.gamma_multiply(1.0 - k));
            }
        });
    }

    // =========================================================
    //  Play view
    // =========================================================
    fn play_view(&mut self, ui: &mut egui::Ui, width: f32) {
        let pad = 34.0;
        let w = (width - pad * 2.0).max(320.0);

        let (h, _) = ui.allocate_exact_size(vec2(w, 62.0), Sense::hover());
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(h.min.x, h.min.y), "sector / 01", theme::body(11.0), theme::TEXT_FAINT);
            paint::text_left(&painter, pos2(h.min.x, h.min.y + 15.0), "Launch Control", theme::display(23.0), theme::TEXT);
        }

        // version dropdown
        let dd = Rect::from_min_size(
            pos2(h.max.x - 210.0, h.min.y + 8.0),
            vec2(210.0, 40.0),
        );
        self.version_dropdown(ui, dd);

        ui.add_space(20.0);

        let stage_h = 330.0;
        let (stage, _) = ui.allocate_exact_size(vec2(w, stage_h), Sense::hover());
        self.launch_stage(ui, stage);

        ui.add_space(22.0);

        // install / refresh row
        let (lh, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(lh.min.x, lh.min.y), "Installation", theme::body(14.5), theme::TEXT);
        }
        ui.add_space(12.0);

        let row_h = 64.0;
        let (row, _) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
        {
            let painter = ui.painter();
            paint::rect(&painter, row, paint::cr(14.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
            let game = launcher::game_dir();
            paint::text_left(&painter, pos2(row.min.x + 16.0, row.min.y + 11.0), "Game directory", theme::body(12.0), theme::TEXT_FAINT);
            paint::text_left(&painter, pos2(row.min.x + 16.0, row.min.y + 30.0), &game.display().to_string(), theme::body(12.5), theme::TEXT);
        }
        let open_btn = Rect::from_min_size(pos2(row.max.x - 210.0, row.min.y + 15.0), vec2(96.0, 34.0));
        let refresh_btn = Rect::from_min_size(pos2(row.max.x - 104.0, row.min.y + 15.0), vec2(88.0, 34.0));
        self.small_button(ui, open_btn, "Open folder", |this| {
            this.open_path(&launcher::game_dir());
        });
        self.small_button(ui, refresh_btn, "Refresh", |this| {
            this.refresh_manifest();
        });

        ui.add_space(18.0);
    }

    fn small_button(&mut self, ui: &mut egui::Ui, r: Rect, label: &str, on_click: impl FnOnce(&mut Self)) {
        let resp = ui.interact(r, Id::new(("smallbtn", label)), Sense::click());
        let painter = ui.painter();
        let hover = resp.hovered();
        paint::rect(
            &painter,
            r,
            paint::cr(10.0),
            if hover { Color32::from_rgba_premultiplied(20, 22, 39, 40) } else { Color32::from_white_alpha(12) },
            Stroke::new(1.0_f32, if hover { theme::ACCENT_2.gamma_multiply(0.9) } else { theme::BORDER }),
        );
        paint::text(&painter, r.center(), label, theme::body(12.5), theme::TEXT);
        if resp.clicked() {
            on_click(self);
        }
    }

    fn version_dropdown(&mut self, ui: &mut egui::Ui, r: Rect) {
        let resp = ui.interact(r, Id::new("version_dd"), Sense::click());
        let painter = ui.painter();
        let hover = resp.hovered();
        paint::rect(
            &painter,
            r,
            paint::cr(12.0),
            theme::PANEL_STRONG,
            Stroke::new(1.0_f32, if hover { theme::ACCENT_2.gamma_multiply(0.9) } else { theme::BORDER }),
        );
        let label = if let Some(err) = self.manifest_error() {
            format!("Manifest error: {}", err)
        } else if self.manifest_loading() {
            "Loading versions…".to_string()
        } else if self.selected_version.is_empty() {
            "No version selected".to_string()
        } else {
            self.selected_version.clone()
        };
        paint::text_left(&painter, pos2(r.min.x + 14.0, r.center().y - 8.0), &label, theme::body(13.5), theme::TEXT);
        paint::chevron(&painter, pos2(r.max.x - 16.0, r.center().y), theme::TEXT_DIM, true);

        if resp.clicked() {
            self.version_menu = !self.version_menu;
        }

        if self.version_menu {
            let entries = self.manifest_snapshot();
            let selected = self.selected_version.clone();
            let menu_pos = pos2(r.min.x, r.max.y + 6.0);
            egui::Area::new(Id::new("version_menu"))
                .order(Order::Foreground)
                .fixed_pos(menu_pos)
                .show(ui.ctx(), |mui| {
                    Frame::popup(&mui.ctx().style()).show(mui, |pui| {
                        pui.set_min_width(232.0);
                        ScrollArea::vertical().max_height(340.0).show(pui, |sui| {
                            for e in &entries {
                                let is_sel = e.id == selected;
                                let kind_tag = if e.kind == "release" { "release" } else { "snapshot" };
                                let txt = RichText::new(format!("{}    {}", e.id, kind_tag))
                                    .font(theme::body(12.5))
                                    .color(if is_sel { theme::ACCENT_1 } else { theme::TEXT_DIM });
                                if sui.add(egui::Button::new(txt).frame(false)).clicked() {
                                    self.selected_version = e.id.clone();
                                    self.version_menu = false;
                                    self.toast(format!("Selected version {}", e.id));
                                }
                            }
                            if entries.is_empty() {
                                sui.label(
                                    RichText::new("No versions available").font(theme::body(12.5)).color(theme::TEXT_FAINT),
                                );
                            }
                        });
                    });
                });
        }
    }

    fn launch_stage(&mut self, ui: &mut egui::Ui, r: Rect) {
        let painter = ui.painter();
        paint::rect(&painter, r, paint::cr(16.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
        paint::glow(&painter, r.shrink(60.0).translate(vec2(0.0, -40.0)), paint::cr(60.0), theme::ACCENT_2, 90.0, 0.08);

        let cx = r.center().x;
        let cy = r.min.y + 104.0;

        // orbit ring
        let rot = self.time * 0.9;
        paint::arc(&painter, pos2(cx, cy), 62.0, 1.5, theme::ACCENT_1.gamma_multiply(0.8), rot, 4.5);
        paint::arc(
            &painter,
            pos2(cx, cy),
            62.0,
            1.5,
            theme::ACCENT_3.gamma_multiply(0.8),
            rot + std::f32::consts::PI,
            4.5,
        );
        painter.circle_filled(
            pos2(cx + 62.0 * rot.cos(), cy + 62.0 * rot.sin()),
            3.5,
            theme::ACCENT_1,
        );
        paint::cube(&painter, pos2(cx, cy), 80.0, self.time * 0.7);

        // version chip
        let ver = if self.selected_version.is_empty() {
            "no version"
        } else {
            self.selected_version.as_str()
        };
        let chip = Rect::from_center_size(pos2(cx, r.min.y + 172.0), vec2(130.0, 26.0));
        paint::rect(
            &painter,
            chip,
            paint::cr(13.0),
            Color32::from_rgba_premultiplied(6, 19, 25, 26),
            Stroke::new(1.0_f32, theme::ACCENT_1.gamma_multiply(0.5)),
        );
        paint::text(&painter, chip.center(), ver, theme::body(12.5), theme::ACCENT_1);

        // status from launch report
        let report = self.launch_report.lock().unwrap();
        let status_msg = if report.error.is_some() {
            report.error.clone().unwrap_or_default()
        } else if !report.message.is_empty() {
            report.message.clone()
        } else {
            "Ready — pick a version and launch".to_string()
        };
        let pct = report.progress_pct();
        let phase = report.phase;
        let exit_code = report.exit_code;
        let busy = matches!(
            phase,
            launcher::Phase::Fetching
                | launcher::Phase::Downloading
                | launcher::Phase::Extracting
                | launcher::Phase::Launching
        );
        let failed = phase == launcher::Phase::Error;
        let running = phase == launcher::Phase::Running;
        drop(report);

        paint::text(&painter, pos2(cx, r.min.y + 205.0), "Aevum Launcher", theme::display(15.0), theme::TEXT);
        paint::text(&painter, pos2(cx, r.min.y + 226.0), &status_msg, theme::body(12.0), theme::TEXT_DIM);

        // progress bar
        let bar = Rect::from_center_size(pos2(cx, r.min.y + 250.0), vec2(320.0, 4.0));
        painter.rect_filled(bar, paint::cr(4.0), Color32::from_white_alpha(22));
        if busy && pct > 0.0 {
            let fill = Rect::from_min_max(bar.min, pos2(bar.min.x + bar.width() * pct, bar.max.y));
            painter.rect_filled(fill, paint::cr(4.0), theme::ACCENT_1);
        }

        // action button
        let btn = Rect::from_min_size(pos2(cx - 110.0, r.min.y + 268.0), vec2(220.0, 46.0));
        let label = if failed {
            "RETRY"
        } else if busy {
            if pct > 0.0 {
                &format!("{}%", (pct * 100.0) as i32)
            } else {
                "WORKING…"
            }
        } else if running {
            "STOP GAME"
        } else if exit_code.is_some() {
            "LAUNCH AGAIN"
        } else {
            "LAUNCH"
        };
        let resp = ui.interact(btn, Id::new("launch_btn"), Sense::click());
        let hover = resp.hovered();
        let down = resp.is_pointer_button_down_on();
        let disabled = busy;
        let pulse = 0.5 + 0.5 * (self.time * 2.2).sin();
        let spread = if hover { 34.0 + pulse * 12.0 } else { 20.0 + pulse * 8.0 };
        if !disabled {
            paint::glow(&painter, btn, paint::cr(999.0), theme::ACCENT_2, spread, if hover { 0.5 } else { 0.35 });
        }
        paint::rect(
            &painter,
            btn,
            paint::cr(999.0),
            if disabled {
                Color32::from_rgba_premultiplied(42, 45, 75, 120)
            } else if down {
                theme::ACCENT_3
            } else {
                theme::ACCENT_2
            },
            Stroke::NONE,
        );
        paint::text(
            &painter,
            btn.center(),
            label,
            theme::display_bold(13.5),
            if disabled { theme::TEXT_FAINT } else { Color32::WHITE },
        );

        if resp.clicked() && !disabled {
            if running {
                self.stop_launch();
            } else {
                self.start_launch();
            }
        }
    }

    // =========================================================
    //  Instance view (real game directory)
    // =========================================================
    fn instance_view(&mut self, ui: &mut egui::Ui, width: f32) {
        let pad = 34.0;
        let w = (width - pad * 2.0).max(320.0);

        let (h, _) = ui.allocate_exact_size(vec2(w, 62.0), Sense::hover());
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(h.min.x, h.min.y), "sector / 02", theme::body(11.0), theme::TEXT_FAINT);
            paint::text_left(&painter, pos2(h.min.x, h.min.y + 15.0), "Instance", theme::display(23.0), theme::TEXT);
        }

        ui.add_space(12.0);

        let game = launcher::game_dir();
        let root = launcher::root_dir();
        let mods_dir = game.join("mods");

        // directory card
        let card_h = 150.0;
        let (card, _) = ui.allocate_exact_size(vec2(w, card_h), Sense::hover());
        {
            let painter = ui.painter();
            paint::rect(&painter, card, paint::cr(16.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
            paint::text_left(&painter, pos2(card.min.x + 20.0, card.min.y + 16.0), "Game directory", theme::body(10.5), theme::TEXT_FAINT);
            paint::text_left(&painter, pos2(card.min.x + 20.0, card.min.y + 36.0), &game.display().to_string(), theme::body(13.0), theme::TEXT);
            paint::text_left(&painter, pos2(card.min.x + 20.0, card.min.y + 76.0), "Launcher root", theme::body(10.5), theme::TEXT_FAINT);
            paint::text_left(&painter, pos2(card.min.x + 20.0, card.min.y + 96.0), &root.display().to_string(), theme::body(13.0), theme::TEXT);
        }

        // action buttons
        ui.add_space(12.0);
        let btn_h = 44.0;
        let btn_w = (w - 10.0 * 3.0) / 4.0;
        for (i, (label, target)) in [
            ("Game", game.clone()),
            ("Mods", mods_dir.clone()),
            ("Logs", game.join("logs")),
            ("Assets", launcher::assets_dir()),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::from_min_size(
                pos2(card.min.x + i as f32 * (btn_w + 10.0), card.max.y + 12.0),
                vec2(btn_w, btn_h),
            );
            let resp = ui.interact(r, Id::new(("dirbtn", i)), Sense::click());
            let painter = ui.painter();
            let hover = resp.hovered();
            paint::rect(
                &painter,
                r,
                paint::cr(12.0),
                if hover { Color32::from_rgba_premultiplied(20, 22, 39, 40) } else { Color32::from_white_alpha(12) },
                Stroke::new(1.0_f32, if hover { theme::ACCENT_2.gamma_multiply(0.9) } else { theme::BORDER }),
            );
            paint::text(&painter, r.center(), format!("{} →", label).as_str(), theme::body(12.5), theme::TEXT);
            if resp.clicked() {
                self.open_path(target);
            }
        }

        ui.add_space(26.0);

        // mods header
        let (mh, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(mh.min.x, mh.min.y), "Installed Mods", theme::body(14.5), theme::TEXT);
        }
        ui.add_space(10.0);

        let jars = self.mods_folder_jars();
        if jars.is_empty() {
            let (empty, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
            let painter = ui.painter();
            paint::rect(&painter, empty, paint::cr(12.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
            paint::text(
                &painter,
                empty.center(),
                "No mods yet — drop .jar files into the mods folder",
                theme::body(12.5),
                theme::TEXT_FAINT,
            );
        } else {
            for (i, (name, size)) in jars.iter().enumerate() {
                let (row, _) = ui.allocate_exact_size(vec2(w, 46.0), Sense::hover());
                let painter = ui.painter();
                paint::rect(
                    &painter,
                    row,
                    paint::cr(12.0),
                    theme::PANEL,
                    Stroke::new(1.0_f32, theme::BORDER),
                );
                paint::text_left(&painter, pos2(row.min.x + 16.0, row.center().y - 7.0), name, theme::body(13.0), theme::TEXT);
                paint::text_right(
                    &painter,
                    pos2(row.max.x - 16.0, row.center().y - 7.0),
                    &format!("{:.1} MB", *size as f64 / 1048576.0),
                    theme::body(11.5),
                    theme::TEXT_FAINT,
                );
                let _ = i;
            }
        }

        ui.add_space(18.0);
    }

    // =========================================================
    //  Settings view
    // =========================================================
    fn settings_view(&mut self, ui: &mut egui::Ui, width: f32) {
        let pad = 34.0;
        let w = (width - pad * 2.0).max(320.0);

        let (h, _) = ui.allocate_exact_size(vec2(w, 62.0), Sense::hover());
        let painter = ui.painter();
        paint::text_left(&painter, pos2(h.min.x, h.min.y), "sector / 03", theme::body(11.0), theme::TEXT_FAINT);
        paint::text_left(&painter, pos2(h.min.x, h.min.y + 15.0), "System Settings", theme::display(23.0), theme::TEXT);

        ui.add_space(12.0);

        let col_w = (w - 16.0) / 2.0;

        let (lrect, _) = ui.allocate_exact_size(vec2(w, 300.0), Sense::hover());
        let left = Rect::from_min_max(lrect.min, lrect.max - vec2(w - col_w, 0.0));
        let right = Rect::from_min_max(pos2(lrect.max.x - col_w, lrect.min.y), lrect.max);
        self.settings_group(ui, left, "Runtime");
        self.settings_group(ui, right, "Experience");

        ui.add_space(16.0);

        let (mrect, _) = ui.allocate_exact_size(vec2(w, 190.0), Sense::hover());
        let mid = Rect::from_min_max(mrect.min, mrect.max - vec2(w - col_w, 0.0));
        let audio = Rect::from_min_max(pos2(mrect.max.x - col_w, mrect.min.y), mrect.max);
        self.audio_group(ui, mid);
        self.about_group(ui, audio);

        ui.add_space(18.0);
    }

    fn settings_group(&mut self, ui: &mut egui::Ui, r: Rect, title: &str) {
        {
            let painter = ui.painter();
            paint::rect(&painter, r, paint::cr(16.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
            paint::text_left(&painter, pos2(r.min.x + 20.0, r.min.y + 18.0), title, theme::body(10.5), theme::TEXT_FAINT);
        }

        // memory row
        let mut y = r.min.y + 52.0;
        let row = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row.min.x, row.min.y), "Allocated Memory", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row.min.x, row.min.y + 18.0), "RAM reserved for the game instance", theme::body(11.0), theme::TEXT_FAINT);
            paint::text_right(&painter, pos2(row.max.x, row.min.y), &format!("{} GB", self.mem_gb as i32), theme::body(13.0), theme::ACCENT_1);
        }
        let track = Rect::from_min_size(pos2(row.min.x, row.min.y + 42.0), vec2(row.width(), 4.0));
        slider(ui, track, &mut self.mem_gb, 2.0, 16.0, Id::new("mem_slider"));
        y += 66.0;

        // java row
        let row2 = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row2.min.x, row2.min.y), "Java Executable", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row2.min.x, row2.min.y + 18.0), "Bundled runtime", theme::body(11.0), theme::TEXT_FAINT);
            paint::text_right(&painter, pos2(row2.max.x, row2.min.y), "Auto", theme::body(13.0), theme::TEXT_DIM);
        }

        // animations switch
        let mut y = r.min.y + 52.0 + 66.0 + 54.0;
        let row3 = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row3.min.x, row3.min.y), "Animations", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row3.min.x, row3.min.y + 18.0), "UI motion & transition effects", theme::body(11.0), theme::TEXT_FAINT);
        }
        let sw = Rect::from_center_size(pos2(row3.max.x - 21.0, row3.center().y), vec2(42.0, 24.0));
        if switch(ui, sw, &mut self.anim_on, Id::new("anim_switch")) {
            self.reduce_motion = !self.anim_on;
        }
        y += 54.0;

        // reduce motion switch
        let row4 = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row4.min.x, row4.min.y), "Reduce Motion", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row4.min.x, row4.min.y + 18.0), "Limit ambient effects", theme::body(11.0), theme::TEXT_FAINT);
        }
        let sw2 = Rect::from_center_size(pos2(row4.max.x - 21.0, row4.center().y), vec2(42.0, 24.0));
        if switch(ui, sw2, &mut self.reduce_motion, Id::new("motion_switch")) {
            self.anim_on = !self.reduce_motion;
        }
    }

    fn audio_group(&mut self, ui: &mut egui::Ui, r: Rect) {
        {
            let painter = ui.painter();
            paint::rect(&painter, r, paint::cr(16.0), theme::PANEL, Stroke::new(1.0_f32, theme::BORDER));
            paint::text_left(&painter, pos2(r.min.x + 20.0, r.min.y + 18.0), "Audio", theme::body(10.5), theme::TEXT_FAINT);
        }

        let mut y = r.min.y + 52.0;
        let row = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row.min.x, row.min.y), "Ambient Sound", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row.min.x, row.min.y + 18.0), "Space ambience on launch screen", theme::body(11.0), theme::TEXT_FAINT);
        }
        let sw = Rect::from_center_size(pos2(row.max.x - 21.0, row.center().y), vec2(42.0, 24.0));
        switch(ui, sw, &mut self.sound_on, Id::new("sound_switch"));
        y += 54.0;

        let row2 = Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(r.width() - 40.0, 34.0));
        {
            let painter = ui.painter();
            paint::text_left(&painter, pos2(row2.min.x, row2.min.y), "Master Volume", theme::body(13.5), theme::TEXT);
            paint::text_left(&painter, pos2(row2.min.x, row2.min.y + 18.0), "Global output level", theme::body(11.0), theme::TEXT_FAINT);
            paint::text_right(&painter, pos2(row2.max.x, row2.min.y), &format!("{}%", self.vol as i32), theme::body(13.0), theme::ACCENT_1);
        }
        let track = Rect::from_min_size(pos2(row2.min.x, row2.min.y + 42.0), vec2(row2.width(), 4.0));
        slider(ui, track, &mut self.vol, 0.0, 100.0, Id::new("vol_slider"));
    }

    fn about_group(&mut self, ui: &mut egui::Ui, r: Rect) {
        let painter = ui.painter();
        paint::rect(
            &painter,
            r,
            paint::cr(16.0),
            Color32::from_rgba_premultiplied(7, 8, 14, 14),
            Stroke::new(1.0_f32, theme::BORDER),
        );

        let logo = Rect::from_center_size(pos2(r.min.x + 42.0, r.min.y + 58.0), vec2(46.0, 46.0));
        paint::glow(&painter, logo, paint::cr(13.0), theme::ACCENT_2, 22.0, 0.5);
        paint::rect(&painter, logo, paint::cr(13.0), theme::ACCENT_2, Stroke::NONE);
        paint::text(&painter, logo.center(), "A", theme::display_bold(20.0), Color32::WHITE);

        paint::text_left(&painter, pos2(r.min.x + 76.0, r.min.y + 40.0), "Aevum Launcher", theme::body(14.5), theme::TEXT);
        paint::text_left(&painter, pos2(r.min.x + 76.0, r.min.y + 61.0), "v2.1.0-stable", theme::body(11.0), theme::ACCENT_1);
        paint::text_left(
            &painter,
            pos2(r.min.x + 20.0, r.min.y + 84.0),
            "Engineered for the void. An open-source client launcher with a minimal, space-age aesthetic.",
            theme::body(11.5),
            theme::TEXT_DIM,
        );

        let btn = Rect::from_center_size(pos2(r.max.x - 58.0, r.max.y - 26.0), vec2(76.0, 32.0));
        let resp = ui.interact(btn, Id::new("about_btn"), Sense::click());
        paint::rect(
            &painter,
            btn,
            paint::cr(10.0),
            if resp.hovered() { Color32::from_rgba_premultiplied(20, 22, 39, 40) } else { Color32::from_white_alpha(12) },
            Stroke::new(1.0_f32, if resp.hovered() { theme::ACCENT_2.gamma_multiply(0.9) } else { theme::BORDER }),
        );
        paint::text(&painter, btn.center(), "About", theme::body(12.5), theme::TEXT);
        if resp.clicked() {
            self.toast("Aevum Launcher v2.1.0 — engineered for the void".into());
        }
    }

    // =========================================================
    //  Account modal
    // =========================================================
    fn account_modal_ui(&mut self, ctx: &egui::Context) {
        let k = ctx.animate_bool(Id::new("account_modal"), self.account_modal);
        let sr = ctx.screen_rect();
        let pw = 380.0;
        let ph = 230.0;

        egui::Area::new(Id::new("account_modal_area"))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let backdrop = ui.interact(sr, Id::new("acc_backdrop"), Sense::click());
                let painter =
                    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("account_modal_area")));
                painter.rect_filled(
                    sr,
                    paint::zero(),
                    Color32::from_rgba_premultiplied(4, 4, 12, (0.62 * k * 255.0) as u8),
                );
                if backdrop.clicked() {
                    self.account_modal = false;
                }

                let panel = Rect::from_center_size(
                    sr.center() + vec2(0.0, (1.0 - k) * 18.0),
                    vec2(pw, ph),
                );

                paint::rect(
                    &painter,
                    panel,
                    paint::cr(18.0),
                    theme::PANEL_STRONG.gamma_multiply(k),
                    Stroke::new(1.0_f32, theme::BORDER.gamma_multiply(k)),
                );

                paint::text_left(
                    &painter,
                    pos2(panel.min.x + 22.0, panel.min.y + 20.0),
                    "Offline Profile",
                    theme::display(14.0),
                    theme::TEXT.gamma_multiply(k),
                );

                // close button
                let close_btn = Rect::from_center_size(pos2(panel.max.x - 26.0, panel.min.y + 26.0), vec2(28.0, 28.0));
                let cr = ui.interact(close_btn, Id::new("acc_close"), Sense::click());
                if cr.hovered() {
                    painter.rect_filled(close_btn, paint::cr(8.0), Color32::from_white_alpha(22));
                }
                let cx = close_btn.center();
                let s = Stroke::new(1.5_f32, theme::TEXT_DIM.gamma_multiply(k));
                painter.line_segment([pos2(cx.x - 4.0, cx.y - 4.0), pos2(cx.x + 4.0, cx.y + 4.0)], s);
                painter.line_segment([pos2(cx.x - 4.0, cx.y + 4.0), pos2(cx.x + 4.0, cx.y - 4.0)], s);
                if cr.clicked() {
                    self.account_modal = false;
                }

                paint::text_left(
                    &painter,
                    pos2(panel.min.x + 24.0, panel.min.y + 62.0),
                    "Username",
                    theme::body(10.5),
                    theme::TEXT_FAINT.gamma_multiply(k),
                );

                let field = Rect::from_min_size(pos2(panel.min.x + 24.0, panel.min.y + 82.0), vec2(pw - 48.0, 42.0));
                paint::rect(
                    &painter,
                    field,
                    paint::cr(10.0),
                    Color32::from_white_alpha(10).gamma_multiply(k),
                    Stroke::new(1.0_f32, theme::ACCENT_2.gamma_multiply(0.35 * k)),
                );
                let edit = egui::TextEdit::singleline(&mut self.username)
                    .font(theme::body(14.0))
                    .text_color(theme::TEXT)
                    .margin(egui::vec2(12.0, 9.0))
                    .frame(false);
                ui.put(field, edit);

                paint::text_left(
                    &painter,
                    pos2(panel.min.x + 24.0, panel.min.y + 132.0),
                    "Offline profile — no Microsoft account required. Joins offline-compatible servers.",
                    theme::body(11.0),
                    theme::TEXT_FAINT.gamma_multiply(k),
                );

                let save = Rect::from_min_size(pos2(panel.min.x + 24.0, panel.min.y + 162.0), vec2(pw - 48.0, 44.0));
                let resp = ui.interact(save, Id::new("acc_save"), Sense::click());
                let hover = resp.hovered();
                paint::rect(
                    &painter,
                    save,
                    paint::cr(999.0),
                    if hover { theme::ACCENT_3 } else { theme::ACCENT_2 }.gamma_multiply(k),
                    Stroke::NONE,
                );
                paint::text(
                    &painter,
                    save.center(),
                    "SAVE PROFILE",
                    theme::display_bold(12.5),
                    Color32::WHITE.gamma_multiply(k),
                );
                if resp.clicked() {
                    if self.username.trim().is_empty() {
                        self.username = "Player".to_string();
                    } else {
                        self.username = self.username.trim().to_string();
                    }
                    self.account_modal = false;
                    self.toast(format!("Profile set — {}", self.username));
                }
            });
    }

    // =========================================================
    //  Launch overlay
    // =========================================================
    fn launch_ui(&mut self, ctx: &egui::Context) {
        let report = self.launch_report.lock();
        let report = match report {
            Ok(r) => r,
            Err(_) => return,
        };
        let phase = report.phase;
        if phase == launcher::Phase::Idle {
            return;
        }

        let active = matches!(
            phase,
            launcher::Phase::Fetching
                | launcher::Phase::Downloading
                | launcher::Phase::Extracting
                | launcher::Phase::Launching
                | launcher::Phase::Running
        );
        let k = ctx.animate_bool(Id::new("launch_fade"), active);

        let msg = report.message.clone();
        let pct = report.progress_pct();
        let pid = report.pid;
        let exit = report.exit_code;
        let err = report.error.clone();
        let files_done = report.files_done;
        let files_total = report.files_total;
        drop(report);

        let sr = ctx.screen_rect();
        let c = sr.center();
        let t = self.time;
        let mut dismiss = false;

        egui::Area::new(Id::new("launch_overlay"))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.interact(sr, Id::new("launch_block"), Sense::click());
                let painter = ui.painter();
                painter.rect_filled(
                    sr,
                    paint::zero(),
                    Color32::from_rgba_premultiplied(5, 5, 13, (0.86 * k * 255.0) as u8),
                );

                let (headline, sub) = match phase {
                    launcher::Phase::Fetching => ("RESOLVING", msg.as_str()),
                    launcher::Phase::Downloading => ("SYNCING ASSETS", msg.as_str()),
                    launcher::Phase::Extracting => ("EXTRACTING", msg.as_str()),
                    launcher::Phase::Launching => ("IGNITION", msg.as_str()),
                    launcher::Phase::Running => ("GAME RUNNING", msg.as_str()),
                    launcher::Phase::Exited => ("GAME EXITED", msg.as_str()),
                    launcher::Phase::Error => (
                        "LAUNCH FAILED",
                        err.as_deref().unwrap_or_else(|| msg.as_str()),
                    ),
                    launcher::Phase::Idle => return,
                };

                if active {
                    let spin = t * 3.0;
                    paint::arc(&painter, c, 40.0, 3.0, theme::ACCENT_1.gamma_multiply(k), spin, 3.4);
                    paint::arc(
                        &painter,
                        c,
                        40.0,
                        3.0,
                        theme::ACCENT_3.gamma_multiply(k),
                        spin + std::f32::consts::PI,
                        2.2,
                    );
                } else {
                    let color = if phase == launcher::Phase::Error {
                        theme::ACCENT_3
                    } else {
                        theme::ACCENT_1
                    };
                    let r = 34.0;
                    painter.circle_stroke(c, r, Stroke::new(2.5_f32, color.gamma_multiply(k)));
                    if phase == launcher::Phase::Error {
                        let s2 = Stroke::new(3.0_f32, color.gamma_multiply(k));
                        painter.line_segment([pos2(c.x - 12.0, c.y - 12.0), pos2(c.x + 12.0, c.y + 12.0)], s2);
                        painter.line_segment([pos2(c.x + 12.0, c.y - 12.0), pos2(c.x - 12.0, c.y + 12.0)], s2);
                    } else {
                        let s2 = Stroke::new(3.0_f32, color.gamma_multiply(k));
                        painter.line_segment([pos2(c.x - 14.0, c.y), pos2(c.x - 4.0, c.y + 12.0)], s2);
                        painter.line_segment([pos2(c.x - 4.0, c.y + 12.0), pos2(c.x + 14.0, c.y - 12.0)], s2);
                    }
                }

                paint::text(&painter, pos2(c.x, c.y + 86.0), headline, theme::display(17.0), theme::TEXT.gamma_multiply(k));
                paint::text(&painter, pos2(c.x, c.y + 114.0), sub, theme::body(13.0), theme::TEXT_DIM.gamma_multiply(k));

                if phase == launcher::Phase::Running {
                    if let Some(pid) = pid {
                        paint::text(
                            &painter,
                            pos2(c.x, c.y + 136.0),
                            &format!("process {} — stop it from the Play view", pid),
                            theme::body(11.5),
                            theme::TEXT_FAINT.gamma_multiply(k),
                        );
                    }
                } else if phase == launcher::Phase::Exited {
                    let code = exit.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                    paint::text(
                        &painter,
                        pos2(c.x, c.y + 136.0),
                        &format!("exit code {}", code),
                        theme::body(11.5),
                        theme::TEXT_FAINT.gamma_multiply(k),
                    );
                }

                let bar = Rect::from_center_size(pos2(c.x, c.y + 170.0), vec2(300.0, 5.0));
                painter.rect_filled(bar, paint::cr(5.0), Color32::from_white_alpha(22));
                if active && pct > 0.0 {
                    painter.rect_filled(
                        Rect::from_min_max(bar.min, pos2(bar.min.x + bar.width() * pct, bar.max.y)),
                        paint::cr(5.0),
                        theme::ACCENT_1.gamma_multiply(k),
                    );
                }
                let pct_label = if phase == launcher::Phase::Running && pid.is_some() {
                    format!("{} / {} files", files_done, files_total)
                } else {
                    format!("{:.0}%", pct * 100.0)
                };
                paint::text(
                    &painter,
                    pos2(c.x, bar.max.y + 20.0),
                    &pct_label,
                    theme::body(12.0),
                    theme::TEXT_FAINT.gamma_multiply(k),
                );

                if !active {
                    let btn = Rect::from_center_size(pos2(c.x, c.y + 236.0), vec2(220.0, 44.0));
                    let resp = ui.interact(btn, Id::new("launch_dismiss"), Sense::click());
                    let hover = resp.hovered();
                    paint::rect(
                        &painter,
                        btn,
                        paint::cr(999.0),
                        if hover { theme::ACCENT_3 } else { theme::ACCENT_2 }.gamma_multiply(k),
                        Stroke::NONE,
                    );
                    paint::text(
                        &painter,
                        btn.center(),
                        "DISMISS",
                        theme::display_bold(12.5),
                        Color32::WHITE.gamma_multiply(k),
                    );
                    if resp.clicked() {
                        dismiss = true;
                    }
                }
            });

        if dismiss {
            if let Ok(mut rep) = self.launch_report.lock() {
                *rep = launcher::LaunchReport::default();
            }
        }
    }

    // =========================================================
    //  Toast
    // =========================================================
    fn toast_ui(&mut self, ctx: &egui::Context) {
        if let Some((msg, age)) = &mut self.toast {
            *age += self.dt;
            let fade_in = (*age / 0.18).min(1.0);
            let fade_out = ((2.6 - *age) / 0.4).clamp(0.0, 1.0);
            let a = fade_in.min(fade_out);
            if a <= 0.0 {
                self.toast = None;
                return;
            }
            let sr = ctx.screen_rect();
            let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("toast")));
            let txt_w = msg.len() as f32 * 7.2 + 56.0;
            let r = Rect::from_center_size(
                pos2(sr.center().x, sr.max.y - 40.0 + (1.0 - fade_in) * 14.0),
                vec2(txt_w.max(180.0), 40.0),
            );
            paint::rect(
                &painter,
                r,
                paint::cr(12.0),
                Color32::from_rgba_premultiplied(18, 18, 42, 235).gamma_multiply(a),
                Stroke::new(1.0_f32, theme::ACCENT_2.gamma_multiply(0.5 * a)),
            );
            paint::text(
                &painter,
                r.center(),
                msg,
                theme::body(13.5),
                theme::TEXT.gamma_multiply(a),
            );
        }
    }
}
