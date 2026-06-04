//! Tests for the sandboxed Rhai scripting runtime.

use std::rc::Rc;

use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue, World};
use hayate_model::{History, Transaction};

use crate::{builtins, run_script};

/// A world with a single framed shape; returns (world, entity-id).
fn framed_world() -> (World, u64) {
    let mut w = World::new();
    let e = w.spawn();
    w.set(e, CompValue::Frame(RectEmu::new(10, 20, 100, 50)));
    (w, e.0)
}

/// Commit a script's ops to `w` as one transaction and return the resulting world.
fn apply(mut w: World, ops: Vec<hayate_model::Operation>) -> World {
    let mut h = History::new();
    h.commit(&mut w, Transaction::new("script", ops));
    w
}

#[test]
fn generated_command_function_issues_ops() {
    let (w, e) = framed_world();
    let out = run_script(
        Rc::new(builtins()),
        &w,
        &format!("shape_move({e}, 100, 0);"),
    )
    .expect("script runs");
    assert!(!out.ops.is_empty(), "shape_move issued operations");

    let w = apply(w, out.ops);
    let frame = w.get(crate_entity(e), CompKind::Frame);
    assert_eq!(
        frame,
        Some(CompValue::Frame(RectEmu::new(110, 20, 100, 50))),
        "frame translated by dx=100"
    );
}

#[test]
fn entities_query_drives_a_loop() {
    let mut w = World::new();
    let a = w.spawn();
    w.set(a, CompValue::Frame(RectEmu::new(0, 0, 10, 10)));
    let b = w.spawn();
    w.set(b, CompValue::Frame(RectEmu::new(50, 50, 10, 10)));

    let out = run_script(
        Rc::new(builtins()),
        &w,
        "for e in entities() { shape_move(e, 5, 0); }",
    )
    .expect("script runs");

    let w = apply(w, out.ops);
    assert_eq!(
        w.get(a, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(5, 0, 10, 10)))
    );
    assert_eq!(
        w.get(b, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(55, 50, 10, 10)))
    );
}

#[test]
fn later_calls_see_earlier_effects() {
    // set_font_size then set_text on the same entity in one script: the second call observes the
    // text created/edited by surrounding state on the scratch world. Here we just verify two
    // sequential ops on one entity both land.
    let (w, e) = framed_world();
    let out = run_script(
        Rc::new(builtins()),
        &w,
        &format!("shape_move({e}, 10, 0); shape_move({e}, 0, 7);"),
    )
    .expect("script runs");
    let w = apply(w, out.ops);
    assert_eq!(
        w.get(crate_entity(e), CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(20, 27, 100, 50))),
        "both translations applied in sequence"
    );
}

#[test]
fn print_is_captured() {
    let (w, _e) = framed_world();
    let out = run_script(Rc::new(builtins()), &w, r#"print("hello");"#).expect("runs");
    assert!(out.log.iter().any(|l| l.contains("hello")));
}

#[test]
fn sandbox_caps_runaway_loop() {
    let (w, _e) = framed_world();
    let err = run_script(Rc::new(builtins()), &w, "let x = 0; loop { x += 1; }");
    assert!(err.is_err(), "an infinite loop must hit the operation cap");
}

#[test]
fn unknown_function_errors() {
    let (w, _e) = framed_world();
    assert!(run_script(Rc::new(builtins()), &w, "no_such_command(1);").is_err());
}

/// Build an `Entity` from a raw id for assertions.
fn crate_entity(id: u64) -> hayate_ir::world::Entity {
    hayate_ir::world::Entity(id)
}
