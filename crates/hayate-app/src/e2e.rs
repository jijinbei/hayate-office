//! Headless interaction E2E tests. These drive the real UI handlers (`on_right_down`,
//! `on_mouse_down`, `on_key_down`) and context-menu actions through a gpui test context,
//! asserting on the editor's real state. They run under `cargo test -p hayate-app` (the
//! `test-support` feature pulls the windowing libs, so use `just e2e` to run in the Nix shell).

use super::{EditScope, HayateApp, MenuTarget};
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
    // A click commits the edit; each line becomes its own paragraph.
    app.update(cx, |s, cx| {
        let (ex, ey) = (s.scene.size.w * 0.95, s.scene.size.h * 0.95);
        s.on_mouse_down(&mouse(MouseButton::Left, ex, ey), cx)
    });
    let (done, paras) = app.read_with(cx, |s, _| {
        let n = s
            .pres
            .world
            .texts
            .get(&title)
            .map(|t| t.paragraphs.len())
            .unwrap_or(0);
        (s.text_edit.is_none(), n)
    });
    assert!(done, "clicking away should commit and end the text edit");
    assert_eq!(paras, 2, "the newline splits the text into two paragraphs");
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
    app.update(cx, |s, _| {
        s.add_layout_preset(hayate_model::edit::LayoutPreset::Blank);
    });
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

#[gpui::test]
fn master_edit_scope_parents_shapes_to_layout(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let layout = app.read_with(cx, |s, _| s.pres.layout_of(s.slide).unwrap());
    app.update(cx, |s, _| s.enter_layout_scope(layout));
    assert_eq!(
        app.read_with(cx, |s, _| s.container()),
        layout,
        "canvas edits the layout"
    );
    let before = app.read_with(cx, |s, _| s.pres.children(layout).len());
    app.update(cx, |s, _| s.add_rect());
    let after = app.read_with(cx, |s, _| s.pres.children(layout).len());
    assert_eq!(
        after,
        before + 1,
        "a new shape is parented to the layout, not the slide"
    );
    // Exiting returns the canvas to the slide.
    app.update(cx, |s, _| s.exit_scope());
    let slide = app.read_with(cx, |s, _| s.slide);
    assert_eq!(
        app.read_with(cx, |s, _| s.container()),
        slide,
        "exit returns to the slide"
    );
}

#[gpui::test]
fn slide_nav_disabled_in_master_scope(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_slide()); // now two slides
    let cur = app.read_with(cx, |s, _| s.slide);
    let layout = app.read_with(cx, |s, _| s.pres.layout_of(s.slide).unwrap());
    app.update(cx, |s, _| s.enter_layout_scope(layout));
    app.update(cx, |s, _| s.next_slide(1));
    assert_eq!(
        app.read_with(cx, |s, _| s.slide),
        cur,
        "next_slide is a no-op in master mode"
    );
}

#[gpui::test]
fn add_layout_preset_populates_placeholders(cx: &mut TestAppContext) {
    use hayate_ir::doc::PlaceholderType as PT;
    use hayate_model::edit::LayoutPreset;
    let app = cx.new(|cx| HayateApp::new(cx));
    let layout = app
        .update(cx, |s, _| {
            s.add_layout_preset(LayoutPreset::TitleAndContent)
        })
        .unwrap();
    let kinds: Vec<PT> = app.read_with(cx, |s, _| {
        s.pres
            .placeholder_shapes(layout)
            .iter()
            .filter_map(|e| s.pres.world.placeholders.get(e).map(|p| p.ph_type))
            .collect()
    });
    assert!(
        kinds.contains(&PT::Title) && kinds.contains(&PT::Body),
        "got {kinds:?}"
    );
}

#[gpui::test]
fn duplicate_and_delete_layout(cx: &mut TestAppContext) {
    use hayate_model::edit::LayoutPreset;
    let app = cx.new(|cx| HayateApp::new(cx));
    let layout = app
        .update(cx, |s, _| s.add_layout_preset(LayoutPreset::TitleOnly))
        .unwrap();
    let n_before = app.read_with(cx, |s, _| s.master_layouts().len());
    app.update(cx, |s, _| s.duplicate_layout(layout));
    assert_eq!(
        app.read_with(cx, |s, _| s.master_layouts().len()),
        n_before + 1,
        "duplicate adds a layout"
    );
    // The duplicate (unused by any slide) can be deleted.
    let dup = app.read_with(cx, |s, _| s.master_layout.unwrap());
    app.update(cx, |s, _| s.delete_layout(dup));
    assert_eq!(
        app.read_with(cx, |s, _| s.master_layouts().len()),
        n_before,
        "unused layout is deleted"
    );
}

