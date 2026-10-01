//! Hand-drawn widgets: vector icons (no icon font or image assets), round
//! buttons, seek bars, pills and truncated text.

use std::f32::consts::PI;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Margin, Pos2, Rect, Response, Sense, Shape,
    Stroke, Ui, Vec2, pos2, vec2,
};

use super::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Prev,
    Heart { filled: bool },
    Volume { level: u8 },
    Back,
    Queue,
    Refresh,
    Shuffle,
    Repeat { one: bool },
    Search,
    Disc,
    Library,
    Settings,
    Artist,
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new((r * 0.15).clamp(1.4, 2.8), color);
    match icon {
        Icon::Play => {
            let pts = [
                pos2(c.x - r * 0.32, c.y - r * 0.52),
                pos2(c.x + r * 0.55, c.y),
                pos2(c.x - r * 0.32, c.y + r * 0.52),
            ];
            painter.add(Shape::convex_polygon(rounded(&pts, r * 0.12), color, Stroke::NONE));
        }
        Icon::Pause => {
            let w = r * 0.26;
            let h = r * 0.95;
            for dx in [-r * 0.28, r * 0.28] {
                let bar = Rect::from_center_size(pos2(c.x + dx, c.y), vec2(w, h));
                painter.rect_filled(bar, CornerRadius::same((w * 0.3) as u8), color);
            }
        }
        Icon::Next | Icon::Prev => {
            let dir = if icon == Icon::Next { 1.0 } else { -1.0 };
            let pts = [
                pos2(c.x - dir * r * 0.42, c.y - r * 0.45),
                pos2(c.x + dir * r * 0.28, c.y),
                pos2(c.x - dir * r * 0.42, c.y + r * 0.45),
            ];
            painter.add(Shape::convex_polygon(rounded(&pts, r * 0.1), color, Stroke::NONE));
            let bar = Rect::from_center_size(pos2(c.x + dir * r * 0.42, c.y), vec2(r * 0.17, r * 0.9));
            painter.rect_filled(bar, CornerRadius::same((r * 0.085) as u8), color);
        }
        Icon::Heart { filled } => {
            if filled {
                // Union of two circles and a triangle: a clean filled heart made of
                // convex shapes only.
                let lobe = r * 0.3;
                let top = c.y - r * 0.12;
                painter.circle_filled(pos2(c.x - lobe * 0.95, top), lobe, color);
                painter.circle_filled(pos2(c.x + lobe * 0.95, top), lobe, color);
                let tri = vec![
                    pos2(c.x - lobe * 1.9, top + lobe * 0.25),
                    pos2(c.x + lobe * 1.9, top + lobe * 0.25),
                    pos2(c.x, c.y + r * 0.58),
                ];
                painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
            } else {
                painter.add(Shape::closed_line(heart_points(c, r * 0.62), stroke));
            }
        }
        Icon::Volume { level } => {
            let body = vec![
                pos2(c.x - r * 0.6, c.y - r * 0.2),
                pos2(c.x - r * 0.35, c.y - r * 0.2),
                pos2(c.x - r * 0.02, c.y - r * 0.5),
                pos2(c.x - r * 0.02, c.y + r * 0.5),
                pos2(c.x - r * 0.35, c.y + r * 0.2),
                pos2(c.x - r * 0.6, c.y + r * 0.2),
            ];
            painter.add(Shape::convex_polygon(body, color, Stroke::NONE));
            if level == 0 {
                let x = c.x + r * 0.4;
                let d = r * 0.2;
                painter.line_segment([pos2(x - d, c.y - d), pos2(x + d, c.y + d)], stroke);
                painter.line_segment([pos2(x - d, c.y + d), pos2(x + d, c.y - d)], stroke);
            } else {
                for i in 0..level.min(2) {
                    let radius = r * (0.35 + 0.28 * f32::from(i));
                    painter.add(arc(pos2(c.x - r * 0.05, c.y), radius, -PI / 4.0, PI / 4.0, stroke));
                }
            }
        }
        Icon::Back => {
            painter.line_segment([pos2(c.x + r * 0.15, c.y - r * 0.42), pos2(c.x - r * 0.27, c.y)], stroke);
            painter.line_segment([pos2(c.x - r * 0.27, c.y), pos2(c.x + r * 0.15, c.y + r * 0.42)], stroke);
        }
        Icon::Queue => {
            for (i, w) in [0.9, 0.9, 0.55].iter().enumerate() {
                let y = c.y - r * 0.4 + i as f32 * r * 0.4;
                painter.line_segment([pos2(c.x - r * 0.55, y), pos2(c.x - r * 0.55 + r * w, y)], stroke);
            }
            let tri = vec![
                pos2(c.x + r * 0.2, c.y + r * 0.2),
                pos2(c.x + r * 0.6, c.y + r * 0.42),
                pos2(c.x + r * 0.2, c.y + r * 0.64),
            ];
            painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
        }
        Icon::Refresh => {
            // Circular arrow: an arc with a gap on the right, arrowhead at its end.
            let radius = r * 0.62;
            let (from, to) = (PI * 0.3, PI * 1.9);
            painter.add(arc(c, radius, from, to, stroke));
            arrow_head(
                painter,
                c + vec2(to.cos(), to.sin()) * radius,
                vec2(-to.sin(), to.cos()),
                r * 0.3,
                color,
            );
        }
        Icon::Shuffle => {
            // Two crossing paths with arrowheads on the right.
            let (l, rr) = (c.x - r * 0.62, c.x + r * 0.5);
            let (top, bottom) = (c.y - r * 0.38, c.y + r * 0.38);
            let mid = c.x - r * 0.1;
            painter.add(Shape::line(
                vec![pos2(l, top), pos2(mid - r * 0.1, top), pos2(mid + r * 0.25, bottom), pos2(rr, bottom)],
                stroke,
            ));
            painter.add(Shape::line(
                vec![pos2(l, bottom), pos2(mid - r * 0.1, bottom), pos2(mid + r * 0.25, top), pos2(rr, top)],
                stroke,
            ));
            arrow_head(painter, pos2(rr + r * 0.08, top), vec2(1.0, 0.0), r * 0.26, color);
            arrow_head(painter, pos2(rr + r * 0.08, bottom), vec2(1.0, 0.0), r * 0.26, color);
        }
        Icon::Repeat { one } => {
            // A rounded loop in two halves, each ending with an arrowhead: top
            // half going right, bottom half (the same, turned around) going left.
            let (hw, hh, corner) = (r * 0.66, r * 0.42, r * 0.3);
            let head = r * 0.24;
            let half: Vec<Pos2> = [pos2(c.x - hw, c.y + hh * 0.35)]
                .into_iter()
                .chain((0..=8).map(|i| {
                    let a = PI + PI * 0.5 * i as f32 / 8.0;
                    pos2(c.x - hw + corner, c.y - hh + corner) + vec2(a.cos(), a.sin()) * corner
                }))
                .chain([pos2(c.x + hw * 0.45, c.y - hh)])
                .collect();
            for flip in [1.0, -1.0] {
                let turn = |q: Pos2| c + (q - c) * flip;
                painter.add(Shape::line(half.iter().copied().map(turn).collect(), stroke));
                let tip = turn(pos2(c.x + hw * 0.45 + head * 0.9, c.y - hh));
                arrow_head(painter, tip, vec2(flip, 0.0), head, color);
            }
            if one {
                painter.text(c, Align2::CENTER_CENTER, "1", FontId::proportional(r * 0.62), color);
            }
        }
        Icon::Search => {
            let lens = c + vec2(-r * 0.12, -r * 0.12);
            painter.circle_stroke(lens, r * 0.42, stroke);
            let from = lens + vec2(r * 0.3, r * 0.3);
            painter.line_segment([from, from + vec2(r * 0.32, r * 0.32)], stroke);
        }
        Icon::Disc => {
            painter.circle_stroke(c, r * 0.6, stroke);
            painter.circle_filled(c, r * 0.14, color);
        }
        Icon::Library => {
            // Three books: two upright, one leaning.
            for dx in [-0.45, -0.12] {
                let x = c.x + r * dx;
                painter.line_segment([pos2(x, c.y - r * 0.55), pos2(x, c.y + r * 0.55)], stroke);
            }
            painter.line_segment(
                [pos2(c.x + r * 0.2, c.y - r * 0.5), pos2(c.x + r * 0.5, c.y + r * 0.55)],
                stroke,
            );
        }
        Icon::Artist => {
            // Head and shoulders.
            painter.circle_stroke(pos2(c.x, c.y - r * 0.28), r * 0.3, stroke);
            painter.add(arc(pos2(c.x, c.y + r * 0.78), r * 0.62, PI * 1.12, PI * 1.88, stroke));
        }
        Icon::Settings => {
            // Two sliders.
            for (dy, knob) in [(-0.3, 0.25), (0.3, -0.25)] {
                let y = c.y + r * dy;
                painter.line_segment([pos2(c.x - r * 0.6, y), pos2(c.x + r * 0.6, y)], stroke);
                painter.circle_filled(pos2(c.x + r * knob, y), r * 0.17, color);
            }
        }
    }
}

