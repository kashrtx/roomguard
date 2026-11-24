#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! RoomGuard: a camera based room alarm built around faithful emulations of
//! the DSC PC1550 and PowerSeries PC1832 panels.

mod app;
mod audio;
mod autostart;
mod config;
mod engine;
mod keypads;
mod lcd_font;
mod notify;
mod paint;
mod png;
mod power;
mod theme;
mod update;
mod util;
mod views;
mod vision;

#[cfg(test)]
mod tests;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // --screenshots <folder>: run the demo and save README pictures.
    let shots = args
        .iter()
        .position(|a| a == "--screenshots")
        .map(|i| std::path::PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| "screenshots".to_string())));
    // --minimized: used when RoomGuard opens at sign in.
    let minimized = args.iter().any(|a| a == "--minimized");
    if shots.is_some() {
        app::prepare_demo_data();
    }
    util::log_init();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RoomGuard")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1180.0, 740.0])
        .with_icon(app::window_icon());
    if shots.is_some() {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "RoomGuard",
        options,
        Box::new(move |cc| Ok(Box::new(app::RoomGuardApp::new(cc, shots, minimized)))),
    )
}
