//! PowerSeries 2000s LCD keypad (PC1832 / PK5500 form factor), unbranded:
//! glossy pearl-white curved-arch ABS housing, recessed emerald-green backlit
//! 2x16 dot-matrix LCD with optical polarizer depth shadow, four translucent
//! backlit status icons on the right, cool pale blue-grey pill rubber keys,
//! and a working bottom-hinged flip cover that goes UP to close and DOWN to
//! open, revealing the printed quick-reference card inside.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use super::{KeyInput, Xf};
use crate::audio::AudioHandle;
use crate::engine::types::Key;
use crate::engine::{EngineHandle, View};
use crate::lcd_font;
use crate::paint::{self, alpha, dim, mix};
use crate::theme;

/// Design space accommodating the keypad body + bottom-hinged open flip cover.
pub const SIZE: Vec2 = Vec2::new(420.0, 412.0);

const BX: f32 = 16.0;
const BY: f32 = 10.0;
const BW: f32 = 388.0;
const BH: f32 = 250.0;
const HINGE_Y: f32 = BY + BH - 2.0; // 258.0
const SEAM_Y: f32 = BY + 108.0; // 118.0
const COVER_H: f32 = 136.0;

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Text(&'static str),
    Small(&'static str),
    Chevron(bool),
    Fire,
    Aux,
    Panic,
}

/// Generates the iconic curved-arch PK5500 outer shell polygon.
fn shell_polygon(xf: &Xf, inset: f32) -> Vec<Pos2> {
    let x0 = BX + inset;
    let x1 = BX + BW - inset;
    let y0 = BY + inset;
    let y1 = BY + BH - inset;
    let w = x1 - x0;
    let h = y1 - y0;
    let r_top = (52.0 - inset).max(10.0);
    let r_bot = (46.0 - inset).max(10.0);
    // Subtle organic curve: sides curve gently inward toward the bottom
    let pinch = 8.0 * (1.0 - inset / 60.0).max(0.0);
    let seg = 10usize;
    let mut pts = Vec::with_capacity(4 * (seg + 1) + 4);

    // Top-right arch
    let c_tr = Pos2::new(x1 - r_top, y0 + r_top);
    for i in 0..=seg {
        let a = (-90.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tr.x + r_top * a.cos(), c_tr.y + r_top * a.sin()));
    }
    // Right curved side down to bottom-right arch
    pts.push(xf.p(x1 - pinch * 0.25, y0 + h * 0.56));
    let c_br = Pos2::new(x1 - pinch - r_bot, y1 - r_bot);
    for i in 0..=seg {
        let a = (0.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_br.x + r_bot * a.cos(), c_br.y + r_bot * a.sin()));
    }
    // Bottom-left arch
    let c_bl = Pos2::new(x0 + pinch + r_bot, y1 - r_bot);
    for i in 0..=seg {
        let a = (90.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_bl.x + r_bot * a.cos(), c_bl.y + r_bot * a.sin()));
    }
    // Left curved side up to top-left arch
    pts.push(xf.p(x0 + pinch * 0.25, y0 + h * 0.56));
    let c_tl = Pos2::new(x0 + r_top, y0 + r_top);
    for i in 0..=seg {
        let a = (180.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tl.x + r_top * a.cos(), c_tl.y + r_top * a.sin()));
    }
    let _ = w;
    pts
}

/// Generates the upper glossy LCD visor polygon (top half above the parting seam).
fn visor_polygon(xf: &Xf, inset: f32) -> Vec<Pos2> {
    let x0 = BX + inset;
    let x1 = BX + BW - inset;
    let y0 = BY + inset;
    let y_seam = SEAM_Y;
    let r_top = (50.0 - inset).max(10.0);
    let seg = 10usize;
    let mut pts = Vec::with_capacity(2 * (seg + 1) + 2);

    let c_tr = Pos2::new(x1 - r_top, y0 + r_top);
    for i in 0..=seg {
        let a = (-90.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tr.x + r_top * a.cos(), c_tr.y + r_top * a.sin()));
    }
    pts.push(xf.p(x1, y_seam));
    pts.push(xf.p(x0, y_seam));
    let c_tl = Pos2::new(x0 + r_top, y0 + r_top);
    for i in 0..=seg {
        let a = (180.0 + 90.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tl.x + r_top * a.cos(), c_tl.y + r_top * a.sin()));
    }
    pts
}

