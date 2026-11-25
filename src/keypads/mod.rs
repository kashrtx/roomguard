//! The two keypads, drawn entirely in code with realistic glossy molded
//! plastics, optical LCD depth, recessed LED lenses, and animated flip covers.

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

/// Clicking the keypad takes focus away from any text box so the computer
/// keyboard drives the keypad straight away.
fn surrender_text_focus(ui: &egui::Ui) {
    ui.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

/// Mouse, keyboard, and flip-cover animation state for the keypads.
pub struct KeyInput {
    down: HashSet<Key>,
    kb_down: HashSet<Key>,
    flash: Vec<(Key, f64)>,
    /// 0.0 = closed (right, over keys), 1.0 = open (swung left).
    pub door_anim_1550: f32,
    /// 0.0 = closed (up, over keys), 1.0 = open (flipped down).
    pub door_anim_5500: f32,
    doors_init: bool,
}

impl Default for KeyInput {
    fn default() -> Self {
        KeyInput {
            down: HashSet::new(),
            kb_down: HashSet::new(),
            flash: Vec::new(),
            door_anim_1550: 1.0,
            door_anim_5500: 1.0,
            doors_init: false,
        }
    }
}

impl KeyInput {
    /// Smoothly advances the open/close cover animations toward the saved config.
    pub fn sync_doors(&mut self, ctx: &egui::Context, open_1550: bool, open_5500: bool) {
        let t1550 = if open_1550 { 1.0 } else { 0.0 };
        let t5500 = if open_5500 { 1.0 } else { 0.0 };
        if !self.doors_init {
            self.door_anim_1550 = t1550;
            self.door_anim_5500 = t5500;
            self.doors_init = true;
            return;
        }
        let dt = ctx.input(|i| i.unstable_dt).clamp(0.001, 0.05);
        let speed = 1.0 - (-dt * 11.0).exp();
        let mut moving = false;
        for (cur, tgt) in [(&mut self.door_anim_1550, t1550), (&mut self.door_anim_5500, t5500)] {
            if (*cur - tgt).abs() > 0.002 {
                *cur += (tgt - *cur) * speed;
                moving = true;
            } else {
                *cur = tgt;
            }
        }
        if moving {
            ctx.request_repaint();
        }
    }

    /// One on screen key. Returns true while it's held down.
    pub fn key(&mut self, ui: &mut egui::Ui, rect: Rect, key: Key, engine: &EngineHandle, audio: &AudioHandle) -> bool {
        let id = egui::Id::new(("roomguard-key", key_id(key)));
        let resp = ui
            .interact(rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let down = resp.is_pointer_button_down_on();
        // A quick tap, or the click that also focuses the window, can deliver
        // press and release in the same frame. Then `clicked()` is true but
        // the button is never seen held, so handle that case explicitly.
        let tapped = resp.clicked();
        let was = self.down.contains(&key);
        let now = ui.input(|i| i.time);
        if down && !was {
            self.down.insert(key);
            engine.send(EngineMsg::Key { key, down: true });
            audio.send(AudioCmd::Click);
            surrender_text_focus(ui);
            ui.ctx().request_repaint();
        } else if !down && was {
            self.down.remove(&key);
            engine.send(EngineMsg::Key { key, down: false });
            ui.ctx().request_repaint();
        } else if tapped && !down && !was {
            engine.send(EngineMsg::Key { key, down: true });
            engine.send(EngineMsg::Key { key, down: false });
            audio.send(AudioCmd::Click);
            self.flash.push((key, now));
            surrender_text_focus(ui);
            ui.ctx().request_repaint();
        }
        self.flash.retain(|(_, t)| now - *t < 0.12);
        down || self.kb_down.contains(&key) || self.flash.iter().any(|(k, _)| *k == key)
    }

    /// Typing on the computer keyboard presses keypad keys, as long as no
    /// text box has focus. Returns true if any keypad key was pressed.
    pub fn keyboard(&mut self, ctx: &egui::Context, engine: &EngineHandle, model: Model) -> bool {
        if ctx.memory(|m| m.focused().is_some()) {
            return false;
        }
        let mut any = false;
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
                            any = true;
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
                            any = true;
                        } else {
                            self.kb_down.remove(&k);
                        }
                        engine.send(EngineMsg::Key { key: k, down: pressed });
                    }
                }
                _ => {}
            }
        }
        any
    }
}
