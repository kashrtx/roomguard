// Release builds open no console window. Test builds keep their console so
// `cargo test --release` can print its results.
#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

//! RoomGuard: a camera based room alarm built around faithful emulations of
//! the DSC PC1550 and PowerSeries PC1832 panels.

mod app;
mod audio;
mod autostart;
mod config;
mod desktop;
mod engine;
mod keypads;
mod lcd_font;
mod notify;
mod home_assistant;
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    } else {
        desktop::ensure_linux_icon();
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
    // Draw on the graphics card through wgpu: Direct3D 12 on Windows, Vulkan
    // on Linux, Metal on macOS. If that can't start on this computer, fall
    // back to OpenGL. "Use OpenGL" in Settings, or ROOMGUARD_RENDERER=gl,
    // picks OpenGL from the start.
    let want_gl = config::AppConfig::load().ui.opengl
        || std::env::var("ROOMGUARD_RENDERER").map_or(false, |v| v.eq_ignore_ascii_case("gl") || v.eq_ignore_ascii_case("opengl"));
    let started = Arc::new(AtomicBool::new(false));
    let first = if want_gl { eframe::Renderer::Glow } else { eframe::Renderer::Wgpu };
    let mut result = run(first, shots.clone(), minimized, guard.clone(), started.clone());
    if !want_gl && result.is_err() && !started.load(Ordering::SeqCst) {
        if let Err(e) = &result {
            util::log(&format!("the GPU renderer couldn't start ({}), drawing with OpenGL instead", e));
        }
        result = run(eframe::Renderer::Glow, shots, minimized, guard, started);
    }
    // Anything saved during shutdown is still on its way to the disk.
    util::flush_saves(std::time::Duration::from_secs(4));
    result
}

fn run(
    renderer: eframe::Renderer,
    shots: Option<std::path::PathBuf>,
    minimized: bool,
    guard: Option<single::Guard>,
    started: Arc<AtomicBool>,
) -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(util::app_name())
        .with_app_id(if update::experimental_build().is_some() { "roomguard-experimental" } else { "roomguard" })
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1180.0, 740.0])
        .with_icon(app::window_icon());
    if shots.is_some() {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions { viewport, renderer, ..Default::default() };
    eframe::run_native(
        "RoomGuard",
        options,
        Box::new(move |cc| {
            started.store(true, Ordering::SeqCst);
            if let Some(g) = &guard {
                g.attach(&cc.egui_ctx);
            }
            let how = if cc.wgpu_render_state.is_some() { "the GPU through wgpu" } else { "OpenGL" };
            util::log(&format!("drawing with {}", how));
            Ok(Box::new(app::RoomGuardApp::new(cc, shots, minimized)) as Box<dyn eframe::App>)
        }),
    )
}
