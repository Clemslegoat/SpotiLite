//! Hand-drawn widgets: vector icons (no icon font or image assets), seek bars,
//! pill buttons and truncated text.

use std::f32::consts::PI;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use eframe::epaint::text::{LayoutJob, TextWrapping};

use super::theme::Palette;

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
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new((r * 0.16).clamp(1.3, 2.2), color);
    match icon {
        Icon::Play => {
            let pts = vec![
                pos2(c.x - r * 0.32, c.y - r * 0.5),
                pos2(c.x + r * 0.52, c.y),
                pos2(c.x - r * 0.32, c.y + r * 0.5),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Pause => {
            let w = r * 0.24;
            let h = r * 0.95;
            for dx in [-r * 0.27, r * 0.27] {
                let bar = Rect::from_center_size(pos2(c.x + dx, c.y), vec2(w, h));
                painter.rect_filled(bar, CornerRadius::same(1), color);
            }
        }
        Icon::Next | Icon::Prev => {
            let dir = if icon == Icon::Next { 1.0 } else { -1.0 };
            let pts = vec![
                pos2(c.x - dir * r * 0.42, c.y - r * 0.45),
                pos2(c.x + dir * r * 0.28, c.y),
                pos2(c.x - dir * r * 0.42, c.y + r * 0.45),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
            let bar = Rect::from_center_size(pos2(c.x + dir * r * 0.42, c.y), vec2(r * 0.16, r * 0.9));
            painter.rect_filled(bar, CornerRadius::same(1), color);
        }
        Icon::Heart { filled } => {
            let points = heart_points(c, r * 0.62);
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
                painter.add(Shape::closed_line(points, stroke));
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
            painter.line_segment([pos2(c.x + r * 0.15, c.y - r * 0.4), pos2(c.x - r * 0.25, c.y)], stroke);
            painter.line_segment([pos2(c.x - r * 0.25, c.y), pos2(c.x + r * 0.15, c.y + r * 0.4)], stroke);
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
            let end = c + vec2(to.cos(), to.sin()) * radius;
            let tangent = vec2(-to.sin(), to.cos());
            let normal = vec2(to.cos(), to.sin());
            let head = r * 0.3;
            let tri = vec![end + tangent * head, end + normal * head * 0.9, end - normal * head * 0.9];
            painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
        }
    }
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

/// Round icon button. `emphasis` draws the accent-filled "play" style button.
pub fn icon_button(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    size: f32,
    active: bool,
    emphasis: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        let color = if emphasis {
            painter.circle_filled(rect.center(), size * 0.5, if hovered { p.text } else { p.accent });
            p.on_accent
        } else {
            if hovered {
                painter.circle_filled(rect.center(), size * 0.5, p.hover);
            }
            if active {
                p.accent
            } else if hovered {
                p.text
            } else {
                p.dim
            }
        };
        let inner = rect.shrink(size * if emphasis { 0.26 } else { 0.2 });
        paint_icon(painter, inner, icon, color);
    }
    response
}

/// Small-caps text toggle ("ALÉA", "BOUCLE").
pub fn text_toggle(ui: &mut Ui, p: &Palette, label: &str, on: bool, min_width: f32) -> Response {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let size = vec2((galley.size().x + 10.0).max(min_width), galley.size().y + 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let color = if on {
        p.accent
    } else if response.hovered() {
        p.text
    } else {
        p.faint
    };
    let painter = ui.painter();
    painter.galley(rect.center() - galley.size() * 0.5, galley, color);
    if on {
        painter.circle_filled(pos2(rect.center().x, rect.bottom() - 1.0), 1.6, p.accent);
    }
    response
}

/// Pill button: accent filled (`primary`) or subtle.
pub fn pill(ui: &mut Ui, p: &Palette, label: &str, primary: bool) -> Response {
    let font = super::theme::strong_font(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 28.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let hovered = response.hovered();
    let (fill, fg) = match (primary, hovered) {
        (true, false) => (p.accent, p.on_accent),
        (true, true) => (p.text, p.bg),
        (false, false) => (p.raised, p.text),
        (false, true) => (p.hover, p.text),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(15), fill);
    if !primary {
        painter.rect_stroke(rect, CornerRadius::same(15), Stroke::new(1.0, p.line), StrokeKind::Inside);
    }
    painter.galley(rect.center() - galley.size() * 0.5, galley, fg);
    response
}

/// Horizontal bar used for seeking and volume. Returns the committed value
/// (click or end of drag) and draws the in-progress drag position.
pub fn bar(ui: &mut Ui, p: &Palette, width: f32, value: f32, enabled: bool) -> (Response, Option<f32>, f32) {
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 16.0),
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
    let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), if active { 5.0 } else { 3.0 }));
    painter.rect_filled(rail, CornerRadius::same(3), p.line);
    let mut filled = rail;
    filled.set_right(rail.left() + rail.width() * shown);
    painter.rect_filled(filled, CornerRadius::same(3), if active { p.accent } else { p.dim });
    if active {
        painter.circle_filled(pos2(filled.right(), rect.center().y), 6.0, p.text);
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
