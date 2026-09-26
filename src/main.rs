// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! SpotiLite: a native, lightweight Spotify Premium client.

mod backend;
mod config;
mod logger;
mod media;
mod model;
mod queue;
mod sys;
mod ui;

use std::sync::Arc;

use eframe::egui;

fn main() -> eframe::Result {
    let paths = config::Paths::detect();
    logger::init(&paths.log_file());
    log::info!("SpotiLite {} starting", env!("CARGO_PKG_VERSION"));
    let settings = config::Settings::load(&paths);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SpotiLite")
            .with_app_id("spotilite")
            .with_inner_size(settings.window_size)
            .with_min_inner_size([560.0, 380.0])
            .with_icon(Arc::new(window_icon())),
        renderer: eframe::Renderer::Glow,
        // No MSAA, depth or stencil buffers: egui antialiases in its tessellator,
        // and every extra buffer costs video memory.
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        dithering: false,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "SpotiLite",
        options,
        Box::new(move |cc| {
            let app = ui::App::new(cc, paths, settings);
            #[cfg(debug_assertions)]
            let app = app.with_demo();
            Ok(Box::new(app))
        }),
    )
}

/// The icon is drawn in code (three bars of a level meter on an amber tile), so
/// the executable carries no image file.
pub fn window_icon() -> egui::IconData {
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let accent = [0xe8, 0xb0, 0x4b];
    let ink = [0x16, 0x12, 0x0a];
    let radius = 14.0f32;
    let bars = [(18.0, 22.0), (29.0, 34.0), (40.0, 16.0)];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square coverage (1 px antialiasing).
            let dx = (radius - fx).max(fx - (size as f32 - radius)).max(0.0);
            let dy = (radius - fy).max(fy - (size as f32 - radius)).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - radius;
            let alpha = (0.5 - dist).clamp(0.0, 1.0);
            let on_bar =
                bars.iter().any(|&(bx, h)| (fx - bx - 3.0).abs() <= 3.0 && fy >= 46.0 - h && fy <= 46.0);
            let color = if on_bar { ink } else { accent };
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (alpha * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: size, height: size }
}
