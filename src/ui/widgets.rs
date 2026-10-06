//! Hand-drawn widgets: vector icons (no icon font or image file), round
//! buttons, seek bars, pills and truncated text.

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Margin, Pos2, Rect, Response, Sense, Shape,
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

/// Draws an icon in `rect`. Icons are designed on a 24 × 24 grid, like icon
/// fonts: round caps and joins, finely sampled curves, filled shapes with
/// rounded corners. Large icons (30 px and more) get extra details.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let pen = Pen::new(painter, rect, color);
    let detailed = pen.unit >= 1.25;
    match icon {
        Icon::Play => pen.fill_rounded(&[(8.0, 5.0), (19.4, 12.0), (8.0, 19.0)], 1.9),
        Icon::Pause => {
            pen.fill_rect((6.6, 5.0), (10.4, 19.0), 1.4);
            pen.fill_rect((13.6, 5.0), (17.4, 19.0), 1.4);
        }
        Icon::Next | Icon::Prev => {
            // Drawn for "next", mirrored for "previous".
            let m = |x: f32| if icon == Icon::Next { x } else { 24.0 - x };
            pen.fill_rounded(&[(m(5.4), 5.6), (m(15.6), 12.0), (m(5.4), 18.4)], 1.7);
            let (a, b) = (m(16.6), m(19.0));
            pen.fill_rect((a.min(b), 5.6), (a.max(b), 18.4), 1.2);
        }
        Icon::Heart { filled } => {
            let outline = pen.heart();
            if filled {
                pen.fill_star(pen.at(12.0, 12.5), outline);
            } else {
                pen.closed(outline);
            }
        }
        Icon::Volume { level } => {
            // Rounded body and cone, then the sound waves (or a cross when muted).
            pen.fill_rect((3.2, 9.0), (8.2, 15.0), 1.3);
            pen.fill_rounded(&[(7.0, 9.0), (12.2, 4.6), (12.2, 19.4), (7.0, 15.0)], 1.1);
            if level == 0 {
                pen.path(&[(15.6, 9.6), (20.4, 14.4)]);
                pen.path(&[(15.6, 14.4), (20.4, 9.6)]);
            } else {
                pen.path_points(pen.arc(13.0, 12.0, 3.6, -48.0, 48.0));
                if level >= 2 {
                    pen.path_points(pen.arc(13.0, 12.0, 7.4, -52.0, 52.0));
                }
            }
        }
        Icon::Back => pen.path(&[(14.8, 5.6), (8.4, 12.0), (14.8, 18.4)]),
        Icon::Queue => {
            pen.path(&[(3.8, 6.0), (20.2, 6.0)]);
            pen.path(&[(3.8, 11.0), (20.2, 11.0)]);
            pen.path(&[(3.8, 16.0), (11.0, 16.0)]);
            pen.fill_rounded(&[(14.4, 12.8), (20.8, 16.4), (14.4, 20.0)], 1.0);
        }
        Icon::Refresh => {
            // A circle that turns, ending in a solid arrowhead.
            let (from, to) = (-62.0_f32, 236.0_f32);
            pen.path_points(pen.arc(12.0, 12.0, 7.4, from, to));
            let a = to.to_radians();
            let end = vec2(12.0 + 7.4 * a.cos(), 12.0 + 7.4 * a.sin());
            let dir = vec2(-a.sin(), a.cos());
            let normal = vec2(-dir.y, dir.x);
            let tip = end + dir * 2.6;
            let base = end - dir * 1.4;
            let (l, r) = (base + normal * 3.0, base - normal * 3.0);
            pen.fill_rounded(&[(tip.x, tip.y), (l.x, l.y), (r.x, r.y)], 0.7);
        }
        Icon::Shuffle => {
            // Two crossing paths, each ending in an arrow.
            let curve = |y0: f32, y1: f32| {
                let mut points = vec![pen.at(3.2, y0), pen.at(6.2, y0)];
                points.extend(pen.cubic((6.2, y0), (11.2, y0), (11.8, y1), (16.6, y1)));
                points.push(pen.at(19.6, y1));
                points
            };
            pen.path_points(curve(7.4, 16.6));
            pen.path_points(curve(16.6, 7.4));
            pen.path(&[(16.8, 4.6), (19.8, 7.4), (16.8, 10.2)]);
            pen.path(&[(16.8, 13.8), (19.8, 16.6), (16.8, 19.4)]);
        }
        Icon::Repeat { one } => {
            // A rounded loop in two halves, each ending in an arrow.
            let mut top = vec![pen.at(4.0, 13.2)];
            top.extend(pen.arc(7.6, 10.6, 3.6, 180.0, 270.0));
            top.push(pen.at(18.6, 7.0));
            pen.path_points(top);
            pen.path(&[(16.0, 4.4), (18.8, 7.0), (16.0, 9.6)]);
            let mut bottom = vec![pen.at(20.0, 10.8)];
            bottom.extend(pen.arc(16.4, 13.4, 3.6, 0.0, 90.0));
            bottom.push(pen.at(5.4, 17.0));
            pen.path_points(bottom);
            pen.path(&[(8.0, 14.4), (5.2, 17.0), (8.0, 19.6)]);
            if one {
                let size = (6.4 * pen.unit).round().max(6.0);
                painter.text(pen.at(12.0, 11.8), Align2::CENTER_CENTER, "1", theme::strong_font(size), color);
            }
        }
        Icon::Search => {
            pen.closed(pen.arc(10.4, 10.4, 6.4, 0.0, 360.0));
            pen.with_width(pen.width * 1.25).path(&[(15.3, 15.3), (20.2, 20.2)]);
            if detailed {
                // A glint on the lens.
                pen.with_width(pen.width * 0.6).path_points(pen.arc(10.4, 10.4, 3.7, 200.0, 255.0));
            }
        }
        Icon::Disc => {
            // A record: rim, grooves catching the light, label and spindle hole.
            pen.closed(pen.arc(12.0, 12.0, 9.0, 0.0, 360.0));
            let fine = pen.with_width(pen.width * 0.55);
            fine.path_points(pen.arc(12.0, 12.0, 6.3, 200.0, 250.0));
            fine.path_points(pen.arc(12.0, 12.0, 6.3, 20.0, 70.0));
            if detailed {
                fine.path_points(pen.arc(12.0, 12.0, 7.7, 205.0, 235.0));
                fine.path_points(pen.arc(12.0, 12.0, 7.7, 25.0, 55.0));
            }
            pen.closed(pen.arc(12.0, 12.0, 3.0, 0.0, 360.0));
            pen.disc(12.0, 12.0, 0.9);
        }
        Icon::Library => {
            // Two beamed notes.
            pen.path(&[(8.8, 7.0), (8.8, 17.2)]);
            pen.path(&[(18.2, 5.2), (18.2, 15.4)]);
            pen.fill_rounded(&[(8.0, 5.4), (19.0, 3.2), (19.0, 6.6), (8.0, 8.8)], 0.6);
            pen.disc(6.4, 17.4, 2.8);
            pen.disc(15.8, 15.6, 2.8);
        }
        Icon::Artist => {
            pen.closed(pen.arc(12.0, 7.9, 3.9, 0.0, 360.0));
            pen.path_points(pen.arc(12.0, 22.0, 8.2, 207.0, 333.0));
        }
        Icon::Settings => {
            // A gear: eight teeth around a hub.
            let (outer, inner) = (9.6, 7.2);
            let mut outline = Vec::new();
            for k in 0..8 {
                let a = k as f32 * 45.0;
                for (angle, radius) in
                    [(a - 13.0, inner), (a - 8.0, outer), (a + 8.0, outer), (a + 13.0, inner)]
                {
                    let t = angle.to_radians();
                    outline.push(pen.at(12.0 + radius * t.cos(), 12.0 + radius * t.sin()));
                }
                outline.extend(pen.arc(12.0, 12.0, inner, a + 16.0, a + 29.0));
            }
            pen.closed(outline);
            pen.closed(pen.arc(12.0, 12.0, 3.2, 0.0, 360.0));
        }
    }
}

