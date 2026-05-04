use egui::{Color32, FontFamily, FontId, Margin, RichText, Ui, Visuals};

// ── Palette ───────────────────────────────────────────────────────────────────

pub struct Palette;

impl Palette {
    // Surfaces
    pub const SURFACE: Color32 = Color32::from_rgb(0xFA, 0xFB, 0xFC);
    pub const SURFACE_ALT: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF7);
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0xE8, 0xEC, 0xF2);
    pub const SEPARATOR: Color32 = Color32::from_rgb(0xD0, 0xD5, 0xDD);
    #[allow(dead_code)]
    pub const HEADER_BG: Color32 = Color32::from_rgb(0xEB, 0xEE, 0xF5);

    // Accent
    pub const ACCENT: Color32 = Color32::from_rgb(0x1F, 0x6F, 0xEB);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0xD6, 0xE4, 0xFB);
    pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

    // Semantic
    pub const SUCCESS: Color32 = Color32::from_rgb(0x16, 0x7D, 0x48);
    pub const SUCCESS_BG: Color32 = Color32::from_rgb(0xD3, 0xF0, 0xE3);
    pub const WARN: Color32 = Color32::from_rgb(0xB4, 0x5B, 0x09);
    pub const WARN_BG: Color32 = Color32::from_rgb(0xFD, 0xEE, 0xD0);
    pub const DANGER: Color32 = Color32::from_rgb(0xB4, 0x18, 0x18);
    pub const DANGER_BG: Color32 = Color32::from_rgb(0xFC, 0xDB, 0xDB);

    // Text
    pub const TEXT_STRONG: Color32 = Color32::from_rgb(0x0D, 0x0F, 0x14);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x5A, 0x64, 0x78);
    pub const TEXT_MONO: Color32 = Color32::from_rgb(0x1A, 0x28, 0x40);
}

// ── Phosphor icon shortcuts ───────────────────────────────────────────────────

pub mod icons {
    pub use egui_phosphor::regular::{
        ARROW_DOWN, ARROW_UP, HARD_DRIVES, MAGNIFYING_GLASS, X,
    };

    pub const DEVICE_HDD: &str = egui_phosphor::regular::HARD_DRIVES;
    pub const DEVICE_SSD: &str = egui_phosphor::regular::LIGHTNING;
    pub const DEVICE_USB: &str = egui_phosphor::regular::USB;
    pub const FOLDER_OPEN: &str = egui_phosphor::regular::FOLDER_OPEN;
    pub const PLAY: &str = egui_phosphor::regular::PLAY;
    pub const CANCEL: &str = egui_phosphor::regular::STOP;
    pub const REFRESH: &str = egui_phosphor::regular::ARROWS_CLOCKWISE;
    pub const AUDIT: &str = egui_phosphor::regular::CLIPBOARD_TEXT;
    pub const HASH: &str = egui_phosphor::regular::HASH;
    pub const CHECK: &str = egui_phosphor::regular::CHECK_CIRCLE;
    pub const DATABASE: &str = egui_phosphor::regular::DATABASE;
}

// ── Apply theme ───────────────────────────────────────────────────────────────

pub fn apply(ctx: &egui::Context) {
    // Install phosphor icon font
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    let mut visuals = Visuals::light();

    // Backgrounds
    visuals.panel_fill = Palette::SURFACE;
    visuals.window_fill = Palette::SURFACE;
    visuals.extreme_bg_color = Palette::SURFACE_ALT; // table row bg

    // Selection
    visuals.selection.bg_fill = Palette::ACCENT_DIM;
    visuals.selection.stroke.color = Palette::ACCENT;

    // Widget states
    let fg = egui::Stroke::new(1.0, Palette::SEPARATOR);
    visuals.widgets.noninteractive.bg_fill = Palette::SURFACE;
    visuals.widgets.noninteractive.bg_stroke = fg;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, Palette::TEXT_STRONG);

    visuals.widgets.inactive.bg_fill = Palette::SURFACE;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, Palette::SEPARATOR);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, Palette::TEXT_DIM);

    visuals.widgets.hovered.bg_fill = Palette::SURFACE_HOVER;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, Palette::ACCENT);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, Palette::TEXT_STRONG);

    visuals.widgets.active.bg_fill = Palette::ACCENT_DIM;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, Palette::ACCENT);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, Palette::ACCENT);

    // Geometry
    visuals.window_corner_radius = 4.0.into();
    visuals.menu_corner_radius = 4.0.into();
    visuals.window_shadow = egui::Shadow::NONE;

    ctx.set_visuals(visuals);

    // Spacing: tighter than egui defaults for dense forensic UI
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.window_margin = Margin::same(8);
    ctx.set_style(style);
}

// ── Widget helpers ────────────────────────────────────────────────────────────

/// A small colored badge label (e.g. "RO", "HPA", "WARN").
pub fn chip(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(3.0)
        .inner_margin(Margin { left: 4, right: 4, top: 1, bottom: 1 })
        .show(ui, |ui| {
            ui.add(egui::Label::new(
                RichText::new(text).color(fg).small().strong(),
            ));
        });
}

pub fn chip_success(ui: &mut Ui, text: &str) {
    chip(ui, text, Palette::SUCCESS, Palette::SUCCESS_BG);
}

pub fn chip_warn(ui: &mut Ui, text: &str) {
    chip(ui, text, Palette::WARN, Palette::WARN_BG);
}

pub fn chip_danger(ui: &mut Ui, text: &str) {
    chip(ui, text, Palette::DANGER, Palette::DANGER_BG);
}

pub fn chip_info(ui: &mut Ui, text: &str) {
    chip(ui, text, Palette::ACCENT, Palette::ACCENT_DIM);
}

/// A segmented control: renders a row of selectable labels as exclusive toggles.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, options: &[(&str, T)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (label, value) in options {
            let selected = *current == *value;
            let rich = if selected {
                RichText::new(*label).color(Palette::ACCENT_TEXT).strong()
            } else {
                RichText::new(*label)
            };
            let btn = egui::Button::new(rich).corner_radius(3.0);
            let btn = if selected {
                btn.fill(Palette::ACCENT).stroke(egui::Stroke::new(1.0, Palette::ACCENT))
            } else {
                btn.fill(Palette::SURFACE).stroke(egui::Stroke::new(1.0, Palette::SEPARATOR))
            };
            if ui.add(btn).clicked() {
                *current = *value;
            }
        }
    });
}

/// A small section heading (lighter than ui.heading).
pub fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.add(egui::Label::new(
        RichText::new(text)
            .color(Palette::TEXT_DIM)
            .font(FontId::new(11.0, FontFamily::Proportional))
            .strong(),
    ));
    ui.add_space(2.0);
}

/// A hash-toggle chip: filled when enabled, outlined when disabled.
pub fn hash_chip(ui: &mut Ui, label: &str, enabled: &mut bool) {
    let (fg, bg) = if *enabled {
        (Palette::ACCENT_TEXT, Palette::ACCENT)
    } else {
        (Palette::TEXT_DIM, Palette::SURFACE_ALT)
    };
    let btn = egui::Button::new(RichText::new(label).color(fg).small())
        .corner_radius(3.0)
        .fill(bg)
        .stroke(egui::Stroke::new(1.0, if *enabled { Palette::ACCENT } else { Palette::SEPARATOR }));
    if ui.add(btn).clicked() {
        *enabled = !*enabled;
    }
}