/// Draws the PC1832 / PK5500 keypad. Returns `true` when the user clicks the cover to toggle it.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    area: Rect,
    view: &View,
    input: &mut KeyInput,
    engine: &EngineHandle,
    audio: &AudioHandle,
    lights: f32,
    t: f64,
) -> bool {
    let anim = input.door_anim_5500.clamp(0.0, 1.0);
    let ease = anim * anim * (3.0 - 2.0 * anim);
    let mut xf = Xf::fit(area, SIZE, 16.0);
    // When cover is closed (up), gently center the body vertically on the wall.
    let closed_shift_y = (1.0 - ease) * 58.0;
    xf.origin.y += xf.s(closed_shift_y);

    let p = ui.painter().clone();
    let l = lights;

    // 1. Multi-layered soft wall shadow beneath the glossy shell
    let outer = shell_polygon(&xf, 0.0);
    paint::poly_shadow(&p, &outer, Vec2::new(xf.s(6.0), xf.s(11.0)), xf.s(20.0), Color32::from_black_alpha(150));
    paint::housing_depth(&p, &outer, Vec2::new(xf.s(1.8), xf.s(3.4)), Color32::from_rgb(0xD8, 0xDA, 0xD6), l);

    // Outer molded glossy pearl-white ABS shell
    paint::grad_poly_v(
        &p,
        &outer,
        xf.p(0.0, BY).y,
        xf.p(0.0, BY + BH).y,
        dim(Color32::from_rgb(0xF6, 0xF7, 0xF5), l),
        dim(Color32::from_rgb(0xD7, 0xDA, 0xD7), l),
        Some(Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0xAC, 0xB0, 0xAC), l))),
    );

    // Subtle recessed perimeter accent groove (signature PK5500 outer molding channel)
    let groove = shell_polygon(&xf, 5.0);
    p.add(Shape::closed_line(groove, Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0xC2, 0xC6, 0xC2), l))));
    let inner_rim = shell_polygon(&xf, 7.5);
    paint::grad_poly_v(
        &p,
        &inner_rim,
        xf.p(0.0, BY + 7.5).y,
        xf.p(0.0, BY + BH - 7.5).y,
        dim(Color32::from_rgb(0xF2, 0xF3, 0xF1), l),
        dim(Color32::from_rgb(0xE2, 0xE5, 0xE2), l),
        Some(Stroke::new(xf.s(0.8), alpha(Color32::WHITE, 0.72 * l + 0.08))),
    );

    // 2. Lower recessed keypad dish (behind the flip cover)
    let dish_rect = xf.r(BX + 14.0, SEAM_Y + 2.0, BW - 28.0, BH - 118.0);
    let dish_pts = paint::rounded_points_4(dish_rect, xf.s(4.0), xf.s(4.0), xf.s(36.0), xf.s(36.0), 8);
    paint::grad_poly_v(
        &p,
        &dish_pts,
        dish_rect.top(),
        dish_rect.bottom(),
        dim(Color32::from_rgb(0xDE, 0xE0, 0xDD), l),
        dim(Color32::from_rgb(0xEE, 0xF0, 0xED), l),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xBA, 0xBE, 0xBA), l))),
    );
    // Cast top inner shadow from the visor overhang onto the keypad dish
    let overhang_sh = Rect::from_min_size(xf.p(BX + 14.0, SEAM_Y + 1.0), Vec2::new(xf.s(BW - 28.0), xf.s(10.0)));
    paint::grad_rrect(
        &p,
        overhang_sh,
        0.0,
        Color32::from_rgba_unmultiplied(20, 24, 22, (65.0 * l + 22.0) as u8),
        Color32::TRANSPARENT,
        Some(Stroke::NONE),
    );

    // Side clasp ribs on left and right lower edges (visible in Photos 3 & 4)
    for left_side in [true, false] {
        let rx = if left_side { BX + 1.5 } else { BX + BW - 7.5 };
        let clasp = xf.r(rx, SEAM_Y + 26.0, 6.0, 66.0);
        paint::grad_rrect(
            &p,
            clasp,
            xf.s(3.0),
            dim(Color32::from_rgb(0xF4, 0xF5, 0xF3), l),
            dim(Color32::from_rgb(0xD2, 0xD6, 0xD2), l),
            Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0xB0, 0xB4, 0xB0), l))),
        );
    }

    // 3. Upper Glossy Translucent-Edge LCD Visor Hood
    let visor = visor_polygon(&xf, 6.0);
    paint::grad_poly_v(
        &p,
        &visor,
        xf.p(0.0, BY + 6.0).y,
        xf.p(0.0, SEAM_Y).y,
        dim(Color32::from_rgb(0xEB, 0xF0, 0xF4), l),
        dim(Color32::from_rgb(0xCD, 0xD8, 0xE0), l),
        Some(Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0xA6, 0xB4, 0xBE), l))),
    );
    // Glossy specular sheen band across upper visor curve
    let visor_gloss = visor_polygon(&xf, 10.0);
    let visor_clip = Rect::from_min_max(xf.p(BX + 10.0, BY + 8.0), xf.p(BX + BW - 10.0, BY + 42.0));
    paint::grad_poly_v(
        &p.with_clip_rect(visor_clip),
        &visor_gloss,
        xf.p(0.0, BY + 8.0).y,
        xf.p(0.0, BY + 42.0).y,
        alpha(Color32::WHITE, 0.26 * l),
        alpha(Color32::WHITE, 0.0),
        Some(Stroke::NONE),
    );
    // Crisp horizontal parting shelf below visor
    p.line_segment(
        [xf.p(BX + 6.0, SEAM_Y), xf.p(BX + BW - 6.0, SEAM_Y)],
        Stroke::new(xf.s(1.3), dim(Color32::from_rgb(0xA6, 0xAA, 0xA6), l)),
    );
    p.line_segment(
        [xf.p(BX + 8.0, SEAM_Y - 1.3), xf.p(BX + BW - 8.0, SEAM_Y - 1.3)],
        Stroke::new(xf.s(0.9), alpha(Color32::WHITE, 0.72 * l)),
    );

    // 4. Authentic Recessed Green Backlit 2x16 Dot-Matrix LCD
    let lcd_outer = xf.r(BX + 54.0, BY + 25.0, 260.0, 62.0);
    let bl = view.backlight.clamp(0.0, 1.0);
    // Ambient green light bleed onto the glossy white plastic around the LCD in dark room
    if bl > 0.05 && l < 0.96 {
        paint::glow(
            &p,
            lcd_outer.center(),
            lcd_outer.width() * 0.72,
            alpha(Color32::from_rgb(0x20, 0xDC, 0x26), 0.24 * bl * (1.0 - l)),
        );
    }
    // Recessed plastic aperture bevel (dark upper/left depth wall, bright lower/right rim)
    p.rect_filled(lcd_outer.expand(xf.s(2.4)), xf.s(7.5), dim(Color32::from_rgb(0xCE, 0xD2, 0xCE), l));
    p.rect_filled(
        lcd_outer.expand(xf.s(1.5)),
        xf.s(6.5),
        dim(Color32::from_rgb(0x6E, 0x74, 0x6F), l),
    );
    p.line_segment(
        [
            lcd_outer.left_bottom() + Vec2::new(xf.s(5.0), xf.s(2.0)),
            lcd_outer.right_bottom() + Vec2::new(-xf.s(5.0), xf.s(2.0)),
        ],
        Stroke::new(xf.s(1.2), alpha(Color32::WHITE, 0.75 * l)),
    );

    let glass = lcd_outer.shrink(xf.s(1.8));
    // Authentic PK5500 emerald/kelly-green LCD backlight color
    let unlit_top = dim(Color32::from_rgb(0x52, 0x6F, 0x40), l);
    let unlit_bot = dim(Color32::from_rgb(0x41, 0x5B, 0x32), l);
    let lit_top = Color32::from_rgb(0x16, 0xB8, 0x1A);
    let lit_bot = Color32::from_rgb(0x10, 0x9C, 0x14);
    let bg_top = mix(unlit_top, lit_top, bl * 0.94);
    let bg_bot = mix(unlit_bot, lit_bot, bl * 0.94);
    paint::grad_rrect(
        &p,
        glass,
        xf.s(4.5),
        bg_top,
        bg_bot,
        Some(Stroke::new(xf.s(1.1), Color32::from_rgb(0x0E, 0x38, 0x0E))),
    );
    // Subtle side-lit backlight luminance hot spots (like real edge-lit LCD LEDs)
    if bl > 0.05 {
        let gp = p.with_clip_rect(glass);
        paint::glow(
            &gp,
            Pos2::new(glass.left() + glass.width() * 0.16, glass.center().y),
            glass.width() * 0.45,
            alpha(Color32::from_rgb(0x5A, 0xF0, 0x32), 0.22 * bl),
        );
        paint::glow(
            &gp,
            Pos2::new(glass.right() - glass.width() * 0.16, glass.center().y),
            glass.width() * 0.45,
            alpha(Color32::from_rgb(0x5A, 0xF0, 0x32), 0.18 * bl),
        );
    }
    // Inner top recess shadow inside LCD glass
    let inner_sh = Rect::from_min_size(glass.min, Vec2::new(glass.width(), xf.s(5.5)));
    paint::grad_rrect(
        &p.with_clip_rect(glass),
        inner_sh,
        xf.s(3.0),
        Color32::from_rgba_unmultiplied(4, 24, 6, 90),
        Color32::TRANSPARENT,
        Some(Stroke::NONE),
    );
    // Draw the 2x16 dot-matrix pixels with optical polarizer shadow
    draw_lcd(&p, &xf, glass, &view.lcd, view.lcd_cursor, view.contrast, bl, l, t);
    paint::lcd_reflection(&p, glass, l);

    // 5. Four Right-Side Translucent Backlit Status Icons (no overflowing text, matching photos 3 and 4)
    let status_colors = [
        Color32::from_rgb(0x1E, 0xE6, 0x3E), // Ready (green)
        Color32::from_rgb(0xFF, 0x2E, 0x24), // Armed (red)
        Color32::from_rgb(0xFF, 0xB0, 0x16), // Trouble (amber)
        Color32::from_rgb(0x1E, 0xE6, 0x3E), // AC Power (green)
    ];
    let icon_x = BX + 336.0;
    for (i, color) in status_colors.iter().enumerate() {
        let y = BY + 33.0 + i as f32 * 15.6;
        let c = xf.p(icon_x, y);
        let lit = view.lcd_leds[i].lit(t);
        if lit {
            paint::glow(&p, c, xf.s(13.0), alpha(*color, 0.42 + 0.28 * (1.0 - l)));
            paint::glow(&p, c, xf.s(6.5), alpha(*color, 0.72));
        }
        let col = if lit {
            mix(*color, Color32::WHITE, 0.16)
        } else {
            dim(Color32::from_rgb(0xB6, 0xBB, 0xB8), l)
        };
        let sz = xf.s(9.5);
        match i {
            0 => paint::icon_check(&p, c, sz, col),
            1 => paint::icon_padlock(&p, c, sz, col),
            2 => paint::icon_warning(&p, c, sz, col),
            _ => paint::icon_ac_power(&p, c, sz, col),
        }
    }

    // 6. Authentic PK5500 Pill Silicone Rubber Keys (active when flip cover is open)
    let keys_active = ease > 0.52;
    let mut keys: Vec<(Key, Face, Rect)> = Vec::new();
    // Left group: < > scroll pills, then Fire, Aux, Panic wide pills
    keys.push((Key::Left, Face::Chevron(false), xf.r(BX + 36.0, SEAM_Y + 12.0, 29.0, 18.0)));
    keys.push((Key::Right, Face::Chevron(true), xf.r(BX + 71.0, SEAM_Y + 12.0, 29.0, 18.0)));
    keys.push((Key::Fire, Face::Fire, xf.r(BX + 36.0, SEAM_Y + 42.0, 64.0, 20.0)));
    keys.push((Key::Aux, Face::Aux, xf.r(BX + 36.0, SEAM_Y + 72.0, 64.0, 20.0)));
    keys.push((Key::Panic, Face::Panic, xf.r(BX + 36.0, SEAM_Y + 102.0, 64.0, 20.0)));

    // Center 3x4 numeric pill pad
    let nums = [["1", "2", "3"], ["4", "5", "6"], ["7", "8", "9"], ["*", "0", "#"]];
    for (row, line) in nums.iter().enumerate() {
        for (col, label) in line.iter().enumerate() {
            let key = match *label {
                "*" => Key::Star,
                "#" => Key::Hash,
                d => Key::Digit(d.parse().unwrap_or(0)),
            };
            let x = BX + 125.0 + col as f32 * 51.0;
            let y = SEAM_Y + 12.0 + row as f32 * 30.0;
            keys.push((key, Face::Text(*label), xf.r(x, y, 40.0, 21.0)));
        }
    }

    // Right group: 5 stacked function pill keys
    let fns = [
        (Key::Stay, "Stay"),
        (Key::Away, "Away"),
        (Key::Chime, "Chime"),
        (Key::Reset, "Reset"),
        (Key::Exit, "Exit"),
    ];
    for (i, (key, label)) in fns.iter().enumerate() {
        keys.push((*key, Face::Small(*label), xf.r(BX + 298.0, SEAM_Y + 11.0 + i as f32 * 23.0, 52.0, 17.5)));
    }

    for (key, face, rect) in keys {
        let held = if keys_active {
            input.key(ui, rect, key, engine, audio)
        } else {
            false
        };
        let hold = view.hold.filter(|h| h.0 == key).map(|h| h.1);
        draw_pill_key(&p, &xf, rect, face, held, l, bl, hold);
    }

    // 7. Molded bottom barrel hinge
    draw_hinge(&p, &xf, l);

    // 8. Working Bottom-Hinged Flip Cover (UP = Close, DOWN = Open)
    draw_flip_cover(ui, &p, &xf, ease, l)
}

