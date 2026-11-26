//! VistaGuard 20P Alpha Display Voice Keypad (6160V form factor), re-badged RmG:
//! wide glossy pearl-white rectangular ABS housing, 4 tapered speaker grille
//! vents on the upper-left, oval ARMED / READY / MSG LEDs on the lower-left,
//! sculpted angled visor with emerald-green backlit 2x16 dot-matrix LCD,
//! 4 vertical backlit function keys (A-D) that remain exposed through a cutout
//! when the cover is closed, 12 backlit rectangular dual-legend numeric keys,
//! and a working bottom-hinged flip cover with the Quick Operating Instructions
//! card printed inside.

use std::f32::consts::FRAC_PI_2;

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use super::{KeyInput, Xf};
use crate::audio::AudioHandle;
use crate::engine::types::{Key, Led};
use crate::engine::{EngineHandle, View};
use crate::lcd_font;
use crate::paint::{self, alpha, dim, mix};
use crate::theme;

/// Design space accommodating the wide 6160V body + bottom-hinged open flip cover.
pub const SIZE: Vec2 = Vec2::new(440.0, 428.0);

const BX: f32 = 16.0;
const BY: f32 = 12.0;
const BW: f32 = 408.0;
const BH: f32 = 250.0;
const SPLIT_X: f32 = BX + 108.0; // 124.0: separates left LED/speaker column from right LCD/keys bay
const SEAM_Y: f32 = BY + 100.0; // 112.0: bottom of upper LCD visor
const HINGE_Y: f32 = BY + BH; // 262.0: bottom hinge of flip-down cover
const COVER_H: f32 = 148.0;

