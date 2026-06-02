//! Headless interaction E2E tests. These drive the real UI handlers (`on_right_down`,
//! `on_mouse_down`, `on_key_down`) and context-menu actions through a gpui test context,
//! asserting on the editor's real state. They run under `cargo test -p hayate-app` (the
//! `test-support` feature pulls the windowing libs, so use `just e2e` to run in the Nix shell).

use super::{HayateApp, MenuTarget};
use gpui::{
    point, px, AppContext, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, TestAppContext,
};
use hayate_render::scene::prim_bounds;

/// A mouse-down event of `button` at output position (x, y).
fn mouse(button: MouseButton, x: f32, y: f32) -> MouseDownEvent {
    MouseDownEvent {
        button,
        position: point(px(x), px(y)),
        ..Default::default()
    }
}

/// A key-down event for the named keystroke (e.g. "escape").
fn keydown(name: &str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(name).expect("valid keystroke"),
        is_held: false,
        prefer_character_input: false,
    }
}

#[gpui::test]
fn right_click_on_shape_opens_shape_menu(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Center of the first scene node (a real shape) in scene/output coordinates.
    let (x, y) = app.read_with(cx, |a, _| {
        let b = prim_bounds(&a.scene.nodes[0].prim);
        (b.x + b.w * 0.5, b.y + b.h * 0.5)
    });
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, x, y), cx)
    });
    let (target, has_sel) = app.read_with(cx, |a, _| {
        (
            a.context_menu.as_ref().map(|m| m.target),
            a.selection.is_some(),
        )
    });
    assert_eq!(target, Some(MenuTarget::Shape));
    assert!(has_sel, "right-clicking a shape should select it");
}

#[gpui::test]
fn right_click_empty_canvas_opens_canvas_menu(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Bottom-right corner of the slide, away from the sample content.
    let (x, y) = app.read_with(cx, |a, _| (a.scene.size.w * 0.97, a.scene.size.h * 0.97));
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, x, y), cx)
    });
    let target = app.read_with(cx, |a, _| a.context_menu.as_ref().map(|m| m.target));
    assert_eq!(target, Some(MenuTarget::Canvas));
}

#[gpui::test]
fn escape_closes_menu(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, 10.0, 10.0), cx)
    });
    assert!(app.read_with(cx, |a, _| a.context_menu.is_some()));
    app.update(cx, |a, cx| a.on_key_down(&keydown("escape"), cx));
    assert!(
        app.read_with(cx, |a, _| a.context_menu.is_none()),
        "Esc should dismiss the menu"
    );
}

#[gpui::test]
fn left_click_closes_menu(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, 10.0, 10.0), cx)
    });
    assert!(app.read_with(cx, |a, _| a.context_menu.is_some()));
    app.update(cx, |a, cx| {
        a.on_mouse_down(&mouse(MouseButton::Left, 10.0, 10.0), cx)
    });
    assert!(
        app.read_with(cx, |a, _| a.context_menu.is_none()),
        "a left click should dismiss the menu"
    );
}

#[gpui::test]
fn duplicate_action_adds_a_shape(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let before = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    // Select a shape via right-click, then run the menu's Duplicate action.
    let (x, y) = app.read_with(cx, |a, _| {
        let b = prim_bounds(&a.scene.nodes[1].prim);
        (b.x + b.w * 0.5, b.y + b.h * 0.5)
    });
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, x, y), cx)
    });
    app.update(cx, |a, _| a.duplicate_selection());
    let after = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    assert_eq!(
        after,
        before + 1,
        "Duplicate should add one shape to the slide"
    );
}

#[gpui::test]
fn toggle_bold_command_flips_selected_text(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let title = app.read_with(cx, |a, _| a.pres.children(a.slide)[0]);
    let bold_of = |a: &HayateApp| {
        a.pres
            .world
            .texts
            .get(&title)
            .and_then(|tb| tb.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.bold)
            .unwrap_or(false)
    };
    let before = app.read_with(cx, |a, _| bold_of(a));
    app.update(cx, |a, _| {
        a.selection = Some(title);
        a.run_on_selection("shape.toggle_bold");
    });
    let after = app.read_with(cx, |a, _| bold_of(a));
    assert_ne!(before, after, "toggle_bold should flip the run's bold flag");
}

#[gpui::test]
fn insert_image_bytes_adds_a_picture(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let before = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    let png = hayate_render::encode_png(&[255u8, 0, 0, 255], 1, 1);
    app.update(cx, |a, _| a.insert_image_bytes(png));
    let (after, has_pic) = app.read_with(cx, |a, _| {
        let kids = a.pres.children(a.slide);
        (
            kids.len(),
            kids.iter().any(|e| a.pres.world.pictures.contains_key(e)),
        )
    });
    assert_eq!(after, before + 1, "inserting an image should add one shape");
    assert!(has_pic, "the new shape should carry a picture component");
}

#[gpui::test]
fn grouping_links_and_unlinks_shapes(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Select the first two accent rects (shapes 1 and 2) and group them.
    let (a, b) = app.read_with(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        (kids[1], kids[2])
    });
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
        s.group_selection();
    });
    // Both shapes now share a (nonzero) group key.
    let (ga, gb) = app.read_with(cx, |s, _| {
        (
            s.pres.world.groups.get(&a).copied(),
            s.pres.world.groups.get(&b).copied(),
        )
    });
    assert!(
        ga.is_some() && ga == gb,
        "grouped shapes share a key: {ga:?} {gb:?}"
    );
    // group_members expands from either member to both.
    let members = app.read_with(cx, |s, _| {
        hayate_model::edit::group_members(&s.pres.world, a)
    });
    assert_eq!(members.len(), 2);
    // Ungroup removes the membership.
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.ungroup_selection();
    });
    let after = app.read_with(cx, |s, _| s.pres.world.groups.get(&a).copied());
    assert_eq!(after, None, "ungroup should clear the group key");
}

#[gpui::test]
fn reorder_slide_moves_it_before_target(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Start with three slides; remember their order.
    app.update(cx, |a, _| {
        a.add_slide();
        a.add_slide();
    });
    let order = app.read_with(cx, |a, _| a.pres.slides());
    assert_eq!(order.len(), 3);
    let (first, _second, third) = (order[0], order[1], order[2]);
    // Drag the last slide to sit before the first.
    app.update(cx, |a, _| a.reorder_slide(third, first));
    let after = app.read_with(cx, |a, _| a.pres.slides());
    assert_eq!(after[0], third, "dragged slide should now be first");
    assert_eq!(after.len(), 3, "reorder must not add or drop slides");
}

#[gpui::test]
fn slide_add_and_delete_round_trips(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let start = app.read_with(cx, |a, _| a.pres.slides().len());
    app.update(cx, |a, _| a.add_slide());
    assert_eq!(app.read_with(cx, |a, _| a.pres.slides().len()), start + 1);
    app.update(cx, |a, _| a.delete_slide());
    assert_eq!(app.read_with(cx, |a, _| a.pres.slides().len()), start);
}
