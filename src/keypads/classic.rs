//! Classic 1990s LED keypad (PC1550 / PC1500RK form factor), branded RmG:
//! glossy off-white molded plastic housing with 45-degree chamfered corner,
//! vertical strip of 11 recessed square LEDs, 12 round dark-grey rubber keys,
//! 3 tall pill emergency keys (F, A, P), and a working left-hinged ribbed cover
//! that swings LEFT to open and RIGHT to close.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, Stroke, Vec2};

use super::{KeyInput, Xf};
use crate::audio::AudioHandle;
use crate::engine::types::Key;
use crate::engine::{EngineHandle, View};
use crate::paint::{self, alpha, dim, mix};
use crate::theme;

/// Design space accommodating the keypad body + left-swinging open door.
pub const SIZE: Vec2 = Vec2::new(476.0, 276.0);

// Keypad body coordinates inside SIZE (door swings left into x = 10..164).
const BX: f32 = 166.0;
const BY: f32 = 12.0;
const KP_W: f32 = 176.0;
const KP_H: f32 = 240.0;
const LED_X: f32 = BX + KP_W; // 342.0
const LED_W: f32 = 124.0;
const LED_H: f32 = 250.0;

const LED_GREEN: Color32 = Color32::from_rgb(0x2E, 0xE6, 0x5E);
const LED_RED: Color32 = Color32::from_rgb(0xFF, 0x33, 0x24);
const LED_AMBER: Color32 = Color32::from_rgb(0xFF, 0xA8, 0x1E);

