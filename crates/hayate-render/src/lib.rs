//! Scene builder (DESIGN 6.7): turns a `Presentation` slide into a backend-agnostic display
//! list with resolved colors/fonts. gpui-free, so it is unit-testable and reusable for
//! headless/offscreen rendering (thumbnails, PDF/video export).

pub mod build;
pub mod grid;
pub mod guides;
pub mod handles;
pub mod hit;
pub mod linebreak;
pub mod pdf;
pub mod pdf_font;
pub mod png;
pub mod raster;
pub mod scene;
pub mod svg;

pub use build::build_container_scene;
pub use build::build_slide_scene;
pub use build::build_slide_scene_at;
pub use build::prompt_text;
pub use grid::{grid_lines, GridLines};
pub use guides::{alignment_guides, Guide, GuideKind};
pub use handles::resize_handles;
pub use hit::hit_test;
pub use linebreak::{DefaultBreaker, Item, JapaneseBreaker, LineBreaker};
pub use pdf::export_pdf;
pub use png::encode_png;
pub use raster::rasterize;
pub use scene::{
    Paint, Primitive, PxRect, PxSize, Scene, SceneNode, StrokePx, TextBlock, Viewport,
};
pub use svg::export_svg;
