//! The window: winit for the window and events, wgpu (through egui-wgpu) for drawing, egui-winit for input.
//!
//! This is deliberately thin. It owns the window and the GPU surface, forwards window events to egui and
//! to the app (focus lost, close requested), runs one egui frame per redraw and paints it.

use crate::app::App;
use egui::ViewportId;
use egui_wgpu::winit::Painter;
use egui_wgpu::{wgpu, RendererOptions, WgpuConfiguration};
use pg_ui_model::types::UiEvent;
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

struct Gui {
    window: Arc<Window>,
    painter: Painter,
    state: egui_winit::State,
}

struct Shell {
    app: App,
    ctx: egui::Context,
    gui: Option<Gui>,
    vsync: bool,
    mode: String,
}

impl Shell {
    fn apply_window_mode(&mut self) {
        let wanted = self
            .app
            .controller
            .setting_text("ui.window_mode")
            .unwrap_or("windowed")
            .to_owned();
        if wanted == self.mode {
            return;
        }
        if let Some(g) = &self.gui {
            g.window.set_fullscreen(match wanted.as_str() {
                "fullscreen" | "borderless" => Some(Fullscreen::Borderless(None)),
                _ => None,
            });
        }
        self.mode = wanted;
    }

    fn create_gui(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("Playground")
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(800.0, 560.0));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("could not create the window: {e}");
                el.exit();
                return;
            }
        };
        let mut config = WgpuConfiguration::default();
        config.surface.present_mode = if self.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        let mut painter = pollster::block_on(Painter::new(
            self.ctx.clone(),
            config,
            false,
            RendererOptions::default(),
        ));
        if let Err(e) =
            pollster::block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
        {
            eprintln!("could not set up the graphics device: {e}");
            el.exit();
            return;
        }
        let gpu = painter
            .render_state()
            .map(|rs| crate::game_view::install_gpu(&rs))
            .is_some();
        self.app.set_gpu(gpu);
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            window.theme(),
            painter.max_texture_side(),
        );
        self.gui = Some(Gui {
            window,
            painter,
            state,
        });
        self.mode = String::new();
        self.apply_window_mode();
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let Some(mut gui) = self.gui.take() else {
            return;
        };
        let raw = gui.state.take_egui_input(&gui.window);
        let mut out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        gui.state
            .handle_platform_output(&gui.window, out.platform_output.clone());
        let ppp = out.pixels_per_point;
        let prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), ppp);
        gui.painter.paint_and_update_textures(
            ViewportId::ROOT,
            ppp,
            [0.08, 0.09, 0.11, 1.0],
            &prims,
            &mut out.textures_delta,
            Vec::new(),
            &gui.window,
        );
        self.gui = Some(gui);
        self.apply_window_mode();
        if self.app.wants_exit() {
            self.app.shutdown();
            el.exit();
        }
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gui.is_none() {
            self.create_gui(el);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gui) = self.gui.as_mut() else {
            return;
        };
        let response = gui.state.on_window_event(&gui.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                // The save finishes before the window goes.
                self.app.dispatch(UiEvent::CloseRequested);
                self.app.shutdown();
                el.exit();
            }
            WindowEvent::Focused(gained) => {
                self.app.dispatch(if gained {
                    UiEvent::FocusGained
                } else {
                    UiEvent::FocusLost
                });
            }
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    gui.painter.on_window_resized(ViewportId::ROOT, w, h);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(el),
            _ => {}
        }
        if response.repaint {
            if let Some(g) = &self.gui {
                g.window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(g) = &self.gui {
            g.window.request_redraw();
        }
    }
}

/// Opens the window and runs until the player quits.
pub fn run(app: App, vsync: bool) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("no event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut shell = Shell {
        app,
        ctx: egui::Context::default(),
        gui: None,
        vsync,
        mode: String::new(),
    };
    event_loop
        .run_app(&mut shell)
        .map_err(|e| format!("the window loop failed: {e}"))?;
    shell.app.shutdown();
    Ok(())
}