/// Draws the PC1550 keypad. Returns `true` when the user clicks the cover to toggle it.
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
    let anim = input.door_anim_1550.clamp(0.0, 1.0);
    // Ease smoothly so the closed unit sits centered on the wall and pans gently as the door opens left.
    let ease = anim * anim * (3.0 - 2.0 * anim);
    let mut xf = Xf::fit(area, SIZE, 18.0);
    let closed_shift = (1.0 - ease) * (-76.0);
    xf.origin.x += xf.s(closed_shift);

    let p = ui.painter().clone();
    let l = lights;

    // 1. Main housing silhouette (stepped keypad bay + chamfered right LED housing)
    let outer_pts = vec![
        xf.p(BX, BY + 2.0),
        xf.p(LED_X + LED_W - 2.0, BY),
        xf.p(LED_X + LED_W, BY + 2.0),
        xf.p(LED_X + LED_W, BY + LED_H - 46.0),
        xf.p(LED_X + LED_W - 46.0, BY + LED_H),
        xf.p(LED_X + 18.0, BY + LED_H),
        xf.p(LED_X, BY + KP_H + 2.0),
        xf.p(BX, BY + KP_H + 2.0),
    ];
    paint::poly_shadow(&p, &outer_pts, Vec2::new(xf.s(6.0), xf.s(9.0)), xf.s(18.0), Color32::from_black_alpha(150));

    // Left keypad recessed chassis base
    let kp_body = xf.r(BX, BY + 2.0, KP_W + 4.0, KP_H);
    paint::grad_rrect(
        &p,
        kp_body,
        xf.s(2.5),
        dim(Color32::from_rgb(0xEE, 0xEC, 0xE4), l),
        dim(Color32::from_rgb(0xD8, 0xD4, 0xC8), l),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xA6, 0xA2, 0x96), l))),
    );
    // Subtle glossy inner faceplate
    let kp_face = xf.r(BX + 5.0, BY + 7.0, KP_W - 7.0, KP_H - 10.0);
    paint::grad_rrect(
        &p,
        kp_face,
        xf.s(1.5),
        dim(Color32::from_rgb(0xE8, 0xE5, 0xDC), l),
        dim(Color32::from_rgb(0xD4, 0xD0, 0xC4), l),
        Some(Stroke::new(xf.s(0.7), dim(Color32::from_rgb(0xB8, 0xB3, 0xA6), l))),
    );
    // Subtle specular gloss highlight along upper left faceplate
    paint::gloss_band(&p, kp_face, xf.s(2.0), 0.24 * l + 0.03);

    // Molded left-edge hinge latch slots (visible when door is open, like the real PC1500RK)
    for ly in [BY + 68.0, BY + 190.0] {
        let slot = xf.r(BX + 3.0, ly, 4.5, 16.0);
        p.rect_filled(slot, xf.s(1.0), dim(Color32::from_rgb(0x9A, 0x95, 0x89), l));
        p.rect_stroke(slot, xf.s(1.0), Stroke::new(xf.s(0.6), dim(Color32::from_rgb(0x76, 0x72, 0x68), l)));
    }
    // Top and bottom hinge knuckles on left edge of body
    for hy in [BY + 2.0, BY + KP_H - 12.0] {
        let knuckle = xf.r(BX - 3.5, hy, 8.0, 12.0);
        paint::grad_rrect(
            &p,
            knuckle,
            xf.s(2.0),
            dim(Color32::from_rgb(0xF2, 0xF0, 0xE8), l),
            dim(Color32::from_rgb(0xC9, 0xC5, 0xB9), l),
            Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0x9E, 0x9A, 0x8F), l))),
        );
    }

    // 2. Keys (interactive only when cover is open enough to reach them)
    let keys_active = ease > 0.52;
    let num_rows: [[(Key, &str); 3]; 4] = [
        [(Key::Digit(1), "1"), (Key::Digit(2), "2"), (Key::Digit(3), "3")],
        [(Key::Digit(4), "4"), (Key::Digit(5), "5"), (Key::Digit(6), "6")],
        [(Key::Digit(7), "7"), (Key::Digit(8), "8"), (Key::Digit(9), "9")],
        [(Key::Star, "*"), (Key::Digit(0), "0"), (Key::Hash, "#")],
    ];
    for (row, line) in num_rows.iter().enumerate() {
        for (col, (key, label)) in line.iter().enumerate() {
            let cx = BX + 37.0 + col as f32 * 51.0;
            let cy = BY + 33.0 + row as f32 * 37.0;
            let r = 14.8;
            let hit = xf.r(cx - r, cy - r, r * 2.0, r * 2.0);
            let held = if keys_active {
                input.key(ui, hit, *key, engine, audio)
            } else {
                false
            };
            draw_round_key(&p, &xf, xf.p(cx, cy), xf.s(r), label, held, l);
        }
    }

    // Recessed rectangular divider frame for F, A, P emergency keys
    let fap_box = xf.r(BX + 14.0, BY + 174.0, KP_W - 26.0, 58.0);
    p.rect_stroke(fap_box, xf.s(2.0), Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0xB4, 0xAF, 0xA2), l)));
    p.line_segment(
        [fap_box.left_top() + Vec2::new(xf.s(1.0), xf.s(1.0)), fap_box.right_top() + Vec2::new(-xf.s(1.0), xf.s(1.0))],
        Stroke::new(xf.s(0.7), alpha(Color32::WHITE, 0.45 * l)),
    );
    let emg = [(Key::Fire, "F"), (Key::Aux, "A"), (Key::Panic, "P")];
    for (col, (key, label)) in emg.iter().enumerate() {
        let cx = BX + 37.0 + col as f32 * 51.0;
        let cy = BY + 203.0;
        let rect = xf.r(cx - 13.5, cy - 21.0, 27.0, 42.0);
        let held = if keys_active {
            input.key(ui, rect, *key, engine, audio)
        } else {
            false
        };
        let hold = view.hold.filter(|h| h.0 == *key).map(|h| h.1);
        draw_pill_emergency_key(&p, &xf, rect, label, held, l, hold);
    }

    // 3. Right LED housing module (raised glossy plastic with chamfered corner & diagonal accent grooves)
    draw_led_module(&p, &xf, view, l, t);

    // 4. Hinged working cover: swings LEFT (open, anim=1) and RIGHT (close, anim=0)
    draw_side_cover(ui, &p, &xf, ease, l)
}

