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
mod window;

fn main() {
    let paths = config::Paths::detect();
    logger::init(&paths.log_file());
    log::info!("SpotiLite {} starting", env!("CARGO_PKG_VERSION"));
    let settings = config::Settings::load(&paths);

    let options = window::Options {
        title: "SpotiLite",
        inner_size: settings.window_size,
        min_size: [600.0, 420.0],
        icon: window_icon(),
    };
    let result = window::run(options, move |ctx, window| {
        let app = ui::App::new(ctx, window, paths, settings);
        #[cfg(debug_assertions)]
        let app = app.with_demo();
        app
    });
    if let Err(e) = result {
        log::error!("{e}");
    }
}

/// The icon is drawn in code (three white level-meter bars on a black tile with a
/// thin grey ring, visible on dark taskbars), so the executable carries no image file.
pub fn window_icon() -> egui::IconData {
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let tile = [0x00, 0x00, 0x00];
    let ring = [0x4a, 0x4a, 0x4a];
    let ink = [0xff, 0xff, 0xff];
    let radius = 14.0f32;
    let bars = [(18.0, 22.0), (29.0, 34.0), (40.0, 16.0)];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Signed distance to the rounded square (negative inside).
            let dx = (radius - fx).max(fx - (size as f32 - radius)).max(0.0);
            let dy = (radius - fy).max(fy - (size as f32 - radius)).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - radius;
            let alpha = (0.5 - dist).clamp(0.0, 1.0);
            let on_bar =
                bars.iter().any(|&(bx, h)| (fx - bx - 3.0).abs() <= 3.0 && fy >= 46.0 - h && fy <= 46.0);
            let color = if on_bar {
                ink
            } else if dist > -2.0 {
                ring
            } else {
                tile
            };
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (alpha * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: size, height: size }
}
