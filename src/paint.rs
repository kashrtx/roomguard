//! Drawing helpers for the hand drawn hardware: gradients, soft shadows,
//! LED glows and small vector icons. Everything is drawn in code.

use eframe::egui::epaint::{Mesh, Shadow};
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// Darkens a colour for the "room lights off" look.
pub fn dim(c: Color32, lights: f32) -> Color32 {
    let k = 0.28 + 0.72 * lights.clamp(0.0, 1.0);
    let f = |x: u8| (x as f32 * k) as u8;
    Color32::from_rgba_premultiplied(f(c.r()), f(c.g()), f(c.b()), c.a())
}

pub fn rounded_points(rect: Rect, r: f32, seg: usize) -> Vec<Pos2> {
    let r = r.min(rect.width() * 0.5).min(rect.height() * 0.5).max(0.0);
    let seg = seg.max(1);
    let corners = [
        (Pos2::new(rect.right() - r, rect.top() + r), -90.0f32),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    for (c, start) in corners {
        for i in 0..=seg {
            let a = (start + 90.0 * i as f32 / seg as f32).to_radians();
            pts.push(Pos2::new(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    pts
}

/// A rounded rectangle with a vertical gradient and a crisp outline.
pub fn grad_rrect(p: &Painter, rect: Rect, r: f32, top: Color32, bottom: Color32, outline: Option<Stroke>) {
    let pts = rounded_points(rect, r, 6);
    let mut mesh = Mesh::default();
    let col = |q: Pos2| mix(top, bottom, (q.y - rect.top()) / rect.height().max(1.0));
    let c = rect.center();
    mesh.colored_vertex(c, col(c));
    for q in &pts {
        mesh.colored_vertex(*q, col(*q));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
    let edge = outline.unwrap_or(Stroke::new(0.8, bottom));
    p.add(Shape::closed_line(pts, edge));
}

pub fn shadow(p: &Painter, rect: Rect, r: f32, offset: Vec2, blur: f32, color: Color32) {
    let s = Shadow { offset, blur, spread: 0.0, color };
    p.add(s.as_shape(rect, Rounding::same(r)));
}

/// A radial glow, bright in the middle and fading to nothing.
pub fn glow(p: &Painter, center: Pos2, radius: f32, color: Color32) {
    if radius <= 0.5 {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    let n = 40u32;
    for i in 0..n {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        mesh.colored_vertex(center + Vec2::new(a.cos(), a.sin()) * radius, Color32::TRANSPARENT);
    }
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
}

/// A round panel LED: lens, lit core and glow.
pub fn led(p: &Painter, c: Pos2, r: f32, color: Color32, lit: bool, lights: f32) {
    let off = mix(Color32::from_rgb(40, 38, 36), color, 0.18);
    p.circle_filled(c, r + 1.2, Color32::from_rgba_premultiplied(0, 0, 0, 90));
    if lit {
        glow(p, c, r * 4.2, alpha(color, 0.45 + 0.25 * (1.0 - lights)));
        p.circle_filled(c, r, color);
        p.circle_filled(c - Vec2::new(r * 0.3, r * 0.35), r * 0.38, alpha(Color32::WHITE, 0.55));
    } else {
        p.circle_filled(c, r, off);
        p.circle_filled(c - Vec2::new(r * 0.3, r * 0.35), r * 0.3, alpha(Color32::WHITE, 0.12));
    }
}

pub fn arc(p: &Painter, c: Pos2, r: f32, from: f32, to: f32, stroke: Stroke) {
    let steps = (((to - from).abs() / 0.12).ceil() as usize).max(2);
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(pts, stroke));
}

pub fn dashed_rect(p: &Painter, rect: Rect, stroke: Stroke, dash: f32) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    for w in corners.windows(2) {
        p.extend(Shape::dashed_line(&[w[0], w[1]], stroke, dash, dash * 0.8));
    }
}

// ----- small vector icons -----

pub fn icon_check(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![c + Vec2::new(-0.45, 0.02) * s, c + Vec2::new(-0.12, 0.34) * s, c + Vec2::new(0.48, -0.32) * s];
    p.add(Shape::line(pts, Stroke::new(s * 0.16, color)));
}

pub fn icon_house(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let st = Stroke::new(s * 0.12, color);
    let roof = vec![c + Vec2::new(-0.5, -0.02) * s, c + Vec2::new(0.0, -0.46) * s, c + Vec2::new(0.5, -0.02) * s];
    p.add(Shape::line(roof, st));
    let body = Rect::from_min_max(c + Vec2::new(-0.34, -0.08) * s, c + Vec2::new(0.34, 0.44) * s);
    p.rect_stroke(body, 0.0, st);
}

pub fn icon_warning(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![c + Vec2::new(0.0, -0.48) * s, c + Vec2::new(0.52, 0.42) * s, c + Vec2::new(-0.52, 0.42) * s];
    p.add(Shape::closed_line(pts, Stroke::new(s * 0.11, color)));
    p.line_segment([c + Vec2::new(0.0, -0.16) * s, c + Vec2::new(0.0, 0.14) * s], Stroke::new(s * 0.11, color));
    p.circle_filled(c + Vec2::new(0.0, 0.27) * s, s * 0.06, color);
}

pub fn icon_plug(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let st = Stroke::new(s * 0.11, color);
    let body = Rect::from_min_max(c + Vec2::new(-0.3, -0.18) * s, c + Vec2::new(0.3, 0.2) * s);
    p.rect_stroke(body, s * 0.08, st);
    p.line_segment([c + Vec2::new(-0.14, -0.18) * s, c + Vec2::new(-0.14, -0.46) * s], st);
    p.line_segment([c + Vec2::new(0.14, -0.18) * s, c + Vec2::new(0.14, -0.46) * s], st);
    p.line_segment([c + Vec2::new(0.0, 0.2) * s, c + Vec2::new(0.0, 0.5) * s], st);
}

/// A flame: a teardrop with its point up, and a smaller, lighter one inside.
pub fn icon_flame(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let drop = |scale: f32, dy: f32| -> Vec<Pos2> {
        (0..32)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 32.0;
                let x = 0.40 * t.sin() * (t * 0.5).sin();
                let y = -0.50 * t.cos();
                c + Vec2::new(x * scale, y * scale + dy) * s
            })
            .collect()
    };
    p.add(Shape::convex_polygon(drop(1.0, 0.0), color, Stroke::NONE));
    p.add(Shape::convex_polygon(drop(0.48, 0.2), mix(color, Color32::WHITE, 0.5), Stroke::NONE));
}

pub fn icon_cross(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let a = s * 0.14;
    let b = s * 0.4;
    p.rect_filled(Rect::from_center_size(c, Vec2::new(a * 2.0, b * 2.0)), s * 0.04, color);
    p.rect_filled(Rect::from_center_size(c, Vec2::new(b * 2.0, a * 2.0)), s * 0.04, color);
}

pub fn icon_shield(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![
        c + Vec2::new(0.0, -0.48) * s,
        c + Vec2::new(0.42, -0.32) * s,
        c + Vec2::new(0.36, 0.12) * s,
        c + Vec2::new(0.0, 0.5) * s,
        c + Vec2::new(-0.36, 0.12) * s,
        c + Vec2::new(-0.42, -0.32) * s,
    ];
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

pub fn icon_sun(p: &Painter, c: Pos2, s: f32, color: Color32) {
    p.circle_filled(c, s * 0.2, color);
    for i in 0..8 {
        let a = std::f32::consts::TAU * i as f32 / 8.0;
        let d = Vec2::new(a.cos(), a.sin());
        p.line_segment([c + d * s * 0.32, c + d * s * 0.46], Stroke::new(s * 0.08, color));
    }
}

pub fn icon_moon(p: &Painter, c: Pos2, s: f32, color: Color32, bg: Color32) {
    p.circle_filled(c, s * 0.36, color);
    p.circle_filled(c + Vec2::new(s * 0.16, -s * 0.12), s * 0.3, bg);
}

pub fn icon_gear(p: &Painter, c: Pos2, s: f32, color: Color32) {
    for i in 0..8 {
        let a = std::f32::consts::TAU * i as f32 / 8.0;
        let d = Vec2::new(a.cos(), a.sin());
        p.line_segment([c + d * s * 0.26, c + d * s * 0.46], Stroke::new(s * 0.16, color));
    }
    p.circle_filled(c, s * 0.3, color);
    p.circle_filled(c, s * 0.13, Color32::from_rgb(0x1C, 0x1F, 0x22));
}
