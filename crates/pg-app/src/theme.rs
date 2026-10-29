//! The look of the game's own screens: one palette, one shape language, in one place.
//!
//! The idea is a town at dusk. Surfaces are a deep blue-slate in three steps (the screen, a card on it, a
//! window above that), text is a soft off-white that steps down to muted and faint, and there are two accents
//! with fixed jobs: **lantern amber** means "you can act here or you are here" (focus ring, the main button,
//! section titles) and **teal** means "information" (selections, bars, sliders). Amber and terracotta
//! are kept for warnings and errors. Shapes are softly rounded, borders are hairlines, and nothing is pure
//! black or pure white, so the pixel-art map is the brightest thing on screen.
//!
//! Everything here is a constant or one function that applies them, so a change of look is a change to this
//! file. The widget model never names a colour.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke};

// ---- palette -------------------------------------------------------------------------------------------------

/// The screen behind everything.
pub const BG: Color32 = Color32::from_rgb(0x14, 0x17, 0x1F);
/// A card or panel sitting on the screen.
pub const SURFACE: Color32 = Color32::from_rgb(0x1C, 0x21, 0x2C);
/// A window or popup floating above the screen.
pub const RAISED: Color32 = Color32::from_rgb(0x23, 0x29, 0x36);
/// A resting button or input field.
pub const CONTROL: Color32 = Color32::from_rgb(0x2C, 0x33, 0x44);
/// A control under the pointer.
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x37, 0x40, 0x56);
/// A control being pressed.
pub const CONTROL_ACTIVE: Color32 = Color32::from_rgb(0x42, 0x4C, 0x66);

/// Hairlines: the quiet outline of a card, a divider.
pub const HAIRLINE: Color32 = Color32::from_rgb(0x34, 0x3C, 0x4E);
/// Outlines that should be seen: a column's frame, a hovered control.
pub const EDGE: Color32 = Color32::from_rgb(0x5B, 0x66, 0x7E);

pub const TEXT: Color32 = Color32::from_rgb(0xE7, 0xE9, 0xEF);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x9C, 0xA5, 0xB8);

/// Lantern amber: act here, you are here.
pub const ACCENT: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xF8, 0xCB, 0x73);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(0xD9, 0xA0, 0x38);
/// Text on an amber fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1E, 0x16, 0x06);
/// Section titles: amber turned down so it labels without shouting.
pub const ACCENT_QUIET: Color32 = Color32::from_rgb(0xD8, 0xB0, 0x6A);

/// Teal: information (selections, bars, sliders).
pub const INFO: Color32 = Color32::from_rgb(0x3F, 0x86, 0x96);
pub const INFO_STRONG: Color32 = Color32::from_rgb(0x6C, 0xB8, 0xC8);

pub const WARN: Color32 = ACCENT;
pub const BAD: Color32 = Color32::from_rgb(0xE5, 0x76, 0x6B);

// ---- shape and space -----------------------------------------------------------------------------------------

/// Corner radius of a control.
pub const R_CONTROL: u8 = 6;
/// Corner radius of a card or column.
pub const R_CARD: u8 = 8;
/// Corner radius of a window.
pub const R_WINDOW: u8 = 10;

/// Type sizes (points).
pub const T_TITLE: f32 = 30.0;
pub const T_SECTION: f32 = 17.0;
pub const T_BODY: f32 = 16.0;
pub const T_CONTROL: f32 = 16.0;
pub const T_NOTE: f32 = 13.0;

/// Applies the palette and shapes to every egui style in `ctx`.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 9.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = Margin::same(14);
        s.spacing.menu_margin = Margin::same(8);

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.weak_text_color = Some(TEXT_MUTED);
        v.panel_fill = SURFACE;
        v.window_fill = RAISED;
        v.extreme_bg_color = Color32::from_rgb(0x10, 0x13, 0x1A);
        v.faint_bg_color = Color32::from_rgb(0x20, 0x26, 0x32);
        v.code_bg_color = Color32::from_rgb(0x10, 0x13, 0x1A);
        v.hyperlink_color = INFO_STRONG;
        v.warn_fg_color = WARN;
        v.error_fg_color = BAD;

        v.window_corner_radius = CornerRadius::same(R_WINDOW);
        v.menu_corner_radius = CornerRadius::same(R_CONTROL);
        v.window_stroke = Stroke::new(1.0, HAIRLINE);
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(110),
        };
        v.popup_shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(100),
        };

        v.selection.bg_fill = INFO;
        v.selection.stroke = Stroke::new(1.0, INFO_STRONG);

        let radius = CornerRadius::same(R_CONTROL);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SURFACE;
        w.noninteractive.weak_bg_fill = SURFACE;
        w.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = CONTROL;
        w.inactive.weak_bg_fill = CONTROL;
        w.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = radius;

        w.hovered.bg_fill = CONTROL_HOVER;
        w.hovered.weak_bg_fill = CONTROL_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, EDGE);
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = CONTROL_ACTIVE;
        w.active.weak_bg_fill = CONTROL_ACTIVE;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = CONTROL_HOVER;
        w.open.weak_bg_fill = CONTROL_HOVER;
        w.open.bg_stroke = Stroke::new(1.0, EDGE);
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = radius;
    });
}
