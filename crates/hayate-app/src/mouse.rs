//! Mouse interaction on the canvas: selection, drag-to-move with guide snapping, and
//! resize-by-handle.

use gpui::{Context, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_ir::world::{CompValue, Entity};
use hayate_model::{edit, Operation, Transaction};
use hayate_render::{alignment_guides, hit_test, resize_handles};

use crate::util::{prim_bounds, resize_frame};
use crate::{Drag, HayateApp, LineDrag, ResizeDrag};

impl HayateApp {
    /// Pixels per EMU (width-fit).
    pub(crate) fn scale(&self) -> f64 {
        self.scene.size.w as f64 / self.pres.slide_size.w.max(1) as f64
    }

    pub(crate) fn on_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        // This low-level handler fires even when the click lands on the context-menu overlay
        // (which sits above the canvas). While a menu is open, do nothing here — the menu's
        // backdrop/items handle dismissal on click (mouse-up). Closing it on mouse-down would
        // remove the menu before the item's click (e.g. Group) could run, and acting on the
        // click would start a marquee that clears the selection the menu action needs.
        if self.context_menu.is_some() {
            return;
        }
        // A click commits an in-progress text edit (Enter now inserts newlines instead). Revert
        // the live preview, then commit the final buffer as one undoable transaction.
        if let Some(te) = self.text_edit.take() {
            // Revert the live preview to the original, then commit the final Typst source as one
            // undoable transaction (so undo restores the pre-edit box, not the live-preview state).
            self.revert_text_live(te.entity, &te.original);
            let tx = edit::set_typst_source(&self.pres.world, te.entity, te.buf.clone());
            self.commit_tx(tx);
        }
        let o = self.canvas_origin.get();
        let x = f32::from(ev.position.x - o.x);
        let y = f32::from(ev.position.y - o.y);
        // Double-click detection. The Linux backends do not populate `click_count`, so detect it
        // ourselves: a second press near the previous one within 450ms. (Tests set `click_count`
        // explicitly, which is honored too.) On a double, consume the streak so a following click
        // is a fresh single click.
        let now = std::time::Instant::now();
        let is_double = ev.click_count >= 2
            || self.last_click.is_some_and(|(t, lx, ly)| {
                now.duration_since(t).as_millis() <= 450 && (x - lx).hypot(y - ly) <= 6.0
            });
        self.last_click = if is_double { None } else { Some((now, x, y)) };
        // Double-click drills into a group: select just the shape under the cursor (clearing the
        // group/multi-selection) so it can be moved or edited on its own. For a text shape it
        // also starts in-canvas text editing.
        if is_double {
            if let Some(e) = hit_test(&self.scene, x, y) {
                self.selection = Some(e);
                self.also.clear();
                if self.pres.world.texts.contains_key(&e) {
                    self.begin_text_edit(e);
                }
                cx.notify();
                return;
            }
        }
        // Double-click an inherited placeholder/prompt on a slide: promote it to an editable
        // slide-level override and start typing. Inherited placeholders are display-only (no scene
        // `source`), so they are not found by `hit_test`; test their resolved bounds directly.
        // Gated on a double-click (and no real shape under the cursor) so a single click leaves the
        // locked, layout-owned placeholder untouched — matching how the panel shows it as locked and
        // how every other text shape begins editing on double-click. A bare click no longer creates
        // a phantom slide-level copy.
        if is_double && self.scope.is_slide() && hit_test(&self.scene, x, y).is_none() {
            let scale = self.scale();
            if scale > 0.0 {
                let ex = (x as f64 / scale) as i64;
                let ey = (y as f64 / scale) as i64;
                for ph in self.pres.effective_placeholders(self.slide) {
                    if self.pres.find_placeholder(self.slide, ph).is_some() {
                        continue; // already overridden on the slide (hit-tested normally)
                    }
                    if let Some(fr) = self.pres.ph_frame(self.slide, ph) {
                        let (x0, x1) = (
                            fr.origin.x.min(fr.origin.x + fr.size.w),
                            fr.origin.x.max(fr.origin.x + fr.size.w),
                        );
                        let (y0, y1) = (
                            fr.origin.y.min(fr.origin.y + fr.size.h),
                            fr.origin.y.max(fr.origin.y + fr.size.h),
                        );
                        if ex >= x0 && ex <= x1 && ey >= y0 && ey <= y1 {
                            self.promote_and_edit(ph);
                            cx.notify();
                            return;
                        }
                    }
                }
            }
        }
        // Grab a line endpoint? A line is moved by its two endpoints, which lets it point in any
        // direction (the frame size may become negative).
        if let Some(sel) = self.selection {
            if let Some(node) = self.scene.nodes.iter().find(|n| n.source == Some(sel)) {
                if let hayate_render::scene::Primitive::Line { from, to, .. } = &node.prim {
                    let near = |hx: f32, hy: f32| (x - hx).hypot(y - hy) < 8.0;
                    let on_from = near(from.0, from.1);
                    let on_to = near(to.0, to.1);
                    if on_from || on_to {
                        if let Some(f) = self.pres.world.frames.get(&sel).copied() {
                            // World endpoints: from = origin, to = origin + size (signed).
                            let from_w = f.origin;
                            let to_w = hayate_ir::geom::PointEmu::new(
                                f.origin.x + f.size.w,
                                f.origin.y + f.size.h,
                            );
                            // Grabbing the END drags `to`; the START (`from`) stays fixed.
                            let drag_end = on_to && !on_from;
                            let fixed = if drag_end { from_w } else { to_w };
                            self.line_drag = Some(LineDrag {
                                entity: sel,
                                drag_end,
                                fixed,
                                start_frame: f,
                            });
                            cx.notify();
                            return;
                        }
                    }
                }
            }
        }
        // Grab a resize handle on the current (axis-aligned) selection? Locked placeholders keep
        // their layout-defined geometry, so they expose no resize handles.
        if let Some(sel) = self.selection.filter(|&s| !self.is_locked_placeholder(s)) {
            if let Some(node) = self.scene.nodes.iter().find(|n| n.source == Some(sel)) {
                if node.rotation_deg.abs() < 1e-3 {
                    let r = prim_bounds(&node.prim);
                    for (i, (hx, hy)) in resize_handles(r, 0.0).iter().enumerate() {
                        if (x - hx).hypot(y - hy) < 8.0 {
                            if let Some(f) = self.pres.world.frames.get(&sel).copied() {
                                self.resize = Some(ResizeDrag {
                                    handle: i,
                                    start_frame: f,
                                    start_cursor: ev.position,
                                });
                                cx.notify();
                                return;
                            }
                        }
                    }
                }
            }
        }
        let hit = hit_test(&self.scene, x, y);
        if ev.modifiers.shift {
            // Shift-click adds/keeps a multi-selection (no drag).
            if let Some(h) = hit {
                if Some(h) != self.selection && !self.also.contains(&h) {
                    self.also.push(h);
                }
            }
            cx.notify();
            return;
        }
        // Dragging on empty canvas starts a marquee (rubber-band) selection.
        if hit.is_none() {
            self.selection = None;
            self.also.clear();
            self.drag = None;
            self.marquee = Some((x, y, x, y));
            cx.notify();
            return;
        }
        // If the clicked shape is already part of the current selection (a multi-select or a
        // group), keep the whole selection so the drag moves them all together. Otherwise
        // select just this shape, expanded to its group.
        let already_selected = hit.map_or(false, |h| self.selected_all().contains(&h));
        if !already_selected {
            self.also.clear();
            self.selection = hit;
            if let Some(h) = hit {
                let members = hayate_model::edit::group_members(&self.pres.world, h);
                if members.len() > 1 {
                    self.also = members.into_iter().filter(|&m| m != h).collect();
                }
            }
        }
        // Drag moves every selected shape (group / multi-select) together — except a locked
        // placeholder, whose position is fixed by the layout (it can still be selected and have its
        // text edited, just not moved).
        let entities: Vec<(Entity, RectEmu)> = self
            .selected_all()
            .into_iter()
            .filter_map(|e| self.pres.world.frames.get(&e).map(|f| (e, *f)))
            .collect();
        let hit_locked = hit.is_some_and(|h| self.is_locked_placeholder(h));
        self.drag = hit
            .filter(|_| !entities.is_empty() && !hit_locked)
            .map(|h| Drag {
                entities,
                primary: h,
                start_cursor: ev.position,
            });
        cx.notify();
    }

    pub(crate) fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        // Dragging a line endpoint: keep the fixed end, move the other to the cursor. The frame
        // size may go negative, so the line can point in any of the 360 degrees.
        if let Some(ld) = self.line_drag {
            let scale = self.scale();
            if scale <= 0.0 {
                return;
            }
            let o = self.canvas_origin.get();
            let cx_emu = (f32::from(ev.position.x - o.x) as f64 / scale) as i64;
            let cy_emu = (f32::from(ev.position.y - o.y) as f64 / scale) as i64;
            let cursor = PointEmu::new(cx_emu, cy_emu);
            let (from, to) = if ld.drag_end {
                (ld.fixed, cursor)
            } else {
                (cursor, ld.fixed)
            };
            let nf = RectEmu {
                origin: from,
                size: hayate_ir::geom::SizeEmu::new(to.x - from.x, to.y - from.y),
            };
            self.pres.world.frames.insert(ld.entity, nf);
            self.rebuild();
            cx.notify();
            return;
        }
        // While a marquee is active, just track the current corner.
        if let Some((sx, sy, _, _)) = self.marquee {
            let o = self.canvas_origin.get();
            let x = f32::from(ev.position.x - o.x);
            let y = f32::from(ev.position.y - o.y);
            self.marquee = Some((sx, sy, x, y));
            cx.notify();
            return;
        }
        if let Some(rd) = self.resize.clone() {
            let scale = self.scale();
            if scale <= 0.0 {
                return;
            }
            let Some(e) = self.selection else { return };
            let dx = (f32::from(ev.position.x - rd.start_cursor.x) as f64 / scale) as i64;
            let dy = (f32::from(ev.position.y - rd.start_cursor.y) as f64 / scale) as i64;
            let mut nf = resize_frame(rd.handle, rd.start_frame, dx, dy);
            if ev.modifiers.shift {
                // Shift preserves the original aspect ratio (height follows width); skip snapping
                // so the locked ratio is not broken by an edge snap.
                let sf = rd.start_frame;
                if sf.size.h > 0 {
                    let ar = sf.size.w as f64 / sf.size.h as f64;
                    nf.size.h = ((nf.size.w as f64 / ar).round() as i64).max(12_700);
                }
            } else {
                // Snap the moving edge(s) to the same alignment lines used while dragging.
                nf = self.snap_resize(rd.handle, e, nf);
            }
            self.pres.world.frames.insert(e, nf);
            self.rebuild();
            self.update_guides(e);
            cx.notify();
            return;
        }
        // Bail before taking the drag so an early return here doesn't drop an active drag.
        let scale = self.scale();
        if scale <= 0.0 {
            return;
        }
        // Take the drag out so the snap/guide code below can borrow `&mut self`; restore it on
        // every path that doesn't legitimately end the drag.
        let Some(d) = self.drag.take() else { return };
        let dx = (f32::from(ev.position.x - d.start_cursor.x) as f64 / scale) as i64;
        let dy = (f32::from(ev.position.y - d.start_cursor.y) as f64 / scale) as i64;
        // Snap the primary shape to guides, then apply the same (snapped) delta to every member.
        let Some(&(_, prim_start)) = d.entities.iter().find(|(e, _)| *e == d.primary) else {
            // Primary should always be in `entities`; restore the drag and bail without dropping it.
            self.drag = Some(d);
            return;
        };
        let prim_nf = RectEmu {
            origin: PointEmu::new(prim_start.origin.x + dx, prim_start.origin.y + dy),
            size: prim_start.size,
        };
        let snapped = self.snap_frame(d.primary, prim_nf);
        let (sdx, sdy) = (
            snapped.origin.x - prim_start.origin.x,
            snapped.origin.y - prim_start.origin.y,
        );
        for (e, start) in &d.entities {
            let nf = RectEmu {
                origin: PointEmu::new(start.origin.x + sdx, start.origin.y + sdy),
                size: start.size,
            };
            self.pres.world.frames.insert(*e, nf); // live preview, no history
        }
        self.rebuild();
        self.update_guides(d.primary);
        cx.notify();
        // Restore the transiently-taken drag so the move stays active.
        self.drag = Some(d);
    }

    /// Snap `nf` so the moving shape's edges/center align to nearby guide lines: the slide
    /// edges & center and every other shape's edges & center, within a small pixel radius.
    /// Candidate alignment lines (in EMU) the moving shape can snap to: the slide edges & center
    /// on each axis, plus every other shape's edges & center. Shared by move- and resize-snapping.
    fn snap_candidates(&self, moving: Entity) -> (Vec<i64>, Vec<i64>) {
        let (sw, sh) = (self.pres.slide_size.w, self.pres.slide_size.h);
        let mut xs = vec![0, sw / 2, sw];
        let mut ys = vec![0, sh / 2, sh];
        for other in self.pres.children(self.container()) {
            if other == moving {
                continue;
            }
            if let Some(f) = self.pres.world.frames.get(&other) {
                xs.extend([f.origin.x, f.origin.x + f.size.w / 2, f.origin.x + f.size.w]);
                ys.extend([f.origin.y, f.origin.y + f.size.h / 2, f.origin.y + f.size.h]);
            }
        }
        (xs, ys)
    }

    /// Snap a resized frame so the edge(s) the grabbed handle moves align to nearby guide lines.
    /// Only the moving edges are adjusted (the anchored edges stay put), and a minimum size is
    /// preserved. `handle` follows the [`resize_handles`] ordering (TL, T, TR, R, BR, B, BL, L).
    pub(crate) fn snap_resize(&self, handle: usize, moving: Entity, nf: RectEmu) -> RectEmu {
        let scale = self.scale();
        if scale <= 0.0 {
            return nf;
        }
        let thr = (8.0 / scale) as i64;
        let min = 12_700; // 1pt, matching resize_frame
        let (xs, ys) = self.snap_candidates(moving);
        // Smallest delta moving `v` onto a candidate within the snap radius.
        let nearest = |v: i64, cands: &[i64]| -> Option<i64> {
            let mut best: Option<i64> = None;
            for &c in cands {
                let d = c - v;
                if d.abs() <= thr && best.map_or(true, |b: i64| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
            best
        };
        let (mut x, mut w) = (nf.origin.x, nf.size.w);
        let (mut y, mut h) = (nf.origin.y, nf.size.h);
        if matches!(handle, 2 | 3 | 4) {
            // Right edge moves: snap it, keeping the left edge fixed.
            if let Some(d) = nearest(x + w, &xs) {
                w = (w + d).max(min);
            }
        } else if matches!(handle, 0 | 6 | 7) {
            // Left edge moves: snap it, keeping the right edge fixed.
            if let Some(d) = nearest(x, &xs) {
                let right = x + w;
                x = (x + d).min(right - min);
                w = right - x;
            }
        }
        if matches!(handle, 4 | 5 | 6) {
            // Bottom edge moves: snap it, keeping the top edge fixed.
            if let Some(d) = nearest(y + h, &ys) {
                h = (h + d).max(min);
            }
        } else if matches!(handle, 0 | 1 | 2) {
            // Top edge moves: snap it, keeping the bottom edge fixed.
            if let Some(d) = nearest(y, &ys) {
                let bottom = y + h;
                y = (y + d).min(bottom - min);
                h = bottom - y;
            }
        }
        RectEmu {
            origin: PointEmu::new(x, y),
            size: SizeEmu::new(w, h),
        }
    }

    pub(crate) fn snap_frame(&self, moving: Entity, nf: RectEmu) -> RectEmu {
        let scale = self.scale();
        if scale <= 0.0 {
            return nf;
        }
        // Snap radius (~8 device px) expressed in EMU.
        let thr = (8.0 / scale) as i64;
        let (xs, ys) = self.snap_candidates(moving);
        // Best (smallest) delta aligning any of [start, center, end] to a candidate line.
        let best = |start: i64, size: i64, cands: &[i64]| -> Option<i64> {
            let anchors = [start, start + size / 2, start + size];
            let mut best: Option<i64> = None;
            for a in anchors {
                for &c in cands {
                    let d = c - a;
                    if d.abs() <= thr && best.map_or(true, |b: i64| d.abs() < b.abs()) {
                        best = Some(d);
                    }
                }
            }
            best
        };
        let ox = nf.origin.x + best(nf.origin.x, nf.size.w, &xs).unwrap_or(0);
        let oy = nf.origin.y + best(nf.origin.y, nf.size.h, &ys).unwrap_or(0);
        RectEmu {
            origin: PointEmu::new(ox, oy),
            size: nf.size,
        }
    }

    /// Recompute alignment guides for the moving shape against the others (scene px coords).
    pub(crate) fn update_guides(&mut self, moving_entity: Entity) {
        let mut moving = None;
        let mut others = Vec::new();
        for n in &self.scene.nodes {
            let r = prim_bounds(&n.prim);
            if n.source == Some(moving_entity) {
                moving = Some(r);
            } else {
                others.push(r);
            }
        }
        self.guides = match moving {
            Some(m) => alignment_guides(m, &others, 6.0),
            None => Vec::new(),
        };
    }

    pub(crate) fn on_mouse_up(&mut self, _ev: &MouseUpEvent, cx: &mut Context<Self>) {
        // Commit a line endpoint drag as one undoable step (revert the live preview first).
        if let Some(ld) = self.line_drag.take() {
            if let Some(final_f) = self.pres.world.frames.get(&ld.entity).copied() {
                if final_f != ld.start_frame {
                    self.pres.world.frames.insert(ld.entity, ld.start_frame);
                    let tx = edit::set_frame(ld.entity, final_f);
                    self.commit_tx(tx);
                }
            }
            cx.notify();
            return;
        }
        // Finalize a marquee: select every shape whose bounds intersect the rect.
        if let Some((sx, sy, cx0, cy0)) = self.marquee.take() {
            let rx = sx.min(cx0);
            let ry = sy.min(cy0);
            let rw = (sx - cx0).abs();
            let rh = (sy - cy0).abs();
            self.also.clear();
            // Ignore tiny marquees (treat as a click on empty space): clear selection.
            if rw < 3.0 && rh < 3.0 {
                self.selection = None;
                cx.notify();
                return;
            }
            let mut hits: Vec<Entity> = Vec::new();
            for node in &self.scene.nodes {
                let Some(src) = node.source else { continue };
                let b = prim_bounds(&node.prim);
                // Standard AABB overlap test.
                let disjoint = b.x + b.w < rx || rx + rw < b.x || b.y + b.h < ry || ry + rh < b.y;
                if !disjoint {
                    hits.push(src);
                }
            }
            self.selection = hits.first().copied();
            self.also = hits.into_iter().skip(1).collect();
            cx.notify();
            return;
        }
        if let Some(rd) = self.resize.take() {
            self.guides.clear();
            if let Some(e) = self.selection {
                if let Some(final_f) = self.pres.world.frames.get(&e).copied() {
                    if final_f != rd.start_frame {
                        self.pres.world.frames.insert(e, rd.start_frame);
                        let tx = edit::set_frame(e, final_f);
                        self.commit_tx(tx);
                    }
                }
            }
            cx.notify();
            return;
        }
        self.guides.clear();
        let Some(d) = self.drag.take() else { return };
        // Revert every member to its start, then commit the whole move as one undoable step.
        let mut ops = Vec::new();
        for (e, start) in &d.entities {
            if let Some(final_f) = self.pres.world.frames.get(e).copied() {
                if final_f != *start {
                    self.pres.world.frames.insert(*e, *start);
                    ops.push(Operation::SetComponent {
                        entity: *e,
                        value: CompValue::Frame(final_f),
                    });
                }
            }
        }
        if !ops.is_empty() {
            self.commit_tx(Transaction::new("move", ops));
        }
        cx.notify();
    }
}
