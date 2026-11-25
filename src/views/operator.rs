//! The monitoring center conversation, and the call card when the operator
//! phones you about an alarm.

use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText};

use crate::engine::types::{CallState, CallView, ChatFrom, ChatMsg};
use crate::engine::{EngineHandle, EngineMsg, OpInput, View};
use crate::theme;
use crate::util;
use crate::vision::frame;

/// How many chat messages are drawn at a time.
const CHAT_PAGE: usize = 60;

#[derive(Default)]
pub struct OperatorView {
    passcode: String,
    photos: HashMap<String, Option<egui::TextureHandle>>,
    /// Extra older messages shown after "Show earlier messages".
    older: usize,
}

impl OperatorView {
    pub fn ui(&mut self, ui: &mut egui::Ui, view: &View, engine: &EngineHandle, operator_name: &str, phone_on: bool) {
        ui.horizontal(|ui| {
            ui.label(theme::heading("Monitoring center"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if phone_on {
                    super::chip(ui, "Phone alerts on", theme::GREEN);
                } else {
                    super::chip(ui, "Phone alerts off", theme::FAINT);
                }
            });
        });
        ui.add_space(4.0);
        if let Some(call) = &view.call {
            self.call_card(ui, call, engine);
            ui.add_space(6.0);
        }
        let footer = 30.0;
        let h = (ui.available_height() - footer).max(120.0);
        egui::ScrollArea::vertical()
            .id_salt("operator-chat")
            .max_height(h)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                if view.chat.is_empty() {
                    ui.label(theme::muted(format!(
                        "Nothing yet. When the panel reports an alarm, {} at RoomGuard Central messages you here and calls you to check that you're okay.",
                        operator_name
                    )));
                }
                // Drawing hundreds of old messages every frame slowed the
                // whole window, so only the newest are drawn until you ask.
                let total = view.chat.len();
                if total <= CHAT_PAGE {
                    self.older = 0;
                }
                let shown = (CHAT_PAGE + self.older).min(total);
                if shown < total && ui.small_button(format!("Show earlier messages ({} more)", total - shown)).clicked() {
                    self.older += CHAT_PAGE;
                }
                for msg in view.chat[total - shown..].iter() {
                    self.bubble(ui, msg);
                }
            });
        ui.horizontal(|ui| {
            if !view.notify_status.is_empty() {
                ui.add(egui::Label::new(theme::muted(view.notify_status.clone())).truncate())
                    .on_hover_text(&view.notify_status);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !view.chat.is_empty() && ui.small_button("Clear").clicked() {
                    engine.send(EngineMsg::Op(OpInput::Clear));
                }
            });
        });
    }

    fn call_card(&mut self, ui: &mut egui::Ui, call: &CallView, engine: &EngineHandle) {
        theme::card().fill(Color32::from_rgb(0x1D, 0x31, 0x28)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match call.state {
                CallState::Ringing => {
                    ui.label(RichText::new(format!("{} is calling", call.caller)).font(theme::semi(15.0)));
                    ui.label(theme::muted("They're calling about the alarm. Answer to talk to them."));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if super::accent_button(ui, "Answer", theme::GREEN).clicked() {
                            engine.send(EngineMsg::Op(OpInput::Answer));
                        }
                        if super::accent_button(ui, "Decline", theme::RED).clicked() {
                            engine.send(EngineMsg::Op(OpInput::Decline));
                        }
                    });
                }
                CallState::Talking => {
                    let secs = ((util::now_ms() - call.since_ms).max(0) / 1000) as u64;
                    ui.label(RichText::new(format!("On a call with {}   {}:{:02}", call.caller, secs / 60, secs % 60)).font(theme::semi(14.0)));
                    if call.ask_passcode {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.passcode).password(true).hint_text("Your passcode").desired_width(155.0));
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if (ui.button("Tell them").clicked() || enter) && !self.passcode.trim().is_empty() {
                                let word = std::mem::take(&mut self.passcode);
                                engine.send(EngineMsg::Op(OpInput::Passcode(word)));
                            }
                        });
                    }
                    if call.ask_dispatch {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            if super::accent_button(ui, "Send help", theme::RED).clicked() {
                                engine.send(EngineMsg::Op(OpInput::Dispatch(true)));
                            }
                            if ui.button("It's a false alarm").clicked() {
                                engine.send(EngineMsg::Op(OpInput::Dispatch(false)));
                            }
                        });
                    }
                }
            }
        });
    }

    fn photo(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.photos.get(path) {
            return t.clone();
        }
        // Each photo is a texture on the graphics card. A long chat history
        // shouldn't keep every one of them loaded forever; the visible ones
        // reload from disk in a moment.
        if self.photos.len() >= 40 {
            self.photos.clear();
        }
        let tex = std::fs::read(path).ok().and_then(|b| frame::decode_jpeg_file(&b)).map(|img| {
            let ci = egui::ColorImage::from_rgb([img.w, img.h], &img.data);
            ctx.load_texture(format!("photo-{}", path), ci, egui::TextureOptions::LINEAR)
        });
        self.photos.insert(path.to_string(), tex.clone());
        tex
    }

    fn bubble(&mut self, ui: &mut egui::Ui, msg: &ChatMsg) {
        match msg.from {
            ChatFrom::System => {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(format!("{}   {}", util::fmt_when(msg.ts), msg.text)).size(11.5).color(theme::FAINT));
                });
            }
            ChatFrom::Operator | ChatFrom::Me => {
                let me = msg.from == ChatFrom::Me;
                let layout = if me { egui::Layout::right_to_left(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
                let max_w = (ui.available_width() * 0.85).max(120.0);
                let photo = msg.photo.as_ref().and_then(|p| self.photo(ui.ctx(), p));
                ui.with_layout(layout, |ui| {
                    let fill = if me { Color32::from_rgb(0x2B, 0x4D, 0x3F) } else { theme::CARD_HI };
                    egui::Frame::none()
                        .fill(fill)
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                        .show(ui, |ui| {
                            ui.set_max_width(max_w);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&msg.text).color(theme::TEXT));
                                if let Some(tex) = &photo {
                                    let size = tex.size_vec2();
                                    let w = 220.0f32.min(size.x).min(max_w - 20.0).max(60.0);
                                    let h = w * size.y / size.x.max(1.0);
                                    ui.add(egui::Image::new((tex.id(), egui::vec2(w, h))).rounding(6.0));
                                }
                                ui.label(RichText::new(util::fmt_when(msg.ts)).size(10.5).color(theme::FAINT));
                            });
                        });
                });
            }
        }
        ui.add_space(4.0);
    }
}