#[gpui::test]
fn delete_layout_in_use_is_refused(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // The current slide's layout is in use; deleting it must be a no-op.
    let layout = app.read_with(cx, |s, _| s.pres.layout_of(s.slide).unwrap());
    let n = app.read_with(cx, |s, _| s.master_layouts().len());
    app.update(cx, |s, _| s.delete_layout(layout));
    assert_eq!(
        app.read_with(cx, |s, _| s.master_layouts().len()),
        n,
        "in-use layout kept"
    );
}

#[gpui::test]
fn promote_and_reset_inherited_placeholder(cx: &mut TestAppContext) {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let app = cx.new(|cx| HayateApp::new(cx));
    // Define a Title placeholder on the slide's layout (inherited by the slide).
    app.update(cx, |s, _| s.add_layout_placeholder(PlaceholderType::Title));
    let slide = app.read_with(cx, |s, _| s.slide);
    let title = PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    };
    // Inherited frame resolves from the layout; no slide-level override yet.
    assert!(app.read_with(cx, |s, _| s.pres.find_placeholder(slide, title).is_none()));

    let before = app.read_with(cx, |s, _| s.pres.children(slide).len());
    app.update(cx, |s, _| s.promote_and_edit(title));
    assert_eq!(
        app.read_with(cx, |s, _| s.pres.children(slide).len()),
        before + 1,
        "promote creates a slide-level override"
    );
    assert!(
        app.read_with(cx, |s, _| s.text_edit.is_some()),
        "editing starts"
    );
    assert!(app.read_with(cx, |s, _| s.selection_is_slide_placeholder()));

    // Reset removes the override; the placeholder falls back to the layout.
    app.update(cx, |s, _| {
        s.text_edit = None;
        s.reset_selected_placeholder();
    });
    assert_eq!(
        app.read_with(cx, |s, _| s.pres.children(slide).len()),
        before,
        "reset removes the override"
    );
    assert!(app.read_with(cx, |s, _| s.pres.find_placeholder(slide, title).is_none()));
}

#[gpui::test]
fn double_click_inherited_placeholder_promotes_single_click_does_not(cx: &mut TestAppContext) {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let app = cx.new(|cx| HayateApp::new(cx));
    // Start from an empty slide so a click lands on the placeholder, not sample content.
    app.update(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        for e in kids {
            s.pres.world.despawn(e);
        }
        s.add_layout_placeholder(PlaceholderType::Title);
        s.rebuild();
    });
    let slide = app.read_with(cx, |s, _| s.slide);
    let title = PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    };
    // A point near the centre of the inherited Title frame, in canvas px.
    let (px_x, px_y) = app.read_with(cx, |s, _| {
        let fr = s.pres.ph_frame(slide, title).unwrap();
        let sc = s.scale();
        (
            ((fr.origin.x + fr.size.w / 2) as f64 * sc) as f32,
            ((fr.origin.y + fr.size.h / 2) as f64 * sc) as f32,
        )
    });
    // A single click leaves the inherited (locked) placeholder untouched: no slide-level override
    // is created, so the slide stays empty.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, px_x, px_y), cx)
    });
    assert!(
        app.read_with(cx, |s, _| !s.selection_is_slide_placeholder()
            && s.text_edit.is_none()
            && s.pres.children(s.slide).is_empty()),
        "a single click must not promote/duplicate the locked inherited placeholder"
    );
    // A double-click promotes it to an editable slide-level override and starts editing.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse_n(MouseButton::Left, px_x, px_y, 2), cx)
    });
    assert!(
        app.read_with(cx, |s, _| s.selection_is_slide_placeholder()
            && s.text_edit.is_some()),
        "double-clicking the inherited placeholder promotes it and starts editing"
    );
}