fn draw_hinge(p: &Painter, xf: &Xf, l: f32) {
    let hinge = xf.r(BX + 78.0, HINGE_Y - 3.0, BW - 156.0, 8.0);
    paint::grad_rrect(
        p,
        hinge,
        xf.s(3.5),
        dim(Color32::from_rgb(0xF2, 0xF3, 0xF1), l),
        dim(Color32::from_rgb(0xC2, 0xC6, 0xC2), l),
        Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0x9E, 0xA2, 0x9E), l))),
    );
    // Hinge knuckle seams
    for frac in [0.16f32, 0.84] {
        let sx = hinge.left() + hinge.width() * frac;
        p.line_segment(
            [Pos2::new(sx, hinge.top() + xf.s(0.8)), Pos2::new(sx, hinge.bottom() - xf.s(0.8))],
            Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0x9A, 0x9E, 0x9A), l)),
        );
    }
}

/// Cool pale ice-blue-grey capsule rubber key (matches Photos 3 & 4).
#[allow(clippy::too_many_arguments)]
fn draw_pill_key(p: &Painter, xf: &Xf, rect: Rect, face: Face, held: bool, l: f32, bl: f32, hold: Option<f32>) {
    let r = rect.height() * 0.48;
    // Recessed cutout well in the white keypad dish
    p.rect_filled(rect.expand(xf.s(1.7)), r + xf.s(1.7), dim(Color32::from_rgb(0xB2, 0xB7, 0xB6), l));
    p.rect_filled(rect.expand(xf.s(0.9)), r + xf.s(0.9), dim(Color32::from_rgb(0x6E, 0x76, 0x78), l));

    // Soft green/cyan key backlight when room lights are dimmed
    let back = (1.0 - l).clamp(0.0, 1.0) * bl.max(0.25);
    if back > 0.05 {
        paint::glow(p, rect.center(), rect.width() * 0.58, alpha(Color32::from_rgb(0x62, 0xE8, 0x8A), 0.18 * back));
    }

    if !held {
        paint::shadow(
            p,
            rect.translate(Vec2::new(0.0, xf.s(1.6))),
            r,
            Vec2::ZERO,
            xf.s(3.0),
            Color32::from_black_alpha(75),
        );
    }
    let k = rect.translate(Vec2::new(0.0, if held { xf.s(1.2) } else { 0.0 }));

    // Pale cool ice-blue-grey rubber keycap gradient (like Photo 3)
    let (top, bot) = if held {
        (Color32::from_rgb(0xA4, 0xB6, 0xBC), Color32::from_rgb(0x8D, 0xA0, 0xA7))
    } else {
        (Color32::from_rgb(0xC6, 0xD6, 0xDC), Color32::from_rgb(0xA3, 0xB6, 0xBE))
    };
    let top_lit = mix(dim(top, l), Color32::from_rgb(0x4E, 0x9C, 0x62), back * 0.45);
    let bot_lit = mix(dim(bot, l), Color32::from_rgb(0x36, 0x78, 0x48), back * 0.45);
    paint::grad_rrect(
        p,
        k,
        r,
        top_lit,
        bot_lit,
        Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0x74, 0x82, 0x87), l))),
    );
    // Glossy convex upper sheen on silicone pill
    paint::gloss_band(p, k, r, if held { 0.06 * l } else { 0.17 * l });

    let glow_ink = Color32::from_rgb(0xD8, 0xFF, 0xD6);
    let ink = mix(dim(Color32::from_rgb(0x1E, 0x25, 0x29), l.max(0.55)), glow_ink, back * 0.92);
    let c = k.center();
    match face {
        Face::Text(s) => {
            let size = if s == "*" { 16.0 } else { 12.8 };
            let lift = if s == "*" { xf.s(2.0) } else { 0.0 };
            p.text(c + Vec2::new(0.0, lift), Align2::CENTER_CENTER, s, theme::semi(xf.s(size)), ink);
        }
        Face::Small(s) => {
            p.text(c, Align2::CENTER_CENTER, s, theme::semi(xf.s(8.8)), ink);
        }
        Face::Chevron(right) => {
            let d = if right { 1.0 } else { -1.0 };
            let pts = vec![
                c + Vec2::new(-2.8 * d, -4.0) * xf.scale,
                c + Vec2::new(2.8 * d, 0.0) * xf.scale,
                c + Vec2::new(-2.8 * d, 4.0) * xf.scale,
            ];
            p.add(Shape::line(pts, Stroke::new(xf.s(1.7), ink)));
        }
        Face::Fire => paint::icon_flame(p, c, xf.s(11.5), ink),
        Face::Aux => paint::icon_aux_triangle(p, c, xf.s(10.5), ink),
        Face::Panic => paint::icon_shield(p, c, xf.s(10.5), ink),
    }

    if let Some(h) = hold {
        let ring_r = (k.height() * 0.65).max(xf.s(14.0));
        paint::arc(p, c, ring_r, -FRAC_PI_2, -FRAC_PI_2 + TAU * h, Stroke::new(xf.s(2.2), theme::AMBER));
    }
}

