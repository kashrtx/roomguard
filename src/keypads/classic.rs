//! The 1990s style LED keypad: warm off white plastic, a smoked window with
//! five status lights and six zone lights, and fifteen chunky keys with an
//! amber backlight that shows when the room is dark.

use std::f32::consts::{FRAC_PI_2, TAU};

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, Stroke, Vec2};

use super::{KeyInput, Xf};
use crate::audio::AudioHandle;
use crate::engine::types::Key;
use crate::engine::{EngineHandle, View};
use crate::paint::{self, alpha, dim, mix};
use crate::theme;

pub const SIZE: Vec2 = Vec2::new(300.0, 396.0);

const LED_GREEN: Color32 = Color32::from_rgb(0x3D, 0xE0, 0x6E);
const LED_RED: Color32 = Color32::from_rgb(0xFF, 0x3D, 0x2E);
const LED_AMBER: Color32 = Color32::from_rgb(0xFF, 0xB0, 0x26);

#[allow(clippy::too_many_arguments)]
pub fn draw(ui: &mut egui::Ui, area: Rect, view: &View, input: &mut KeyInput, engine: &EngineHandle, audio: &AudioHandle, lights: f32, t: f64) {
    let xf = Xf::fit(area, SIZE, 28.0);
    let p = ui.painter().clone();
    let body = xf.r(0.0, 0.0, SIZE.x, SIZE.y);
    paint::shadow(&p, body.translate(Vec2::new(0.0, xf.s(7.0))), xf.s(18.0), Vec2::ZERO, xf.s(28.0), Color32::from_black_alpha(120));
    paint::grad_rrect(
        &p,
        body,
        xf.s(18.0),
        dim(Color32::from_rgb(0xEE, 0xE9, 0xDC), lights),
        dim(Color32::from_rgb(0xD9, 0xD2, 0xC2), lights),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xA9, 0xA1, 0x90), lights))),
    );
    p.rect_stroke(body.shrink(xf.s(5.0)), xf.s(14.0), Stroke::new(xf.s(0.8), alpha(Color32::WHITE, 0.45 * lights + 0.05)));

    // Smoked window with the lights.
    let win = xf.r(22.0, 20.0, 256.0, 100.0);
    paint::grad_rrect(
        &p,
        win,
        xf.s(8.0),
        Color32::from_rgb(0x31, 0x2C, 0x28),
        Color32::from_rgb(0x19, 0x16, 0x14),
        Some(Stroke::new(xf.s(1.2), Color32::from_rgb(0x10, 0x0E, 0x0C))),
    );
    let gloss = Rect::from_min_max(win.min + Vec2::splat(xf.s(3.0)), Pos2::new(win.max.x - xf.s(3.0), win.min.y + win.height() * 0.42));
    paint::grad_rrect(&p, gloss, xf.s(6.0), alpha(Color32::WHITE, 0.09), alpha(Color32::WHITE, 0.0), Some(Stroke::NONE));
    let legend = Color32::from_rgb(0xC9, 0xC1, 0xB2);
    let names = ["READY", "ARMED", "MEMORY", "BYPASS", "TROUBLE"];
    let colors = [LED_GREEN, LED_RED, LED_AMBER, LED_AMBER, LED_AMBER];
    for i in 0..5 {
        let cx = 22.0 + 256.0 * (i as f32 + 0.5) / 5.0;
        paint::led(&p, xf.p(cx, 44.0), xf.s(4.6), colors[i], view.leds[i].lit(t), lights);
        p.text(xf.p(cx, 61.0), Align2::CENTER_CENTER, names[i], theme::semi(xf.s(7.4)), legend);
    }
    p.line_segment([xf.p(34.0, 72.0), xf.p(266.0, 72.0)], Stroke::new(xf.s(0.6), alpha(legend, 0.25)));
    for z in 0..6 {
        let cx = 22.0 + 256.0 * (z as f32 + 0.5) / 6.0;
        paint::led(&p, xf.p(cx, 88.0), xf.s(4.2), LED_RED, view.leds[5 + z].lit(t), lights);
        p.text(xf.p(cx, 104.0), Align2::CENTER_CENTER, format!("{}", z + 1), theme::semi(xf.s(8.6)), legend);
    }

    let rows: [[(Key, &str, &str); 3]; 5] = [
        [(Key::Digit(1), "1", ""), (Key::Digit(2), "2", ""), (Key::Digit(3), "3", "")],
        [(Key::Digit(4), "4", ""), (Key::Digit(5), "5", ""), (Key::Digit(6), "6", "")],
        [(Key::Digit(7), "7", ""), (Key::Digit(8), "8", ""), (Key::Digit(9), "9", "")],
        [(Key::Star, "*", ""), (Key::Digit(0), "0", ""), (Key::Hash, "#", "")],
        [(Key::Fire, "F", "FIRE"), (Key::Aux, "A", "AUX"), (Key::Panic, "P", "PANIC")],
    ];
    for (row, line) in rows.iter().enumerate() {
        for (col, (key, label, sub)) in line.iter().enumerate() {
            let x = 30.0 + col as f32 * 86.0;
            let y = 140.0 + row as f32 * 49.5;
            let rect = xf.r(x, y, 68.0, 38.0);
            let held = input.key(ui, rect, *key, engine, audio);
            let hold = view.hold.filter(|h| h.0 == *key).map(|h| h.1);
            draw_key(&p, &xf, rect, label, sub, held, row == 4, lights, hold);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_key(p: &Painter, xf: &Xf, rect: Rect, label: &str, sub: &str, held: bool, emergency: bool, lights: f32, hold: Option<f32>) {
    let r = xf.s(9.0);
    p.rect_filled(rect.expand(xf.s(1.8)), r + xf.s(1.8), dim(Color32::from_rgb(0xBE, 0xB6, 0xA5), lights));
    if !held {
        paint::shadow(p, rect.translate(Vec2::new(0.0, xf.s(2.4))), r, Vec2::ZERO, xf.s(4.0), Color32::from_black_alpha(70));
    }
    let k = rect.translate(Vec2::new(0.0, if held { xf.s(1.6) } else { 0.0 }));
    let (top, bot) = if held {
        (Color32::from_rgb(0xDC, 0xD6, 0xC8), Color32::from_rgb(0xCD, 0xC6, 0xB6))
    } else {
        (Color32::from_rgb(0xF7, 0xF3, 0xEA), Color32::from_rgb(0xDA, 0xD3, 0xC5))
    };
    paint::grad_rrect(p, k, r, dim(top, lights), dim(bot, lights), Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0xA6, 0x9E, 0x8E), lights))));
    let back = (1.0 - lights).clamp(0.0, 1.0);
    if back > 0.05 {
        paint::glow(p, k.center(), xf.s(30.0), alpha(Color32::from_rgb(0xFF, 0xA6, 0x3A), 0.14 * back));
    }
    let ink = if emergency { Color32::from_rgb(0x9C, 0x2C, 0x22) } else { Color32::from_rgb(0x3A, 0x37, 0x32) };
    let color = mix(dim(ink, lights), Color32::from_rgb(0xFF, 0xBE, 0x66), back * 0.92);
    let lift = if label == "*" { xf.s(3.5) } else { 0.0 };
    let label_y = if sub.is_empty() { k.center().y + lift } else { k.center().y - xf.s(4.5) };
    let size = if label == "*" { 21.0 } else { 17.0 };
    p.text(Pos2::new(k.center().x, label_y), Align2::CENTER_CENTER, label, theme::semi(xf.s(size)), color);
    if !sub.is_empty() {
        p.text(Pos2::new(k.center().x, k.center().y + xf.s(10.5)), Align2::CENTER_CENTER, sub, theme::semi(xf.s(6.6)), alpha(color, 0.85));
    }
    if let Some(h) = hold {
        paint::arc(p, k.center(), xf.s(25.0), -FRAC_PI_2, -FRAC_PI_2 + TAU * h, Stroke::new(xf.s(2.4), theme::AMBER));
    }
}
