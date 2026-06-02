//! Scene builder (DESIGN 6.7): turns a `Presentation` slide into a backend-agnostic display
//! list with resolved colors/fonts. gpui-free, so it is unit-testable and reusable for
//! headless/offscreen rendering (thumbnails, PDF/video export).

pub mod build;
pub mod hit;
pub mod scene;

pub use build::build_slide_scene;
pub use hit::hit_test;
pub use scene::{Paint, Primitive, PxRect, PxSize, Scene, SceneNode, StrokePx, TextBlock, Viewport};
