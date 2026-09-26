//! SpotiLite's own look: flat surfaces, one warm accent, text-first layout.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle,
    Vec2,
};

use crate::config::ThemeChoice;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub dark: bool,
}

impl Palette {
    pub fn for_choice(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::Dark => Self {
                bg: Color32::from_rgb(0x0f, 0x11, 0x15),
                panel: Color32::from_rgb(0x13, 0x16, 0x1b),
                raised: Color32::from_rgb(0x1a, 0x1e, 0x25),
                hover: Color32::from_rgb(0x1f, 0x24, 0x2c),
                line: Color32::from_rgb(0x24, 0x2a, 0x33),
                text: Color32::from_rgb(0xe8, 0xe6, 0xe3),
                dim: Color32::from_rgb(0x8b, 0x93, 0xa1),
                faint: Color32::from_rgb(0x5a, 0x61, 0x6d),
                accent: Color32::from_rgb(0xe8, 0xb0, 0x4b),
                on_accent: Color32::from_rgb(0x1a, 0x12, 0x06),
                danger: Color32::from_rgb(0xe5, 0x48, 0x4d),
                dark: true,
            },
            ThemeChoice::Light => Self {
                bg: Color32::from_rgb(0xf6, 0xf4, 0xef),
                panel: Color32::from_rgb(0xef, 0xec, 0xe5),
                raised: Color32::from_rgb(0xff, 0xff, 0xff),
                hover: Color32::from_rgb(0xe7, 0xe3, 0xda),
                line: Color32::from_rgb(0xdd, 0xd8, 0xcd),
                text: Color32::from_rgb(0x1b, 0x1d, 0x21),
                dim: Color32::from_rgb(0x5f, 0x66, 0x72),
                faint: Color32::from_rgb(0x9a, 0xa0, 0xaa),
                accent: Color32::from_rgb(0xa8, 0x66, 0x0a),
                on_accent: Color32::WHITE,
                danger: Color32::from_rgb(0xc6, 0x2f, 0x35),
                dark: false,
            },
        }
    }
}

pub const ROW_HEIGHT: f32 = 30.0;
pub const PLAYER_HEIGHT: f32 = 72.0;
pub const SIDEBAR_WIDTH: f32 = 212.0;

pub fn heading_font() -> FontId {
    FontId::new(22.0, FontFamily::Name("semibold".into()))
}

pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn body_font() -> FontId {
    FontId::proportional(14.0)
}

pub fn small_font() -> FontId {
    FontId::proportional(12.0)
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let theme = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |style| {
        style.text_styles = [
            (TextStyle::Heading, heading_font()),
            (TextStyle::Body, body_font()),
            (TextStyle::Button, body_font()),
            (TextStyle::Small, small_font()),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.window_margin = Margin::same(14);
        style.spacing.slider_rail_height = 4.0;
        style.animation_time = 0.08;

        let v = &mut style.visuals;
        v.dark_mode = p.dark;
        v.override_text_color = None;
        v.panel_fill = p.bg;
        v.window_fill = p.raised;
        v.window_stroke = Stroke::new(1.0, p.line);
        v.window_corner_radius = CornerRadius::same(8);
        v.menu_corner_radius = CornerRadius::same(6);
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
        v.extreme_bg_color = p.panel;
        v.text_edit_bg_color = Some(p.raised);
        v.faint_bg_color = p.panel;
        v.hyperlink_color = p.accent;
        v.error_fg_color = p.danger;
        v.warn_fg_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.striped = false;

        let radius = CornerRadius::same(5);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.bg;
        w.noninteractive.weak_bg_fill = p.bg;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.noninteractive.corner_radius = radius;
        for (state, fill, fg) in [
            (&mut w.inactive, p.raised, p.text),
            (&mut w.hovered, p.hover, p.text),
            (&mut w.active, p.line, p.text),
            (&mut w.open, p.hover, p.text),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.5, fg);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
        // Visible outlines for checkboxes, radio buttons and text fields.
        w.inactive.bg_stroke = Stroke::new(1.0, p.line);
        w.hovered.bg_stroke = Stroke::new(1.0, p.faint);
    });
}

/// Uses the system font (Segoe UI on Windows) instead of bundling one: nothing to
/// download, nothing added to the executable. Falls back to egui's built-in fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let regular = read_system_font(&["segoeui.ttf", "DejaVuSans.ttf", "NotoSans-Regular.ttf"]);
    let semibold = read_system_font(&["seguisb.ttf", "DejaVuSans-Bold.ttf", "NotoSans-SemiBold.ttf"]);

    if let Some(bytes) = regular {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(bytes)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "system".into());
    }
    let mut semibold_family = Vec::new();
    if let Some(bytes) = semibold {
        fonts.font_data.insert("system-semibold".into(), Arc::new(FontData::from_owned(bytes)));
        semibold_family.push("system-semibold".to_string());
    }
    semibold_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name("semibold".into()), semibold_family);
    ctx.set_fonts(fonts);
}

fn read_system_font(names: &[&str]) -> Option<Vec<u8>> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(std::path::PathBuf::from(windir).join("Fonts"));
    } else {
        dirs.push("/usr/share/fonts/truetype/dejavu".into());
        dirs.push("/usr/share/fonts/truetype/noto".into());
        dirs.push("/usr/share/fonts/TTF".into());
    }
    names
        .iter()
        .flat_map(|name| dirs.iter().map(move |d| d.join(name)))
        .find_map(|path| std::fs::read(path).ok())
}