/// Draws the VistaGuard 20P / VG6160V keypad. Returns `true` when the user clicks
/// the flip cover to open or close it.
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
    let xf = Xf::fit(area, SIZE, 8.0);
    let p = ui.painter().clone();
    let l = lights;
    let bl = view.backlight;
    let open = input.door_anim_6160;
    let mut toggled = false;

    // 1. Multi-layer wall drop shadow behind the wide rectangular ABS body
    let body = xf.r(BX, BY, BW, BH);
    let body_pts = [body.left_top(), body.right_top(), body.right_bottom(), body.left_bottom()];
    paint::poly_shadow(&p, &body_pts, Vec2::new(xf.s(3.0), xf.s(7.0)), xf.s(14.0), Color32::BLACK);

    // 2. Main glossy pearl-white rectangular housing
    paint::grad_rrect(
        &p,
        body,
        xf.s(6.0),
        dim(Color32::from_rgb(0xFB, 0xFA, 0xF7), l),
        dim(Color32::from_rgb(0xE1, 0xDF, 0xD9), l),
        Some(Stroke::new(1.2_f32, dim(Color32::from_rgb(0xC2, 0xBF, 0xB8), l))),
    );
    // Subtle glossy top specular rim
    p.line_segment(
        [xf.p(BX + 6.0, BY + 1.5), xf.p(BX + BW - 6.0, BY + 1.5)],
        Stroke::new(xf.s(1.6), alpha(Color32::WHITE, 0.72 * l)),
    );

    // 3. Vertical parting seam separating the left speaker/LED bay from the right bay
    p.line_segment(
        [xf.p(SPLIT_X, BY + 2.0), xf.p(SPLIT_X, BY + BH - 2.0)],
        Stroke::new(xf.s(1.2), dim(Color32::from_rgb(0xC6, 0xC3, 0xBC), l)),
    );
    p.line_segment(
        [xf.p(SPLIT_X + 1.2, BY + 2.0), xf.p(SPLIT_X + 1.2, BY + BH - 2.0)],
        Stroke::new(xf.s(0.9), alpha(Color32::WHITE, 0.55 * l)),
    );

    // 4. Upper-left 6160V voice speaker grille: 4 horizontal tapered vents
    for (i, half_w) in [24.0f32, 29.0, 29.0, 24.0].iter().enumerate() {
        let vy = BY + 36.0 + i as f32 * 11.5;
        let cx = BX + 54.0;
        let slot = Rect::from_center_size(xf.p(cx, vy), Vec2::new(xf.s(*half_w * 2.0), xf.s(4.2)));
        p.rect_filled(slot, xf.s(2.1), dim(Color32::from_rgb(0x28, 0x2B, 0x2E), l.max(0.25)));
        p.line_segment(
            [Pos2::new(slot.left() + xf.s(3.0), slot.bottom() + xf.s(0.6)), Pos2::new(slot.right() - xf.s(3.0), slot.bottom() + xf.s(0.6))],
            Stroke::new(xf.s(0.9), alpha(Color32::WHITE, 0.65 * l)),
        );
    }

    // 5. Lower-left recessed status plate: ARMED (red), READY (green), MSG (amber)
    let plate = xf.r(BX + 14.0, BY + 114.0, 80.0, 118.0);
    paint::grad_rrect(
        &p,
        plate,
        xf.s(5.0),
        dim(Color32::from_rgb(0xF6, 0xF5, 0xF2), l),
        dim(Color32::from_rgb(0xEA, 0xE8, 0xE3), l),
        Some(Stroke::new(0.9_f32, dim(Color32::from_rgb(0xD0, 0xCD, 0xC6), l))),
    );
    let led_ready = view.lcd_leds[0];
    let led_armed = view.lcd_leds[1];
    let led_msg = view.lcd_leds[2];
    draw_oval_led(&p, &xf, BX + 24.0, BY + 144.0, "ARMED", led_armed, Color32::from_rgb(0xFF, 0x3B, 0x2F), l, t, view.all_on);
    draw_oval_led(&p, &xf, BX + 24.0, BY + 178.0, "READY", led_ready, Color32::from_rgb(0x30, 0xE8, 0x52), l, t, view.all_on);
    draw_oval_led(&p, &xf, BX + 24.0, BY + 210.0, "MSG", led_msg, Color32::from_rgb(0xFF, 0xB8, 0x28), l, t, view.all_on);

    // 6. Sculpted upper-right visor with slight upward-sloping top arch
    let visor_pts = vec![
        xf.p(SPLIT_X + 2.0, BY + 14.0),
        xf.p(BX + BW - 4.0, BY + 4.0),
        xf.p(BX + BW - 4.0, SEAM_Y),
        xf.p(SPLIT_X + 2.0, SEAM_Y),
    ];
    p.add(Shape::convex_polygon(
        visor_pts.clone(),
        dim(Color32::from_rgb(0xF5, 0xF4, 0xF0), l),
        Stroke::new(1.0_f32, dim(Color32::from_rgb(0xCC, 0xC9, 0xC2), l)),
    ));
    p.line_segment(
        [xf.p(SPLIT_X + 3.0, SEAM_Y), xf.p(BX + BW - 4.0, SEAM_Y)],
        Stroke::new(xf.s(1.6), dim(Color32::from_rgb(0xB8, 0xB5, 0xAE), l)),
    );

    // 7. Deep-recessed 2x16 green backlit Alpha LCD
    let bezel = xf.r(SPLIT_X + 20.0, BY + 22.0, 262.0, 66.0);
    paint::grad_rrect(
        &p,
        bezel,
        xf.s(4.0),
        dim(Color32::from_rgb(0xCF, 0xCC, 0xC4), l),
        dim(Color32::from_rgb(0xF6, 0xF5, 0xF1), l),
        Some(Stroke::new(0.9_f32, dim(Color32::from_rgb(0xB2, 0xAF, 0xA8), l))),
    );
    let lcd = bezel.shrink2(Vec2::new(xf.s(5.0), xf.s(5.0)));
    draw_lcd(&p, &xf, lcd, &view.lcd, view.lcd_cursor, bl, view.contrast, l, t, view.all_on);

    // 8. Lower-right keypad bay (always draw keys underneath; closed cover masks numeric keys)
    let keys_interactive = open > 0.45;
    let fn_keys: [(Key, &str); 4] = [
        (Key::Fire, "FIRE"),
        (Key::Panic, "POLICE"),
        (Key::Aux, "MED"),
        (Key::Away, "AWAY"),
    ];
    // Left vertical function key channel (A, B, C, D)
    let chan = xf.r(SPLIT_X + 24.0, SEAM_Y + 8.0, 48.0, 124.0);
    paint::grad_rrect(
        &p,
        chan,
        xf.s(3.0),
        dim(Color32::from_rgb(0xD8, 0xD6, 0xD0), l),
        dim(Color32::from_rgb(0xEB, 0xE9, 0xE4), l),
        Some(Stroke::new(0.8_f32, dim(Color32::from_rgb(0xBE, 0xBB, 0xB4), l))),
    );
    for (row, (key, label)) in fn_keys.iter().enumerate() {
        let kr = xf.r(SPLIT_X + 28.0, SEAM_Y + 12.0 + row as f32 * 30.0, 40.0, 24.0);
        // Function keys stay clickable EVEN when cover is closed (via the cover cutout window!)
        let down = input.key(ui, kr, *key, engine, audio);
        draw_vista_key(&p, &xf, kr, "", label, down, bl, l, true);
    }

    // 3x4 numeric key grid with authentic dual legends
    let grid: [[(Key, &str, &str); 3]; 4] = [
        [(Key::Digit(1), "1", "OFF"), (Key::Digit(2), "2", "AWAY"), (Key::Digit(3), "3", "STAY")],
        [(Key::Digit(4), "4", "MAX"), (Key::Digit(5), "5", "TEST"), (Key::Digit(6), "6", "BYPASS")],
        [(Key::Digit(7), "7", "INSTANT"), (Key::Digit(8), "8", "CODE"), (Key::Digit(9), "9", "CHIME")],
        [(Key::Star, "*", "READY"), (Key::Digit(0), "0", ""), (Key::Hash, "#", "")],
    ];
    for (row, rkeys) in grid.iter().enumerate() {
        for (col, (key, big, sub)) in rkeys.iter().enumerate() {
            let kx = SPLIT_X + 86.0 + col as f32 * 64.0;
            let ky = SEAM_Y + 12.0 + row as f32 * 30.0;
            let kr = xf.r(kx, ky, 54.0, 24.0);
            let down = if keys_interactive {
                input.key(ui, kr, *key, engine, audio)
            } else {
                false
            };
            draw_vista_key(&p, &xf, kr, big, sub, down, bl, l, false);
        }
    }

    // Embossed molded "RmG" nameplate recess in lower-right
    let badge = xf.r(BX + BW - 94.0, BY + BH - 20.0, 74.0, 14.0);
    p.rect_stroke(badge, xf.s(2.0), Stroke::new(0.8_f32, dim(Color32::from_rgb(0xC8, 0xC5, 0xBE), l)));
    p.text(
        badge.center(),
        Align2::CENTER_CENTER,
        "RmG",
        theme::semi(xf.s(10.0)),
        dim(Color32::from_rgb(0xA4, 0xA0, 0x97), l.max(0.45)),
    );

    // 9. Bottom-hinged flip cover (0.0 = UP closed with A-D cutout, 1.0 = DOWN open with instructions)
    if draw_flip_cover(ui, &p, &xf, open, l) {
        toggled = true;
    }

    // 10. Hold progress indicator for 2-second emergency/function keys
    if let Some((hk, frac)) = view.hold {
        let label = match hk {
            Key::Fire => "Hold FIRE...",
            Key::Panic => "Hold POLICE...",
            Key::Aux => "Hold MEDICAL...",
            Key::Away => "Hold AWAY...",
            _ => "Hold...",
        };
        let bar_bg = xf.r(SPLIT_X + 20.0, BY + BH + 4.0, 262.0, 14.0);
        p.rect_filled(bar_bg, xf.s(4.0), Color32::from_black_alpha(180));
        let fill = Rect::from_min_size(bar_bg.min, Vec2::new(bar_bg.width() * frac.clamp(0.0, 1.0), bar_bg.height()));
        p.rect_filled(fill, xf.s(4.0), theme::RED);
        p.text(bar_bg.center(), Align2::CENTER_CENTER, label, theme::semi(xf.s(9.0)), Color32::WHITE);
    }

    toggled
}

