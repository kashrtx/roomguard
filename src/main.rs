// Release builds open no console window. Test builds keep their console so
// `cargo test --release` can print its results.
#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

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
mod single;
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
    // --restarted: started by an update or by Start over, so it waits for
    // the old copy to finish closing.
    let restarted = args.iter().any(|a| a == "--restarted");
    if shots.is_some() {
        app::prepare_demo_data();
    }
    util::log_init();
    // Only one copy at a time. Opening it again brings the open window to the
    // front. Screenshot runs use their own throwaway data, so they skip this.
    let guard = if shots.is_some() {
        None
    } else {
        match single::acquire(restarted) {
            single::Start::First(g) => Some(g),
            single::Start::AlreadyRunning => return Ok(()),
        }
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RoomGuard")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1180.0, 740.0])
        .with_icon(app::window_icon());
    if shots.is_some() {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "RoomGuard",
        options,
        Box::new(move |cc| {
            if let Some(g) = &guard {
                g.attach(&cc.egui_ctx);
            }
            Ok(Box::new(app::RoomGuardApp::new(cc, shots, minimized)) as Box<dyn eframe::App>)
        }),
    );
    // Anything saved during shutdown is still on its way to the disk.
    util::flush_saves(std::time::Duration::from_secs(4));
    result
}
