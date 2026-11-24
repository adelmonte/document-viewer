//! Toolbar icons drawn with the painter, so they follow the theme colors at any scale.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sidebar,
    Prev,
    Next,
    Up,
    Down,
    Search,
    Menu,
    Plus,
    Minus,
    Close,
    Back,
    Forward,
}

fn line(p: &Painter, s: Stroke, pts: &[Pos2]) {
    p.add(egui::Shape::line(pts.to_vec(), s));
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let s = Stroke::new((r / 6.0).clamp(1.25, 2.0), color);
    let at = |x: f32, y: f32| c + vec2(x * r, y * r);
    match icon {
        Icon::Sidebar => {
            let b = Rect::from_center_size(c, vec2(r * 1.7, r * 1.4));
            p.rect_stroke(b, 2.0, s, egui::StrokeKind::Middle);
            line(p, s, &[pos2(b.left() + b.width() * 0.36, b.top()), pos2(b.left() + b.width() * 0.36, b.bottom())]);
        }
        Icon::Prev | Icon::Back => line(p, s, &[at(0.25, -0.5), at(-0.25, 0.0), at(0.25, 0.5)]),
        Icon::Next | Icon::Forward => line(p, s, &[at(-0.25, -0.5), at(0.25, 0.0), at(-0.25, 0.5)]),
        Icon::Up => line(p, s, &[at(-0.5, 0.25), at(0.0, -0.25), at(0.5, 0.25)]),
        Icon::Down => line(p, s, &[at(-0.5, -0.25), at(0.0, 0.25), at(0.5, -0.25)]),
        Icon::Search => {
            p.circle_stroke(at(-0.15, -0.15), r * 0.45, s);
            line(p, s, &[at(0.18, 0.18), at(0.6, 0.6)]);
        }
        Icon::Menu => {
            for y in [-0.45, 0.0, 0.45] {
                line(p, s, &[at(-0.6, y), at(0.6, y)]);
            }
        }
        Icon::Plus => {
            line(p, s, &[at(-0.5, 0.0), at(0.5, 0.0)]);
            line(p, s, &[at(0.0, -0.5), at(0.0, 0.5)]);
        }
        Icon::Minus => line(p, s, &[at(-0.5, 0.0), at(0.5, 0.0)]),
        Icon::Close => {
            line(p, s, &[at(-0.4, -0.4), at(0.4, 0.4)]);
            line(p, s, &[at(-0.4, 0.4), at(0.4, -0.4)]);
        }
    }
}

/// A flat button showing an icon; highlighted when `active`.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool, active: bool) -> Response {
    let h = ui.spacing().interact_size.y.max(28.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(h + 4.0, h), if enabled { Sense::click() } else { Sense::hover() });
    let v = ui.visuals();
    let bg = if active || (enabled && resp.is_pointer_button_down_on()) {
        v.widgets.active.bg_fill
    } else if enabled && resp.hovered() {
        v.widgets.hovered.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, v.widgets.inactive.corner_radius, bg);
    let mut color = v.widgets.inactive.fg_stroke.color;
    if !enabled {
        color = color.gamma_multiply(0.35);
    }
    paint(painter, Rect::from_center_size(rect.center(), Vec2::splat(h * 0.5)), icon, color);
    let name = tooltip.split(" (").next().unwrap_or(tooltip).to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, &name));
    let resp = if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) };
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}
