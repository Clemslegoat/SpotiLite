//! The window's title bar, drawn by the application (the system one is hidden):
//! drag to move (Windows snapping included), double-click to maximize, three
//! round buttons in macOS colors, and resizing from the window edges.

use egui::viewport::ResizeDirection;
use egui::{
    Color32, CursorIcon, Frame, Id, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Ui,
    ViewportCommand, pos2, vec2,
};

use super::theme::{self, GAP, Palette};
use super::widgets::paint_text;

pub const HEIGHT: f32 = 40.0;
/// Width of the invisible resize border.
const EDGE: f32 = 5.0;
/// Corners grab a little further, for diagonal resizing.
const CORNER: f32 = 14.0;

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Minimize,
    Maximize,
    Close,
}

impl Button {
    /// macOS colors: fill, then a slightly darker rim.
    fn colors(self) -> (Color32, Color32) {
        match self {
            Button::Close => (Color32::from_rgb(0xff, 0x5f, 0x57), Color32::from_rgb(0xe2, 0x46, 0x3f)),
            Button::Minimize => (Color32::from_rgb(0xfe, 0xbc, 0x2e), Color32::from_rgb(0xe1, 0xa1, 0x16)),
            Button::Maximize => (Color32::from_rgb(0x28, 0xc8, 0x40), Color32::from_rgb(0x12, 0xac, 0x28)),
        }
    }

    fn hint(self, maximized: bool) -> &'static str {
        match self {
            Button::Close => "Close",
            Button::Minimize => "Minimize",
            Button::Maximize if maximized => "Restore",
            Button::Maximize => "Maximize",
        }
    }
}

/// The bar at the top of the window: logo and name on the left, buttons on the right.
pub fn show(ui: &mut Ui, p: &Palette, logo: impl FnOnce(&Ui, Pos2, f32)) {
    egui::Panel::top("titlebar")
        .exact_size(HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| bar(ui, p, logo));
}

fn bar(ui: &mut Ui, p: &Palette, logo: impl FnOnce(&Ui, Pos2, f32)) {
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let (focused, maximized) =
        ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().maximized.unwrap_or(false)));

    // The drag area first: the buttons, added after it, take precedence.
    let drag = ui.interact(rect, Id::new("titlebar-drag"), Sense::click_and_drag());
    if !on_edge(&ctx) {
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    // Logo and name, aligned with the sidebar content.
    let x = rect.left() + f32::from(GAP) + 14.0;
    logo(ui, pos2(x + 10.0, rect.center().y), 22.0);
    let name = if focused { p.dim } else { p.faint };
    paint_text(ui, pos2(x + 30.0, rect.center().y), "SpotiLite", theme::strong_font(13.5), name, 120.0);

    // Buttons: minimize, maximize, close (the close button in the usual Windows corner).
    let spacing = 24.0;
    let last = pos2(rect.right() - f32::from(GAP) - 16.0, rect.center().y);
    let buttons = [Button::Minimize, Button::Maximize, Button::Close];
    let centers = buttons.map(|b| {
        let from_right = (buttons.len() - 1 - buttons.iter().position(|x| *x == b).unwrap_or(0)) as f32;
        last - vec2(from_right * spacing, 0.0)
    });
    let group = Rect::from_min_max(centers[0] - vec2(12.0, 12.0), centers[2] + vec2(12.0, 12.0));
    let group_hovered = ctx.pointer_hover_pos().is_some_and(|pos| group.contains(pos));
    for (button, center) in buttons.into_iter().zip(centers) {
        let response = window_button(ui, button, center, focused || group_hovered, group_hovered, maximized);
        if response.on_hover_text(button.hint(maximized)).clicked() {
            ctx.send_viewport_cmd(match button {
                Button::Close => ViewportCommand::Close,
                Button::Minimize => ViewportCommand::Minimized(true),
                Button::Maximize => ViewportCommand::Maximized(!maximized),
            });
        }
    }
}