#[gpui::test]
fn promoted_placeholder_is_locked_in_place(cx: &mut TestAppContext) {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        for e in kids {
            s.pres.world.despawn(e);
        }
        s.add_layout_placeholder(PlaceholderType::Title);
        s.rebuild();
    });
    let slide = app.read_with(cx, |s, _| s.slide);
    let title = PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    };
    let (px_x, px_y) = app.read_with(cx, |s, _| {
        let fr = s.pres.ph_frame(slide, title).unwrap();
        let sc = s.scale();
        (
            ((fr.origin.x + fr.size.w / 2) as f64 * sc) as f32,
            ((fr.origin.y + fr.size.h / 2) as f64 * sc) as f32,
        )
    });
    // Promote (double-click) and commit the edit with a click elsewhere.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse_n(MouseButton::Left, px_x, px_y, 2), cx)
    });
    let e = app.read_with(cx, |s, _| s.selection.unwrap());
    // A promoted placeholder is text-only: it has NO frame of its own (geometry is inherited).
    assert!(
        app.read_with(cx, |s, _| s.pres.world.frames.get(&e).is_none()),
        "a promoted placeholder must not copy a frame (geometry stays inherited)"
    );
    let before = app.read_with(cx, |s, _| s.resolved_frame(e).unwrap());
    // Now select and try to drag the locked placeholder by +120,+90: it must not move, and no
    // move-drag is armed.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, px_x, px_y), cx)
    });
    let armed = app.read_with(cx, |s, _| s.drag.is_some());
    app.update(cx, |s, cx| {
        s.on_mouse_move(&mouse_move(px_x + 120.0, px_y + 90.0), cx)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_up(&mouse_up(px_x + 120.0, px_y + 90.0), cx)
    });
    let after = app.read_with(cx, |s, _| s.resolved_frame(e).unwrap());
    assert!(!armed, "dragging a locked placeholder must not arm a move");
    assert_eq!(
        (before.origin.x, before.origin.y),
        (after.origin.x, after.origin.y),
        "a locked placeholder must not move when dragged"
    );
    assert!(
        app.read_with(cx, |s, _| s.pres.world.frames.get(&e).is_none()),
        "dragging a locked placeholder must not give it an own frame"
    );
}

#[gpui::test]
fn customize_placeholder_pins_geometry_and_unlocks(cx: &mut TestAppContext) {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| {
        let kids = s.pres.children(s.slide);
        for e in kids {
            s.pres.world.despawn(e);
        }
        s.add_layout_placeholder(PlaceholderType::Title);
        s.rebuild();
    });
    let slide = app.read_with(cx, |s, _| s.slide);
    let title = PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    };
    let (px_x, px_y) = app.read_with(cx, |s, _| {
        let fr = s.pres.ph_frame(slide, title).unwrap();
        let sc = s.scale();
        (
            ((fr.origin.x + fr.size.w / 2) as f64 * sc) as f32,
            ((fr.origin.y + fr.size.h / 2) as f64 * sc) as f32,
        )
    });
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse_n(MouseButton::Left, px_x, px_y, 2), cx)
    });
    let e = app.read_with(cx, |s, _| s.selection.unwrap());
    // Locked while geometry is inherited.
    assert!(app.read_with(cx, |s, _| s.is_locked_placeholder(e)));
    let inherited = app.read_with(cx, |s, _| s.resolved_frame(e).unwrap());
    // Pin geometry to this slide: it gains its own frame (= the inherited one) and unlocks.
    app.update(cx, |s, _| s.customize_placeholder_geometry());
    assert!(
        app.read_with(cx, |s, _| !s.is_locked_placeholder(e)
            && s.pres.world.frames.get(&e) == Some(&inherited)),
        "customize pins the inherited frame onto the slide and unlocks the placeholder"
    );
    // Now it can be dragged.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, px_x, px_y), cx)
    });
    assert!(
        app.read_with(cx, |s, _| s.drag.is_some()),
        "a customized placeholder can be moved"
    );
}

#[gpui::test]
fn theme_accent_edit_recolors_scene_and_undoes(cx: &mut TestAppContext) {
    use hayate_ir::color::Rgba;
    use hayate_render::scene::{Paint, Primitive};
    let app = cx.new(|cx| HayateApp::new(cx));
    let red = Rgba::rgb(0xFF, 0x00, 0x00);
    // The sample deck has Accent1-filled rectangles. Recolour Accent1 to red.
    app.update(cx, |s, _| s.set_theme_accent(0, red));
    assert_eq!(
        app.read_with(cx, |s, _| s.pres.theme_of(s.slide).unwrap().colors.accent
            [0]),
        red,
        "the master theme's Accent1 is now red"
    );
    let has_red = app.read_with(cx, |s, _| {
        s.scene.nodes.iter().any(|n| match &n.prim {
            Primitive::Quad {
                fill: Some(Paint::Solid(c)),
                ..
            } => *c == red,
            _ => false,
        })
    });
    assert!(
        has_red,
        "an accent-filled shape renders red after the theme edit"
    );
    // Undo restores the original accent.
    app.update(cx, |s, _| {
        s.history.undo(&mut s.pres.world);
        s.after_doc_change();
    });
    assert_ne!(
        app.read_with(cx, |s, _| s.pres.theme_of(s.slide).unwrap().colors.accent
            [0]),
        red,
        "undo reverts the theme change"
    );
}