/// The corners of a convex polygon rounded with quadratic curves (still convex).
fn rounded(points: &[Pos2], radius: f32) -> Vec<Pos2> {
    let n = points.len();
    let mut out = Vec::with_capacity(n * 6);
    for i in 0..n {
        let (prev, v, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let a = v + (prev - v).normalized() * radius.min((prev - v).length() * 0.5);
        let b = v + (next - v).normalized() * radius.min((next - v).length() * 0.5);
        for k in 0..=5 {
            let t = k as f32 / 5.0;
            let u = 1.0 - t;
            out.push(pos2(
                u * u * a.x + 2.0 * u * t * v.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * v.y + t * t * b.y,
            ));
        }
    }
    out
}

fn arrow_head(painter: &egui::Painter, tip: Pos2, direction: Vec2, size: f32, color: Color32) {
    let d = direction.normalized();
    let n = vec2(-d.y, d.x);
    let base = tip - d * size;
    let tri = vec![tip, base + n * size * 0.75, base - n * size * 0.75];
    painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
}

fn arc(center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) -> Shape {
    let n = 16;
    let points = (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    Shape::line(points, stroke)
}

fn heart_points(c: Pos2, size: f32) -> Vec<Pos2> {
    (0..40)
        .map(|i| {
            let t = i as f32 / 40.0 * 2.0 * PI;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            pos2(c.x + x / 17.0 * size, c.y - y / 17.0 * size + size * 0.1)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    /// Transparent, a soft disc under the pointer, grey icon (toggles).
    Plain,
    /// Same, white icon (main controls).
    Bright,
    /// White disc, black icon: the main action.
    Accent,
    /// Grey disc.
    Raised,
}

/// Round icon button. `active` marks a toggle that is on (white icon and a dot).
pub fn round_button(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    size: f32,
    style: ButtonStyle,
    active: bool,
) -> Response {
    let shrink = if style == ButtonStyle::Accent { 0.29 } else { 0.22 };
    icon_button(ui, p, icon, size, size * (1.0 - 2.0 * shrink), style, active)
}

/// Round icon button whose icon is drawn in an `icon_size` square: large
/// controls get large, detailed icons.
pub fn icon_button(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    size: f32,
    icon_size: f32,
    style: ButtonStyle,
    active: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        let pressed = response.is_pointer_button_down_on();
        let scale = if pressed { 0.94 } else { 1.0 };
        let radius = size * 0.5 * scale;
        let color = match style {
            ButtonStyle::Accent => {
                let fill = if hovered { p.accent_hover } else { p.accent };
                painter.circle_filled(rect.center(), radius, fill);
                p.on_accent
            }
            ButtonStyle::Raised => {
                painter.circle_filled(rect.center(), radius, if hovered { p.line } else { p.raised });
                p.text
            }
            ButtonStyle::Plain | ButtonStyle::Bright => {
                // Translucent: works on the plain surfaces and on the cover gradient.
                if hovered {
                    painter.circle_filled(rect.center(), radius, Color32::from_white_alpha(26));
                }
                if active || hovered || style == ButtonStyle::Bright { p.text } else { p.dim }
            }
        };
        let color = if active { p.accent } else { color };
        let inner = Rect::from_center_size(rect.center(), Vec2::splat(icon_size * scale));
        paint_icon(painter, inner, icon, color);
        if active {
            let dot = (size * 0.045).clamp(1.8, 2.6);
            painter.circle_filled(pos2(rect.center().x, rect.bottom() - dot), dot, p.accent);
        }
    }
    response
}

/// Pill button: accent filled (`primary`) or subtle.
pub fn pill(ui: &mut Ui, p: &Palette, label: &str, primary: bool) -> Response {
    pill_with_icon(ui, p, None, label, primary)
}

pub fn pill_with_icon(ui: &mut Ui, p: &Palette, icon: Option<Icon>, label: &str, primary: bool) -> Response {
    let font = theme::strong_font(13.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(galley.size().x + 32.0 + icon_w, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let enabled = ui.is_enabled();
    let hovered = response.hovered() && enabled;
    // Secondary pills are lighter than both the surfaces and the cards they sit on.
    let (fill, fg) = match (primary, hovered) {
        (true, false) => (p.accent, p.on_accent),
        (true, true) => (p.accent_hover, p.on_accent),
        (false, false) => (p.line, p.text),
        (false, true) => (Color32::from_rgb(0x33, 0x33, 0x33), p.text),
    };
    let (fill, fg) = if enabled { (fill, fg) } else { (p.line, p.faint) };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(17), fill);
    let mut x = rect.left() + 16.0;
    if let Some(icon) = icon {
        paint_icon(
            painter,
            Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(15.0, 15.0)),
            icon,
            fg,
        );
        x += icon_w;
    }
    painter.galley(pos2(x, rect.center().y - galley.size().y * 0.5), galley, fg);
    response
}

/// Rounded label chip (artists in search results).
pub fn chip(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::body_font(), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 26.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), if response.hovered() { p.line } else { p.raised });
    painter.galley(rect.center() - galley.size() * 0.5, galley, p.text);
    response
}

/// Pill-shaped text field with a leading icon (search, filter).
pub fn search_field(
    ui: &mut Ui,
    p: &Palette,
    text: &mut String,
    hint: &str,
    width: f32,
    icon: Icon,
) -> Response {
    let inner = egui::Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin { left: 12, right: 12, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.set_width(width - 24.0);
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            ui.allocate_ui_with_layout(vec2(width - 24.0, 30.0), layout, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                paint_icon(ui.painter(), rect, icon, p.dim);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(egui::RichText::new(hint).color(p.faint))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(2, 6)),
                )
            })
            .inner
        });
    inner.inner
}