fn draw_oval_led(p: &Painter, xf: &Xf, x: f32, y: f32, label: &str, state: Led, lit_col: Color32, l: f32, t: f64, all_on: bool) {
    let on = all_on || state.lit(t);
    p.text(
        xf.p(x, y),
        Align2::LEFT_CENTER,
        label,
        theme::semi(xf.s(8.8)),
        dim(Color32::from_rgb(0x32, 0x35, 0x38), l.max(0.55)),
    );
    let led_rect = Rect::from_center_size(xf.p(x + 52.0, y), Vec2::new(xf.s(15.0), xf.s(6.2)));
    let lens_col = if on {
        lit_col
    } else {
        dim(mix(lit_col, Color32::from_rgb(0x28, 0x2C, 0x29), 0.78), l.max(0.28))
    };
    if on {
        paint::glow(p, led_rect.center(), xf.s(15.0), alpha(lit_col, 0.34));
    }
    p.rect_filled(led_rect, xf.s(3.1), lens_col);
    p.rect_stroke(led_rect, xf.s(3.1), Stroke::new(0.8_f32, dim(Color32::from_rgb(0x90, 0x8E, 0x88), l)));
    if on {
        p.circle_filled(led_rect.center() - Vec2::new(xf.s(2.0), xf.s(0.8)), xf.s(1.6), alpha(Color32::WHITE, 0.75));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lcd(
    p: &Painter,
    xf: &Xf,
    lcd: Rect,
    lines: &[String; 2],
    cursor: Option<(usize, usize)>,
    bl: f32,
    contrast: f32,
    l: f32,
    t: f64,
    all_on: bool,
) {
    let bg_lit = Color32::from_rgb(0x6D, 0xC6, 0x2A);
    let bg_dark = dim(Color32::from_rgb(0x4E, 0x69, 0x3E), l.max(0.22));
    let bg_top = mix(bg_dark, Color32::from_rgb(0x7B, 0xD6, 0x34), bl);
    let bg_bot = mix(bg_dark, bg_lit, bl);
    paint::grad_rrect(p, lcd, xf.s(2.5), bg_top, bg_bot, Some(Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x3B, 0x1E))));

    // Subtle green backlight glow onto the bezel when lit in dark room
    if bl > 0.2 {
        paint::glow(p, lcd.center(), lcd.width() * 0.55, alpha(Color32::from_rgb(0x60, 0xE0, 0x30), 0.12 * bl * (1.0 - l * 0.6)));
    }

    let ink_on = mix(Color32::from_rgb(0x1D, 0x34, 0x10), Color32::from_rgb(0x0B, 0x18, 0x06), contrast.clamp(0.2, 1.0));
    let ink_ghost = alpha(ink_on, 0.09);
    let pad_x = xf.s(8.0);
    let pad_y = xf.s(7.0);
    let inner = Rect::from_min_max(lcd.min + Vec2::new(pad_x, pad_y), lcd.max - Vec2::new(pad_x, pad_y));
    let cell_w = inner.width() / 16.0;
    let row_h = inner.height() / 2.0;
    let dot_w = (cell_w * 0.82) / 5.0;
    let dot_h = (row_h * 0.80) / 7.0;
    let blink_on = (t * 2.0) as u64 % 2 == 0;

    for (row, text) in lines.iter().enumerate() {
        let chars: Vec<char> = text.chars().take(16).collect();
        for col in 0..16 {
            let ch = chars.get(col).copied().unwrap_or(' ');
            let glyph = lcd_font::glyph(ch);
            let ox = inner.left() + col as f32 * cell_w + (cell_w - dot_w * 5.0) * 0.5;
            let oy = inner.top() + row as f32 * row_h + (row_h - dot_h * 7.0) * 0.5;
            for gx in 0..5 {
                let bits = glyph[gx];
                for gy in 0..7 {
                    let on = all_on || ((bits >> gy) & 1) != 0;
                    let r = Rect::from_min_size(
                        Pos2::new(ox + gx as f32 * dot_w, oy + gy as f32 * dot_h),
                        Vec2::new((dot_w - 0.5).max(0.8), (dot_h - 0.5).max(0.8)),
                    );
                    if on {
                        // Optical polarizer drop shadow
                        p.rect_filled(r.translate(Vec2::new(0.7, 0.9)), 0.0, alpha(ink_on, 0.18));
                        p.rect_filled(r, 0.0, ink_on);
                    } else {
                        p.rect_filled(r, 0.0, ink_ghost);
                    }
                }
            }
            // Authentic HD44780 blinking underline cursor for Time/Date, Schedule, and Macro editing
            if blink_on && cursor == Some((row, col)) {
                let cur = Rect::from_min_size(
                    Pos2::new(ox, oy + 7.2 * dot_h),
                    Vec2::new(dot_w * 5.0 - 0.5, (dot_h * 0.85).max(1.1)),
                );
                p.rect_filled(cur, 0.0, ink_on);
            }
        }
    }
}

