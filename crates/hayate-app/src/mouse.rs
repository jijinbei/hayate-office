//! Mouse interaction on the canvas: selection, drag-to-move with guide snapping, and
//! resize-by-handle.

use gpui::{Context, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

use hayate_ir::geom::{PointEmu, RectEmu};
use hayate_ir::world::{CompValue, Entity};
use hayate_model::{edit, Operation, Transaction};
use hayate_render::{alignment_guides, hit_test, resize_handles};

use crate::util::{prim_bounds, resize_frame};
use crate::{Drag, HayateApp, ResizeDrag};

impl HayateApp {
    /// Pixels per EMU (width-fit).
    pub(crate) fn scale(&self) -> f64 {
        self.scene.size.w as f64 / self.pres.slide_size.w.max(1) as f64
    }

    pub(crate) fn on_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        // This low-level handler fires even when the click lands on the context-menu overlay
        // (which sits above the canvas). If a menu is open, just dismiss it and do nothing else
        // — otherwise a click on a menu item below the shapes would start a marquee and clear
        // the selection before the menu action (e.g. Group) runs.
        if self.context_menu.take().is_some() {
            cx.notify();
            return;
        }
        let o = self.canvas_origin.get();
        let x = f32::from(ev.position.x - o.x);
        let y = f32::from(ev.position.y - o.y);
        // Double-click enters in-canvas text editing on the shape under the cursor.
        if ev.click_count >= 2 {
            if let Some(e) = hit_test(&self.scene, x, y) {
                if self.pres.world.texts.contains_key(&e) {
                    self.selection = Some(e);
                    self.also.clear();
                    self.begin_text_edit(e);
                    cx.notify();
                    return;
                }
            }
        }
        // Grab a resize handle on the current (axis-aligned) selection?
        if let Some(sel) = self.selection {
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
        // Drag moves every selected shape (group / multi-select) together.
        let entities: Vec<(Entity, RectEmu)> = self
            .selected_all()
            .into_iter()
            .filter_map(|e| self.pres.world.frames.get(&e).map(|f| (e, *f)))
            .collect();
        self.drag = hit.filter(|_| !entities.is_empty()).map(|h| Drag {
            entities,
            primary: h,
            start_cursor: ev.position,
        });
        cx.notify();
    }

    pub(crate) fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        // While a marquee is active, just track the current corner.
        if let Some((sx, sy, _, _)) = self.marquee {
            let o = self.canvas_origin.get();
            let x = f32::from(ev.position.x - o.x);
            let y = f32::from(ev.position.y - o.y);
            self.marquee = Some((sx, sy, x, y));
            cx.notify();
            return;
        }
        if let Some(rd) = &self.resize {
            let scale = self.scale();
            if scale <= 0.0 {
                return;
            }
            let dx = (f32::from(ev.position.x - rd.start_cursor.x) as f64 / scale) as i64;
            let dy = (f32::from(ev.position.y - rd.start_cursor.y) as f64 / scale) as i64;
            let mut nf = resize_frame(rd.handle, rd.start_frame, dx, dy);
            // Shift preserves the original aspect ratio (height follows width).
            if ev.modifiers.shift {
                let sf = rd.start_frame;
                if sf.size.h > 0 {
                    let ar = sf.size.w as f64 / sf.size.h as f64;
                    nf.size.h = ((nf.size.w as f64 / ar).round() as i64).max(12_700);
                }
            }
            if let Some(e) = self.selection {
                self.pres.world.frames.insert(e, nf);
                self.rebuild();
                cx.notify();
            }
            return;
        }
        let Some(d) = self.drag.clone() else { return };
        let scale = self.scale();
        if scale <= 0.0 {
            return;
        }
        let dx = (f32::from(ev.position.x - d.start_cursor.x) as f64 / scale) as i64;
        let dy = (f32::from(ev.position.y - d.start_cursor.y) as f64 / scale) as i64;
        // Snap the primary shape to guides, then apply the same (snapped) delta to every member.
        let Some(&(_, prim_start)) = d.entities.iter().find(|(e, _)| *e == d.primary) else {
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
    }

    /// Snap `nf` so the moving shape's edges/center align to nearby guide lines: the slide
    /// edges & center and every other shape's edges & center, within a small pixel radius.
    pub(crate) fn snap_frame(&self, moving: Entity, nf: RectEmu) -> RectEmu {
        let scale = self.scale();
        if scale <= 0.0 {
            return nf;
        }
        // Snap radius (~8 device px) expressed in EMU.
        let thr = (8.0 / scale) as i64;
        let (sw, sh) = (self.pres.slide_size.w, self.pres.slide_size.h);
        let mut xs = vec![0, sw / 2, sw];
        let mut ys = vec![0, sh / 2, sh];
        for other in self.pres.children(self.slide) {
            if other == moving {
                continue;
            }
            if let Some(f) = self.pres.world.frames.get(&other) {
                xs.extend([f.origin.x, f.origin.x + f.size.w / 2, f.origin.x + f.size.w]);
                ys.extend([f.origin.y, f.origin.y + f.size.h / 2, f.origin.y + f.size.h]);
            }
        }
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
