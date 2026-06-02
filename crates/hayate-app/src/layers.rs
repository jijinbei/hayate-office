//! Layers (outline) panel: a left-side list of the slide's objects in z-order, grouped, so
//! the stacking/hierarchy is visible and clickable. FRONT shapes appear at the TOP.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, rgb, ClickEvent, Context};

use hayate_ir::shape::Geometry;
use hayate_ir::world::Entity;
use hayate_model::edit::group_members;

use crate::HayateApp;

impl HayateApp {
    /// The layers panel element: the current slide's objects in stacking order (front at top),
    /// grouped and indented, each row clickable to select that object.
    pub(crate) fn layers_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Children come back in document order (BACK-to-FRONT); the panel shows FRONT first.
        let children: Vec<Entity> = self.pres.children(self.slide);
        let front_to_back: Vec<Entity> = children.iter().rev().copied().collect();
        // Distinguishing labels: number each kind in creation order (Rectangle 1, Ellipse 1, ...).
        let labels = self.numbered_labels(&children);

        let mut panel = div()
            .flex()
            .flex_col()
            .w(px(180.))
            .p_2()
            .bg(rgb(0x202020))
            .text_color(rgb(0xffffff))
            .child(div().text_sm().text_color(rgb(0x8a8a8a)).child("Layers"));

        // Track which group keys have already been emitted so a group is listed once.
        let mut seen_groups: Vec<u64> = Vec::new();
        // Monotonic row index for stable element ids and hover/click identity.
        let mut index: usize = 0;

        for &e in &front_to_back {
            match self.pres.world.groups.get(&e) {
                Some(&key) => {
                    if seen_groups.contains(&key) {
                        // A later member of an already-emitted group: skip.
                        continue;
                    }
                    seen_groups.push(key);
                    let group_no = seen_groups.len();
                    let members: Vec<Entity> = front_to_back
                        .iter()
                        .copied()
                        .filter(|m| self.pres.world.groups.get(m) == Some(&key))
                        .collect();
                    let first = members[0];

                    // Group header row — clicking it selects the whole group.
                    panel = panel.child(
                        div()
                            .id(("group", group_no))
                            .px_2()
                            .py_1()
                            .text_sm()
                            .rounded_md()
                            .hover(|s| s.bg(rgb(0x2f2f2f)))
                            .child(format!("\u{1F4C1} Group {group_no}"))
                            .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                                window.focus(&this.focus, cx);
                                this.selection = Some(first);
                                this.also = group_members(&this.pres.world, first)
                                    .into_iter()
                                    .filter(|&m| m != first)
                                    .collect();
                                cx.notify();
                            })),
                    );

                    // Members of this group, front-to-back, indented under the header.
                    for &m in &members {
                        let lbl = labels.get(&m).cloned().unwrap_or_default();
                        panel = panel.child(self.layer_row(lbl, m, index, 1, cx));
                        index += 1;
                    }
                }
                None => {
                    let lbl = labels.get(&e).cloned().unwrap_or_default();
                    panel = panel.child(self.layer_row(lbl, e, index, 0, cx));
                    index += 1;
                }
            }
        }

        panel.into_any_element()
    }

    /// One clickable layer row for entity `e` with the given `label`. `depth` indents it under
    /// group headers. Single-click selects (expanding to the group); double-click renames it.
    fn layer_row(
        &self,
        label: String,
        e: Entity,
        index: usize,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selected = self.selection == Some(e) || self.also.contains(&e);
        let editing = matches!(&self.renaming, Some((re, _)) if *re == e);
        let initial = label.clone();

        let shown = if editing {
            let buf = self
                .renaming
                .as_ref()
                .map(|(_, b)| b.clone())
                .unwrap_or_default();
            format!("{buf}|")
        } else {
            label
        };

        let mut row = div()
            .id(("layer", index))
            .py_1()
            .pr_2()
            // Indentation conveys nesting depth (each level adds a step).
            .pl(px(8.0 + depth as f32 * 16.0))
            .text_sm()
            .rounded_md()
            .hover(|s| s.bg(rgb(0x2f2f2f)))
            .child(shown);

        if editing {
            row = row.bg(rgb(0x1f3a5f));
        } else if selected {
            row = row.bg(rgb(crate::SELECTION));
        }

        row.on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            if ev.click_count() >= 2 {
                // Double-click: start renaming this layer, pre-filled with its current name.
                this.renaming = Some((e, initial.clone()));
            } else {
                this.selection = Some(e);
                this.also = group_members(&this.pres.world, e)
                    .into_iter()
                    .filter(|&m| m != e)
                    .collect();
            }
            cx.notify();
        }))
        .into_any_element()
    }

    /// Build a distinguishing label per object, numbering each kind in creation order
    /// (Rectangle 1, Rectangle 2, Ellipse 1, Image 1, ...). Text shows its content.
    fn numbered_labels(&self, children: &[Entity]) -> HashMap<Entity, String> {
        let mut labels = HashMap::new();
        let (mut rect, mut ell, mut img, mut txt) = (0u32, 0u32, 0u32, 0u32);
        for &e in children {
            // A user-set name (via the Layers panel) always wins.
            if let Some(name) = self.pres.world.names.get(&e) {
                if !name.trim().is_empty() {
                    labels.insert(e, name.clone());
                    continue;
                }
            }
            let label = if let Some(body) = self.pres.world.texts.get(&e) {
                txt += 1;
                let content = body
                    .paragraphs
                    .first()
                    .and_then(|p| p.runs.first())
                    .map(|r| r.text.trim())
                    .unwrap_or("");
                if content.is_empty() {
                    format!("Text {txt}")
                } else {
                    let t: String = content.chars().take(18).collect();
                    if content.chars().count() > 18 {
                        format!("{t}\u{2026}")
                    } else {
                        t
                    }
                }
            } else if self.pres.world.pictures.contains_key(&e) {
                img += 1;
                format!("Image {img}")
            } else {
                match self.pres.world.geometries.get(&e) {
                    Some(Geometry::Ellipse) => {
                        ell += 1;
                        format!("Ellipse {ell}")
                    }
                    Some(_) => {
                        rect += 1;
                        format!("Rectangle {rect}")
                    }
                    None => "Shape".to_string(),
                }
            };
            labels.insert(e, label);
        }
        labels
    }
}
