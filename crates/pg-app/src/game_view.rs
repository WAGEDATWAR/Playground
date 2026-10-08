//! The in-game map view: the sprite renderer's scene drawn through a wgpu paint callback, with pan, zoom and
//! click to focus a resident.
//!
//! The geometry (camera, culling, pixel-art zoom, sprites, atlas, interpolation) lives in `pg-render` and is
//! unit tested there; this module reads the snapshot, runs the mouse, and hands the draw list to the GPU
//! layer. When no GPU layer is registered (the headless smoke test) the older painter-based drawing is used
//! so the same flows still run.

use egui::{Color32, Pos2, Sense};
use egui_wgpu::wgpu;
use pg_core::id::EntityId;
use pg_core::map::{Dir4, MapData, Tile, FLOOR, GRASS, ROAD, SAND, SIDEWALK, SURFACE_NONE, WATER};
use pg_render::gpu::GpuScene;
use pg_render::{
    build_scene, pick_pawn, snap_zoom, snapped, tile_rects, Atlas, Camera, Facing, Interpolator,
    PawnDraw, Quad, Rgb, Terrain, TileKind, TileSource,
};
use pg_runtime::snapshot::RenderSnapshot;
use pg_ui_model::palette::{pawn_color, tile_color};
use std::time::Instant;

/// The simulation's terrain, surfaces and walls as the renderer's tile kinds.
struct MapTerrain<'a>(&'a MapData);

impl Terrain for MapTerrain<'_> {
    fn width(&self) -> i32 {
        self.0.width()
    }

    fn height(&self) -> i32 {
        self.0.height()
    }

    fn kind(&self, x: i32, y: i32) -> TileKind {
        let t = Tile::new(x, y);
        match self.0.surface_at(t) {
            Some(ROAD) => return TileKind::Road,
            Some(SIDEWALK) => return TileKind::Sidewalk,
            Some(FLOOR) => return TileKind::Floor,
            Some(s) if s != SURFACE_NONE => return TileKind::Unknown,
            _ => {}
        }
        match self.0.terrain_at(t) {
            Some(WATER) => TileKind::Water,
            _ if self.0.is_blocked(t) => TileKind::Building,
            Some(GRASS) => TileKind::Grass,
            Some(SAND) => TileKind::Sand,
            _ => TileKind::Unknown,
        }
    }
}

/// The painter fallback's tile colours.
struct MapColors<'a>(&'a MapData);

impl TileSource for MapColors<'_> {
    fn width(&self) -> i32 {
        self.0.width()
    }

    fn height(&self) -> i32 {
        self.0.height()
    }

    fn color(&self, x: i32, y: i32) -> Rgb {
        tile_color(self.0, Tile::new(x, y))
    }
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn facing_of(d: Dir4) -> Facing {
    match d {
        Dir4::N => Facing::N,
        Dir4::E => Facing::E,
        Dir4::S => Facing::S,
        Dir4::W => Facing::W,
    }
}

/// Draws the scene inside egui's render pass.
struct SceneCallback {
    quads: Vec<Quad>,
    scale: f32,
    view_px: [f32; 2],
}

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<GpuScene>() {
            gpu.prepare(device, queue, &self.quads, self.scale, self.view_px);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(gpu) = resources.get::<GpuScene>() {
            gpu.paint(pass);
        }
    }
}

/// Registers the sprite renderer with egui's wgpu state; call once after the window's device exists.
pub fn install_gpu(state: &egui_wgpu::RenderState) {
    let scene = GpuScene::new(
        &state.device,
        &state.queue,
        state.target_format,
        &Atlas::standard(),
    );
    state.renderer.write().callback_resources.insert(scene);
}

/// What the renderer did last frame, for the overlay.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RenderStats {
    pub quads: usize,
    pub visible_tiles: usize,
    pub draw_calls: usize,
    pub zoom: f32,
}

/// Scroll points that make one zoom step (a wheel notch is about this much).
const SCROLL_PER_STEP: f32 = 40.0;

pub struct GameView {
    scroll: f32,
    camera: Camera,
    fitted: Option<EntityId>,
    /// The player moved the camera: stop re-framing the map as the window changes.
    moved: bool,
    gpu: bool,
    atlas: Atlas,
    interp: Interpolator,
    seen_tick: u64,
    arrived: Instant,
    /// Observed simulation ticks per real second (smoothed), for gliding between snapshots.
    rate: f64,
    selected: Option<EntityId>,
    stats: RenderStats,
}

impl Default for GameView {
    fn default() -> GameView {
        GameView {
            scroll: 0.0,
            camera: Camera::default(),
            fitted: None,
            moved: false,
            gpu: false,
            atlas: Atlas::standard(),
            interp: Interpolator::new(),
            seen_tick: 0,
            arrived: Instant::now(),
            rate: 0.0,
            selected: None,
            stats: RenderStats::default(),
        }
    }
}

