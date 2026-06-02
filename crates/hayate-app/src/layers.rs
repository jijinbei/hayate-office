//! Layers (outline) panel: a left-side list of the slide's objects in z-order, grouped, so
//! the stacking/hierarchy is visible and clickable. FRONT shapes appear at the TOP.

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
                        panel = panel.child(self.layer_row(m, index, true, cx));
                        index += 1;
                    }
                }
                None => {
                    panel = panel.child(self.layer_row(e, index, false, cx));
                    index += 1;
                }
            }
        }

        panel.into_any_element()
    }

    /// One clickable layer row for entity `e`. `indented` nests it under a group header.
    fn layer_row(
        &self,
        e: Entity,
        index: usize,
        indented: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let label = self.layer_label(e);
        let selected = self.selection == Some(e) || self.also.contains(&e);

        let mut row = div()
            .id(("layer", index))
            .px_2()
            .py_1()
            .text_sm()
            .rounded_md()
            .hover(|s| s.bg(rgb(0x2f2f2f)))
            .child(label);

        if indented {
            row = row.pl_3();
        }
        if selected {
            row = row.bg(rgb(crate::SELECTION));
        }

        row.on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            this.selection = Some(e);
            this.also = group_members(&this.pres.world, e)
                .into_iter()
                .filter(|&m| m != e)
                .collect();
            cx.notify();
        }))
        .into_any_element()
    }

    /// Human-readable label for a layer row, derived from the entity's components.
    fn layer_label(&self, e: Entity) -> String {
        if let Some(body) = self.pres.world.texts.get(&e) {
            let first = body
                .paragraphs
                .first()
                .and_then(|p| p.runs.first())
                .map(|r| r.text.as_str())
                .unwrap_or("");
            let trimmed = first.trim();
            let truncated: String = trimmed.chars().take(20).collect();
            let label = if trimmed.chars().count() > 20 {
                format!("{truncated}\u{2026}")
            } else {
                truncated
            };
            return if label.is_empty() {
                "Text".to_string()
            } else {
                label
            };
        }
        if self.pres.world.pictures.contains_key(&e) {
            return "Image".to_string();
        }
        match self.pres.world.geometries.get(&e) {
            Some(Geometry::Ellipse) => "Ellipse".to_string(),
            Some(Geometry::Rect) | Some(Geometry::RoundRect { .. }) => "Rectangle".to_string(),
            None => "Shape".to_string(),
        }
    }
}
