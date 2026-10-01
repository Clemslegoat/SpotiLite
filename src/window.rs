//! The window, without a GPU: winit for the window and input, egui for the
//! interface, and a CPU rasterizer that fills a plain pixel buffer (softbuffer)
//! displayed by the system. No OpenGL or Direct3D driver is loaded in the process,
//! which is often the biggest memory cost of a small desktop application.
//!
//! The interface is only redrawn when something happens (input, backend event,
//! or once per second while music plays), so the CPU stays idle.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use egui::{ViewportId, ViewportInfo};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Theme, Window, WindowId};

pub struct Options {
    pub title: &'static str,
    pub inner_size: [f32; 2],
    pub min_size: [f32; 2],
    pub icon: egui::IconData,
}

/// What the window runs.
pub trait App {
    /// Called before each frame, and on its own while the window is minimized
    /// (backend events and media keys keep being handled).
    fn logic(&mut self, ctx: &egui::Context);
    fn ui(&mut self, ui: &mut egui::Ui);
    fn on_exit(&mut self);
}

/// Wakes the event loop up for a repaint at `when`.
struct Wake {
    when: Instant,
}

pub fn run<A: App, F: FnOnce(&egui::Context, &Window) -> A>(
    options: Options,
    create: F,
) -> Result<(), String> {
    let event_loop = EventLoop::<Wake>::with_user_event().build().map_err(|e| e.to_string())?;
    let proxy: EventLoopProxy<Wake> = event_loop.create_proxy();
    let ctx = egui::Context::default();
    // Repaint requests can come from any thread (the backend, media keys).
    ctx.set_request_repaint_callback(move |info| {
        let _ = proxy.send_event(Wake { when: Instant::now() + info.delay });
    });
    let mut runner = Runner { options, ctx, create: Some(create), window: None, next_repaint: None };
    event_loop.run_app(&mut runner).map_err(|e| e.to_string())
}

struct Runner<A, F> {
    options: Options,
    ctx: egui::Context,
    create: Option<F>,
    window: Option<Running<A>>,
    next_repaint: Option<Instant>,
}

struct Running<A> {
    // Declared before the window: dropped first.
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _display: softbuffer::Context<Rc<Window>>,
    input: egui_winit::State,
    renderer: EguiSoftwareRender,
    info: ViewportInfo,
    app: A,
    window: Rc<Window>,
}

impl<A: App, F: FnOnce(&egui::Context, &Window) -> A> ApplicationHandler<Wake> for Runner<A, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let Some(create) = self.create.take() else { return };
        match self.open(event_loop, create) {
            Ok(running) => {
                running.window.request_redraw();
                self.window = Some(running);
            }
            Err(e) => {
                log::error!("window: {e}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, wake: Wake) {
        let Some(running) = &mut self.window else { return };
        if running.window.is_minimized() == Some(true) {
            // Nothing to draw, but events must still be handled.
            running.app.logic(&self.ctx);
            return;
        }
        if wake.when <= Instant::now() {
            running.window.request_redraw();
        } else {
            self.next_repaint = Some(self.next_repaint.map_or(wake.when, |t| t.min(wake.when)));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.window else { return };
        match &event {
            WindowEvent::CloseRequested => {
                running.app.on_exit();
                running.window.set_visible(false);
                self.window = None;
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = running.paint(&self.ctx) {
                    log::warn!("paint: {e}");
                }
                return;
            }
            _ => {}
        }
        let response = running.input.on_window_event(&running.window, &event);
        if response.repaint
            || matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. })
        {
            running.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_repaint {
            Some(when) if when <= Instant::now() => {
                self.next_repaint = None;
                if let Some(running) = &self.window {
                    running.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(when) => event_loop.set_control_flow(ControlFlow::WaitUntil(when)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl<A: App, F: FnOnce(&egui::Context, &Window) -> A> Runner<A, F> {
    fn open(&self, event_loop: &ActiveEventLoop, create: F) -> Result<Running<A>, String> {
        let builder = egui::ViewportBuilder::default()
            .with_title(self.options.title)
            .with_app_id("spotilite")
            .with_inner_size(self.options.inner_size)
            .with_min_inner_size(self.options.min_size)
            .with_icon(Arc::new(self.options.icon.clone()));
        let window =
            Rc::new(egui_winit::create_window(&self.ctx, event_loop, &builder).map_err(|e| e.to_string())?);
        // Dark title bar on Windows 10/11, to match the black window.
        window.set_theme(Some(Theme::Dark));
        let display = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = softbuffer::Surface::new(&display, window.clone()).map_err(|e| e.to_string())?;
        let input = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            Some(Theme::Dark),
            None,
        );
        let mut info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &self.ctx, &window, true);
        let app = create(&self.ctx, &window);
        // softbuffer pixels are 0x00RRGGBB, i.e. B, G, R, X in memory. The rasterizer
        // draws straight into them: no intermediate canvas kept in memory.
        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra)
            .with_caching(false)
            .with_allow_raster_opt(true)
            .with_convert_tris_to_rects(true);
        Ok(Running { surface, _display: display, input, renderer, info, app, window })
    }
}

impl<A: App> Running<A> {
    fn paint(&mut self, ctx: &egui::Context) -> Result<(), String> {
        egui_winit::update_viewport_info(&mut self.info, ctx, &self.window, false);
        let mut raw_input = self.input.take_egui_input(&self.window);
        raw_input.viewports.insert(ViewportId::ROOT, self.info.clone());
        let app = &mut self.app;
        let mut output = ctx.run_ui(raw_input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        self.input.handle_platform_output(&self.window, output.platform_output);
        for (id, viewport) in output.viewport_output {
            if id == ViewportId::ROOT {
                let mut actions = Vec::new();
                egui_winit::process_viewport_commands(
                    ctx,
                    &mut self.info,
                    viewport.commands,
                    &self.window,
                    &mut actions,
                );
            }
        }

        // Texture changes (glyphs, covers) must be applied even when nothing is drawn.
        let mut textures = std::mem::take(&mut output.textures_delta);
        let size = self.window.inner_size();
        let result = match (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            (Some(width), Some(height)) => {
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                self.draw(width, height, &primitives, &textures, output.pixels_per_point)
            }
            // Minimized: nothing to show.
            _ => {
                self.renderer.apply_textures(&textures);
                Ok(())
            }
        };
        textures.clear();
        result
    }

    fn draw(
        &mut self,
        width: NonZeroU32,
        height: NonZeroU32,
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<(), String> {
        self.surface.resize(width, height).map_err(|e| e.to_string())?;
        let mut buffer = self.surface.buffer_mut().map_err(|e| e.to_string())?;
        // Pure black background (AMOLED).
        buffer.fill(0);
        let pixels: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buffer);
        let mut target = BufferMutRef::new(pixels, width.get() as usize, height.get() as usize);
        self.renderer.render(&mut target, primitives, textures, pixels_per_point);
        buffer.present().map_err(|e| e.to_string())
    }
}