impl GameView {
    /// Forgets the camera, so the next world is framed from scratch.
    pub fn reset(&mut self) {
        self.fitted = None;
        self.moved = false;
        self.selected = None;
        self.interp = Interpolator::new();
        self.seen_tick = 0;
    }

    /// The sprite renderer is registered with the GPU; use it instead of the painter.
    pub fn set_gpu(&mut self, on: bool) {
        self.gpu = on;
    }

    /// Selects a resident as if it had been clicked.
    pub fn select(&mut self, id: Option<EntityId>) {
        self.selected = id;
    }

    /// The resident the player clicked, if any.
    pub fn selected(&self) -> Option<EntityId> {
        self.selected
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// Tracks the arrival of snapshots so residents glide between ticks.
    fn track(&mut self, snap: &RenderSnapshot) {
        let now = Instant::now();
        if snap.tick != self.seen_tick {
            let dt = now.duration_since(self.arrived).as_secs_f64();
            if dt > 0.0 && snap.tick > self.seen_tick && self.seen_tick != 0 {
                let observed = (snap.tick - self.seen_tick) as f64 / dt;
                self.rate = if self.rate == 0.0 {
                    observed
                } else {
                    self.rate * 0.8 + observed * 0.2
                };
            } else if snap.tick < self.seen_tick {
                self.rate = 0.0;
            }
            self.seen_tick = snap.tick;
            self.arrived = now;
            for p in &snap.pawns {
                self.interp.observe(
                    p.id.counter() as u64,
                    (p.tile.x, p.tile.y),
                    snap.tick as f64,
                );
            }
            let alive: Vec<u64> = snap.pawns.iter().map(|p| p.id.counter() as u64).collect();
            self.interp.retain(&alive);
        } else if self.arrived.elapsed().as_secs_f64() > 1.0 {
            // Nothing new for a while (paused): stand still.
            self.rate = 0.0;
        }
    }

    /// The simulation tick being drawn: the last snapshot plus the time since, at the observed rate, never
    /// more than a few ticks ahead.
    fn render_tick(&self, snap: &RenderSnapshot) -> f64 {
        let ahead = (self.arrived.elapsed().as_secs_f64() * self.rate).min(6.0);
        snap.tick as f64 + ahead
    }

    /// Paints the first map of `snap` into the space `ui` has left.
    pub fn show(&mut self, ui: &mut egui::Ui, snap: &RenderSnapshot) {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 24, 30));
        let Some(map) = snap.maps.first() else {
            return;
        };
        let (vw, vh) = (rect.width(), rect.height());
        if (self.fitted != Some(map.id) || !self.moved) && vw > 1.0 && vh > 1.0 {
            self.camera = Camera::fit(map.width(), map.height(), vw, vh);
            self.fitted = Some(map.id);
        }
        // Drag to pan; the wheel steps the zoom (whole multiples of the art size once zoomed in).
        if response.dragged() {
            let d = response.drag_delta();
            self.camera.pan_pixels(d.x, d.y);
            self.moved = true;
        }
        if let Some(hover) = response.hover_pos() {
            // egui spreads one wheel notch over several frames; collect it and take one zoom step per notch
            // (otherwise every frame of the same notch would step again).
            self.scroll += ui.input(|i| i.smooth_scroll_delta.y);
            while self.scroll.abs() >= SCROLL_PER_STEP {
                let up = self.scroll > 0.0;
                self.scroll -= SCROLL_PER_STEP.copysign(self.scroll);
                let z = self.camera.zoom;
                let target = if up {
                    if z < 16.0 {
                        (z * 1.25).min(16.0)
                    } else {
                        snap_zoom(z) + 16.0
                    }
                } else if z <= 16.0 {
                    z / 1.25
                } else {
                    snap_zoom(z) - 16.0
                };
                self.camera.zoom_at(
                    hover.x - rect.min.x,
                    hover.y - rect.min.y,
                    target / z,
                    vw,
                    vh,
                );
                self.camera.zoom = snap_zoom(self.camera.zoom);
                self.moved = true;
            }
        } else {
            self.scroll = 0.0;
        }
        self.camera.clamp_to(map.width(), map.height());
        self.track(snap);

        let step = f64::from(snap.step_ticks.max(1));
        let now = self.render_tick(snap);
        let pawns: Vec<PawnDraw> = snap
            .pawns
            .iter()
            .filter(|p| p.map == map.id)
            .filter_map(|p| {
                let id = p.id.counter() as u64;
                let placed = self.interp.sample(id, now, step)?;
                Some(PawnDraw {
                    id,
                    x: placed.x,
                    y: placed.y,
                    facing: facing_of(p.facing),
                    moving: placed.moving || p.walking && placed.progress < 1.0,
                    progress: placed.progress,
                    selected: self.selected == Some(p.id),
                })
            })
            .collect();

