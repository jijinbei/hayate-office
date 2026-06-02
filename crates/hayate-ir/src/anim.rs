//! Animation timeline types (DESIGN 6.15).
//!
//! The model is a simple step list rather than the OOXML SMIL timing tree, which covers the
//! practical majority of cases. A [`SlideTimeline`] is a per-slide component; within an
//! [`AnimStep`] all anims play in parallel. These are data-only reserved seams: the playback
//! engine lives outside this crate and arrives after the MVP.

use crate::units::Ms;
use crate::world::Entity;
use serde::{Deserialize, Serialize};

/// What starts an [`AnimStep`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    /// Starts on the next click.
    OnClick,
    /// Starts together with the previous step.
    WithPrev,
    /// Starts after the previous step, following `delay`.
    AfterPrev { delay: Ms },
}

/// Interpolation curve for an animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

/// The visual effect family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Fade,
    Fly,
    Wipe,
    Zoom,
}

/// Animation category, parameterized by its effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimKind {
    Entrance(Effect),
    Emphasis(Effect),
    Exit(Effect),
}

/// A single animation applied to one target entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anim {
    /// Entity being animated.
    pub target: Entity,
    /// What the animation does.
    pub kind: AnimKind,
    /// How long the animation runs.
    pub duration: Ms,
    /// Delay before the animation starts within its step.
    pub delay: Ms,
    /// Interpolation curve.
    pub easing: Easing,
}

/// A step in a slide timeline; its anims play in parallel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimStep {
    /// What starts this step.
    pub trigger: Trigger,
    /// Anims that play in parallel within this step.
    pub anims: Vec<Anim>,
}

/// Per-slide animation timeline (a sequence of steps).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideTimeline {
    pub steps: Vec<AnimStep>,
}

/// Slide-transition effect family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    None,
    Fade,
    Push,
    Wipe,
}

/// Screen transition applied to a slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: Ms,
}
