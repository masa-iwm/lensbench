//! 配色（暗い計測器風・明るい・OS に合わせる）とアクセント色。デザイン案（解析ビュー）の色に合わせる。

use egui::{Color32, CornerRadius, Stroke, Theme, ThemePreference, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
    /// OS の明暗に合わせる
    System,
}

/// アクセント色の候補（デザイン案の 4 色）
pub const ACCENTS: [[u8; 3]; 4] = [
    [0x4D, 0xB6, 0xFF],
    [0x7C, 0x9C, 0xFF],
    [0x3D, 0xD6, 0xB5],
    [0xFF, 0x9F, 0x43],
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub mode: Mode,
    pub accent: [u8; 3],
}

impl Default for Look {
    fn default() -> Self {
        Look {
            mode: Mode::Dark,
            accent: ACCENTS[0],
        }
    }
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// 波長・視野の色（短い波長から。デザイン案の F・d・C と光線の色）
pub fn series(i: usize) -> Color32 {
    const C: [u32; 8] = [
        0x4DB6FF, 0x8FD19E, 0xFF9F43, 0xFF7A6B, 0xC792EA, 0xF2C94C, 0x5FD7D7, 0xE6E9EE,
    ];
    rgb(C[i % C.len()])
}

/// 波長 [µm] の色（青 0.45 → 赤 0.68、範囲の外は端の色）
pub fn wave_color(um: f64) -> Color32 {
    let t = ((um - 0.45) / (0.68 - 0.45)).clamp(0.0, 1.0) as f32;
    let stops = [rgb(0x6FA8FF), rgb(0x8FD19E), rgb(0xF2C94C), rgb(0xFF7A6B)];
    let x = t * 3.0;
    let i = (x.floor() as usize).min(2);
    let f = x - i as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * f).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn dark(accent: Color32) -> Visuals {
    let mut v = Visuals::dark();
    let text = rgb(0xE6E9EE);
    let line = rgb(0x262C35);
    v.panel_fill = rgb(0x13171C);
    v.window_fill = rgb(0x161A20);
    v.extreme_bg_color = rgb(0x0F1216);
    v.faint_bg_color = rgb(0x1A1F26);
    v.code_bg_color = rgb(0x1A1F26);
    v.window_stroke = Stroke::new(1.0_f32, line);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = rgb(0x161A20);
    w.noninteractive.weak_bg_fill = rgb(0x161A20);
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);
    w.inactive.bg_fill = rgb(0x1A1F26);
    w.inactive.weak_bg_fill = rgb(0x1A1F26);
    w.inactive.bg_stroke = Stroke::new(1.0_f32, rgb(0x2F3640));
    w.inactive.fg_stroke = Stroke::new(1.0_f32, text);
    w.hovered.bg_fill = rgb(0x222831);
    w.hovered.weak_bg_fill = rgb(0x222831);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, accent.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, text);
    w.active.bg_fill = rgb(0x1F2A36);
    w.active.weak_bg_fill = rgb(0x1F2A36);
    w.active.bg_stroke = Stroke::new(1.0_f32, accent);
    w.active.fg_stroke = Stroke::new(2.0_f32, text);
    w.open.bg_fill = rgb(0x1F2A36);
    w.open.weak_bg_fill = rgb(0x1F2A36);
    w.open.fg_stroke = Stroke::new(1.0_f32, text);
    v
}

fn light(accent: Color32) -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = rgb(0xF4F6F8);
    v.window_fill = rgb(0xFFFFFF);
    v.extreme_bg_color = rgb(0xFFFFFF);
    v.faint_bg_color = rgb(0xEEF1F4);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
    v
}

/// 窓の配色に使う（パネルの見出しの文字など）
pub fn weak_text(v: &Visuals) -> Color32 {
    if v.dark_mode {
        rgb(0x9AA3AF)
    } else {
        rgb(0x5B6573)
    }
}

pub fn apply(ctx: &egui::Context, look: &Look) {
    let a = Color32::from_rgb(look.accent[0], look.accent[1], look.accent[2]);
    for (theme, mut v) in [(Theme::Dark, dark(a)), (Theme::Light, light(a))] {
        v.selection.bg_fill = a.gamma_multiply(if v.dark_mode { 0.45 } else { 0.35 });
        v.selection.stroke = Stroke::new(1.0_f32, a);
        v.hyperlink_color = a;
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.slider_trailing_fill = true;
        ctx.set_visuals_of(theme, v);
    }
    ctx.set_theme(match look.mode {
        Mode::Dark => ThemePreference::Dark,
        Mode::Light => ThemePreference::Light,
        Mode::System => ThemePreference::System,
    });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}

/// アクセント色
pub fn accent(look: &Look) -> Color32 {
    Color32::from_rgb(look.accent[0], look.accent[1], look.accent[2])
}
