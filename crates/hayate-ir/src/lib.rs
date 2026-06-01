//! Core document-model types for HayateOffice (gpui-free, pure data).
//!
//! This crate holds data types only; editing logic (operations/undo) and rendering
//! live in other crates. See `docs/DESIGN.md`.

pub mod color;
pub mod frac;
pub mod geom;
pub mod paint;
pub mod shape;
pub mod units;
pub mod world;
