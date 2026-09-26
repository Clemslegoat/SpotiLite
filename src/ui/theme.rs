//! SpotiLite's look: pure black (AMOLED) surfaces, white as the only accent,
//! neutral greys for hierarchy. Black pixels are switched off on OLED screens.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle,
    Vec2,
};

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
    pub accent_hover: Color32,
    pub on_accent: Color32,
    /// Errors stay monochrome too: they are told apart by a white frame, not a color.
    pub danger: Color32,
}

impl Palette {
    pub const fn amoled() -> Self {
        Self {
            bg: Color32::from_rgb(0x00, 0x00, 0x00),
            panel: Color32::from_rgb(0x00, 0x00, 0x00),
            raised: Color32::from_rgb(0x16, 0x16, 0x16),
            hover: Color32::from_rgb(0x0f, 0x0f, 0x0f),
            line: Color32::from_rgb(0x22, 0x22, 0x22),
            text: Color32::from_rgb(0xe6, 0xe6, 0xe6),
            dim: Color32::from_rgb(0x8c, 0x8c, 0x8c),
            faint: Color32::from_rgb(0x58, 0x58, 0x58),
            accent: Color32::WHITE,
            accent_hover: Color32::from_rgb(0xc8, 0xc8, 0xc8),
            on_accent: Color32::BLACK,
            danger: Color32::WHITE,
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
    let theme = egui::Theme::Dark;
    ctx.set_theme(theme);
    // Dark window title bar on Windows 10/11, to match the black window.
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
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
        v.dark_mode = true;
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
        v.selection.bg_fill = Color32::from_rgb(0x3a, 0x3a, 0x3a);
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
        let hovered = Color32::from_rgb(0x1f, 0x1f, 0x1f);
        let pressed = Color32::from_rgb(0x2a, 0x2a, 0x2a);
        for (state, fill, fg) in [
            (&mut w.inactive, p.raised, p.text),
            (&mut w.hovered, hovered, p.accent),
            (&mut w.active, pressed, p.accent),
            (&mut w.open, hovered, p.accent),
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