fn draw_led_module(p: &Painter, xf: &Xf, view: &View, l: f32, t: f64) {
    // Raised bevelled right housing polygon
    let body_pts = vec![
        xf.p(LED_X, BY + 2.0),
        xf.p(LED_X + 10.0, BY),
        xf.p(LED_X + LED_W - 2.0, BY),
        xf.p(LED_X + LED_W, BY + 2.0),
        xf.p(LED_X + LED_W, BY + LED_H - 46.0),
        xf.p(LED_X + LED_W - 46.0, BY + LED_H),
        xf.p(LED_X + 16.0, BY + LED_H),
        xf.p(LED_X, BY + KP_H + 2.0),
    ];
    // Cast subtle shadow onto the right edge of the keypad bay
    p.rect_filled(
        xf.r(LED_X - 4.0, BY + 4.0, 5.0, KP_H - 4.0),
        0.0,
        Color32::from_rgba_unmultiplied(20, 18, 16, (50.0 * l + 25.0) as u8),
    );
    paint::grad_poly_v(
        p,
        &body_pts,
        xf.p(0.0, BY).y,
        xf.p(0.0, BY + LED_H).y,
        dim(Color32::from_rgb(0xF4, 0xF2, 0xEB), l),
        dim(Color32::from_rgb(0xDC, 0xD8, 0xCD), l),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xA7, 0xA3, 0x97), l))),
    );
    // Vertical glossy contour ridge along the angled step-up on the left side of LED module
    let ridge_pts = vec![
        xf.p(LED_X + 1.0, BY + 2.0),
        xf.p(LED_X + 13.0, BY + 1.0),
        xf.p(LED_X + 13.0, BY + LED_H - 2.0),
        xf.p(LED_X + 1.0, BY + KP_H + 1.0),
    ];
    paint::grad_poly_h(
        p,
        &ridge_pts,
        xf.p(LED_X, 0.0).x,
        xf.p(LED_X + 13.0, 0.0).x,
        dim(Color32::from_rgb(0xCF, 0xCB, 0xBF), l),
        dim(Color32::from_rgb(0xF6, 0xF5, 0xEF), l),
        None,
    );

    // Glossy inner LED faceplate (chamfered bezel)
    let fx0 = LED_X + 18.0;
    let fy0 = BY + 9.0;
    let fx1 = LED_X + LED_W - 11.0;
    let fy1 = BY + LED_H - 16.0;
    let ch = 41.0;
    let face_pts = vec![
        xf.p(fx0, fy0),
        xf.p(fx1, fy0),
        xf.p(fx1, fy1 - ch),
        xf.p(fx1 - ch, fy1),
        xf.p(fx0, fy1),
    ];
    paint::grad_poly_v(
        p,
        &face_pts,
        xf.p(0.0, fy0).y,
        xf.p(0.0, fy1).y,
        dim(Color32::from_rgb(0xEB, 0xEA, 0xE5), l),
        dim(Color32::from_rgb(0xDC, 0xDA, 0xD3), l),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0x6D, 0x6B, 0x66), l))),
    );
    // Top specular reflection on glossy faceplate
    let gloss_pts = vec![
        xf.p(fx0 + 1.5, fy0 + 1.5),
        xf.p(fx1 - 1.5, fy0 + 1.5),
        xf.p(fx1 - 1.5, fy0 + 70.0),
        xf.p(fx0 + 1.5, fy0 + 95.0),
    ];
    paint::grad_poly_v(
        p,
        &gloss_pts,
        xf.p(0.0, fy0).y,
        xf.p(0.0, fy0 + 95.0).y,
        alpha(Color32::WHITE, 0.28 * l + 0.04),
        alpha(Color32::WHITE, 0.0),
        Some(Stroke::NONE),
    );

    // Two signature diagonal molded grooves below the bottom-right chamfer
    let groove_dark = dim(Color32::from_rgb(0xB4, 0xB0, 0xA4), l);
    let groove_hi = alpha(Color32::WHITE, 0.55 * l);
    for off in [5.5f32, 13.5] {
        let a = xf.p(fx1 + off, fy1 - ch - 4.0 + off);
        let b = xf.p(fx1 - ch - 4.0 + off, fy1 + off);
        p.line_segment([a, b], Stroke::new(xf.s(1.2), groove_dark));
        p.line_segment([a + Vec2::splat(xf.s(1.1)), b + Vec2::splat(xf.s(1.1))], Stroke::new(xf.s(0.9), groove_hi));
    }

    // 11 Vertical Square Recessed LEDs + Printed Labels
    let ink = dim(Color32::from_rgb(0x28, 0x27, 0x25), l.max(0.45));
    let led_x = fx0 + 14.5;
    let label_x = fx0 + 25.0;
    let led_sz = Vec2::splat(xf.s(6.8));

    let status_items = [
        ("Ready", LED_GREEN),
        ("Armed", LED_RED),
        ("Memory", LED_AMBER),
        ("Bypass", LED_AMBER),
        ("Trouble", LED_AMBER),
    ];
    for (i, (name, color)) in status_items.iter().enumerate() {
        let y = fy0 + 17.5 + i as f32 * 15.0;
        let lit = view.leds[i].lit(t);
        paint::square_led(p, xf.p(led_x, y), led_sz, *color, lit, l);
        p.text(xf.p(label_x, y), Align2::LEFT_CENTER, *name, theme::semi(xf.s(9.2)), ink);
    }

    for z in 0..6usize {
        let y = fy0 + 104.0 + z as f32 * 14.8;
        let lit = view.leds[5 + z].lit(t);
        paint::square_led(p, xf.p(led_x, y), led_sz, LED_RED, lit, l);
        p.text(xf.p(label_x, y), Align2::LEFT_CENTER, format!("Zone {}", z + 1), theme::semi(xf.s(9.2)), ink);
    }

    // Retro blocky "RmG" branding at the bottom of the LED strip (replacing DSC)
    draw_rmg_logo(p, xf, Pos2::new(fx0 + 8.0, fy1 - 24.0), l);
}

