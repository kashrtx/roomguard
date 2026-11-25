//! Drawing helpers for the hand drawn hardware: glossy molded plastics,
//! custom gradient polygons, soft shadows, LED glows and crisp vector icons.
//! Everything is drawn in code.

use eframe::egui::epaint::{Mesh, Shadow};
use eframe::egui::{Color32, Painter, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

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
    let k = 0.24 + 0.76 * lights.clamp(0.0, 1.0);
    let f = |x: u8| (x as f32 * k) as u8;
    Color32::from_rgba_premultiplied(f(c.r()), f(c.g()), f(c.b()), c.a())
}

pub fn rounded_points(rect: Rect, r: f32, seg: usize) -> Vec<Pos2> {
    rounded_points_4(rect, r, r, r, r, seg)
}

/// Rounded rectangle points with independent corner radii (TL, TR, BR, BL).
pub fn rounded_points_4(rect: Rect, tl: f32, tr: f32, br: f32, bl: f32, seg: usize) -> Vec<Pos2> {
    let max_r = (rect.width() * 0.5).min(rect.height() * 0.5).max(0.0);
    let tl = tl.clamp(0.0, max_r);
    let tr = tr.clamp(0.0, max_r);
    let br = br.clamp(0.0, max_r);
    let bl = bl.clamp(0.0, max_r);
    let seg = seg.max(1);
    let corners = [
        (Pos2::new(rect.right() - tr, rect.top() + tr), tr, -90.0f32),
        (Pos2::new(rect.right() - br, rect.bottom() - br), br, 0.0),
        (Pos2::new(rect.left() + bl, rect.bottom() - bl), bl, 90.0),
        (Pos2::new(rect.left() + tl, rect.top() + tl), tl, 180.0),
    ];
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    for (c, r, start) in corners {
        if r <= 0.05 {
            pts.push(c);
        } else {
            for i in 0..=seg {
                let a = (start + 90.0 * i as f32 / seg as f32).to_radians();
                pts.push(Pos2::new(c.x + r * a.cos(), c.y + r * a.sin()));
            }
        }
    }
    pts
}

/// A rounded rectangle with a vertical gradient and a crisp outline.
pub fn grad_rrect(p: &Painter, rect: Rect, r: f32, top: Color32, bottom: Color32, outline: Option<Stroke>) {
    let pts = rounded_points(rect, r, 4);
    grad_poly_v(p, &pts, rect.top(), rect.bottom(), top, bottom, outline);
}

/// A convex / star-shaped polygon filled with a vertical linear gradient.
pub fn grad_poly_v(p: &Painter, pts: &[Pos2], y0: f32, y1: f32, top: Color32, bottom: Color32, outline: Option<Stroke>) {
    if pts.len() < 3 {
        return;
    }
    let span = (y1 - y0).abs().max(1.0);
    let col = |q: Pos2| mix(top, bottom, (q.y - y0) / span);
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    for q in pts {
        cx += q.x;
        cy += q.y;
    }
    let c = Pos2::new(cx / pts.len() as f32, cy / pts.len() as f32);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, col(c));
    for q in pts {
        mesh.colored_vertex(*q, col(*q));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
    if let Some(edge) = outline {
        if edge.width > 0.0 {
            p.add(Shape::closed_line(pts.to_vec(), edge));
        }
    } else {
        p.add(Shape::closed_line(pts.to_vec(), Stroke::new(0.8f32, bottom)));
    }
}

/// A convex / star-shaped polygon filled with a horizontal linear gradient.
pub fn grad_poly_h(p: &Painter, pts: &[Pos2], x0: f32, x1: f32, left: Color32, right: Color32, outline: Option<Stroke>) {
    if pts.len() < 3 {
        return;
    }
    let span = (x1 - x0).abs().max(1.0);
    let col = |q: Pos2| mix(left, right, (q.x - x0) / span);
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    for q in pts {
        cx += q.x;
        cy += q.y;
    }
    let c = Pos2::new(cx / pts.len() as f32, cy / pts.len() as f32);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, col(c));
    for q in pts {
        mesh.colored_vertex(*q, col(*q));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
    if let Some(edge) = outline {
        if edge.width > 0.0 {
            p.add(Shape::closed_line(pts.to_vec(), edge));
        }
    }
}