        // Click to focus a resident (click on nothing clears it).
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let hit = pick_pawn(
                    &self.camera,
                    vw,
                    vh,
                    &pawns,
                    pos.x - rect.min.x,
                    pos.y - rect.min.y,
                );
                self.selected = hit.and_then(|id| {
                    snap.pawns
                        .iter()
                        .find(|p| p.id.counter() as u64 == id)
                        .map(|p| p.id)
                });
            }
        }

        let painter = painter.with_clip_rect(rect);
        let cam = snapped(&self.camera);
        if self.gpu {
            let scene = build_scene(&cam, vw, vh, &MapTerrain(map), &pawns, &self.atlas);
            self.stats = RenderStats {
                quads: scene.quads.len(),
                visible_tiles: scene.visible_tiles,
                draw_calls: 1,
                zoom: cam.zoom,
            };
            let ppp = ui.ctx().pixels_per_point();
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                SceneCallback {
                    quads: scene.quads,
                    scale: ppp,
                    view_px: [vw * ppp, vh * ppp],
                },
            ));
        } else {
            self.show_painter(&painter, rect, map, snap);
        }

        // Speech bubbles for conversations someone can hear.
        if cam.zoom >= 16.0 {
            for (speaker, text) in &snap.bubbles {
                let id = speaker.counter() as u64;
                let Some(d) = pawns.iter().find(|d| d.id == id) else {
                    continue;
                };
                let (sx, sy) = cam.to_screen(d.x + 0.5, d.y, vw, vh);
                let anchor = rect.min + egui::vec2(sx, sy - 4.0);
                if !rect.expand(cam.zoom).contains(anchor) {
                    continue;
                }
                let galley = painter.layout(
                    text.clone(),
                    egui::FontId::proportional(12.0),
                    Color32::from_rgb(30, 30, 36),
                    170.0,
                );
                let size = galley.size() + egui::vec2(12.0, 8.0);
                let bubble = egui::Rect::from_min_size(
                    anchor - egui::vec2(size.x / 2.0, size.y + 6.0),
                    size,
                );
                painter.rect_filled(
                    bubble,
                    6.0,
                    Color32::from_rgba_unmultiplied(250, 250, 245, 235),
                );
                painter.rect_stroke(
                    bubble,
                    6.0,
                    egui::Stroke::new(1.0, Color32::from_rgb(90, 90, 100)),
                    egui::StrokeKind::Inside,
                );
                // A little tail toward the speaker.
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        anchor + egui::vec2(-4.0, -6.0),
                        anchor + egui::vec2(4.0, -6.0),
                        anchor,
                    ],
                    Color32::from_rgba_unmultiplied(250, 250, 245, 235),
                    egui::Stroke::NONE,
                ));
                painter.galley(bubble.min + egui::vec2(6.0, 4.0), galley, Color32::BLACK);
            }
        }

        // Names and the hover note, on top of the scene.
        let mut hovered: Option<(String, Pos2)> = None;
        for p in snap.pawns.iter().filter(|p| p.map == map.id) {
            let id = p.id.counter() as u64;
            let Some(d) = pawns.iter().find(|d| d.id == id) else {
                continue;
            };
            let (sx, sy) = cam.to_screen(d.x + 0.5, d.y + 0.5, vw, vh);
            let center = rect.min + egui::vec2(sx, sy);
            if !rect.expand(cam.zoom).contains(center) {
                continue;
            }
            if cam.zoom >= 24.0 || d.selected {
                painter.text(
                    center + egui::vec2(0.0, -cam.zoom * 0.5 - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    &p.name,
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
            if let Some(h) = response.hover_pos() {
                if h.distance(center) <= cam.zoom * 0.55 + 3.0 {
                    hovered = Some((format!("{} ({})", p.name, p.activity), center));
                }
            }
        }
        if let Some((text, at)) = hovered {
            painter.text(
                at + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(14.0),
                Color32::from_rgb(255, 240, 180),
            );
        }
    }

    /// The painter-based drawing, kept for the headless smoke test and as a fallback.
    fn show_painter(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        map: &MapData,
        snap: &RenderSnapshot,
    ) {
        let (vw, vh) = (rect.width(), rect.height());
        for r in tile_rects(&self.camera, vw, vh, &MapColors(map)) {
            let min = rect.min + egui::vec2(r.x, r.y);
            // A hair of overlap hides seams between rectangles at fractional zoom.
            painter.rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(r.w + 0.5, r.h + 0.5)),
                0.0,
                c32(r.color),
            );
        }
        let zoom = self.camera.zoom;
        for p in snap.pawns.iter().filter(|p| p.map == map.id) {
            let (sx, sy) =
                self.camera
                    .to_screen(p.tile.x as f32 + 0.5, p.tile.y as f32 + 0.5, vw, vh);
            let center = rect.min + egui::vec2(sx, sy);
            if !rect.expand(zoom).contains(center) {
                continue;
            }
            let radius = (zoom * 0.38).max(2.0);
            painter.circle_filled(center, radius, c32(pawn_color(p.id)));
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(1.0, Color32::from_black_alpha(160)),
            );
        }
    }
}