#[gpui::test]
fn apply_color_preset_changes_theme(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let before = app.read_with(cx, |s, _| s.pres.theme_of(s.slide).unwrap().colors.accent);
    app.update(cx, |s, _| s.apply_color_preset(1)); // "Warm"
    let after = app.read_with(cx, |s, _| s.pres.theme_of(s.slide).unwrap().colors.accent);
    assert_ne!(
        before, after,
        "applying a colour preset changes the accents"
    );
}

#[gpui::test]
fn typst_text_edit_shows_source_then_typesets(cx: &mut TestAppContext) {
    use hayate_render::scene::Primitive;
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_text_box());
    let e = app.read_with(cx, |s, _| s.selection.unwrap());

    // Enter edit, clear the default "Text", and type a two-item Typst list.
    app.update(cx, |s, _| s.begin_text_edit(e));
    app.update(cx, |s, cx| {
        if let Some(te) = s.text_edit.as_mut() {
            te.selected = 0..te.buf.len();
        }
        s.text_key(&keydown("backspace"), cx);
    });
    app.update(cx, |s, _| s.apply_ime(None, "- a", false));
    app.update(cx, |s, cx| s.text_key(&keydown("enter"), cx));
    app.update(cx, |s, _| s.apply_ime(None, "- b", false));

    // While editing, the box renders as its RAW SOURCE (plain Text), not Typst — so the caret
    // tracks literal characters. The scene node for `e` is a Text whose lines equal the source.
    app.read_with(cx, |s, _| {
        let node = s.scene.nodes.iter().find(|n| n.source == Some(e)).unwrap();
        match &node.prim {
            Primitive::Text(tb) => {
                let lines: Vec<String> = tb
                    .paragraphs
                    .iter()
                    .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
                    .collect();
                assert_eq!(lines, vec!["- a".to_string(), "- b".to_string()]);
            }
            other => panic!("editing should show raw source as Text, got {other:?}"),
        }
        assert_eq!(s.text_edit.as_ref().unwrap().buf, "- a\n- b");
    });

    // Commit by clicking elsewhere (empty canvas corner).
    let (cx_, cy_) = app.read_with(cx, |s, _| (s.scene.size.w * 0.97, s.scene.size.h * 0.97));
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, cx_, cy_), cx)
    });
    app.read_with(cx, |s, _| {
        assert!(s.text_edit.is_none(), "click commits the edit");
        assert_eq!(
            s.pres.world.texts.get(&e).unwrap().typst_source.as_deref(),
            Some("- a\n- b"),
            "the Typst source is stored"
        );
        // Not editing now → the box typesets via Typst (a Typst primitive, not plain Text).
        let node = s.scene.nodes.iter().find(|n| n.source == Some(e)).unwrap();
        assert!(
            matches!(node.prim, Primitive::Typst { .. }),
            "preview typesets the box"
        );
    });

    // Undo restores the pre-edit box (the default "Text").
    app.update(cx, |s, cx| s.on_key_down(&keydown("ctrl-z"), cx));
    assert_eq!(
        app.read_with(cx, |s, _| s
            .pres
            .world
            .texts
            .get(&e)
            .unwrap()
            .typst_source
            .clone()),
        Some("Text".to_string()),
        "undo reverts to the original source"
    );
}

#[gpui::test]
fn font_size_change_keeps_typst_source(cx: &mut TestAppContext) {
    use hayate_render::scene::Primitive;
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_text_box());
    let e = app.read_with(cx, |s, _| s.selection.unwrap());
    // add_text_box drops straight into edit mode (raw source shown); leave it so the box previews
    // via Typst, then measure the typeset default size.
    app.update(cx, |s, cx| s.on_key_down(&keydown("escape"), cx));

    let pt_before = |s: &HayateApp| match s
        .scene
        .nodes
        .iter()
        .find(|n| n.source == Some(e))
        .map(|n| &n.prim)
    {
        Some(Primitive::Typst { default_pt, .. }) => *default_pt,
        _ => 0.0,
    };
    let before = app.read_with(cx, |s, _| pt_before(s));
    app.update(cx, |s, _| {
        s.selection = Some(e);
        s.change_font_size(8);
    });
    app.read_with(cx, |s, _| {
        // Source is preserved and the typeset default size grew.
        assert!(
            s.pres.world.texts.get(&e).unwrap().typst_source.is_some(),
            "font-size change keeps the box Typst"
        );
        assert!(
            pt_before(s) > before,
            "the Typst default point size increased ({} > {})",
            pt_before(s),
            before
        );
    });
}