/// Draws the animated bottom-hinged flip cover (UP = Close, DOWN = Open).
/// Returns true when clicked to toggle.
fn draw_flip_cover(ui: &mut egui::Ui, p: &Painter, xf: &Xf, ease: f32, l: f32) -> bool {
    let theta = ease * PI;
    let cos_t = theta.cos(); // +1 closed (UP over keys), -1 open (DOWN below hinge)
    let sin_t = theta.sin(); // +1 at 90 degrees

    let proj_h = COVER_H * cos_t.abs().max(0.05);
    let persp_x = 16.0 * sin_t;

    // Dynamic cast shadow onto the recessed keys while closing/opening
    if cos_t > -0.15 {
        let sh_h = (COVER_H * cos_t.max(0.0) + 20.0 * sin_t).min(COVER_H);
        let sh_rect = xf.r(BX + 18.0, HINGE_Y - sh_h, BW - 36.0, sh_h);
        p.rect_filled(sh_rect, xf.s(10.0), Color32::from_black_alpha((64.0 * (0.2 + 0.8 * sin_t)) as u8));
    }

    let x_left = BX + 12.0;
    let x_right = BX + BW - 12.0;
    let front_visible = cos_t >= 0.0;

    let hit_rect = if front_visible {
        // Flipping UP (closed over keypad bay, leaving LCD & status icons visible!)
        let y_free = HINGE_Y - proj_h;
        let cover_rect = Rect::from_min_max(xf.p(x_left - persp_x * 0.5, y_free), xf.p(x_right + persp_x * 0.5, HINGE_Y));
        let r_bot = xf.s(36.0 * cos_t.max(0.25));
        let pts = paint::rounded_points_4(cover_rect, xf.s(5.0), xf.s(5.0), r_bot, r_bot, 8);

        if sin_t > 0.05 {
            paint::poly_shadow(p, &pts, Vec2::new(xf.s(3.0), xf.s(8.0 * sin_t)), xf.s(12.0), Color32::from_black_alpha(110));
        }

        let light_k = (0.76 + 0.24 * cos_t).clamp(0.48, 1.0);
        let top_c = dim(mix(Color32::from_rgb(0xCE, 0xD2, 0xCE), Color32::from_rgb(0xF8, 0xF9, 0xF7), light_k), l);
        let bot_c = dim(mix(Color32::from_rgb(0xB8, 0xBC, 0xB8), Color32::from_rgb(0xDD, 0xE0, 0xDD), light_k), l);
        paint::grad_poly_v(
            p,
            &pts,
            cover_rect.top(),
            cover_rect.bottom(),
            top_c,
            bot_c,
            Some(Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0xAE, 0xB2, 0xAE), l))),
        );

        // Glossy curved plastic sheen on the closed unbranded cover
        let sheen = Rect::from_min_max(
            cover_rect.min + Vec2::new(xf.s(14.0), xf.s(3.0)),
            Pos2::new(cover_rect.right() - xf.s(14.0), cover_rect.top() + cover_rect.height() * 0.42),
        );
        paint::gloss_band(p, sheen, xf.s(4.0), 0.46 * l * light_k);

        // Subtle curved finger-pull recess ridge at the top edge of the closed cover
        if cos_t > 0.35 {
            let grip = Rect::from_center_size(
                Pos2::new(cover_rect.center().x, cover_rect.top() + xf.s(9.0 * cos_t)),
                Vec2::new(xf.s(64.0), xf.s(4.5 * cos_t)),
            );
            paint::grad_rrect(
                p,
                grip,
                xf.s(2.2),
                dim(Color32::from_rgb(0xC6, 0xCA, 0xC6), l),
                dim(Color32::from_rgb(0xF6, 0xF7, 0xF5), l),
                Some(Stroke::new(xf.s(0.6), dim(Color32::from_rgb(0xB4, 0xB8, 0xB4), l))),
            );
        }

        // Subtle "Pull down to open" cue near bottom-center when fully closed (NO DSC branding!)
        if cos_t > 0.92 {
            let hint_c = alpha(dim(Color32::from_rgb(0x82, 0x88, 0x86), l), 0.88);
            paint::tiny_tri(
                p,
                Pos2::new(cover_rect.center().x - xf.s(38.0), cover_rect.bottom() - xf.s(14.0)),
                xf.s(5.5),
                1,
                hint_c,
            );
            p.text(
                Pos2::new(cover_rect.center().x + xf.s(4.0), cover_rect.bottom() - xf.s(14.0)),
                Align2::CENTER_CENTER,
                "Pull down to open",
                theme::regular(xf.s(8.2)),
                hint_c,
            );
        }
        cover_rect
    } else {
        // Flipping DOWN (open below the hinge, revealing the printed quick-reference card, Photos 3 & 4!)
        let y_free = HINGE_Y + proj_h;
        let open_k = (-cos_t).clamp(0.0, 1.0);
        // Slightly tapered trapezoid matching the real open flip door contour in Photos 3 & 4
        let top_inset = 22.0;
        let bot_inset = 6.0 - persp_x * 0.4;
        let door_pts = build_open_door_poly(xf, x_left + top_inset, x_right - top_inset, x_left + bot_inset, x_right - bot_inset, HINGE_Y + 2.0, y_free, 32.0 * open_k.max(0.25));

        // Realistic shadow cast onto the wall beneath the open cover
        paint::poly_shadow(p, &door_pts, Vec2::new(xf.s(5.0), xf.s(9.0)), xf.s(14.0), Color32::from_black_alpha(125));

        let light_k = (0.72 + 0.28 * open_k).clamp(0.45, 1.0);
        let top_c = dim(mix(Color32::from_rgb(0xC9, 0xCC, 0xC9), Color32::from_rgb(0xF1, 0xF2, 0xEF), light_k), l);
        let bot_c = dim(mix(Color32::from_rgb(0xB6, 0xBA, 0xB6), Color32::from_rgb(0xDF, 0xE2, 0xDF), light_k), l);
        paint::grad_poly_v(
            p,
            &door_pts,
            xf.p(0.0, HINGE_Y + 2.0).y,
            xf.p(0.0, y_free).y,
            top_c,
            bot_c,
            Some(Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0xAC, 0xB0, 0xAC), l))),
        );

        // Two small molded latch tabs on bottom left/right edges (just like Photos 3 & 4)
        if open_k > 0.45 {
            for lx in [x_left + bot_inset - 3.5, x_right - bot_inset - 1.0] {
                let tab = xf.r(lx, y_free - 30.0 * open_k, 4.5, 22.0 * open_k);
                paint::grad_rrect(
                    p,
                    tab,
                    xf.s(1.5),
                    top_c,
                    bot_c,
                    Some(Stroke::new(xf.s(0.7), dim(Color32::from_rgb(0xA6, 0xAA, 0xA6), l))),
                );
            }
        }

        // Recessed printed instruction label card inside the open cover
        if open_k > 0.35 {
            let card_rect = Rect::from_min_max(
                xf.p(x_left + 32.0, HINGE_Y + 10.0 * open_k),
                xf.p(x_right - 32.0, y_free - 8.0 * open_k),
            );
            paint::grad_rrect(
                p,
                card_rect,
                xf.s(8.0 * open_k),
                dim(Color32::from_rgb(0xF6, 0xF6, 0xF3), l),
                dim(Color32::from_rgb(0xEA, 0xEB, 0xE7), l),
                Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0xC6, 0xCA, 0xC6), l))),
            );
            // Fade in printed instructions smoothly once mostly open
            let text_a = ((open_k - 0.62) / 0.38).clamp(0.0, 1.0);
            if text_a > 0.05 {
                draw_door_instructions(p, xf, card_rect, l, text_a);
            }
        }

        Rect::from_min_max(xf.p(x_left + bot_inset, HINGE_Y + 2.0), xf.p(x_right - bot_inset, y_free))
    };

    let tip = if front_visible {
        "Click cover to flip open (down)"
    } else {
        "Click cover to flip closed (up)"
    };
    let resp = ui
        .interact(hit_rect, egui::Id::new("pc1832-cover"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    resp.clicked()
}

#[allow(clippy::too_many_arguments)]
fn build_open_door_poly(xf: &Xf, tx0: f32, tx1: f32, bx0: f32, bx1: f32, y0: f32, y1: f32, r_top: f32) -> Vec<Pos2> {
    let h = (y1 - y0).max(2.0);
    let r = r_top.min(h * 0.45).min((tx1 - tx0) * 0.25).max(1.0);
    let seg = 6usize;
    let mut pts = Vec::with_capacity(2 * (seg + 1) + 2);
    // Top-right rounded corner (near hinge)
    let c_tr = Pos2::new(tx1 - r, y0 + r);
    for i in 0..=seg {
        let a = (-90.0 + 75.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tr.x + r * a.cos(), c_tr.y + r * a.sin()));
    }
    // Slanted right edge to bottom-right
    pts.push(xf.p(bx1, y1 - 3.0));
    pts.push(xf.p(bx1 - 3.0, y1));
    // Bottom edge to bottom-left
    pts.push(xf.p(bx0 + 3.0, y1));
    pts.push(xf.p(bx0, y1 - 3.0));
    // Top-left rounded corner (near hinge)
    let c_tl = Pos2::new(tx0 + r, y0 + r);
    for i in 0..=seg {
        let a = (195.0 + 75.0 * i as f32 / seg as f32).to_radians();
        pts.push(xf.p(c_tl.x + r * a.cos(), c_tl.y + r * a.sin()));
    }
    pts
}