/// Horizontal bar used for seeking and volume. Returns the committed value
/// (click or end of drag) and draws the in-progress drag position.
pub fn bar(ui: &mut Ui, p: &Palette, width: f32, value: f32, enabled: bool) -> (Response, Option<f32>, f32) {
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 18.0),
        if enabled { Sense::click_and_drag() } else { Sense::hover() },
    );
    let mut shown = value.clamp(0.0, 1.0);
    let mut committed = None;
    if enabled && let Some(pos) = response.interact_pointer_pos() {
        let v = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        if response.dragged() || response.is_pointer_button_down_on() {
            shown = v;
        }
        if response.drag_stopped() || response.clicked() {
            committed = Some(v);
            shown = v;
        }
    }
    let active = enabled && (response.hovered() || response.dragged());
    let painter = ui.painter();
    let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), if active { 6.0 } else { 4.0 }));
    // Translucent rail: readable on any background, including the cover gradient.
    painter.rect_filled(rail, CornerRadius::same(3), Color32::from_white_alpha(48));
    let mut filled = rail;
    filled.set_right(rail.left() + rail.width() * shown);
    painter.rect_filled(filled, CornerRadius::same(3), if active { p.accent } else { p.text });
    if active {
        painter.circle_filled(pos2(filled.right(), rect.center().y), 7.0, p.accent);
    }
    let response = if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response };
    (response, committed, shown)
}

