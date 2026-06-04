//! Tests for the AI script-authoring self-repair loop, using deterministic mock generators.

use std::cell::Cell;
use std::rc::Rc;

use hayate_ir::geom::RectEmu;
use hayate_ir::presentation::Presentation;
use hayate_ir::theme::Theme;
use hayate_ir::world::CompValue;

use super::{author_script, Attempt, ScriptGenerator};
use crate::builtins;

/// A deck with one slide and a selected shape.
fn sample() -> (Presentation, crate::ScriptContext) {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let shape = p.add_shape(slide);
    p.world
        .set(shape, CompValue::Frame(RectEmu::new(0, 0, 10, 10)));
    let ctx = crate::ScriptContext {
        current_slide: Some(slide),
        selection: vec![shape],
    };
    (p, ctx)
}

/// Always returns the same (valid) script.
struct Fixed(&'static str);
impl ScriptGenerator for Fixed {
    fn generate(
        &self,
        _req: &str,
        system: &str,
        _prior: Option<&Attempt>,
    ) -> Result<String, String> {
        // The system prompt must carry the tool catalogue + examples.
        assert!(system.contains("shape_set_fill"));
        assert!(system.contains("Example scripts"));
        Ok(self.0.to_string())
    }
}

#[test]
fn author_returns_working_script() {
    let (p, ctx) = sample();
    let g = Fixed("for e in selection() { shape_set_fill(e, \"#ff0000\"); }");
    let (src, out) = author_script(&g, Rc::new(builtins()), &p, &ctx, "make it red", 3)
        .expect("a valid script authors successfully");
    assert!(src.contains("shape_set_fill"));
    assert!(!out.ops.is_empty(), "the authored script issues operations");
}

/// First call returns a broken script; once an error is fed back, returns a fixed one.
struct SelfHeal {
    calls: Cell<u32>,
}
impl ScriptGenerator for SelfHeal {
    fn generate(
        &self,
        _req: &str,
        _system: &str,
        prior: Option<&Attempt>,
    ) -> Result<String, String> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        if n == 0 {
            assert!(prior.is_none(), "no prior on the first attempt");
            Ok("this is not valid rhai @@@".to_string())
        } else {
            // The previous error is provided for repair.
            assert!(prior.is_some(), "the failed attempt is fed back");
            Ok("let s = current_slide(); shape_add_rect(s, 0, 0, 50, 50);".to_string())
        }
    }
}

#[test]
fn author_self_repairs_after_an_error() {
    let (p, ctx) = sample();
    let g = SelfHeal {
        calls: Cell::new(0),
    };
    let (_src, out) = author_script(&g, Rc::new(builtins()), &p, &ctx, "add a box", 3)
        .expect("the loop repairs the broken first attempt");
    assert_eq!(g.calls.get(), 2, "it took one repair round");
    assert!(out
        .ops
        .iter()
        .any(|op| matches!(op, hayate_model::Operation::Spawn { .. })));
}

#[test]
fn author_gives_up_after_max_attempts() {
    let (p, ctx) = sample();
    struct AlwaysBad;
    impl ScriptGenerator for AlwaysBad {
        fn generate(&self, _r: &str, _s: &str, _p: Option<&Attempt>) -> Result<String, String> {
            Ok("@@@ not rhai @@@".to_string())
        }
    }
    let err = author_script(&AlwaysBad, Rc::new(builtins()), &p, &ctx, "x", 3)
        .expect_err("never produces a valid script");
    assert_eq!(
        err.len(),
        3,
        "it tried the full budget and recorded each failure"
    );
    assert!(err.iter().all(|a| !a.error.is_empty()));
}
