//! Headless interaction E2E tests. These drive the real UI handlers (`on_right_down`,
//! `on_mouse_down`, `on_key_down`) and context-menu actions through a gpui test context,
//! asserting on the editor's real state. They run under `cargo test -p hayate-app` (the
//! `test-support` feature pulls the windowing libs, so use `just e2e` to run in the Nix shell).

use super::{HayateApp, MenuTarget};
use gpui::{
    point, px, AppContext, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, TestAppContext,
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

/// A mouse-move event at output position (x, y) with the left button held.
fn mouse_move(x: f32, y: f32) -> MouseMoveEvent {
    MouseMoveEvent {
        position: point(px(x), px(y)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    }
}

/// A left mouse-up event at output position (x, y).
fn mouse_up(x: f32, y: f32) -> MouseUpEvent {
    MouseUpEvent {
        button: MouseButton::Left,
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
fn mousedown_is_noop_while_menu_open(cx: &mut TestAppContext) {
    // While a menu is open the canvas mouse-down must do nothing (the menu's backdrop/items
    // handle dismissal on click). Esc still closes it.
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |a, cx| {
        a.on_right_down(&mouse(MouseButton::Right, 10.0, 10.0), cx)
    });
    assert!(app.read_with(cx, |a, _| a.context_menu.is_some()));
    app.update(cx, |a, cx| {
        a.on_mouse_down(&mouse(MouseButton::Left, 10.0, 10.0), cx)
    });
    assert!(
        app.read_with(cx, |a, _| a.context_menu.is_some()),
        "mouse-down should not close the menu (the overlay does, on click-up)"
    );
    app.update(cx, |a, cx| a.on_key_down(&keydown("escape"), cx));
    assert!(app.read_with(cx, |a, _| a.context_menu.is_none()));
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

#[gpui::test]
fn grouped_shapes_drag_together(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Two accent rects (shapes 1 and 2).
    let (a, b) = app.read_with(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        (kids[1], kids[2])
    });
    // Group them (as the context-menu "Group" does on a multi-selection).
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
        s.group_selection();
    });
    // Record starting origins.
    let origin = |s: &HayateApp, e| {
        let f = s.pres.world.frames.get(&e).unwrap();
        (f.origin.x, f.origin.y)
    };
    let (a0, b0) = app.read_with(cx, |s, _| (origin(s, a), origin(s, b)));
    // Click the center of A to select it (should expand to the whole group and arm a drag).
    let (cx_, cy_) = app.read_with(cx, |s, _| {
        let n = s.scene.nodes.iter().find(|n| n.source == Some(a)).unwrap();
        let r = prim_bounds(&n.prim);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, cx_, cy_), cx)
    });
    let drag_len = app.read_with(cx, |s, _| s.drag.as_ref().map(|d| d.entities.len()));
    assert_eq!(
        drag_len,
        Some(2),
        "dragging a grouped shape should arm a 2-shape drag"
    );
    // Drag by +120px,+80px and release.
    app.update(cx, |s, cx| {
        s.on_mouse_move(&mouse_move(cx_ + 120.0, cy_ + 80.0), cx)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_up(&mouse_up(cx_ + 120.0, cy_ + 80.0), cx)
    });
    let (a1, b1) = app.read_with(cx, |s, _| (origin(s, a), origin(s, b)));
    // Both shapes moved, by the same nonzero delta.
    let da = (a1.0 - a0.0, a1.1 - a0.1);
    let db = (b1.0 - b0.0, b1.1 - b0.1);
    assert!(da.0 != 0 || da.1 != 0, "shape A should have moved");
    assert_eq!(
        da, db,
        "grouped shapes must move by the same delta: {da:?} vs {db:?}"
    );
}

#[gpui::test]
fn marquee_selects_intersecting_shapes(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Bounding region covering the two accent rects (shapes 1 and 2).
    let (a, b, rect) = app.read_with(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        let (a, b) = (kids[1], kids[2]);
        let nb = |e| {
            let n = s.scene.nodes.iter().find(|n| n.source == Some(e)).unwrap();
            prim_bounds(&n.prim)
        };
        let (ba, bb) = (nb(a), nb(b));
        let x0 = ba.x.min(bb.x) - 4.0;
        let y0 = ba.y.min(bb.y) - 4.0;
        let x1 = (ba.x + ba.w).max(bb.x + bb.w) + 4.0;
        let y1 = (ba.y + ba.h).max(bb.y + bb.h) + 4.0;
        (a, b, (x0, y0, x1, y1))
    });
    // Start the marquee on empty space (top-left of the region), drag to the far corner.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, rect.0, rect.1), cx)
    });
    app.update(cx, |s, cx| s.on_mouse_move(&mouse_move(rect.2, rect.3), cx));
    app.update(cx, |s, cx| s.on_mouse_up(&mouse_up(rect.2, rect.3), cx));
    let selected = app.read_with(cx, |s, _| {
        let mut v: Vec<_> = s.selection.into_iter().collect();
        v.extend(s.also.iter().copied());
        v
    });
    assert!(
        selected.contains(&a) && selected.contains(&b),
        "marquee should select both rects: {selected:?}"
    );
}