/// Renders the authentic quick-reference instruction card inside the open PK5500 flip cover (zero DSC branding).
fn draw_door_instructions(p: &Painter, xf: &Xf, card: Rect, l: f32, text_a: f32) {
    let blue = alpha(dim(Color32::from_rgb(0x28, 0x4E, 0x86), l.max(0.55)), text_a);
    let ink = alpha(dim(Color32::from_rgb(0x2B, 0x46, 0x70), l.max(0.55)), text_a);

    let lx = card.left() + xf.s(12.0);
    let mut y = card.top() + xf.s(8.0);
    p.text(Pos2::new(lx, y), Align2::LEFT_TOP, "Message Display Security Station", theme::semi(xf.s(8.2)), blue);
    y += xf.s(14.5);

    let rows = [
        ("*", "Press to select function"),
        ("#", "Press to exit / return"),
        ("<>", "More info available; press either key"),
        (">", "Advance to next function or message"),
        ("<", "Return to previous function or message"),
    ];
    for (k, desc) in rows {
        p.text(Pos2::new(lx + xf.s(4.0), y), Align2::CENTER_TOP, k, theme::semi(xf.s(7.2)), blue);
        p.text(Pos2::new(lx + xf.s(16.0), y), Align2::LEFT_TOP, desc, theme::regular(xf.s(7.0)), ink);
        y += xf.s(13.2);
    }

    p.text(
        Pos2::new(lx, card.bottom() - xf.s(9.0)),
        Align2::LEFT_BOTTOM,
        "TEST SYSTEM WEEKLY",
        theme::semi(xf.s(7.2)),
        blue,
    );

    // Right column: 4 status indicator legend items
    let rx = card.right() - xf.s(94.0);
    let mut ry = card.top() + xf.s(19.0);
    let legend = ["Ready", "Armed", "Trouble", "AC Power"];
    for (i, label) in legend.iter().enumerate() {
        let ic = Pos2::new(rx + xf.s(6.0), ry + xf.s(4.5));
        let sz = xf.s(7.8);
        match i {
            0 => paint::icon_check(p, ic, sz, blue),
            1 => paint::icon_padlock(p, ic, sz, blue),
            2 => paint::icon_warning(p, ic, sz, blue),
            _ => paint::icon_ac_power(p, ic, sz, blue),
        }
        p.text(Pos2::new(rx + xf.s(16.0), ry), Align2::LEFT_TOP, *label, theme::regular(xf.s(7.2)), ink);
        ry += xf.s(13.8);
    }

    // Bottom-right: 3 mini emergency symbol boxes (matching Photo 4)
    let box_y = card.bottom() - xf.s(26.0);
    let box_sz = Vec2::new(xf.s(20.0), xf.s(16.0));
    let fire_col = alpha(dim(Color32::from_rgb(0xC2, 0x36, 0x2B), l.max(0.55)), text_a);
    let aux_col = alpha(dim(Color32::from_rgb(0xB8, 0x5C, 0x1E), l.max(0.55)), text_a);
    let panic_col = alpha(dim(Color32::from_rgb(0x2C, 0x34, 0x40), l.max(0.55)), text_a);
    let border = alpha(dim(Color32::from_rgb(0xB2, 0xB8, 0xC2), l), text_a);
    for i in 0..3 {
        let bx = rx + i as f32 * xf.s(23.5);
        let r = Rect::from_min_size(Pos2::new(bx, box_y), box_sz);
        p.rect_stroke(r, xf.s(2.0), Stroke::new(xf.s(0.8), border));
        let c = r.center();
        let sz = xf.s(9.5);
        match i {
            0 => paint::icon_flame(p, c, sz, fire_col),
            1 => paint::icon_aux_triangle(p, c, sz, aux_col),
            _ => paint::icon_shield(p, c, sz, panic_col),
        }
    }

    // Subtle close cue at top-right of inside card
    let close_c = alpha(ink, 0.72);
    paint::tiny_tri(
        p,
        Pos2::new(card.right() - xf.s(31.0), card.top() + xf.s(11.0)),
        xf.s(5.0),
        0,
        close_c,
    );
    p.text(
        Pos2::new(card.right() - xf.s(8.0), card.top() + xf.s(7.0)),
        Align2::RIGHT_TOP,
        "Close",
        theme::regular(xf.s(7.0)),
        close_c,
    );
}

