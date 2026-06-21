//! Tests for the sandboxed Rhai scripting runtime.

use std::rc::Rc;

use hayate_ir::geom::RectEmu;
use hayate_ir::presentation::Presentation;
use hayate_ir::theme::Theme;
use hayate_ir::world::{CompKind, CompValue, Entity};
use hayate_model::{History, Operation, Transaction};

use crate::{builtins, run_script, script_api_metadata, ScriptContext};

/// A presentation with one framed shape directly in the world; returns (pres, shape id).
fn framed_pres() -> (Presentation, u64) {
    let mut p = Presentation::new();
    let e = p.world.spawn();
    p.world
        .set(e, CompValue::Frame(RectEmu::new(10, 20, 100, 50)));
    (p, e.0)
}

/// Commit a script's ops to `pres` as one transaction.
fn apply(pres: &mut Presentation, ops: Vec<Operation>) {
    let mut h = History::new();
    h.commit(&mut pres.world, Transaction::new("script", ops));
}

#[test]
fn generated_command_function_issues_ops() {
    let (mut p, e) = framed_pres();
    let out = run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        &format!("shape_move({e}, 100, 0);"),
    )
    .expect("script runs");
    assert!(!out.ops.is_empty());
    apply(&mut p, out.ops);
    assert_eq!(
        p.world.get(Entity(e), CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(110, 20, 100, 50)))
    );
}

#[test]
fn create_returns_new_entity_id_and_chains() {
    let mut p = Presentation::new();
    let parent = p.world.spawn();
    // The new rect's id flows straight into set_fill.
    let out = run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        &format!(
            "let e = shape_add_rect({0}, 0, 0, 100, 50); shape_set_fill(e, \"#ff0000\");",
            parent.0
        ),
    )
    .expect("script runs");
    apply(&mut p, out.ops);

    // A new child of `parent` exists, carries a Frame, and is filled red.
    let child = p
        .world
        .iter()
        .find(|&c| matches!(p.world.get(c, CompKind::Parent), Some(CompValue::Parent(pp)) if pp == parent))
        .expect("a child shape was created");
    assert!(matches!(
        p.world.get(child, CompKind::Frame),
        Some(CompValue::Frame(_))
    ));
    assert!(
        matches!(p.world.get(child, CompKind::Fill), Some(CompValue::Fill(_))),
        "the returned id chained into shape_set_fill"
    );
}

#[test]
fn sequential_creates_get_distinct_ids() {
    let mut p = Presentation::new();
    let parent = p.world.spawn();
    let out = run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        &format!(
            "shape_add_rect({0}, 0, 0, 10, 10); shape_add_rect({0}, 20, 0, 10, 10);",
            parent.0
        ),
    )
    .expect("runs");
    apply(&mut p, out.ops);
    let children = p
        .world
        .iter()
        .filter(|&c| matches!(p.world.get(c, CompKind::Parent), Some(CompValue::Parent(pp)) if pp == parent))
        .count();
    assert_eq!(children, 2, "two distinct shapes were created");
}

#[test]
fn queries_expose_slides_and_selection() {
    // A real slide via master/layout/slide, plus a framed shape on it.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let shape = p.add_shape(slide);
    p.world
        .set(shape, CompValue::Frame(RectEmu::new(0, 0, 10, 10)));

    let ctx = ScriptContext {
        current_slide: Some(slide),
        selection: vec![shape],
    };
    // Move everything in the selection; also confirm slides()/current_slide() are callable.
    let out = run_script(
        Rc::new(builtins()),
        &p,
        &ctx,
        "let n = slides().len(); let s = current_slide(); for e in selection() { shape_move(e, 5, 0); }",
    )
    .expect("runs");
    apply(&mut p, out.ops);
    assert_eq!(
        p.world.get(shape, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(5, 0, 10, 10)))
    );
}

#[test]
fn print_is_captured() {
    let (p, _e) = framed_pres();
    let out = run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        r#"print("hello");"#,
    )
    .expect("runs");
    assert!(out.log.iter().any(|l| l.contains("hello")));
}

#[test]
fn sandbox_caps_runaway_loop() {
    let (p, _e) = framed_pres();
    assert!(run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        "let x = 0; loop { x += 1; }",
    )
    .is_err());
}

#[test]
fn unknown_function_errors() {
    let (p, _e) = framed_pres();
    assert!(run_script(
        Rc::new(builtins()),
        &p,
        &ScriptContext::default(),
        "no_such_command(1);",
    )
    .is_err());
}

#[test]
fn bundled_examples_run_without_error() {
    let reg = Rc::new(builtins());
    // A sample deck: one slide with a shape, and that shape selected.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let a = p.add_shape(slide);
    p.world
        .set(a, CompValue::Frame(RectEmu::new(0, 0, 100, 50)));
    let ctx = ScriptContext {
        current_slide: Some(slide),
        selection: vec![a],
    };
    for (name, src) in crate::script_examples() {
        let out = run_script(Rc::clone(&reg), &p, &ctx, src)
            .unwrap_or_else(|e| panic!("example `{name}` failed to run: {e}"));
        assert!(
            !out.ops.is_empty(),
            "example `{name}` should issue operations"
        );
    }
}

#[test]
fn intro_lt_builds_a_full_deck() {
    let reg = Rc::new(builtins());
    // Empty deck with just a master + layout (as a freshly created presentation has).
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let _layout = p.add_layout(master, "Title and Content");
    let ctx = ScriptContext {
        current_slide: None,
        selection: vec![],
    };
    let before = p.slides().len();

    let src = crate::script_examples()
        .iter()
        .find(|(n, _)| *n == "intro-lt")
        .expect("intro-lt example is registered")
        .1;
    let out = run_script(Rc::clone(&reg), &p, &ctx, src).expect("intro-lt runs");
    apply(&mut p, out.ops);

    // The LT adds 10 slides on top of whatever the deck started with.
    assert_eq!(p.slides().len(), before + 10, "intro-lt creates 10 slides");

    // The background is branded once on the master, so every slide resolves one by inheritance
    // (not a per-slide component). Each slide still owns its title/body Typst text boxes.
    for slide in p.slides() {
        assert!(
            p.background_of(slide).is_some(),
            "each LT slide inherits a background from the master"
        );
        let has_typst = p.children(slide).into_iter().any(|c| {
            matches!(p.world.get(c, CompKind::Text), Some(CompValue::Text(b)) if b.typst_source.is_some())
        });
        assert!(has_typst, "each LT slide has a Typst text box");
    }

    // The brand chrome (accent bars + footer) lives on the master, not duplicated per slide.
    let master = p
        .owning_master(p.slides()[0])
        .expect("slides have a master");
    assert!(
        p.children(master).len() >= 3,
        "master carries the shared brand chrome"
    );
}

#[test]
fn check_script_flags_syntax_errors_only() {
    use crate::check_script;
    assert!(check_script("").is_none(), "empty source is not an error");
    assert!(
        check_script("let x = 1; shape_add_rect(x);").is_none(),
        "well-formed source (even with unknown fns) parses clean"
    );
    assert!(
        check_script("let x = ;").is_some(),
        "a real syntax error is reported"
    );
    assert!(
        check_script("for e in selection() { ").is_some(),
        "an unclosed block is reported"
    );
}

#[test]
fn metadata_lists_callable_functions() {
    let json = script_api_metadata(Rc::new(builtins()));
    assert!(
        json.contains("shape_set_fill"),
        "metadata names command fns"
    );
    assert!(json.contains("entities"), "metadata names query helpers");
}