#[gpui::test]
fn full_group_flow_via_marquee_and_menu(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let (a, b, rect) = app.read_with(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        let (a, b) = (kids[1], kids[2]);
        let nb = |e| {
            let n = s.scene.nodes.iter().find(|n| n.source == Some(e)).unwrap();
            prim_bounds(&n.prim)
        };
        let (ba, bb) = (nb(a), nb(b));
        let x0 = ba.x.min(bb.x) - 4.0;
        let y0 = ba.y.min(bb.y) - 4.0;
        let x1 = (ba.x + ba.w).max(bb.x + bb.w) + 4.0;
        let y1 = (ba.y + ba.h).max(bb.y + bb.h) + 4.0;
        (a, b, (x0, y0, x1, y1))
    });
    // Marquee-select both rects.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, rect.0, rect.1), cx)
    });
    app.update(cx, |s, cx| s.on_mouse_move(&mouse_move(rect.2, rect.3), cx));
    app.update(cx, |s, cx| s.on_mouse_up(&mouse_up(rect.2, rect.3), cx));
    let n_sel = app.read_with(cx, |s, _| s.selected_all().len());
    assert_eq!(n_sel, 2, "marquee should select both rects, got {n_sel}");
    // Right-click one of them (over its center), then run the menu's Group action.
    let (cx_, cy_) = app.read_with(cx, |s, _| {
        let n = s.scene.nodes.iter().find(|n| n.source == Some(a)).unwrap();
        let r = prim_bounds(&n.prim);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    });
    app.update(cx, |s, cx| {
        s.on_right_down(&mouse(MouseButton::Right, cx_, cy_), cx)
    });
    let after_rc = app.read_with(cx, |s, _| s.selected_all().len());
    assert_eq!(
        after_rc, 2,
        "right-click within the selection must keep both, got {after_rc}"
    );
    app.update(cx, |s, _| s.group_selection());
    let grouped = app.read_with(cx, |s, _| {
        let ga = s.pres.world.groups.get(&a).copied();
        let gb = s.pres.world.groups.get(&b).copied();
        ga.is_some() && ga == gb
    });
    assert!(
        grouped,
        "menu Group should put both rects in the same group"
    );
}

#[gpui::test]
fn menu_open_click_keeps_selection(cx: &mut TestAppContext) {
    // Regression: a left mouse-down that dismisses an open context menu must NOT also start a
    // marquee / clear the selection, so a menu action (Group) still sees the selection.
    let app = cx.new(|cx| HayateApp::new(cx));
    let (a, b) = app.read_with(cx, |s, _| {
        let k = s.pres.children(s.slide);
        (k[1], k[2])
    });
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
    });
    // Open a context menu, then a canvas mouse-down on empty space (as a click on a menu item
    // below the shapes triggers): the canvas must NOT clear the selection.
    app.update(cx, |s, _cx| {
        s.open_menu(10.0, 10.0, crate::MenuTarget::Shape)
    });
    let (ex, ey) = app.read_with(cx, |s, _| (s.scene.size.w * 0.97, s.scene.size.h * 0.97));
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, ex, ey), cx)
    });
    let n = app.read_with(cx, |s, _| s.selected_all().len());
    assert_eq!(
        n, 2,
        "the canvas mouse-down must not clear the selection while a menu is open"
    );
    // The menu's Group action then groups both.
    app.update(cx, |s, _| s.group_selection());
    let grouped = app.read_with(cx, |s, _| {
        s.pres.world.groups.get(&a).is_some()
            && s.pres.world.groups.get(&a) == s.pres.world.groups.get(&b)
    });
    assert!(grouped, "Group should succeed after the menu-dismiss click");
}

/// A mouse-down with an explicit click count (1 = single, 2 = double).
fn mouse_n(button: MouseButton, x: f32, y: f32, clicks: usize) -> MouseDownEvent {
    MouseDownEvent {
        button,
        position: point(px(x), px(y)),
        click_count: clicks,
        ..Default::default()
    }
}

#[gpui::test]
fn rename_layer_sets_name(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let r = app.read_with(cx, |s, _| s.pres.children(s.slide)[1]);
    // Start renaming and type "box", then commit.
    app.update(cx, |s, _| s.renaming = Some((r, String::new())));
    for ch in ["b", "o", "x"] {
        app.update(cx, |s, cx| s.on_key_down(&keydown(ch), cx));
    }
    app.update(cx, |s, cx| s.on_key_down(&keydown("enter"), cx));
    let name = app.read_with(cx, |s, _| s.pres.world.names.get(&r).cloned());
    assert_eq!(
        name.as_deref(),
        Some("box"),
        "rename should set the name component"
    );
    assert!(
        app.read_with(cx, |s, _| s.renaming.is_none()),
        "rename should end on Enter"
    );
}

#[gpui::test]
fn double_click_drills_into_group(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let (a, b) = app.read_with(cx, |s, _| {
        let k = s.pres.children(s.slide);
        (k[1], k[2])
    });
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
        s.group_selection();
    });
    // Double-click member `a`: selection becomes just `a` (drilled into the group).
    let (x, y) = app.read_with(cx, |s, _| {
        let n = s.scene.nodes.iter().find(|n| n.source == Some(a)).unwrap();
        let r = prim_bounds(&n.prim);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse_n(MouseButton::Left, x, y, 2), cx)
    });
    let (sel, also_len) = app.read_with(cx, |s, _| (s.selection, s.also.len()));
    assert_eq!(sel, Some(a), "double-click should select just the member");
    assert_eq!(
        also_len, 0,
        "double-click should drop the rest of the group"
    );
}