/// The 2x16 dot-matrix LCD with authentic optical polarizer drop-shadow behind
/// every lit pixel and subtle reflective ghost grid where dots are off.
#[allow(clippy::too_many_arguments)]
fn draw_lcd(
    p: &Painter,
    xf: &Xf,
    glass: Rect,
    lines: &[String; 2],
    cursor: Option<(usize, usize)>,
    contrast: f32,
    bl: f32,
    lights: f32,
    t: f64,
) {
    let cols = 16usize;
    let pad_x = xf.s(9.0);
    let pitch = (glass.width() - pad_x * 2.0) / (cols as f32 * 6.0 - 1.0);
    let dot = pitch * 0.84;
    let total_h = 18.5 * pitch;
    let x0 = glass.left() + pad_x;
    let y0 = glass.center().y - total_h * 0.5;
    let k = contrast.clamp(0.0, 1.0);

    // Liquid crystal dot colors
    let vis = (lights * 0.7 + bl * 0.9).clamp(0.25, 1.0);
    let on = mix(Color32::from_rgb(0x22, 0x4A, 0x1A), Color32::from_rgb(0x06, 0x1A, 0x07), (0.55 + 0.45 * k) * vis);
    let shadow_col = Color32::from_rgba_unmultiplied(6, 36, 8, (48.0 * vis) as u8);
    let ghost = Color32::from_rgba_unmultiplied(10, 44, 10, (12.0 + 10.0 * k) as u8);
    let sh_off = Vec2::new(pitch * 0.34, pitch * 0.38);

    let blink = (t * 2.0).fract() < 0.5;
    for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for col in 0..cols {
            let ch = chars.get(col).copied().unwrap_or(' ');
            let cx = x0 + (col * 6) as f32 * pitch;
            let cy = y0 + row as f32 * 10.5 * pitch;
            let show_cursor = blink && cursor == Some((row, col));
            for gx in 0..5 {
                for gy in 0..8 {
                    let lit = lcd_font::dot(ch, gx, gy) || (gy == 7 && show_cursor);
                    let pos = Pos2::new(cx + gx as f32 * pitch, cy + gy as f32 * pitch);
                    let r = Rect::from_min_size(pos, Vec2::splat(dot));
                    if lit {
                        // Polarizer reflector depth shadow behind glass
                        p.rect_filled(r.translate(sh_off), dot * 0.14, shadow_col);
                        p.rect_filled(r, dot * 0.14, on);
                    } else {
                        p.rect_filled(r, dot * 0.14, ghost);
                    }
                }
            }
        }
    }
}
