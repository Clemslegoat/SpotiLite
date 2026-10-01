//! SpotiLite's look: pure black (AMOLED) window, near-black rounded surfaces,
//! white as the only accent and neutral greys for hierarchy. Black pixels are
//! switched off on OLED screens.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle,
    Vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    /// Window background, between the surfaces.
    pub bg: Color32,
    /// Sidebar, content and player cards.
    pub surface: Color32,
    /// Cards inside a surface, fields, selected rows.
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
            surface: Color32::from_rgb(0x0b, 0x0b, 0x0b),
            raised: Color32::from_rgb(0x17, 0x17, 0x17),
            hover: Color32::from_rgb(0x13, 0x13, 0x13),
            line: Color32::from_rgb(0x24, 0x24, 0x24),
            text: Color32::from_rgb(0xee, 0xee, 0xee),
            dim: Color32::from_rgb(0x9a, 0x9a, 0x9a),
            faint: Color32::from_rgb(0x5e, 0x5e, 0x5e),
            accent: Color32::WHITE,
            accent_hover: Color32::from_rgb(0xd8, 0xd8, 0xd8),
            on_accent: Color32::BLACK,
            danger: Color32::WHITE,
        }
    }
}

/// Corner radii: generous everywhere, nothing square.
pub const RADIUS_SURFACE: u8 = 16;
pub const RADIUS_CARD: u8 = 14;
pub const RADIUS_ROW: u8 = 10;
/// Gap between the window edge and the surfaces, and between surfaces.
pub const GAP: i8 = 8;

pub const ROW_HEIGHT: f32 = 46.0;
pub const PLAYER_HEIGHT: f32 = 116.0;
pub const SIDEBAR_WIDTH: f32 = 236.0;

pub fn heading_font() -> FontId {
    FontId::new(26.0, FontFamily::Name("semibold".into()))
}

pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn body_font() -> FontId {
    FontId::proportional(14.0)
}

pub fn small_font() -> FontId {
    FontId::proportional(12.5)
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let theme = egui::Theme::Dark;
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
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.window_margin = Margin::same(14);
        style.spacing.menu_margin = Margin::same(8);
        style.spacing.slider_rail_height = 4.0;
        style.spacing.icon_width = 16.0;
        style.spacing.icon_width_inner = 8.0;
        // Thin scroll bars that only widen under the pointer.
        let mut scroll = egui::style::ScrollStyle::floating();
        scroll.bar_width = 8.0;
        scroll.floating_width = 3.0;
        style.spacing.scroll = scroll;
        style.animation_time = 0.1;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = p.bg;
        v.window_fill = p.raised;
        v.window_stroke = Stroke::new(1.0, p.line);
        v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;
        v.extreme_bg_color = p.raised;
        v.text_edit_bg_color = Some(Color32::from_rgb(0x21, 0x21, 0x21));
        v.faint_bg_color = p.surface;
        v.hyperlink_color = p.accent;
        v.error_fg_color = p.danger;
        v.warn_fg_color = p.accent;
        v.selection.bg_fill = Color32::from_rgb(0x3c, 0x3c, 0x3c);
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.striped = false;
        v.handle_shape = egui::style::HandleShape::Circle;

        let radius = CornerRadius::same(RADIUS_ROW);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.surface;
        w.noninteractive.weak_bg_fill = p.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.noninteractive.corner_radius = radius;
        let hovered = Color32::from_rgb(0x22, 0x22, 0x22);
        let pressed = Color32::from_rgb(0x2c, 0x2c, 0x2c);
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
        // Checkboxes, radio buttons and fields stand out on the cards too.
        w.inactive.bg_fill = Color32::from_rgb(0x27, 0x27, 0x27);
        w.inactive.weak_bg_fill = p.raised;
        w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3a, 0x3a));
        w.hovered.bg_stroke = Stroke::new(1.0, p.faint);
    });
}

/// Uses the system font (Segoe UI on Windows) instead of bundling one. The files
/// are mapped into memory rather than copied: Windows shares those pages with
/// every other application using the font, so they cost SpotiLite almost nothing.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let regular = map_system_font(&["segoeui.ttf", "DejaVuSans.ttf", "NotoSans-Regular.ttf"]);
    let semibold = map_system_font(&["seguisb.ttf", "DejaVuSans-Bold.ttf", "NotoSans-SemiBold.ttf"]);

    if let Some(bytes) = regular {
        fonts.font_data.insert("system".into(), std::sync::Arc::new(FontData::from_static(bytes)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "system".into());
    }
    let mut semibold_family = Vec::new();
    if let Some(bytes) = semibold {
        fonts.font_data.insert("system-semibold".into(), std::sync::Arc::new(FontData::from_static(bytes)));
        semibold_family.push("system-semibold".to_string());
    }
    semibold_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name("semibold".into()), semibold_family);
    ctx.set_fonts(fonts);
}

fn map_system_font(names: &[&str]) -> Option<&'static [u8]> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(std::path::PathBuf::from(windir).join("Fonts"));
    } else {
        dirs.push("/usr/share/fonts/truetype/dejavu".into());
        dirs.push("/usr/share/fonts/truetype/noto".into());
        dirs.push("/usr/share/fonts/TTF".into());
    }
    names.iter().flat_map(|name| dirs.iter().map(move |d| d.join(name))).find_map(|path| {
        let file = std::fs::File::open(path).ok()?;
        // SAFETY: system font files are not modified while in use (Windows locks them).
        let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
        // Fonts live as long as the application.
        let map: &'static memmap2::Mmap = Box::leak(Box::new(map));
        Some(&map[..])
    })
}