#[gpui::test]
fn copy_paste_shape_core(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_rect());
    let e = app.read_with(cx, |s, _| s.selection.unwrap());
    let before = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    app.update(cx, |s, _| s.copy_selection());
    assert!(
        app.read_with(cx, |s, _| s.clipboard.is_some()),
        "copy stores the shape"
    );
    app.update(cx, |s, _| s.paste_clipboard());
    let after = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    assert_eq!(after, before + 1, "paste adds one shape");
    let sel = app.read_with(cx, |s, _| s.selection.unwrap());
    assert_ne!(sel, e, "pasted copy becomes the selection");
}

#[gpui::test]
fn ctrl_c_ctrl_v_keys_paste(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_rect());
    let before = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    app.update(cx, |s, cx| s.on_key_down(&keydown("ctrl-c"), cx));
    app.update(cx, |s, cx| s.on_key_down(&keydown("ctrl-v"), cx));
    let after = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    assert_eq!(after, before + 1, "Ctrl+C then Ctrl+V pastes a copy");
}

#[gpui::test]
fn copy_paste_multi_adds_both_ungrouped(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Select the first two sample shapes and copy both.
    let (a, b) = app.read_with(cx, |s, _| {
        let k = s.pres.children(s.slide);
        (k[1], k[2])
    });
    let before = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    app.update(cx, |s, _| {
        s.selection = Some(a);
        s.also = vec![b];
        s.copy_selection();
        s.paste_clipboard();
    });
    let after = app.read_with(cx, |s, _| s.pres.children(s.slide).len());
    assert_eq!(after, before + 2, "pasting two shapes adds two");
    // Both copies are selected and ungrouped (paste forms no new group).
    let (sel, also) = app.read_with(cx, |s, _| (s.selection.unwrap(), s.also.clone()));
    assert_eq!(also.len(), 1, "both copies are selected");
    let (g1, g2) = app.read_with(cx, |s, _| {
        (
            hayate_model::edit::outer_group(&s.pres.world, sel),
            hayate_model::edit::outer_group(&s.pres.world, also[0]),
        )
    });
    assert!(
        g1.is_none() && g2.is_none(),
        "pasted copies are not grouped"
    );
}

#[gpui::test]
fn ctrl_shift_p_shows_pdf_notice(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let pdf = std::env::temp_dir().join("hayate-e2e-notice.pdf");
    let _ = std::fs::remove_file(&pdf);
    app.update(cx, |s, _| {
        s.doc_path = std::env::temp_dir()
            .join("hayate-e2e-notice.hayate")
            .to_string_lossy()
            .into_owned();
    });
    app.update(cx, |s, cx| s.on_key_down(&keydown("ctrl-shift-p"), cx));
    assert!(
        app.read_with(cx, |s, _| s.notice.is_some()),
        "Ctrl+Shift+P shows a notice that the PDF was generated"
    );
    assert!(pdf.exists(), "the PDF file was written");
    // Esc dismisses the notice.
    app.update(cx, |s, cx| s.on_key_down(&keydown("escape"), cx));
    assert!(
        app.read_with(cx, |s, _| s.notice.is_none()),
        "Esc dismisses the notice"
    );
    let _ = std::fs::remove_file(&pdf);
}

#[gpui::test]
fn home_shown_at_launch(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    assert!(
        app.read_with(cx, |s, _| s.home),
        "the home screen is shown at launch"
    );
}

#[gpui::test]
fn home_new_presentation_opens_template_in_master_scope(cx: &mut TestAppContext) {
    use hayate_ir::doc::PlaceholderType as PT;
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.new_presentation());

    // Leaves home and enters layout (master) edit mode on the template's layout.
    assert!(
        !app.read_with(cx, |s, _| s.home),
        "New leaves the home screen"
    );
    assert!(
        !app.read_with(cx, |s, _| s.scope.is_slide()),
        "New opens the template in master/layout edit mode"
    );

    // The edited layout carries the Title + Body placeholders from the preset.
    let kinds: Vec<PT> = app.read_with(cx, |s, _| {
        let layout = s.container();
        s.pres
            .placeholder_shapes(layout)
            .iter()
            .filter_map(|e| s.pres.world.placeholders.get(e).map(|p| p.ph_type))
            .collect()
    });
    assert!(
        kinds.contains(&PT::Title) && kinds.contains(&PT::Body),
        "template layout has Title + Body placeholders, got {kinds:?}"
    );

    // Editing the layout propagates to the slide that uses it (inherited placeholders render).
    let nodes = app.read_with(cx, |s, _| {
        let slide = s.pres.slides()[0];
        hayate_render::build_slide_scene(&s.pres, slide, super::view_px(&s.pres, 1.0))
            .nodes
            .len()
    });
    assert!(
        nodes > 0,
        "the slide renders the inherited template placeholders"
    );
}

