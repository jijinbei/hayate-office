//! Structural components for the master/layout/slide hierarchy (DESIGN 6.8), expressed in
//! the data-oriented model: a slide/layout/master is an entity carrying the matching
//! component. Inheritance is represented by presence/absence of overridable columns (e.g.
//! `backgrounds`): a value resolves slide -> layout -> master.

use crate::theme::Theme;
use crate::world::Entity;
use serde::{Deserialize, Serialize};

/// Marks an entity as a slide; references the layout it is based on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideInfo {
    pub layout: Entity,
}

/// Marks an entity as a slide layout; references its master.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutInfo {
    pub master: Entity,
    pub name: String,
}

/// Marks an entity as a slide master; holds the theme (one theme per master, DESIGN 6.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterInfo {
    pub theme: Theme,
}

/// Placeholder kinds. Slide placeholders inherit geometry/style from the matching
/// (type, idx) placeholder in the layout, then the master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlaceholderType {
    Title,
    CenteredTitle,
    Subtitle,
    Body,
    Picture,
    Chart,
    Table,
    Date,
    Footer,
    SlideNumber,
}

/// Links a shape to its inherited placeholder via (type, idx) matching (DESIGN 6.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceholderRef {
    pub ph_type: PlaceholderType,
    pub idx: u32,
}