/// Draws on the 24 × 24 icon grid.
#[derive(Clone, Copy)]
struct Pen<'a> {
    painter: &'a egui::Painter,
    origin: Pos2,
    /// Pixels per grid unit.
    unit: f32,
    color: Color32,
    width: f32,
}

impl<'a> Pen<'a> {
    fn new(painter: &'a egui::Painter, rect: Rect, color: Color32) -> Self {
        let unit = rect.width().min(rect.height()) / 24.0;
        let origin = rect.center() - vec2(12.0, 12.0) * unit;
        Pen { painter, origin, unit, color, width: (1.9 * unit).clamp(1.3, 3.2) }
    }

    fn with_width(self, width: f32) -> Self {
        Pen { width: width.max(1.0), ..self }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    /// Points of a circular arc (degrees, clockwise on screen).
    fn arc(&self, cx: f32, cy: f32, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
        let steps = (((to - from).abs() / 6.0).ceil() as usize).max(6);
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
                self.at(cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect()
    }

    /// Points of a cubic Bézier curve (the first point excluded).
    fn cubic(&self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> Vec<Pos2> {
        (1..=16)
            .map(|i| {
                let t = i as f32 / 16.0;
                let u = 1.0 - t;
                let f = |a: f32, b: f32, c: f32, d: f32| {
                    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                };
                self.at(f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
            })
            .collect()
    }

    /// Rounded heart (two lobes, sides tangent to a rounded tip), clockwise.
    fn heart(&self) -> Vec<Pos2> {
        let (left, right, lobe): ((f32, f32), (f32, f32), f32) = ((8.1, 9.2), (15.9, 9.2), 4.75);
        let (tip, tip_radius): ((f32, f32), f32) = ((12.0, 18.6), 1.7);
        // Outer tangent between the left lobe and the tip circle.
        let (dx, dy) = (tip.0 - left.0, tip.1 - left.1);
        let side = dy.atan2(dx) + ((lobe - tip_radius) / dx.hypot(dy)).acos();
        let side = side.to_degrees();
        // Where the two lobes meet at the top.
        let notch = ((12.0 - left.0) / lobe).acos().to_degrees();
        let mut points = self.arc(right.0, right.1, lobe, 180.0 + notch, 360.0 + 180.0 - side);
        points.extend(self.arc(tip.0, tip.1, tip_radius, 180.0 - side, side));
        points.extend(self.arc(left.0, left.1, lobe, side, 360.0 - notch));
        points
    }

    /// Open line with round caps and joins.
    fn path(&self, points: &[(f32, f32)]) {
        self.path_points(points.iter().map(|&(x, y)| self.at(x, y)).collect());
    }

    fn path_points(&self, points: Vec<Pos2>) {
        let radius = self.width * 0.5;
        // Joins of short polylines (curves are sampled finely enough not to need them).
        let joins = if points.len() <= 4 { points.len() } else { 0 };
        for p in points.iter().take(joins) {
            self.painter.circle_filled(*p, radius, self.color);
        }
        if let (Some(first), Some(last)) = (points.first(), points.last()) {
            self.painter.circle_filled(*first, radius, self.color);
            self.painter.circle_filled(*last, radius, self.color);
        }
        self.painter.add(Shape::line(points, Stroke::new(self.width, self.color)));
    }

    fn closed(&self, points: Vec<Pos2>) {
        self.painter.add(Shape::closed_line(points, Stroke::new(self.width, self.color)));
    }

    fn disc(&self, x: f32, y: f32, radius: f32) {
        self.painter.circle_filled(self.at(x, y), radius * self.unit, self.color);
    }

    /// Convex polygon with rounded corners (radius in grid units).
    fn fill_rounded(&self, points: &[(f32, f32)], radius: f32) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter.add(Shape::convex_polygon(
            rounded(&points, radius * self.unit),
            self.color,
            Stroke::NONE,
        ));
    }

    fn fill_rect(&self, min: (f32, f32), max: (f32, f32), radius: f32) {
        let rect = Rect::from_min_max(self.at(min.0, min.1), self.at(max.0, max.1));
        self.painter.rect_filled(rect, CornerRadius::same((radius * self.unit).round() as u8), self.color);
    }

    /// Fills a shape that is not convex but whose whole outline is visible from
    /// `kernel` (a fan of triangles), with an anti-aliased edge.
    fn fill_star(&self, kernel: Pos2, outline: Vec<Pos2>) {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(kernel, self.color);
        for p in &outline {
            mesh.colored_vertex(*p, self.color);
        }
        let n = outline.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        self.painter.add(Shape::mesh(mesh));
        self.painter.add(Shape::closed_line(outline, Stroke::new(1.0, self.color)));
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

/// Round portrait, or the initials on a dark disc while there is no picture.
pub fn avatar(ui: &Ui, p: &Palette, rect: Rect, texture: Option<egui::TextureId>, name: &str) {
    let radius = rect.width().min(rect.height()) * 0.5;
    if texture.is_some() {
        return cover(ui, p, rect, texture, radius.min(255.0) as u8);
    }
    let painter = ui.painter();
    painter.circle_filled(rect.center(), radius, initials_tint(name));
    let initials: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    // A few fixed sizes only: each font size is rasterized separately.
    let size = ((radius * 0.44) / 4.0).round().max(3.0) * 4.0;
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials,
        theme::strong_font(size),
        Color32::from_gray(0xe6),
    );
}

/// A deep, muted color picked from the name: avatars without a picture are told
/// apart at a glance, without breaking the black and white theme.
fn initials_tint(name: &str) -> Color32 {
    let hash = name.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    let hue = (hash % 360) as f32 / 60.0;
    let (s, v) = (0.38, 0.30);
    let c = v * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let channel = |f: f32| ((f + m) * 255.0) as u8;
    Color32::from_rgb(channel(r), channel(g), channel(b))
}

/// Segmented control: one choice per segment, the selected one in white.
/// Returns the index clicked.
pub fn segmented(ui: &mut Ui, p: &Palette, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let font = theme::strong_font(13.0);
    let (height, inset) = (32.0, 3.0);
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| {
            ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::PLACEHOLDER).size().x + 28.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + inset * 2.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((height * 0.5) as u8), p.line);
    let mut clicked = None;
    let mut x = rect.left() + inset;
    for (i, (label, width)) in labels.iter().zip(&widths).enumerate() {
        let segment = Rect::from_min_size(pos2(x, rect.top() + inset), vec2(*width, height - inset * 2.0));
        let response = ui
            .interact(segment, Id::new(("segment", labels[0], i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let on = selected == Some(i);
        let radius = CornerRadius::same(((height - inset * 2.0) * 0.5) as u8);
        if on {
            painter.rect_filled(segment, radius, p.accent);
        } else if response.hovered() {
            painter.rect_filled(segment, radius, Color32::from_white_alpha(18));
        }
        let color = if on { p.on_accent } else { p.text };
        painter.text(segment.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        if response.clicked() {
            clicked = Some(i);
        }
        x += width;
    }
    clicked
}

/// On/off switch; returns true when it was flipped.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(42.0, 24.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response.clicked() {
        *on = !*on;
    }
    let painter = ui.painter();
    let track = if *on {
        p.accent
    } else if response.hovered() {
        Color32::from_gray(0x3a)
    } else {
        p.line
    };
    painter.rect_filled(rect, CornerRadius::same(12), track);
    let knob_x = if *on { rect.right() - 12.0 } else { rect.left() + 12.0 };
    let knob = if *on { p.on_accent } else { p.text };
    painter.circle_filled(pos2(knob_x, rect.center().y), 8.5, knob);
    response.clicked()
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
