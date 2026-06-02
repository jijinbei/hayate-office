//! Scene builder (DESIGN 6.7): turns a `Presentation` slide into a backend-agnostic display
//! list with resolved colors/fonts. gpui-free, so it is unit-testable and reusable for
//! headless/offscreen rendering (thumbnails, PDF/video export).

pub mod build;
pub mod guides;
pub mod hit;
pub mod linebreak;
pub mod raster;
pub mod scene;
pub mod svg;

pub use build::build_slide_scene;
pub use guides::{Guide, GuideKind, alignment_guides};
pub use svg::export_svg;
pub use hit::hit_test;
pub use raster::rasterize;
pub use linebreak::{DefaultBreaker, Item, JapaneseBreaker, LineBreaker};
pub use scene::{Paint, Primitive, PxRect, PxSize, Scene, SceneNode, StrokePx, TextBlock, Viewport};