/// Retro 1990s bold industrial "RmG" wordmark with subtle horizontal stripe cut.
fn draw_rmg_logo(p: &Painter, xf: &Xf, pos: Pos2, l: f32) {
    let col = dim(Color32::from_rgb(0x28, 0x27, 0x25), l.max(0.45));
    let cut = dim(Color32::from_rgb(0xDF, 0xDD, 0xD6), l);
    let anchor = xf.p(pos.x, pos.y);
    // Extra-bold geometric industrial stamp
    p.text(anchor + Vec2::new(xf.s(0.6), 0.0), Align2::LEFT_TOP, "RmG", theme::semi(xf.s(17.0)), col);
    p.text(anchor + Vec2::new(xf.s(1.2), 0.0), Align2::LEFT_TOP, "RmG", theme::semi(xf.s(17.0)), col);
    // Classic 90s split-stripe accent across the middle of the logo
    let y_mid = anchor.y + xf.s(9.3);
    p.line_segment(
        [Pos2::new(anchor.x, y_mid), Pos2::new(anchor.x + xf.s(46.0), y_mid)],
        Stroke::new(xf.s(1.1), cut),
    );
}

/// Round domed grey rubber numeric key (matches Photo 1).
fn draw_round_key(p: &Painter, xf: &Xf, center: Pos2, r: f32, label: &str, held: bool, l: f32) {
    // Dark recessed circular well in the white plastic faceplate
    p.circle_filled(center, r + xf.s(1.9), dim(Color32::from_rgb(0xB4, 0xAF, 0xA2), l));
    p.circle_filled(center + Vec2::new(0.0, xf.s(0.6)), r + xf.s(1.3), dim(Color32::from_rgb(0x3B, 0x38, 0x33), l));

    // Amber backlight halo in dark room
    let back = (1.0 - l).clamp(0.0, 1.0);
    if back > 0.05 {
        paint::glow(p, center, r * 1.65, alpha(Color32::from_rgb(0xFF, 0xA8, 0x38), 0.24 * back));
    }

    // Unpressed key drop shadow
    if !held {
        p.circle_filled(center + Vec2::new(xf.s(0.6), xf.s(2.0)), r, Color32::from_black_alpha(95));
    }

    let c = center + Vec2::new(0.0, if held { xf.s(1.3) } else { 0.0 });
    let (top, bot) = if held {
        (Color32::from_rgb(0x6C, 0x6B, 0x64), Color32::from_rgb(0x52, 0x51, 0x4B))
    } else {
        (Color32::from_rgb(0x88, 0x86, 0x7E), Color32::from_rgb(0x63, 0x62, 0x5B))
    };
    paint::grad_circle(
        p,
        c,
        r,
        dim(top, l),
        dim(bot, l),
        Some(Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0x3E, 0x3D, 0x38), l))),
    );
    // Curved upper rubber sheen
    paint::grad_circle(
        p,
        c - Vec2::new(0.0, r * 0.24),
        r * 0.62,
        alpha(Color32::WHITE, if held { 0.08 * l } else { 0.18 * l + 0.02 }),
        alpha(Color32::WHITE, 0.0),
        Some(Stroke::NONE),
    );

    let ink_day = Color32::from_rgb(0x16, 0x15, 0x13);
    let ink = mix(dim(ink_day, l.max(0.55)), Color32::from_rgb(0xFF, 0xC4, 0x66), back * 0.9);
    let size = if label == "*" { 19.0 } else { 14.5 };
    let lift = if label == "*" { xf.s(2.4) } else { 0.0 };
    p.text(c + Vec2::new(0.0, lift), Align2::CENTER_CENTER, label, theme::semi(xf.s(size)), ink);
}