fn window_button(
    ui: &mut Ui,
    button: Button,
    center: Pos2,
    lit: bool,
    show_glyph: bool,
    maximized: bool,
) -> Response {
    let hit = Rect::from_center_size(center, vec2(22.0, 22.0));
    let response = ui.interact(hit, Id::new(("titlebar-button", button as u8)), Sense::click());
    let painter = ui.painter();
    let radius = 7.0;
    // Unfocused windows show grey buttons, like macOS.
    let (mut fill, rim) =
        if lit { button.colors() } else { (Color32::from_gray(0x3a), Color32::from_gray(0x32)) };
    if response.is_pointer_button_down_on() {
        fill = fill.gamma_multiply(0.82);
    }
    painter.circle_filled(center, radius, fill);
    painter.circle_stroke(center, radius - 0.4, Stroke::new(0.8, rim));
    if show_glyph {
        let ink = Color32::from_black_alpha(150);
        let stroke = Stroke::new(1.3, ink);
        let d = 3.0;
        match button {
            Button::Close => {
                painter.line_segment([center + vec2(-d, -d), center + vec2(d, d)], stroke);
                painter.line_segment([center + vec2(-d, d), center + vec2(d, -d)], stroke);
            }
            Button::Minimize => {
                painter.line_segment([center + vec2(-3.6, 0.0), center + vec2(3.6, 0.0)], stroke);
            }
            Button::Maximize => {
                // Two small triangles: pointing out to maximize, in to restore.
                for sign in [-1.0, 1.0] {
                    let (corner, leg) = if maximized {
                        (center + vec2(sign * 0.8, sign * 0.8), sign * 4.6)
                    } else {
                        (center + vec2(sign * 3.4, sign * 3.4), -sign * 4.0)
                    };
                    let points = vec![corner, corner + vec2(leg, 0.0), corner + vec2(0.0, leg)];
                    painter.add(Shape::convex_polygon(points, ink, Stroke::NONE));
                }
            }
        }
    }
    response.on_hover_cursor(CursorIcon::Default)
}

/// The resize direction under the pointer, if it is on the window border.
fn edge_under_pointer(ctx: &egui::Context) -> Option<ResizeDirection> {
    let (maximized, fullscreen) =
        ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if maximized || fullscreen {
        return None;
    }
    let pos = ctx.pointer_hover_pos()?;
    edge_at(ctx.content_rect(), pos)
}

fn edge_at(rect: Rect, pos: Pos2) -> Option<ResizeDirection> {
    let near = |distance: f32, limit: f32| (0.0..=limit).contains(&distance);
    let (left, right) = (pos.x - rect.left(), rect.right() - pos.x);
    let (top, bottom) = (pos.y - rect.top(), rect.bottom() - pos.y);
    let direction = if near(left, CORNER) && near(top, CORNER) && (near(left, EDGE) || near(top, EDGE)) {
        ResizeDirection::NorthWest
    } else if near(right, CORNER) && near(top, CORNER) && (near(right, EDGE) || near(top, EDGE)) {
        ResizeDirection::NorthEast
    } else if near(left, CORNER) && near(bottom, CORNER) && (near(left, EDGE) || near(bottom, EDGE)) {
        ResizeDirection::SouthWest
    } else if near(right, CORNER) && near(bottom, CORNER) && (near(right, EDGE) || near(bottom, EDGE)) {
        ResizeDirection::SouthEast
    } else if near(left, EDGE) {
        ResizeDirection::West
    } else if near(right, EDGE) {
        ResizeDirection::East
    } else if near(top, EDGE) {
        ResizeDirection::North
    } else if near(bottom, EDGE) {
        ResizeDirection::South
    } else {
        return None;
    };
    Some(direction)
}

fn on_edge(ctx: &egui::Context) -> bool {
    edge_under_pointer(ctx).is_some()
}

/// Resizing from the window border; called last so its cursor wins.
pub fn resize_edges(ctx: &egui::Context) {
    let Some(direction) = edge_under_pointer(ctx) else { return };
    ctx.set_cursor_icon(match direction {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_edges_and_corners() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let at = |x, y| edge_at(rect, pos2(x, y));
        assert_eq!(at(2.0, 300.0), Some(ResizeDirection::West));
        assert_eq!(at(798.0, 300.0), Some(ResizeDirection::East));
        assert_eq!(at(400.0, 1.0), Some(ResizeDirection::North));
        assert_eq!(at(400.0, 597.0), Some(ResizeDirection::South));
        assert_eq!(at(10.0, 2.0), Some(ResizeDirection::NorthWest));
        assert_eq!(at(797.0, 590.0), Some(ResizeDirection::SouthEast));
        assert_eq!(at(400.0, 300.0), None);
        assert_eq!(at(10.0, 10.0), None, "inside the corner zone but off the border");
    }
}