/// Soft multi-pass drop shadow for a custom convex polygon (wall shadow).
pub fn poly_shadow(p: &Painter, pts: &[Pos2], offset: Vec2, spread: f32, color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    for q in pts {
        cx += q.x;
        cy += q.y;
    }
    let center = Pos2::new(cx / pts.len() as f32, cy / pts.len() as f32);
    let layers = 7usize;
    for layer in (0..layers).rev() {
        let frac = (layer as f32 + 1.0) / layers as f32;
        let ex = spread * frac;
        let falloff = (1.0 - frac * 0.90).max(0.0);
        let a = falloff * falloff * 0.24;
        let off = offset * (0.25 + 0.75 * frac);
        let shifted: Vec<Pos2> = pts
            .iter()
            .map(|q| {
                let d = *q - center;
                let len = d.length().max(1.0);
                *q + off + d / len * ex
            })
            .collect();
        p.add(Shape::convex_polygon(shifted, alpha(color, a), Stroke::NONE));
    }
}

/// Tiny crisp vector triangle arrow (avoids missing Unicode glyphs in Inter).
pub fn tiny_tri(p: &Painter, c: Pos2, s: f32, dir: i8, color: Color32) {
    // dir: 0 = up, 1 = down, 2 = left, 3 = right
    let pts = match dir {
        0 => vec![c + Vec2::new(0.0, -0.45) * s, c + Vec2::new(0.48, 0.38) * s, c + Vec2::new(-0.48, 0.38) * s],
        1 => vec![c + Vec2::new(0.0, 0.45) * s, c + Vec2::new(0.48, -0.38) * s, c + Vec2::new(-0.48, -0.38) * s],
        2 => vec![c + Vec2::new(-0.45, 0.0) * s, c + Vec2::new(0.38, -0.48) * s, c + Vec2::new(0.38, 0.48) * s],
        _ => vec![c + Vec2::new(0.45, 0.0) * s, c + Vec2::new(-0.38, -0.48) * s, c + Vec2::new(-0.38, 0.48) * s],
    };
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Glossy plastic specular sheen band across the upper portion of a rounded rect.
pub fn gloss_band(p: &Painter, rect: Rect, r: f32, strength: f32) {
    if strength <= 0.005 || rect.height() < 4.0 || rect.width() < 4.0 {
        return;
    }
    let band = Rect::from_min_max(
        rect.min + Vec2::new(rect.width() * 0.04, 1.5),
        Pos2::new(rect.right() - rect.width() * 0.04, rect.top() + rect.height() * 0.44),
    );
    grad_rrect(
        p,
        band,
        (r - 1.5).max(1.0),
        alpha(Color32::WHITE, strength),
        alpha(Color32::WHITE, 0.0),
        Some(Stroke::NONE),
    );
}

/// A smooth vertical-gradient circle (for glossy domed round keys).
pub fn grad_circle(p: &Painter, center: Pos2, radius: f32, top: Color32, bottom: Color32, outline: Option<Stroke>) {
    if radius <= 0.5 {
        return;
    }
    let n = 28usize;
    let pts: Vec<Pos2> = (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            center + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    grad_poly_v(p, &pts, center.y - radius, center.y + radius, top, bottom, outline);
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

/// A recessed rectangular/square LED lens (classic PC1550 style).
pub fn square_led(p: &Painter, center: Pos2, size: Vec2, color: Color32, lit: bool, lights: f32) {
    let rect = Rect::from_center_size(center, size);
    let r = (size.x.min(size.y) * 0.22).max(1.0);
    // Molded bezel recess
    p.rect_filled(rect.expand(size.x * 0.14), r + 0.8, dim(Color32::from_rgb(0x6E, 0x6A, 0x64), lights));
    p.rect_filled(rect.expand(size.x * 0.06), r + 0.3, Color32::from_rgb(0x28, 0x24, 0x22));
    if lit {
        let boost = 0.55 + 0.35 * (1.0 - lights);
        // Wide soft bloom onto the glossy faceplate
        glow(p, center, size.x * 2.9, alpha(color, 0.42 * boost));
        glow(p, center, size.x * 1.45, alpha(color, 0.72));
        // Diffused glowing lens body
        p.rect_filled(rect, r, color);
        // Bright hot center core
        p.rect_filled(rect.shrink(size.x * 0.22), r * 0.6, mix(color, Color32::WHITE, 0.62));
        // Specular lens glint
        let glint = Rect::from_min_size(rect.left_top() + Vec2::splat(size.x * 0.12), Vec2::new(size.x * 0.45, size.y * 0.26));
        p.rect_filled(glint, r * 0.4, alpha(Color32::WHITE, 0.48));
    } else {
        // Dark smoky tinted recessed lens when off
        let off_top = mix(dim(Color32::from_rgb(0x46, 0x42, 0x3F), lights), color, 0.14);
        let off_bot = mix(dim(Color32::from_rgb(0x2E, 0x2B, 0x29), lights), color, 0.08);
        grad_rrect(p, rect, r, off_top, off_bot, Some(Stroke::new(0.6f32, Color32::from_rgb(0x1E, 0x1C, 0x1A))));
        let glint = Rect::from_min_size(rect.left_top() + Vec2::splat(size.x * 0.14), Vec2::new(size.x * 0.42, size.y * 0.22));
        p.rect_filled(glint, r * 0.3, alpha(Color32::WHITE, 0.18 * lights + 0.04));
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
    let pts = vec![c + Vec2::new(-0.42, 0.04) * s, c + Vec2::new(-0.10, 0.36) * s, c + Vec2::new(0.46, -0.34) * s];
    p.add(Shape::line(pts, Stroke::new((s * 0.17).max(1.0), color)));
}

pub fn icon_padlock(p: &Painter, c: Pos2, s: f32, color: Color32) {
    // Shackle arch
    let shackle_c = c + Vec2::new(0.0, -0.14) * s;
    let r = s * 0.22;
    arc(p, shackle_c, r, std::f32::consts::PI, std::f32::consts::TAU, Stroke::new((s * 0.12).max(1.0), color));
    p.line_segment(
        [shackle_c + Vec2::new(-r, 0.0), shackle_c + Vec2::new(-r, s * 0.14)],
        Stroke::new((s * 0.12).max(1.0), color),
    );
    p.line_segment(
        [shackle_c + Vec2::new(r, 0.0), shackle_c + Vec2::new(r, s * 0.14)],
        Stroke::new((s * 0.12).max(1.0), color),
    );
    // Lock body
    let body = Rect::from_min_max(c + Vec2::new(-0.34, -0.02) * s, c + Vec2::new(0.34, 0.46) * s);
    p.rect_filled(body, s * 0.08, color);
}

pub fn icon_warning(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![c + Vec2::new(0.0, -0.46) * s, c + Vec2::new(0.48, 0.40) * s, c + Vec2::new(-0.48, 0.40) * s];
    p.add(Shape::closed_line(pts, Stroke::new((s * 0.13).max(1.0), color)));
    p.line_segment([c + Vec2::new(0.0, -0.15) * s, c + Vec2::new(0.0, 0.13) * s], Stroke::new((s * 0.12).max(1.0), color));
    p.circle_filled(c + Vec2::new(0.0, 0.26) * s, (s * 0.065).max(0.8), color);
}

pub fn icon_ac_power(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let st = Stroke::new((s * 0.11).max(1.0), color);
    p.circle_stroke(c, s * 0.44, st);
    // Lightning bolt inside circle (classic AC power icon)
    let pts = vec![
        c + Vec2::new(0.08, -0.32) * s,
        c + Vec2::new(-0.14, 0.04) * s,
        c + Vec2::new(0.06, 0.04) * s,
        c + Vec2::new(-0.08, 0.34) * s,
    ];
    p.add(Shape::line(pts, Stroke::new((s * 0.11).max(1.0), color)));
}

/// Auxiliary / medical triangle icon (authentic PowerSeries PK5500 style).
pub fn icon_aux_triangle(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![
        c + Vec2::new(0.0, -0.44) * s,
        c + Vec2::new(0.46, 0.38) * s,
        c + Vec2::new(-0.46, 0.38) * s,
    ];
    p.add(Shape::closed_line(pts, Stroke::new((s * 0.13).max(1.0), color)));
    // Small inner accent line
    p.line_segment(
        [c + Vec2::new(-0.18, 0.18) * s, c + Vec2::new(0.18, 0.18) * s],
        Stroke::new((s * 0.10).max(0.9), color),
    );
    p.circle_filled(c + Vec2::new(0.0, -0.02) * s, (s * 0.08).max(0.9), color);
}

/// A flame: a teardrop with its point up, and a smaller, lighter one inside.
pub fn icon_flame(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let drop = |scale: f32, dy: f32| -> Vec<Pos2> {
        (0..28)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 28.0;
                let x = 0.38 * t.sin() * (t * 0.5).sin();
                let y = -0.48 * t.cos();
                c + Vec2::new(x * scale, y * scale + dy) * s
            })
            .collect()
    };
    p.add(Shape::convex_polygon(drop(1.0, 0.0), color, Stroke::NONE));
    p.add(Shape::convex_polygon(drop(0.46, 0.18), mix(color, Color32::WHITE, 0.45), Stroke::NONE));
}

pub fn icon_shield(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let pts = vec![
        c + Vec2::new(-0.38, -0.42) * s,
        c + Vec2::new(0.38, -0.42) * s,
        c + Vec2::new(0.38, 0.04) * s,
        c + Vec2::new(0.0, 0.46) * s,
        c + Vec2::new(-0.38, 0.04) * s,
    ];
    p.add(Shape::closed_line(pts, Stroke::new((s * 0.13).max(1.0), color)));
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
