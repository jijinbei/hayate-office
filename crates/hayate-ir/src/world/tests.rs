//! Unit tests for the parent module.

use super::*;

#[test]
fn spawn_and_despawn() {
    let mut w = World::new();
    let e = w.spawn();
    assert!(w.is_alive(e));
    assert_eq!(w.len(), 1);
    assert!(w.despawn(e));
    assert!(!w.is_alive(e));
    assert!(w.is_empty());
    assert!(!w.despawn(e), "double despawn is a no-op");
}

#[test]
fn spawn_at_recreates_same_id() {
    let mut w = World::new();
    let e = w.spawn();
    w.despawn(e);
    w.spawn_at(e);
    assert!(w.is_alive(e), "redo can recreate the same id");
    // A subsequent fresh spawn must not collide with e.
    let e2 = w.spawn();
    assert_ne!(e, e2);
}

#[test]
fn despawn_clears_components() {
    let mut w = World::new();
    let e = w.spawn();
    w.frames.insert(e, RectEmu::new(0, 0, 100, 100));
    w.names.insert(e, "title".to_string());
    w.despawn(e);
    assert!(!w.frames.contains_key(&e));
    assert!(!w.names.contains_key(&e));
}

#[test]
fn generic_set_remove_get_roundtrip() {
    let mut w = World::new();
    let e = w.spawn();
    let v = CompValue::Frame(RectEmu::new(1, 2, 3, 4));
    assert_eq!(v.kind(), CompKind::Frame);

    assert_eq!(w.set(e, v.clone()), None);
    assert_eq!(w.get(e, CompKind::Frame), Some(v.clone()));

    let v2 = CompValue::Frame(RectEmu::new(5, 6, 7, 8));
    assert_eq!(w.set(e, v2.clone()), Some(v));
    assert_eq!(w.get(e, CompKind::Frame), Some(v2.clone()));

    assert_eq!(w.remove(e, CompKind::Frame), Some(v2));
    assert_eq!(w.get(e, CompKind::Frame), None);
    assert_eq!(w.remove(e, CompKind::Frame), None);
}

#[test]
fn picture_timeline_morph_set_get_roundtrip() {
    use crate::anim::{
        Anim, AnimKind, AnimStep, Easing, Effect, SlideTimeline, Transition, TransitionKind,
        Trigger,
    };
    use crate::geom::SizeEmu;
    use crate::image::PictureRef;

    let mut w = World::new();
    let e = w.spawn();

    // Picture
    let pic = CompValue::Picture(PictureRef {
        media_key: "sha256:abc".to_string(),
        natural: SizeEmu::new(640, 480),
    });
    assert_eq!(pic.kind(), CompKind::Picture);
    assert_eq!(w.set(e, pic.clone()), None);
    assert_eq!(w.get(e, CompKind::Picture), Some(pic));

    // Timeline
    let timeline = CompValue::Timeline(SlideTimeline {
        steps: vec![AnimStep {
            trigger: Trigger::AfterPrev { delay: 250 },
            anims: vec![Anim {
                target: e,
                kind: AnimKind::Entrance(Effect::Fade),
                duration: 500,
                delay: 0,
                easing: Easing::EaseInOut,
            }],
        }],
    });
    assert_eq!(timeline.kind(), CompKind::Timeline);
    assert_eq!(w.set(e, timeline.clone()), None);
    assert_eq!(w.get(e, CompKind::Timeline), Some(timeline));

    // Transition
    let transition = CompValue::Transition(Transition {
        kind: TransitionKind::Push,
        duration: 300,
    });
    assert_eq!(transition.kind(), CompKind::Transition);
    assert_eq!(w.set(e, transition.clone()), None);
    assert_eq!(w.get(e, CompKind::Transition), Some(transition));

    // Morph key
    let morph = CompValue::MorphKey("logo".to_string());
    assert_eq!(morph.kind(), CompKind::MorphKey);
    assert_eq!(w.set(e, morph.clone()), None);
    assert_eq!(w.get(e, CompKind::MorphKey), Some(morph));
}

#[test]
fn despawn_clears_new_components() {
    use crate::anim::SlideTimeline;
    use crate::geom::SizeEmu;
    use crate::image::PictureRef;

    let mut w = World::new();
    let e = w.spawn();
    w.pictures.insert(
        e,
        PictureRef {
            media_key: "k".to_string(),
            natural: SizeEmu::new(10, 20),
        },
    );
    w.timelines.insert(e, SlideTimeline::default());
    w.morph_keys.insert(e, "logo".to_string());

    w.despawn(e);
    assert!(!w.pictures.contains_key(&e));
    assert!(!w.timelines.contains_key(&e));
    assert!(!w.morph_keys.contains_key(&e));
}