/// Lays text out on a single line, cut with "…" beyond `max_width`.
pub fn truncated(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width(max_width.max(8.0));
    ui.painter().layout_job(job)
}

/// Paints truncated text vertically centered at `left_center`; returns its rect.
pub fn paint_text(
    ui: &Ui,
    left_center: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Rect {
    let galley = truncated(ui, text, font, color, max_width);
    let rect = Align2::LEFT_CENTER.anchor_size(left_center, galley.size());
    ui.painter().galley(rect.min, galley, color);
    rect
}

/// A clickable link-like text region inside an already allocated row.
pub fn text_link(ui: &mut Ui, id: egui::Id, rect: Rect, p: &Palette) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        ui.painter().line_segment(
            [pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.bottom())],
            Stroke::new(1.0, p.dim),
        );
    }
    response
}

/// Rounded placeholder or cover image.
pub fn cover(ui: &Ui, p: &Palette, rect: Rect, texture: Option<egui::TextureId>, radius: u8) {
    match texture {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            egui::Image::new((texture, rect.size()))
                .uv(uv)
                .corner_radius(CornerRadius::same(radius))
                .paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, CornerRadius::same(radius), p.raised);
            paint_icon(ui.painter(), rect.shrink(rect.width() * 0.3), Icon::Disc, p.faint);
        }
    }
}

pub fn format_duration(ms: u32) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn format_total(ms: u64) -> String {
    let minutes = ms / 60_000;
    if minutes >= 60 {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(61_500), "1:01");
        assert_eq!(format_duration(3_723_000), "1:02:03");
        assert_eq!(format_total(59 * 60_000), "59 min");
        assert_eq!(format_total(133 * 60_000), "2 h 13 min");
    }
}