fn draw_vista_key(
    p: &Painter,
    xf: &Xf,
    rect: Rect,
    big: &str,
    sub: &str,
    down: bool,
    bl: f32,
    l: f32,
    is_fn: bool,
) {
    let r = if down { rect.translate(Vec2::new(0.0, xf.s(1.2))) } else { rect };
    // Subtle green translucent backlight bleed around rubber keys
    if bl > 0.15 {
        p.rect_filled(
            r.expand(xf.s(1.2)),
            xf.s(3.2),
            alpha(Color32::from_rgb(0x52, 0xDF, 0x38), 0.35 * bl * (1.0 - l * 0.45)),
        );
    }
    let top_col = if is_fn {
        dim(Color32::from_rgb(0xE8, 0xEA, 0xE6), l.max(0.35))
    } else {
        dim(Color32::from_rgb(0xDC, 0xDF, 0xDA), l.max(0.35))
    };
    let bot_col = if down {
        dim(Color32::from_rgb(0xB8, 0xBC, 0xB5), l.max(0.35))
    } else {
        dim(Color32::from_rgb(0xC6, 0xCA, 0xC3), l.max(0.35))
    };
    paint::grad_rrect(
        p,
        r,
        xf.s(2.8),
        top_col,
        bot_col,
        Some(Stroke::new(0.9_f32, dim(Color32::from_rgb(0x8A, 0x8E, 0x88), l.max(0.35)))),
    );
    let ink = dim(Color32::from_rgb(0x1E, 0x22, 0x20), l.max(0.55));
    if is_fn {
        p.text(r.center(), Align2::CENTER_CENTER, sub, theme::semi(xf.s(7.6)), ink);
    } else if sub.is_empty() {
        p.text(r.center(), Align2::CENTER_CENTER, big, theme::semi(xf.s(12.5)), ink);
    } else {
        p.text(
            Pos2::new(r.left() + xf.s(7.0), r.center().y),
            Align2::LEFT_CENTER,
            big,
            theme::semi(xf.s(12.0)),
            ink,
        );
        p.text(
            Pos2::new(r.right() - xf.s(5.0), r.center().y + xf.s(0.5)),
            Align2::RIGHT_CENTER,
            sub,
            theme::semi(xf.s(6.6)),
            ink,
        );
    }
}

