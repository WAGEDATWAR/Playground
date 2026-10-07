//! The in-game view's geometry: camera and tile drawing lists (Blueprint §14).
//!
//! Everything here is plain arithmetic over plain data, so it is tested without a window or a GPU. The app
//! paints the rectangles it produces through egui's wgpu backend; the full tile and sprite renderer with
//! atlases arrives with Stage 1. Floating point is fine here: this is presentation, not simulation.

pub mod camera;
pub mod tiles;

pub use camera::Camera;
pub use tiles::{tile_rects, Rect, Rgb, TileSource};