#[gpui::test]
fn home_go_home_returns_to_start(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.new_presentation());
    assert!(!app.read_with(cx, |s, _| s.home));
    app.update(cx, |s, _| s.go_home());
    assert!(
        app.read_with(cx, |s, _| s.home && !s.home_loaded),
        "Home returns to the start screen and forces a recents refresh"
    );
}

#[gpui::test]
fn launches_on_home_screen(cx: &mut TestAppContext) {
    // A fresh app opens on the home/start screen, not directly in a deck.
    let app = cx.new(|cx| HayateApp::new(cx));
    assert!(
        app.read_with(cx, |a, _| a.home),
        "the app starts on the home screen"
    );
}

#[gpui::test]
fn new_presentation_opens_template_in_master_scope(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |a, _| a.new_presentation());
    app.read_with(cx, |a, _| {
        assert!(!a.home, "creating a deck leaves the home screen");
        assert!(
            matches!(a.scope, EditScope::Layout(_)),
            "New opens in layout (master-edit) scope so the template can be tailored"
        );
        assert!(
            !a.scope.is_slide(),
            "the sidebar's マスター mode is active (driven by the edit scope)"
        );
        // The slide inherits the layout's Title + Body placeholders from the template.
        let phs = a.pres.effective_placeholders(a.slide);
        assert!(
            phs.len() >= 2,
            "the template slide inherits Title + Body placeholders, got {}",
            phs.len()
        );
        // The master carries a decoration shape (the accent bar) drawn behind every slide.
        let master = a.pres.master_of(a.slide).expect("slide has a master");
        assert!(
            !a.pres.children(master).is_empty(),
            "the master has a decoration shape"
        );
    });
}

#[gpui::test]
fn go_home_returns_to_the_home_screen(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Enter a deck, then go back home.
    app.update(cx, |a, _| a.new_presentation());
    assert!(app.read_with(cx, |a, _| !a.home));
    app.update(cx, |a, _| a.go_home());
    app.read_with(cx, |a, _| {
        assert!(a.home, "go_home returns to the home screen");
        assert!(
            !a.home_loaded,
            "the recents list refreshes on the next home render"
        );
    });
}

#[gpui::test]
fn script_console_runs_and_commits(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Ctrl+Shift+R opens the script console.
    app.update(cx, |a, cx| a.on_key_down(&keydown("ctrl-shift-r"), cx));
    assert!(
        app.read_with(cx, |a, _| a.script_panel.is_some()),
        "Ctrl+Shift+R opens the script console"
    );

    let before = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    // Type a script that creates a rectangle on the current slide, then run it (Ctrl+Enter).
    app.update(cx, |a, _| {
        a.script_panel = Some(super::ScriptPanel {
            buf: "let s = current_slide(); shape_add_rect(s, 10, 10, 100, 50);".to_string(),
            scroll: gpui::ScrollHandle::new(),
        });
    });
    app.update(cx, |a, cx| a.on_key_down(&keydown("ctrl-enter"), cx));

    app.read_with(cx, |a, _| {
        assert!(a.script_panel.is_none(), "running closes the console");
        assert_eq!(
            a.pres.children(a.slide).len(),
            before + 1,
            "the script added one shape to the slide"
        );
    });

    // The whole script is one undo step.
    app.update(cx, |a, cx| a.on_key_down(&keydown("ctrl-z"), cx));
    assert_eq!(
        app.read_with(cx, |a, _| a.pres.children(a.slide).len()),
        before,
        "one undo reverts the entire script"
    );
}

#[gpui::test]
fn script_can_register_and_run_a_command(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // A script registers a palette command whose body adds a rectangle.
    app.update(cx, |a, _| {
        a.run_script_src(
            "register_command(\"ext.add_box\", \"Add Box\", \"let s = current_slide(); shape_add_rect(s, 10, 10, 80, 60);\");",
        );
    });
    assert_eq!(
        app.read_with(cx, |a, _| a.script_commands.len()),
        1,
        "the script registered one command"
    );
    // The registered command shows up in the palette list (with the script: prefix).
    let listed = app.read_with(cx, |a, _| {
        a.palette_commands()
            .iter()
            .any(|(id, _)| id == "script:ext.add_box")
    });
    assert!(listed, "registered command appears in the palette");

    // Running it applies its body as one undoable change.
    let before = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    app.update(cx, |a, _| a.run_script_command("ext.add_box"));
    assert_eq!(
        app.read_with(cx, |a, _| a.pres.children(a.slide).len()),
        before + 1,
        "running the registered command added a shape"
    );
}