fn draw_flip_cover(ui: &mut egui::Ui, p: &Painter, xf: &Xf, open: f32, l: f32) -> bool {
    let cover_x = SPLIT_X + 2.0;
    let cover_w = BX + BW - 4.0 - cover_x;
    let angle = open.clamp(0.0, 1.0) * std::f32::consts::PI;
    let cos_a = angle.cos(); // +1 closed (UP), -1 open (DOWN)
    let proj_h = (COVER_H * cos_a.abs()).max(10.0);

    let (door_rect, showing_inside) = if angle < FRAC_PI_2 {
        // Closed (UP): sits over numeric keys ONLY, with cutout window over A-D keys!
        (xf.r(cover_x, HINGE_Y - proj_h, cover_w, proj_h), false)
    } else {
        // Open (DOWN): hangs below bottom hinge showing Quick Operating Instructions
        (xf.r(cover_x, HINGE_Y, cover_w, proj_h), true)
    };

    if !showing_inside {
        // Draw closed door in 3 clean rectangles around the A-D function key cutout window
        // so the 4 function keys remain visibly exposed and clickable!
        let cut_x0 = SPLIT_X + 22.0;
        let cut_x1 = SPLIT_X + 74.0;
        let cut_y0 = SEAM_Y + 6.0;
        let cut_y1 = SEAM_Y + 134.0;
        let fill_top = dim(Color32::from_rgb(0xF8, 0xF7, 0xF3), l);
        let fill_bot = dim(Color32::from_rgb(0xE6, 0xE4, 0xDE), l);
        let stroke = Stroke::new(1.0_f32, dim(Color32::from_rgb(0xC0, 0xBD, 0xB6), l));

        // Left strip before A-D cutout
        let left_r = Rect::from_min_max(door_rect.left_top(), Pos2::new(xf.p(cut_x0, 0.0).x, door_rect.bottom()));
        paint::grad_rrect(p, left_r, xf.s(2.0), fill_top, fill_bot, None);
        // Right main panel covering numeric keys 1-#
        let right_r = Rect::from_min_max(Pos2::new(xf.p(cut_x1, 0.0).x, door_rect.top()), door_rect.right_bottom());
        paint::grad_rrect(p, right_r, xf.s(2.0), fill_top, fill_bot, None);
        // Top & bottom thin strips above/below cutout
        let top_r = Rect::from_min_max(Pos2::new(xf.p(cut_x0, 0.0).x, door_rect.top()), Pos2::new(xf.p(cut_x1, 0.0).x, xf.p(0.0, cut_y0).y.max(door_rect.top())));
        if top_r.height() > 1.0 {
            p.rect_filled(top_r, 0.0, fill_top);
        }
        let bot_r = Rect::from_min_max(Pos2::new(xf.p(cut_x0, 0.0).x, xf.p(0.0, cut_y1).y.min(door_rect.bottom())), Pos2::new(xf.p(cut_x1, 0.0).x, door_rect.bottom()));
        if bot_r.height() > 1.0 {
            p.rect_filled(bot_r, 0.0, fill_bot);
        }
        // Outer door border & cutout bezel
        p.rect_stroke(door_rect, xf.s(3.0), stroke);
        if open < 0.15 {
            let cut_r = xf.r(cut_x0, cut_y0, cut_x1 - cut_x0, cut_y1 - cut_y0);
            p.rect_stroke(cut_r, xf.s(2.5), Stroke::new(1.1_f32, dim(Color32::from_rgb(0xAE, 0xAB, 0xA4), l)));
            // Embossed RmG badge on closed cover
            let badge = xf.r(BX + BW - 94.0, BY + BH - 24.0, 74.0, 15.0);
            p.rect_stroke(badge, xf.s(2.0), Stroke::new(0.8_f32, dim(Color32::from_rgb(0xC8, 0xC5, 0xBE), l)));
            p.text(badge.center(), Align2::CENTER_CENTER, "RmG", theme::semi(xf.s(10.0)), dim(Color32::from_rgb(0xA4, 0xA0, 0x97), l.max(0.45)));
            p.text(
                Pos2::new(right_r.center().x, right_r.center().y - xf.s(4.0)),
                Align2::CENTER_CENTER,
                "Click cover to open",
                theme::regular(xf.s(9.0)),
                dim(Color32::from_rgb(0x9A, 0x97, 0x90), l.max(0.45)),
            );
        }
        // Only the numeric-key area of the closed cover toggles it (so A-D keys stay clickable!)
        let resp = ui
            .interact(right_r, egui::Id::new("vista-cover-closed"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.clicked()
    } else {
        // Open (DOWN): drop shadow + recessed white inside door + Quick Operating Instructions card
        let door_pts = [door_rect.left_top(), door_rect.right_top(), door_rect.right_bottom(), door_rect.left_bottom()];
        paint::poly_shadow(p, &door_pts, Vec2::new(xf.s(2.0), xf.s(4.0)), xf.s(9.0), Color32::BLACK);
        paint::grad_rrect(
            p,
            door_rect,
            xf.s(4.0),
            dim(Color32::from_rgb(0xE8, 0xE6, 0xE0), l),
            dim(Color32::from_rgb(0xF6, 0xF5, 0xF1), l),
            Some(Stroke::new(1.1_f32, dim(Color32::from_rgb(0xBC, 0xB9, 0xB2), l))),
        );
        // Hinge bar
        p.line_segment(
            [door_rect.left_top(), door_rect.right_top()],
            Stroke::new(xf.s(2.0), dim(Color32::from_rgb(0xA8, 0xA5, 0x9E), l)),
        );
        if open > 0.72 {
            // Cutout hole visible on the left of the open door (matches Photo 3!)
            let hole = Rect::from_min_size(
                door_rect.left_top() + Vec2::new(xf.s(20.0), xf.s(14.0)),
                Vec2::new(xf.s(50.0), door_rect.height() - xf.s(24.0)),
            );
            p.rect_filled(hole, xf.s(2.0), dim(Color32::from_rgb(0xC6, 0xC1, 0xB4), l));
            p.rect_stroke(hole, xf.s(2.0), Stroke::new(1.0_f32, dim(Color32::from_rgb(0xAA, 0xA6, 0x9E), l)));

            // Printed Quick Operating Instructions card
            let card = Rect::from_min_max(
                Pos2::new(hole.right() + xf.s(10.0), door_rect.top() + xf.s(10.0)),
                door_rect.right_bottom() - Vec2::new(xf.s(10.0), xf.s(10.0)),
            );
            p.rect_filled(card, xf.s(2.0), dim(Color32::from_rgb(0xF8, 0xF7, 0xF4), l));
            p.rect_stroke(card, xf.s(2.0), Stroke::new(0.7_f32, dim(Color32::from_rgb(0xD2, 0xCF, 0xC8), l)));

            let ink = dim(Color32::from_rgb(0x32, 0x36, 0x3B), l.max(0.45));
            let mut y = card.top() + xf.s(6.0);
            p.text(Pos2::new(card.left() + xf.s(7.0), y), Align2::LEFT_TOP, "Quick Operating Instructions", theme::semi(xf.s(8.4)), ink);
            y += xf.s(13.0);
            for line in [
                "Away:     Code + AWAY (2)",
                "Stay:     Code + STAY (3)",
                "Instant:  Code + INSTANT (7)",
                "Off:      Code + OFF (1)",
                "Chime:    Code + CHIME (9)",
                "Bypass:   Code + BYPASS + Zn",
                "Test:     Code + TEST (5) + 0",
                "Status:   # + * (Voice)",
            ] {
                p.text(Pos2::new(card.left() + xf.s(7.0), y), Align2::LEFT_TOP, line, theme::regular(xf.s(7.4)), ink);
                y += xf.s(11.8);
            }
        }
        let resp = ui
            .interact(door_rect, egui::Id::new("vista-cover-open"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.clicked()
    }
}
