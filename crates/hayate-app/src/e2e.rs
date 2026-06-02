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
            hayate_model::edit::outer_group(&s.pres.world, a),
            hayate_model::edit::outer_group(&s.pres.world, b),
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
    let after = app.read_with(cx, |s, _| hayate_model::edit::outer_group(&s.pres.world, a));
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
        let ga = hayate_model::edit::outer_group(&s.pres.world, a);
        let gb = hayate_model::edit::outer_group(&s.pres.world, b);
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
        let ga = hayate_model::edit::outer_group(&s.pres.world, a);
        ga.is_some() && ga == hayate_model::edit::outer_group(&s.pres.world, b)
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

#[gpui::test]
fn nested_group_wraps_existing_group(cx: &mut TestAppContext) {
    use hayate_model::edit::{group_members, outer_group};
    let app = cx.new(|cx| HayateApp::new(cx));
    let (a, b, c) = app.read_with(cx, |s, _| {
        let k = s.pres.children(s.slide);
        (k[1], k[2], k[3])
    });
    // Group a + b -> inner group K1.
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
        s.group_selection();
    });
    let k1 = app
        .read_with(cx, |s, _| outer_group(&s.pres.world, a))
        .unwrap();
    // Select that whole group, add c, and group again -> outer group K2 wrapping K1 + c.
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = group_members(&s.pres.world, a)
            .into_iter()
            .filter(|&m| m != a)
            .collect();
        s.also.push(c);
        s.group_selection();
    });
    let (k2, cg, apath, members) = app.read_with(cx, |s, _| {
        (
            outer_group(&s.pres.world, a).unwrap(),
            outer_group(&s.pres.world, c),
            s.pres.world.groups.get(&a).cloned().unwrap_or_default(),
            group_members(&s.pres.world, a).len(),
        )
    });
    assert_ne!(k2, k1, "the new outer group must be a different key");
    assert_eq!(cg, Some(k2), "c joins the outer group");
    assert_eq!(apath, vec![k2, k1], "a's path nests K1 inside K2");
    assert_eq!(members, 3, "the outer group has all three shapes");
}

#[gpui::test]
fn add_line_creates_arrow_shape(cx: &mut TestAppContext) {
    use hayate_ir::shape::Geometry;
    let app = cx.new(|cx| HayateApp::new(cx));
    let before = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    app.update(cx, |s, _| s.add_line(true));
    let (after, is_arrow) = app.read_with(cx, |s, _| {
        use hayate_ir::shape::ArrowHead;
        let sel = s.selection.unwrap();
        (
            s.pres.children(s.slide).len(),
            matches!(
                s.pres.world.geometries.get(&sel),
                Some(Geometry::Line {
                    end: ArrowHead::Arrow,
                    ..
                })
            ),
        )
    });
    assert_eq!(after, before + 1, "add_line should add one shape");
    assert!(is_arrow, "the new shape should be a Line with arrow=true");
}

#[gpui::test]
fn enter_inserts_newline_and_click_commits(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let title = app.read_with(cx, |s, _| s.pres.children(s.slide)[0]);
    app.update(cx, |s, _| s.begin_text_edit(title));
    // Enter inserts a newline into the edit buffer (does not commit).
    app.update(cx, |s, cx| s.on_key_down(&keydown("enter"), cx));
    let (has_nl, still_editing) = app.read_with(cx, |s, _| {
        (
            s.text_edit
                .as_ref()
                .map(|t| t.buf.contains('\n'))
                .unwrap_or(false),
            s.text_edit.is_some(),
        )
    });
    assert!(has_nl, "Enter should insert a newline");
    assert!(still_editing, "Enter should not end editing");
    // A click commits the edit (text edit ends and the run text keeps the newline).
    app.update(cx, |s, cx| {
        let (ex, ey) = (s.scene.size.w * 0.95, s.scene.size.h * 0.95);
        s.on_mouse_down(&mouse(MouseButton::Left, ex, ey), cx)
    });
    let (done, run_has_nl) = app.read_with(cx, |s, _| {
        let run = s
            .pres
            .world
            .texts
            .get(&title)
            .and_then(|t| t.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.text.contains('\n'))
            .unwrap_or(false);
        (s.text_edit.is_none(), run)
    });
    assert!(done, "clicking away should commit and end the text edit");
    assert!(run_has_nl, "the committed text should keep the newline");
}

#[gpui::test]
fn arrow_heads_and_stroke_width_edit(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // A plain line: no heads.
    app.update(cx, |s, _| s.add_line(false));
    assert_eq!(
        app.read_with(cx, |s, _| s.sel_line_heads()),
        Some((false, false))
    );
    // Turn on the start head and off (toggle) — set start=true, end stays false.
    app.update(cx, |s, _| s.set_arrow_head(false, true));
    assert_eq!(
        app.read_with(cx, |s, _| s.sel_line_heads()),
        Some((true, false))
    );
    // Set the end head too.
    app.update(cx, |s, _| s.set_arrow_head(true, true));
    assert_eq!(
        app.read_with(cx, |s, _| s.sel_line_heads()),
        Some((true, true))
    );
    // Stroke width edit.
    app.update(cx, |s, _| s.set_stroke_width(6));
    assert_eq!(app.read_with(cx, |s, _| s.sel_stroke_pt()), Some(6));
}