/// Tall vertical stadium-pill grey rubber emergency key (F, A, P).
fn draw_pill_emergency_key(p: &Painter, xf: &Xf, rect: Rect, label: &str, held: bool, l: f32, hold: Option<f32>) {
    let r = rect.width() * 0.5;
    // Recessed oval well
    p.rect_filled(rect.expand(xf.s(1.8)), r + xf.s(1.8), dim(Color32::from_rgb(0xB4, 0xAF, 0xA2), l));
    p.rect_filled(rect.expand(xf.s(1.1)), r + xf.s(1.1), dim(Color32::from_rgb(0x3B, 0x38, 0x33), l));

    let back = (1.0 - l).clamp(0.0, 1.0);
    if back > 0.05 {
        paint::glow(p, rect.center(), rect.height() * 0.85, alpha(Color32::from_rgb(0xFF, 0x98, 0x30), 0.22 * back));
    }
    if !held {
        paint::shadow(p, rect.translate(Vec2::new(xf.s(0.5), xf.s(2.0))), r, Vec2::ZERO, xf.s(3.5), Color32::from_black_alpha(95));
    }
    let k = rect.translate(Vec2::new(0.0, if held { xf.s(1.3) } else { 0.0 }));
    let (top, bot) = if held {
        (Color32::from_rgb(0x6E, 0x6D, 0x66), Color32::from_rgb(0x54, 0x53, 0x4D))
    } else {
        (Color32::from_rgb(0x8A, 0x88, 0x80), Color32::from_rgb(0x65, 0x64, 0x5D))
    };
    paint::grad_rrect(
        p,
        k,
        r,
        dim(top, l),
        dim(bot, l),
        Some(Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0x3E, 0x3D, 0x38), l))),
    );
    paint::gloss_band(p, k, r, if held { 0.08 * l } else { 0.18 * l + 0.02 });

    let ink = mix(dim(Color32::from_rgb(0x16, 0x15, 0x13), l.max(0.55)), Color32::from_rgb(0xFF, 0xBE, 0x58), back * 0.9);
    p.text(k.center(), Align2::CENTER_CENTER, label, theme::semi(xf.s(14.5)), ink);

    if let Some(h) = hold {
        paint::arc(p, k.center(), k.height() * 0.56, -FRAC_PI_2, -FRAC_PI_2 + TAU * h, Stroke::new(xf.s(2.4), theme::AMBER));
    }
}