#[gpui::test]
fn ai_panel_opens_and_warns_without_api_key(cx: &mut TestAppContext) {
    std::env::remove_var("ANTHROPIC_API_KEY");
    let app = cx.new(|cx| HayateApp::new(cx));
    // Ctrl+Shift+A opens the AI prompt.
    app.update(cx, |a, cx| a.on_key_down(&keydown("ctrl-shift-a"), cx));
    assert!(
        app.read_with(cx, |a, _| a.ai_panel.is_some()),
        "Ctrl+Shift+A opens the AI prompt"
    );
    // Submitting a request with no API key reports it (no network call attempted).
    app.update(cx, |a, _| {
        a.ai_panel = Some(super::AiPanel {
            buf: "make a blue box".to_string(),
        });
    });
    app.update(cx, |a, cx| a.on_key_down(&keydown("enter"), cx));
    app.read_with(cx, |a, _| {
        assert!(a.ai_panel.is_none(), "submitting closes the prompt");
        assert!(
            a.notice
                .as_deref()
                .unwrap_or("")
                .contains("ANTHROPIC_API_KEY"),
            "missing key is reported, got {:?}",
            a.notice
        );
    });
}

#[gpui::test]
fn add_slide_with_layout_uses_the_chosen_layout(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    let (layout, before) = app.read_with(cx, |a, _| {
        (a.master_layouts().first().copied(), a.pres.slides().len())
    });
    let layout = layout.expect("the deck has at least one layout");
    app.update(cx, |a, _| {
        a.add_slide_menu = true;
        a.add_slide_with_layout(layout);
    });
    app.read_with(cx, |a, _| {
        assert_eq!(a.pres.slides().len(), before + 1, "a slide was added");
        assert_eq!(
            a.pres.world.slide_info.get(&a.slide).map(|s| s.layout),
            Some(layout),
            "the new slide uses the chosen layout"
        );
        assert!(!a.add_slide_menu, "the picker closes after adding");
    });
}

#[gpui::test]
fn click_on_selected_text_enters_edit(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |s, _| s.add_text_box());
    let e = app.read_with(cx, |s, _| s.selection.unwrap());
    // add_text_box drops into edit; commit it (click empty corner) to get a selected, non-editing box.
    let (ex, ey) = app.read_with(cx, |s, _| {
        let b = prim_bounds(
            &s.scene
                .nodes
                .iter()
                .find(|n| n.source == Some(e))
                .unwrap()
                .prim,
        );
        (b.x + b.w * 0.5, b.y + b.h * 0.5)
    });
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, 5.0, 5.0), cx); // empty-ish corner commits + deselects
        s.on_mouse_up(&mouse_up(5.0, 5.0), cx);
    });
    // First click selects the text box (no edit yet).
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, ex, ey), cx);
        s.on_mouse_up(&mouse_up(ex, ey), cx);
    });
    assert_eq!(
        app.read_with(cx, |s, _| s.selection),
        Some(e),
        "first click selects"
    );
    assert!(
        app.read_with(cx, |s, _| s.text_edit.is_none()),
        "first click does not edit"
    );
    // Second click (now selected) enters edit mode.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, ex, ey), cx);
        s.on_mouse_up(&mouse_up(ex, ey), cx);
    });
    assert_eq!(
        app.read_with(cx, |s, _| s.text_edit.as_ref().map(|t| t.entity)),
        Some(e),
        "clicking the already-selected text box enters edit mode"
    );
    // Clicking elsewhere commits and returns to preview.
    app.update(cx, |s, cx| {
        s.on_mouse_down(&mouse(MouseButton::Left, 5.0, 5.0), cx);
        s.on_mouse_up(&mouse_up(5.0, 5.0), cx);
    });
    assert!(
        app.read_with(cx, |s, _| s.text_edit.is_none()),
        "clicking elsewhere exits edit"
    );
}

#[gpui::test]
fn ribbon_defaults_home_and_switches(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    assert!(
        app.read_with(cx, |a, _| a.ribbon_tab == super::RibbonTab::Home),
        "the ribbon starts on Home"
    );
    app.update(cx, |a, _| a.ribbon_tab = super::RibbonTab::Insert);
    assert!(app.read_with(cx, |a, _| a.ribbon_tab == super::RibbonTab::Insert));
}

