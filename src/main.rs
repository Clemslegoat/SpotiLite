// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! SpotiLite: a native, lightweight Spotify Premium client.

mod backend;
mod config;
mod logger;
mod logo;
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
        icon: logo::window_icon(),
    };
    let result = window::run(options, move |ctx, window| {
        let app = ui::App::new(ctx, window, paths, settings);
        #[cfg(debug_assertions)]
        let app = app.with_demo(ctx);
        app
    });
    if let Err(e) = result {
        log::error!("{e}");
    }
}
