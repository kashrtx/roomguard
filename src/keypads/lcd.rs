//! The 2000s style LCD keypad: a 2 x 16 dot matrix display with its
//! backlight, four indicator lights, scroll keys, the fire, auxiliary and
//! panic keys, a 3 x 4 number pad and five function keys.

use std::f32::consts::{FRAC_PI_2, TAU};

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, Stroke, Vec2};

use super::{KeyInput, Xf};
use crate::audio::AudioHandle;
use crate::engine::types::Key;
use crate::engine::{EngineHandle, View};
use crate::lcd_font;
use crate::paint::{self, alpha, dim, mix};
use crate::theme;

pub const SIZE: Vec2 = Vec2::new(330.0, 424.0);

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Text(&'static str),
    Small(&'static str),
    Chevron(bool),
    Fire,
    Aux,
    Panic,
}

#[allow(clippy::too_many_arguments)]
pub fn draw(ui: &mut egui::Ui, area: Rect, view: &View, input: &mut KeyInput, engine: &EngineHandle, audio: &AudioHandle, lights: f32, t: f64) {
    let xf = Xf::fit(area, SIZE, 28.0);
    let p = ui.painter().clone();
    let body = xf.r(0.0, 0.0, SIZE.x, SIZE.y);
    paint::shadow(&p, body.translate(Vec2::new(0.0, xf.s(7.0))), xf.s(22.0), Vec2::ZERO, xf.s(28.0), Color32::from_black_alpha(120));
    paint::grad_rrect(
        &p,
        body,
        xf.s(22.0),
        dim(Color32::from_rgb(0xF4, 0xF5, 0xF3), lights),
        dim(Color32::from_rgb(0xDF, 0xE1, 0xDE), lights),
        Some(Stroke::new(xf.s(1.0), dim(Color32::from_rgb(0xB9, 0xBD, 0xB8), lights))),
    );
    p.rect_stroke(body.shrink(xf.s(5.0)), xf.s(18.0), Stroke::new(xf.s(0.8), alpha(Color32::WHITE, 0.5 * lights + 0.05)));

    // Display.
    let bezel = xf.r(22.0, 22.0, 236.0, 82.0);
    paint::grad_rrect(&p, bezel, xf.s(7.0), Color32::from_rgb(0x3D, 0x41, 0x44), Color32::from_rgb(0x28, 0x2B, 0x2E), Some(Stroke::new(xf.s(1.0), Color32::from_rgb(0x1E, 0x20, 0x22))));
    let glass = bezel.shrink(xf.s(6.5));
    let bl = view.backlight.clamp(0.0, 1.0);
    let lit = mix(Color32::from_rgb(0x70, 0x7E, 0x4E), Color32::from_rgb(0xB4, 0xD9, 0x5B), bl);
    let bg_top = mix(dim(lit, lights.max(bl)), lit, bl * 0.8);
    if bl > 0.05 && lights < 0.95 {
        paint::glow(&p, glass.center(), glass.width() * 0.75, alpha(Color32::from_rgb(0xA8, 0xE0, 0x50), 0.16 * bl * (1.0 - lights)));
    }
    paint::grad_rrect(&p, glass, xf.s(3.0), bg_top, mix(bg_top, Color32::from_rgb(0x55, 0x66, 0x30), 0.25), Some(Stroke::new(xf.s(1.0), Color32::from_rgb(0x2B, 0x33, 0x1C))));
    draw_lcd(&p, &xf, glass, &view.lcd, view.lcd_cursor, view.contrast, t);

    // Indicators: Ready, Armed, Trouble, AC.
    let leds = [
        (Color32::from_rgb(0x3D, 0xE0, 0x6E), "READY"),
        (Color32::from_rgb(0xFF, 0x3D, 0x2E), "ARMED"),
        (Color32::from_rgb(0xFF, 0xB0, 0x26), "TROUBLE"),
        (Color32::from_rgb(0x3D, 0xE0, 0x6E), "AC"),
    ];
    let ink = dim(Color32::from_rgb(0x55, 0x59, 0x5E), lights);
    for (i, (color, name)) in leds.iter().enumerate() {
        let y = 31.0 + i as f32 * 21.0;
        paint::led(&p, xf.p(274.0, y), xf.s(4.2), *color, view.lcd_leds[i].lit(t), lights);
        let c = xf.p(292.0, y);
        match i {
            0 => paint::icon_check(&p, c, xf.s(12.0), ink),
            1 => paint::icon_house(&p, c, xf.s(12.0), ink),
            2 => paint::icon_warning(&p, c, xf.s(12.0), ink),
            _ => paint::icon_plug(&p, c, xf.s(12.0), ink),
        }
        p.text(xf.p(306.0, y), Align2::LEFT_CENTER, *name, theme::semi(xf.s(5.6)), alpha(ink, 0.9));
    }

    // Keys.
    let mut keys: Vec<(Key, Face, Rect)> = Vec::new();
    keys.push((Key::Left, Face::Chevron(false), xf.r(24.0, 124.0, 24.0, 30.0)));
    keys.push((Key::Right, Face::Chevron(true), xf.r(52.0, 124.0, 24.0, 30.0)));
    keys.push((Key::Fire, Face::Fire, xf.r(24.0, 168.0, 52.0, 42.0)));
    keys.push((Key::Aux, Face::Aux, xf.r(24.0, 220.0, 52.0, 42.0)));
    keys.push((Key::Panic, Face::Panic, xf.r(24.0, 272.0, 52.0, 42.0)));
    let nums = [["1", "2", "3"], ["4", "5", "6"], ["7", "8", "9"], ["*", "0", "#"]];
    for (row, line) in nums.iter().enumerate() {
        for (col, label) in line.iter().enumerate() {
            let key = match *label {
                "*" => Key::Star,
                "#" => Key::Hash,
                d => Key::Digit(d.parse().unwrap_or(0)),
            };
            keys.push((key, Face::Text(*label), xf.r(92.0 + col as f32 * 52.0, 124.0 + row as f32 * 55.0, 42.0, 45.0)));
        }
    }
    let fns = [(Key::Stay, "Stay"), (Key::Away, "Away"), (Key::Chime, "Chime"), (Key::Reset, "Reset"), (Key::Exit, "Exit")];
    for (i, (key, label)) in fns.iter().enumerate() {
        keys.push((*key, Face::Small(*label), xf.r(254.0, 124.0 + i as f32 * 44.0, 52.0, 34.0)));
    }
    for (key, face, rect) in keys {
        let held = input.key(ui, rect, key, engine, audio);
        let hold = view.hold.filter(|h| h.0 == key).map(|h| h.1);
        draw_key(&p, &xf, rect, face, held, lights, hold);
    }

    // Printed help strip and speaker slots.
    let help = dim(Color32::from_rgb(0x7D, 0x82, 0x87), lights);
    p.text(xf.p(165.0, 356.0), Align2::CENTER_CENTER, "(*) select     (#) exit     < > scroll", theme::regular(xf.s(8.2)), help);
    p.text(xf.p(165.0, 372.0), Align2::CENTER_CENTER, "Hold F, A, P or a function key for 2 seconds", theme::regular(xf.s(7.6)), alpha(help, 0.85));
    for i in 0..5 {
        let x = 132.0 + i as f32 * 14.0;
        p.rect_filled(xf.r(x, 394.0, 8.0, 3.0), xf.s(1.5), dim(Color32::from_rgb(0xB3, 0xB7, 0xB2), lights));
    }
}

fn draw_key(p: &Painter, xf: &Xf, rect: Rect, face: Face, held: bool, lights: f32, hold: Option<f32>) {
    let r = xf.s(9.0);
    p.rect_filled(rect.expand(xf.s(1.6)), r + xf.s(1.6), dim(Color32::from_rgb(0xC6, 0xCA, 0xC5), lights));
    if !held {
        paint::shadow(p, rect.translate(Vec2::new(0.0, xf.s(2.0))), r, Vec2::ZERO, xf.s(3.5), Color32::from_black_alpha(60));
    }
    let k = rect.translate(Vec2::new(0.0, if held { xf.s(1.4) } else { 0.0 }));
    let (top, bot) = if held {
        (Color32::from_rgb(0xE2, 0xE4, 0xE1), Color32::from_rgb(0xD2, 0xD5, 0xD1))
    } else {
        (Color32::from_rgb(0xFC, 0xFC, 0xFB), Color32::from_rgb(0xE3, 0xE5, 0xE2))
    };
    paint::grad_rrect(p, k, r, dim(top, lights), dim(bot, lights), Some(Stroke::new(xf.s(0.8), dim(Color32::from_rgb(0xAE, 0xB2, 0xAD), lights))));
    let back = (1.0 - lights).clamp(0.0, 1.0);
    if back > 0.05 {
        paint::glow(p, k.center(), xf.s(24.0), alpha(Color32::from_rgb(0x9C, 0xD8, 0xFF), 0.12 * back));
    }
    let glow_ink = Color32::from_rgb(0xD2, 0xF0, 0xFF);
    let ink = mix(dim(Color32::from_rgb(0x2F, 0x33, 0x37), lights), glow_ink, back * 0.9);
    let c = k.center();
    match face {
        Face::Text(s) => {
            let size = if s == "*" { 20.0 } else { 16.5 };
            let lift = if s == "*" { xf.s(3.0) } else { 0.0 };
            p.text(c + Vec2::new(0.0, lift), Align2::CENTER_CENTER, s, theme::semi(xf.s(size)), ink);
        }
        Face::Small(s) => {
            p.text(c, Align2::CENTER_CENTER, s, theme::semi(xf.s(9.6)), ink);
        }
        Face::Chevron(right) => {
            let d = if right { 1.0 } else { -1.0 };
            let pts = vec![c + Vec2::new(-3.0 * d, -5.0) * xf.scale, c + Vec2::new(3.0 * d, 0.0) * xf.scale, c + Vec2::new(-3.0 * d, 5.0) * xf.scale];
            p.add(egui::Shape::line(pts, Stroke::new(xf.s(1.8), ink)));
        }
        Face::Fire => paint::icon_flame(p, c, xf.s(20.0), mix(dim(Color32::from_rgb(0xC2, 0x3B, 0x2C), lights), glow_ink, back * 0.8)),
        Face::Aux => paint::icon_cross(p, c, xf.s(17.0), mix(dim(Color32::from_rgb(0x2E, 0x6F, 0xA8), lights), glow_ink, back * 0.8)),
        Face::Panic => paint::icon_shield(p, c, xf.s(19.0), mix(dim(Color32::from_rgb(0x3A, 0x3E, 0x43), lights), glow_ink, back * 0.8)),
    }
    if let Some(h) = hold {
        paint::arc(p, c, xf.s(22.0).max(k.width() * 0.62), -FRAC_PI_2, -FRAC_PI_2 + TAU * h, Stroke::new(xf.s(2.2), theme::AMBER));
    }
}

/// The dot matrix itself: each character is 5 x 8 dots, with faint "ghost"
/// pixels where dots are off, like a real reflective LCD.
fn draw_lcd(p: &Painter, xf: &Xf, glass: Rect, lines: &[String; 2], cursor: Option<(usize, usize)>, contrast: f32, t: f64) {
    let cols = 16usize;
    let pitch = (glass.width() - xf.s(12.0)) / (cols as f32 * 6.0 - 1.0);
    let dot = pitch * 0.82;
    let total_h = 19.0 * pitch;
    let x0 = glass.left() + xf.s(6.0);
    let y0 = glass.center().y - total_h * 0.5;
    let k = contrast.clamp(0.0, 1.0);
    let on = mix(Color32::from_rgb(0x4A, 0x58, 0x28), Color32::from_rgb(0x12, 0x1A, 0x08), 0.45 + 0.55 * k);
    let ghost = Color32::from_rgba_unmultiplied(0x1A, 0x24, 0x0C, (4.0 + 7.0 * k) as u8);
    let blink = (t * 2.0).fract() < 0.5;
    for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for col in 0..cols {
            let ch = chars.get(col).copied().unwrap_or(' ');
            let cx = x0 + (col * 6) as f32 * pitch;
            let cy = y0 + (row * 11) as f32 * pitch;
            let show_cursor = blink && cursor == Some((row, col));
            for gx in 0..5 {
                for gy in 0..8 {
                    let lit = lcd_font::dot(ch, gx, gy) || (gy == 7 && show_cursor);
                    let r = Rect::from_min_size(Pos2::new(cx + gx as f32 * pitch, cy + gy as f32 * pitch), Vec2::splat(dot));
                    p.rect_filled(r, dot * 0.18, if lit { on } else { ghost });
                }
            }
        }
    }
}