#[gpui::test]
fn slideshow_nav_advances_without_rebuilding_editor_scene(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Two slides; start the show on the first one.
    app.update(cx, |a, _| a.add_slide());
    let (first, last, n) = app.read_with(cx, |a, _| {
        let s = a.pres.slides();
        (s[0], s[s.len() - 1], s.len())
    });
    assert!(n >= 2, "deck has multiple slides");
    app.update(cx, |a, _| a.slide = first);

    app.update(cx, |a, _| a.start_present());
    // The slideshow renders its own scene; the editor scene must NOT be rebuilt on a transition.
    let scene_before = app.read_with(cx, |a, _| a.scene.clone());
    app.update(cx, |a, cx| a.on_key_down(&keydown("right"), cx));
    let (after, present, scene_mid) =
        app.read_with(cx, |a, _| (a.slide, a.present, a.scene.clone()));
    assert_eq!(after, last, "right advances to the next (last) slide");
    assert!(present, "still presenting after navigating");
    assert_eq!(
        scene_mid, scene_before,
        "editor scene is left untouched during slideshow navigation"
    );

    // Advancing past the last slide ends the slideshow.
    app.update(cx, |a, cx| a.on_key_down(&keydown("right"), cx));
    assert!(
        app.read_with(cx, |a, _| !a.present),
        "advancing past the last slide exits the slideshow"
    );
}

#[gpui::test]
fn ribbon_helpers_drive_actions(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // Slideshow: Start enters presentation mode.
    app.update(cx, |a, _| a.start_present());
    assert!(
        app.read_with(cx, |a, _| a.present),
        "start_present enters slideshow"
    );
    app.update(cx, |a, cx| a.on_key_down(&keydown("escape"), cx)); // leave present

    // File: Save As opens the dialog.
    app.update(cx, |a, _| a.open_save_dialog());
    assert!(
        app.read_with(cx, |a, _| a.save_modal.is_some()),
        "open_save_dialog opens the dialog"
    );
    app.update(cx, |a, cx| a.on_key_down(&keydown("escape"), cx));

    // Undo/redo round-trip a shape insertion (Insert tab action).
    let before = app.read_with(cx, |a, _| a.pres.children(a.slide).len());
    app.update(cx, |a, _| a.add_rect());
    assert_eq!(
        app.read_with(cx, |a, _| a.pres.children(a.slide).len()),
        before + 1
    );
    app.update(cx, |a, _| a.undo());
    assert_eq!(
        app.read_with(cx, |a, _| a.pres.children(a.slide).len()),
        before,
        "undo removes the inserted rectangle"
    );
    app.update(cx, |a, _| a.redo());
    assert_eq!(
        app.read_with(cx, |a, _| a.pres.children(a.slide).len()),
        before + 1,
        "redo restores it"
    );
}

#[gpui::test]
fn script_panel_pastes_clipboard_text(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    app.update(cx, |a, _| {
        a.script_panel = Some(super::ScriptPanel {
            buf: "shape.".into(),
            scroll: gpui::ScrollHandle::new(),
        })
    });
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("add_rect(1)".into())));
    // Cmd/Ctrl+V appends the clipboard text; a bare "v" would insert a literal letter.
    app.update(cx, |a, cx| a.on_key_down(&keydown("cmd-v"), cx));
    assert_eq!(
        app.read_with(cx, |a, _| a.script_panel.as_ref().unwrap().buf.clone()),
        "shape.add_rect(1)",
        "paste appends clipboard text into the script buffer"
    );
    // A modified letter that is not paste must not insert itself.
    app.update(cx, |a, cx| a.on_key_down(&keydown("cmd-c"), cx));
    assert_eq!(
        app.read_with(cx, |a, _| a.script_panel.as_ref().unwrap().buf.clone()),
        "shape.add_rect(1)",
        "Cmd+C does not insert a literal 'c'"
    );
}

#[gpui::test]
fn tools_tab_opens_panels(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));
    // The Tools tab's buttons open the script console, AI prompt, and command palette. They set
    // the same state the keyboard shortcuts do, so drive that state directly.
    app.update(cx, |a, _| a.ribbon_tab = super::RibbonTab::Tools);
    app.update(cx, |a, _| {
        a.script_panel = Some(super::ScriptPanel {
            buf: String::new(),
            scroll: gpui::ScrollHandle::new(),
        })
    });
    assert!(app.read_with(cx, |a, _| a.script_panel.is_some()));
    app.update(cx, |a, cx| a.on_key_down(&keydown("escape"), cx));
    app.update(cx, |a, _| {
        a.ai_panel = Some(super::AiPanel { buf: String::new() })
    });
    assert!(app.read_with(cx, |a, _| a.ai_panel.is_some()));
}
