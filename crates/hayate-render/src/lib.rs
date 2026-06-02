//! Scene builder (DESIGN 6.7): turns a `Presentation` slide into a backend-agnostic display
//! list with resolved colors/fonts. gpui-free, so it is unit-testable and reusable for
//! headless/offscreen rendering (thumbnails, PDF/video export).

pub mod build;
pub mod grid;
pub mod guides;
pub mod handles;
pub mod hit;
pub mod linebreak;
pub mod png;
pub mod raster;
pub mod scene;
pub mod svg;

pub use build::build_slide_scene;
pub use build::build_slide_scene_at;
pub use grid::{GridLines, grid_lines};
pub use guides::{Guide, GuideKind, alignment_guides};
pub use handles::resize_handles;
pub use svg::export_svg;
pub use hit::hit_test;
pub use png::encode_png;
pub use raster::rasterize;
pub use linebreak::{DefaultBreaker, Item, JapaneseBreaker, LineBreaker};
pub use scene::{Paint, Primitive, PxRect, PxSize, Scene, SceneNode, StrokePx, TextBlock, Viewport};