#[gpui::test]
fn line_endpoint_drag_allows_any_direction(cx: &mut TestAppContext) {
    use hayate_render::scene::Primitive;
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_line(true));
    let sel = app.read_with(cx, |s, _| s.selection.unwrap());
    // Scene endpoints of the line.
    let (from_px, to_px) = app.read_with(cx, |s, _| {
        let n = s
            .scene
            .nodes
            .iter()
            .find(|n| n.source == Some(sel))
            .unwrap();
        match &n.prim {
            Primitive::Line { from, to, .. } => (*from, *to),
            _ => panic!("expected a line"),
        }
    });
    // Grab the END endpoint, then drag it up-left past the start.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, to_px.0, to_px.1), cx)
    });
    assert!(
        app.read_with(cx, |s, _| s.line_drag.is_some()),
        "grabbing an endpoint starts a line drag"
    );
    app.update(cx, |s, cx| {
        s.on_mouse_move(&mouse_move(from_px.0 - 60.0, from_px.1 - 60.0), cx)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_up(&mouse_up(from_px.0 - 60.0, from_px.1 - 60.0), cx)
    });
    let size = app.read_with(cx, |s, _| {
        let f = s.pres.world.frames.get(&sel).unwrap();
        (f.size.w, f.size.h)
    });
    assert!(
        size.0 < 0 && size.1 < 0,
        "line should point up-left (negative frame size): {size:?}"
    );
}

#[gpui::test]
fn image_dimensions_reads_png_header(_cx: &mut TestAppContext) {
    // A real PNG built by the in-tree encoder; the header parser must recover its size.
    let png = hayate_render::encode_png(&vec![0u8; 4 * 40 * 30], 40, 30);
    assert_eq!(crate::paint::image_dimensions(&png), Some((40, 30)));
    assert_eq!(crate::paint::image_dimensions(b"not an image"), None);
}

#[gpui::test]
fn pasted_image_keeps_aspect_ratio(cx: &mut TestAppContext) {
    // A 4:3 image should land in a frame whose width:height is also 4:3 (was a fixed 3:2 box).
    let png = hayate_render::encode_png(&vec![0u8; 4 * 400 * 300], 400, 300);
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.insert_image_bytes(png));
    let (w, h) = app.read_with(cx, |s, _| {
        let e = s.selection.unwrap();
        let f = s.pres.world.frames.get(&e).unwrap();
        (f.size.w as f64, f.size.h as f64)
    });
    let ratio = w / h;
    assert!(
        (ratio - 4.0 / 3.0).abs() < 0.02,
        "frame aspect should be 4:3, got {ratio}"
    );
}

#[gpui::test]
fn resize_snaps_moving_edge_to_slide_center(cx: &mut TestAppContext) {
    use hayate_ir::geom::RectEmu;
    use hayate_ir::units::inch_f;
    let app = cx.new(|cx| HayateApp::new(cx));
    let (sw, e) = app.read_with(cx, |s, _| {
        (s.pres.slide_size.w, s.pres.children(s.slide)[1])
    });
    // A frame whose right edge sits a hair left of the slide's horizontal centre.
    let nf = RectEmu::new(0, inch_f(1.0), sw / 2 - 5000, inch_f(1.0));
    // Handle 3 is the right-middle handle: only the right edge moves.
    let snapped = app.read_with(cx, |s, _| s.snap_resize(3, e, nf));
    assert_eq!(snapped.origin.x, 0, "anchored left edge stays put");
    assert_eq!(
        snapped.origin.x + snapped.size.w,
        sw / 2,
        "moving right edge snaps onto the slide centre line"
    );
}

#[gpui::test]
fn save_dialog_opens_and_edits_filename(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, cx| s.on_key_down(&keydown("ctrl-s"), cx));
    assert!(
        app.read_with(cx, |s, _| s.save_modal.is_some()),
        "Ctrl+S opens the Save dialog"
    );
    for k in ["x", "y"] {
        app.update(cx, |s, cx| s.save_modal_key(&keydown(k), cx));
    }
    let buf = app.read_with(cx, |s, _| s.save_modal.as_ref().unwrap().buf.clone());
    assert!(
        buf.ends_with("xy"),
        "typed chars append to the filename: {buf}"
    );
    let before = app.read_with(cx, |s, _| s.doc_path.clone());
    app.update(cx, |s, cx| s.save_modal_key(&keydown("escape"), cx));
    assert!(
        app.read_with(cx, |s, _| s.save_modal.is_none()),
        "Esc closes it"
    );
    assert_eq!(
        app.read_with(cx, |s, _| s.doc_path.clone()),
        before,
        "cancel leaves the document path unchanged"
    );
}

#[gpui::test]
fn layout_placeholder_renders_on_slide(cx: &mut TestAppContext) {
    use hayate_ir::doc::PlaceholderType;
    let app = cx.new(|cx| HayateApp::new(cx));
    let before = app.read_with(cx, |s, _| s.scene.nodes.len());
    app.update(cx, |s, _| s.add_layout_placeholder(PlaceholderType::Body));
    let eff = app.read_with(cx, |s, _| s.pres.effective_placeholders(s.slide).len());
    assert_eq!(eff, 1, "the layout now defines one placeholder");
    let after = app.read_with(cx, |s, _| s.scene.nodes.len());
    assert_eq!(
        after,
        before + 1,
        "the inherited placeholder adds one node to every slide using the layout"
    );
}

#[gpui::test]
fn new_layout_switches_current_slide(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let orig = app.read_with(cx, |s, _| s.pres.layout_of(s.slide));
    app.update(cx, |s, _| s.add_layout());
    let nl = app.read_with(cx, |s, _| s.master_layout);
    assert!(
        nl.is_some() && nl != orig,
        "a fresh layout is created and selected"
    );
    app.update(cx, |s, _| s.set_current_slide_layout(nl.unwrap()));
    assert_eq!(
        app.read_with(cx, |s, _| s.pres.layout_of(s.slide)),
        nl,
        "the slide now uses the new layout"
    );
}
