//! The two keypads, drawn entirely in code. They're original designs, built
//! to match the controls of the panels they emulate exactly: the same keys,
//! the same indicator lights and the same screen behaviour.

pub mod classic;
pub mod lcd;

use std::collections::HashSet;

use eframe::egui::{self, Pos2, Rect, Vec2};

use crate::audio::{AudioCmd, AudioHandle};
use crate::engine::types::{Key, Model};
use crate::engine::{EngineHandle, EngineMsg};

/// Maps keypad design units to screen points.
#[derive(Clone, Copy)]
pub struct Xf {
    pub origin: Pos2,
    pub scale: f32,
}

impl Xf {
    pub fn fit(area: Rect, size: Vec2, margin: f32) -> Xf {
        let s = ((area.width() - margin * 2.0) / size.x)
            .min((area.height() - margin * 2.0) / size.y)
            .clamp(0.3, 3.0);
        Xf { origin: area.center() - size * s * 0.5, scale: s }
    }
    pub fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.scale
    }
    pub fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.p(x, y), Vec2::new(w, h) * self.scale)
    }
    pub fn s(&self, v: f32) -> f32 {
        v * self.scale
    }
}

fn key_id(k: Key) -> u32 {
    match k {
        Key::Digit(d) => d as u32,
        Key::Star => 10,
        Key::Hash => 11,
        Key::Fire => 12,
        Key::Aux => 13,
        Key::Panic => 14,
        Key::Left => 15,
        Key::Right => 16,
        Key::Stay => 17,
        Key::Away => 18,
        Key::Chime => 19,
        Key::Reset => 20,
        Key::Exit => 21,
    }
}

/// Mouse and keyboard input for the keypad.
#[derive(Default)]
pub struct KeyInput {
    down: HashSet<Key>,
    kb_down: HashSet<Key>,
    flash: Vec<(Key, f64)>,
}

impl KeyInput {
    /// One on screen key. Returns true while it's held down.
    pub fn key(&mut self, ui: &mut egui::Ui, rect: Rect, key: Key, engine: &EngineHandle, audio: &AudioHandle) -> bool {
        let id = egui::Id::new(("roomguard-key", key_id(key)));
        let resp = ui
            .interact(rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let down = resp.is_pointer_button_down_on();
        let was = self.down.contains(&key);
        if down && !was {
            self.down.insert(key);
            engine.send(EngineMsg::Key { key, down: true });
            audio.send(AudioCmd::Click);
        }
        if !down && was {
            self.down.remove(&key);
            engine.send(EngineMsg::Key { key, down: false });
        }
        let now = ui.input(|i| i.time);
        self.flash.retain(|(_, t)| now - *t < 0.12);
        down || self.kb_down.contains(&key) || self.flash.iter().any(|(k, _)| *k == key)
    }

    /// Typing on the computer keyboard presses keypad keys, as long as no
    /// text box has focus. Digits, * and # type; F, A and P are the
    /// emergency keys (hold them); Enter or Esc is [#]; on the LCD keypad the
    /// arrow keys scroll and F1 to F5 are Stay, Away, Chime, Reset and Exit.
    pub fn keyboard(&mut self, ctx: &egui::Context, engine: &EngineHandle, model: Model) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (events, now) = ctx.input(|i| (i.events.clone(), i.time));
        for ev in events {
            match ev {
                egui::Event::Text(t) => {
                    for ch in t.chars() {
                        let k = match ch {
                            '0'..='9' => Some(Key::Digit(ch as u8 - b'0')),
                            '*' => Some(Key::Star),
                            '#' => Some(Key::Hash),
                            _ => None,
                        };
                        if let Some(k) = k {
                            engine.send(EngineMsg::Key { key: k, down: true });
                            engine.send(EngineMsg::Key { key: k, down: false });
                            self.flash.push((k, now));
                        }
                    }
                }
                egui::Event::Key { key, pressed, repeat, .. } => {
                    if repeat {
                        continue;
                    }
                    let lcd = model == Model::Pc1832;
                    let k = match key {
                        egui::Key::F => Some(Key::Fire),
                        egui::Key::A => Some(Key::Aux),
                        egui::Key::P => Some(Key::Panic),
                        egui::Key::Enter | egui::Key::Escape => Some(Key::Hash),
                        egui::Key::ArrowLeft if lcd => Some(Key::Left),
                        egui::Key::ArrowRight if lcd => Some(Key::Right),
                        egui::Key::F1 if lcd => Some(Key::Stay),
                        egui::Key::F2 if lcd => Some(Key::Away),
                        egui::Key::F3 if lcd => Some(Key::Chime),
                        egui::Key::F4 if lcd => Some(Key::Reset),
                        egui::Key::F5 if lcd => Some(Key::Exit),
                        _ => None,
                    };
                    if let Some(k) = k {
                        if pressed {
                            self.kb_down.insert(k);
                        } else {
                            self.kb_down.remove(&k);
                        }
                        engine.send(EngineMsg::Key { key: k, down: pressed });
                    }
                }
                _ => {}
            }
        }
    }
}