/// Draws the animated left-hinged cover (Left = Open, Right = Close).
/// Returns true when clicked to toggle.
fn draw_side_cover(ui: &mut egui::Ui, p: &Painter, xf: &Xf, ease: f32, l: f32) -> bool {
    let hinge_x = BX + 1.0;
    let y_top = BY + 2.0;
    let y_bot = BY + KP_H + 2.0;
    let door_w = KP_W + 2.0;

    let theta = ease * PI;
    let cos_t = theta.cos(); // +1 closed (right), -1 open (left)
    let sin_t = theta.sin(); // 0 at ends, +1 at 90 degrees

    // Dynamic shadow cast onto the keypad bay while door is closing/opening
    if cos_t > -0.15 {
        let sh_w = (door_w * cos_t.max(0.0) + 18.0 * sin_t).min(door_w);
        let sh_rect = xf.r(hinge_x, y_top + 2.0, sh_w + 6.0, KP_H - 2.0);
        p.rect_filled(sh_rect, xf.s(2.0), Color32::from_black_alpha((70.0 * (0.25 + 0.75 * sin_t)) as u8));
    }

    let proj_w = door_w * cos_t.abs().max(0.04);
    let persp_y = 14.0 * sin_t;

    let front_visible = cos_t >= 0.0;
    let hit_rect = if front_visible {
        // Swung to the RIGHT (closed over keypad)
        let x0 = hinge_x;
        let x1 = hinge_x + proj_w;
        let quad = vec![
            xf.p(x0, y_top),
            xf.p(x1, y_top - persp_y * 0.5),
            xf.p(x1, y_bot + persp_y * 0.5),
            xf.p(x0, y_bot),
        ];
        // Subtle shadow to the right edge of closed door
        if sin_t > 0.05 {
            paint::poly_shadow(p, &quad, Vec2::new(xf.s(6.0 * sin_t), xf.s(5.0)), xf.s(10.0), Color32::from_black_alpha(110));
        }

        // Shade slightly darker as door turns away from overhead light
        let light_k = (0.76 + 0.24 * cos_t).clamp(0.5, 1.0);
        let top_c = dim(mix(Color32::from_rgb(0xCF, 0xCC, 0xC2), Color32::from_rgb(0xF6, 0xF5, 0xF0), light_k), l);
        let bot_c = dim(mix(Color32::from_rgb(0xB8, 0xB5, 0xAA), Color32::from_rgb(0xE0, 0xDD, 0xD3), light_k), l);
        paint::grad_poly_v(
            p,
            &quad,
            xf.p(0.0, y_top).y,
            xf.p(0.0, y_bot).y,
            top_c,
            bot_c,
            Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xA6, 0xA2, 0x97), l))),
        );

        // Bottom-right finger pull notch extending slightly down (just like Photo 2)
        if cos_t > 0.35 {
            let notch_x0 = x0 + proj_w * 0.72;
            let notch_pts = vec![
                xf.p(notch_x0, y_bot - 1.0),
                xf.p(x1, y_bot - 1.0 + persp_y * 0.5),
                xf.p(x1, y_bot + 7.5 + persp_y * 0.5),
                xf.p(notch_x0 + proj_w * 0.05, y_bot + 7.5),
            ];
            paint::grad_poly_v(
                p,
                &notch_pts,
                xf.p(0.0, y_bot).y,
                xf.p(0.0, y_bot + 8.0).y,
                bot_c,
                dim(Color32::from_rgb(0xCD, 0xC9, 0xBE), l),
                Some(Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0xA2, 0x9E, 0x93), l))),
            );
        }

        // Iconic 45-degree diagonal molded ribs on upper 68% of outer cover (Photo 2!)
        let seam_y = y_top + KP_H * 0.68;
        let seam_r_y = seam_y + persp_y * 0.18;
        let rib_clip = Rect::from_min_max(xf.p(x0 + 1.0, y_top + 1.0), xf.p(x1 - 1.0, seam_y));
        let cp = p.with_clip_rect(rib_clip);
        let rib_dark = dim(Color32::from_rgb(0xC6, 0xC2, 0xB7), l);
        let rib_hi = alpha(Color32::WHITE, 0.75 * l * light_k);
        let step = 13.5 * cos_t.max(0.2);
        let mut s = -KP_H;
        while s < proj_w + KP_H {
            let a = xf.p(x0 + s, seam_y);
            let b = xf.p(x0 + s + KP_H * 0.68 * cos_t.max(0.15), y_top);
            cp.line_segment([a, b], Stroke::new(xf.s(1.4), rib_dark));
            cp.line_segment(
                [a + Vec2::new(xf.s(1.3), xf.s(0.5)), b + Vec2::new(xf.s(1.3), xf.s(0.5))],
                Stroke::new(xf.s(1.1), rib_hi),
            );
            s += step;
        }

        // Crisp parting line between upper ribbed section and lower smooth glossy section
        p.line_segment(
            [xf.p(x0 + 1.0, seam_y), xf.p(x1 - 1.0, seam_r_y)],
            Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xBA, 0xB6, 0xAA), l)),
        );
        p.line_segment(
            [xf.p(x0 + 1.0, seam_y + 1.1), xf.p(x1 - 1.0, seam_r_y + 1.1)],
            Stroke::new(xf.s(0.9), alpha(Color32::WHITE, 0.65 * l)),
        );

        // Specular gloss band across the smooth lower section
        let lower_rect = Rect::from_min_max(xf.p(x0 + 2.0, seam_y + 2.0), xf.p(x1 - 2.0, y_bot - 2.0));
        paint::gloss_band(p, lower_rect, xf.s(2.0), 0.22 * l * light_k);

        // Subtle engraved open arrow hint near right edge when fully closed
        if cos_t > 0.92 {
            let hint_c = alpha(dim(Color32::from_rgb(0x84, 0x80, 0x76), l), 0.85);
            paint::tiny_tri(p, xf.p(x1 - 38.0, y_bot - 13.5), xf.s(5.5), 2, hint_c);
            p.text(
                xf.p(x1 - 12.0, y_bot - 13.5),
                Align2::RIGHT_CENTER,
                "Open",
                theme::regular(xf.s(8.0)),
                hint_c,
            );
        }

        Rect::from_min_max(xf.p(x0, y_top), xf.p(x1, y_bot + 8.0))
    } else {
        // Swung to the LEFT (open, showing molded inner face of the cover, Photo 1!)
        let open_scale = 0.88; // rests naturally at ~155 deg against wall
        let w = proj_w * open_scale;
        let x1 = hinge_x;
        let x0 = hinge_x - w;
        let quad = vec![
            xf.p(x0, y_top - persp_y * 0.45),
            xf.p(x1, y_top),
            xf.p(x1, y_bot),
            xf.p(x0, y_bot + persp_y * 0.45),
        ];
        // Soft shadow cast onto the wall behind the open door
        paint::poly_shadow(p, &quad, Vec2::new(xf.s(-4.0), xf.s(8.0)), xf.s(14.0), Color32::from_black_alpha(115));

        let light_k = (0.72 + 0.28 * (-cos_t)).clamp(0.45, 1.0);
        let top_c = dim(mix(Color32::from_rgb(0xC6, 0xC2, 0xB7), Color32::from_rgb(0xE2, 0xDF, 0xD5), light_k), l);
        let bot_c = dim(mix(Color32::from_rgb(0xB4, 0xB0, 0xA4), Color32::from_rgb(0xCE, 0xCA, 0xBE), light_k), l);
        paint::grad_poly_v(
            p,
            &quad,
            xf.p(0.0, y_top).y,
            xf.p(0.0, y_bot).y,
            top_c,
            bot_c,
            Some(Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0x9C, 0x98, 0x8D), l))),
        );

        // Recessed molded interior frame (just like the hollow plastic door inside in Photo 1!)
        if -cos_t > 0.25 {
            let inset_x = 7.0 * (-cos_t);
            let inner = Rect::from_min_max(xf.p(x0 + inset_x, y_top + 7.0), xf.p(x1 - inset_x, y_bot - 7.0));
            paint::grad_rrect(
                p,
                inner,
                xf.s(2.0),
                dim(Color32::from_rgb(0xD3, 0xCF, 0xC4), l),
                dim(Color32::from_rgb(0xC7, 0xC3, 0xB8), l),
                Some(Stroke::new(xf.s(0.9), dim(Color32::from_rgb(0xA6, 0xA2, 0x96), l))),
            );
            // Molded inner horizontal stiffening rib (visible in Photo 1)
            let rib_y = inner.top() + inner.height() * 0.68;
            p.line_segment(
                [Pos2::new(inner.left() + xf.s(2.0), rib_y), Pos2::new(inner.right() - xf.s(2.0), rib_y)],
                Stroke::new(xf.s(1.1), dim(Color32::from_rgb(0xAC, 0xA8, 0x9C), l)),
            );
            p.line_segment(
                [Pos2::new(inner.left() + xf.s(2.0), rib_y + xf.s(1.2)), Pos2::new(inner.right() - xf.s(2.0), rib_y + xf.s(1.2))],
                Stroke::new(xf.s(0.8), alpha(Color32::WHITE, 0.35 * l)),
            );

            // Subtle printed user quick-reference inside the door (fades in when mostly open)
            let text_a = ((-cos_t - 0.60) / 0.40).clamp(0.0, 1.0);
            if text_a > 0.05 {
                let ink = alpha(dim(Color32::from_rgb(0x46, 0x44, 0x3E), l.max(0.5)), text_a);
                let muted = alpha(dim(Color32::from_rgb(0x5E, 0x5B, 0x54), l.max(0.5)), text_a);
                let tx = inner.left() + xf.s(8.0);
                let mut ty = inner.top() + xf.s(12.0);
                p.text(Pos2::new(tx, ty), Align2::LEFT_TOP, "SYSTEM COMMANDS", theme::semi(xf.s(7.4)), ink);
                ty += xf.s(15.0);
                for (cmd, desc) in [
                    ("* 1", "Zone Bypass"),
                    ("* 2", "View Troubles"),
                    ("* 3", "Alarm Memory"),
                    ("* 5", "Access Codes"),
                    ("* 6", "User Functions"),
                    ("* 7", "Utility / Reset"),
                    ("* 8", "Installer Prog"),
                    ("* 0", "Quick Arm / Exit"),
                ] {
                    p.text(Pos2::new(tx, ty), Align2::LEFT_TOP, cmd, theme::semi(xf.s(7.4)), ink);
                    p.text(Pos2::new(tx + xf.s(24.0), ty), Align2::LEFT_TOP, desc, theme::regular(xf.s(7.2)), muted);
                    ty += xf.s(14.5);
                }
                p.text(
                    Pos2::new(tx, rib_y + xf.s(9.0)),
                    Align2::LEFT_TOP,
                    "Hold F, A or P for 1 sec",
                    theme::regular(xf.s(7.0)),
                    muted,
                );
                p.text(
                    Pos2::new(tx, rib_y + xf.s(21.0)),
                    Align2::LEFT_TOP,
                    "for Fire, Aux or Panic.",
                    theme::regular(xf.s(7.0)),
                    muted,
                );
                let close_c = alpha(muted, 0.85);
                p.text(
                    Pos2::new(inner.right() - xf.s(15.0), inner.bottom() - xf.s(8.0)),
                    Align2::RIGHT_BOTTOM,
                    "Close",
                    theme::regular(xf.s(7.6)),
                    close_c,
                );
                paint::tiny_tri(
                    p,
                    Pos2::new(inner.right() - xf.s(9.5), inner.bottom() - xf.s(12.2)),
                    xf.s(5.2),
                    3,
                    close_c,
                );
            }
        }

        // Hinge spine highlight
        p.line_segment(
            [xf.p(hinge_x, y_top + 2.0), xf.p(hinge_x, y_bot - 2.0)],
            Stroke::new(xf.s(1.6), dim(Color32::from_rgb(0x9A, 0x96, 0x8B), l)),
        );

        Rect::from_min_max(xf.p(x0, y_top), xf.p(x1, y_bot))
    };

    let tip = if front_visible {
        "Click cover to swing open (left)"
    } else {
        "Click cover to swing closed (right)"
    };
    let resp = ui
        .interact(hit_rect, egui::Id::new("pc1550-cover"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    resp.clicked()
}
