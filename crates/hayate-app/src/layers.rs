//! Layers (outline) panel: a left-side list of the slide's objects in z-order, grouped, so
//! the stacking/hierarchy is visible and clickable. FRONT shapes appear at the TOP.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, rgb, svg, ClickEvent, Context};

use hayate_ir::shape::{ArrowHead, Geometry};
use hayate_ir::world::Entity;
use hayate_model::edit::group_members;

use crate::HayateApp;

impl HayateApp {
    /// The layers panel element. In slide mode it has three stable sections so placeholders never
    /// "jump" as they are filled: **Placeholders** (every effective placeholder, by type, locked —
    /// only its text changes in place), the slide's own **free shapes**, and the locked **layout /
    /// master decorations**. In master mode it falls back to a plain z-order list of the container.
    pub(crate) fn layers_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut index: usize = 0;

        if self.scope.is_slide() {
            // 1) Placeholders — keyed on the placeholder ref, so each one keeps its row whether it
            // is still inherited or has slide-specific text; filling it just updates the row.
            let phs = self.pres.effective_placeholders(self.slide);
            if !phs.is_empty() {
                rows.push(Self::section_header("Placeholders"));
                for ph in phs {
                    rows.push(self.placeholder_row(ph, index, cx));
                    index += 1;
                }
            }
            // 2) The slide's own free shapes (everything that is not a placeholder), front at top.
            let free: Vec<Entity> = self
                .pres
                .children(self.slide)
                .into_iter()
                .filter(|e| !self.pres.world.placeholders.contains_key(e))
                .collect();
            if !free.is_empty() {
                rows.push(Self::section_header("このスライドの図形"));
                let f2b: Vec<Entity> = free.iter().rev().copied().collect();
                let labels = self.numbered_labels(&free);
                let mut group_no: u32 = 0;
                self.build_rows(&f2b, 0, &labels, &mut group_no, &mut index, &mut rows, cx);
            }
            // 3) Locked decorations inherited from the layout and master (non-placeholder shapes).
            if let Some(layout) = self.pres.layout_of(self.slide) {
                let name = self
                    .pres
                    .world
                    .layout_info
                    .get(&layout)
                    .map(|li| li.name.clone())
                    .unwrap_or_else(|| "Layout".to_string());
                self.push_locked_section(
                    format!("レイアウト「{name}」"),
                    layout,
                    &mut index,
                    &mut rows,
                    cx,
                );
            }
            if let Some(master) = self.pres.master_of(self.slide) {
                self.push_locked_section("マスター".to_string(), master, &mut index, &mut rows, cx);
            }
        } else {
            // Master/layout editing: a plain z-order list of the edited container.
            let children = self.pres.children(self.container());
            let f2b: Vec<Entity> = children.iter().rev().copied().collect();
            let labels = self.numbered_labels(&children);
            let mut group_no: u32 = 0;
            rows.push(Self::section_header("Layers"));
            self.build_rows(&f2b, 0, &labels, &mut group_no, &mut index, &mut rows, cx);
        }

        div()
            .flex()
            .flex_col()
            .w(px(180.))
            .p_2()
            .bg(rgb(0x202020))
            .text_color(rgb(0xffffff))
            .children(rows)
            .into_any_element()
    }

    /// A muted section header row.
    fn section_header(title: &str) -> gpui::AnyElement {
        div()
            .pt_2()
            .pb_1()
            .text_sm()
            .text_color(rgb(0x8a8a8a))
            .child(title.to_string())
            .into_any_element()
    }

    /// A human label for a placeholder type (the section row's left part).
    fn ph_type_name(ph: hayate_ir::doc::PlaceholderType) -> &'static str {
        use hayate_ir::doc::PlaceholderType as PT;
        match ph {
            PT::Title => "Title",
            PT::CenteredTitle => "Title",
            PT::Subtitle => "Subtitle",
            PT::Body => "Body",
            PT::Picture => "Picture",
            PT::Chart => "Chart",
            PT::Table => "Table",
            PT::Date => "Date",
            PT::Footer => "Footer",
            PT::SlideNumber => "Slide #",
        }
    }

    /// One row in the unified Placeholders section, keyed on the placeholder ref `ph`. Shows the
    /// type name plus the resolved text (or "(空)" when empty), always locked. Single-click selects
    /// it (materializing a text-only override if needed); double-click edits its text.
    fn placeholder_row(
        &self,
        ph: hayate_ir::doc::PlaceholderRef,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = self.pres.find_placeholder(self.slide, ph);
        let selected = entity.is_some() && self.selection == entity;
        let icon = match ph.ph_type {
            hayate_ir::doc::PlaceholderType::Picture => "image",
            _ => "type",
        };
        let snippet = self
            .pres
            .ph_text(self.slide, ph)
            .and_then(|tb| tb.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.text.trim().to_string())
            .filter(|s| !s.is_empty());
        let name = Self::ph_type_name(ph.ph_type);
        let label = match snippet {
            Some(t) => {
                let t: String = t.chars().take(14).collect();
                format!("{name}  {t}")
            }
            None => format!("{name}  (空)"),
        };
        let mut row = div()
            .id(("layer", index))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .py_1()
            .pr_2()
            .pl(px(8.0))
            .text_sm()
            .rounded_md()
            .child("\u{1F512}")
            .child(Self::row_icon(
                icon,
                if selected { 0xffffff } else { 0x808080 },
            ))
            .child(label);
        if selected {
            row = row.bg(rgb(crate::SELECTION)).text_color(rgb(0xffffff));
        } else {
            row = row
                .text_color(rgb(0xb0b0b0))
                .hover(|s| s.bg(rgb(0x2a2a2a)).text_color(rgb(0xffffff)));
        }
        row.on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            if ev.click_count() >= 2 {
                this.edit_placeholder(ph);
            } else {
                this.select_placeholder(ph);
            }
            cx.notify();
        }))
        .into_any_element()
    }

    /// Append a locked, read-only section for an inherited container (a layout or the master):
    /// a muted "🔒 …" header followed by one dim row per child (front-to-back). Rows are not
    /// draggable/renamable; clicking one jumps to editing that element in master mode.
    fn push_locked_section(
        &self,
        title: String,
        container: Entity,
        index: &mut usize,
        out: &mut Vec<gpui::AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let kids = self.pres.children(container);
        if kids.is_empty() {
            return;
        }
        let labels = self.numbered_labels(&kids);
        // Placeholders are listed in the unified "Placeholders" section, so this locked section
        // shows only the container's decorations (non-placeholder shapes, e.g. accent bars/logos).
        let visible: Vec<Entity> = kids
            .iter()
            .copied()
            .filter(|e| !self.pres.world.placeholders.contains_key(e))
            .collect();
        if visible.is_empty() {
            return;
        }
        out.push(
            div()
                .pt_2()
                .text_sm()
                .text_color(rgb(0x6f6f6f))
                .child(format!("\u{1F512} {title}"))
                .into_any_element(),
        );
        for &e in visible.iter().rev() {
            let label = labels.get(&e).cloned().unwrap_or_default();
            out.push(self.locked_row(label, container, e, *index, cx));
            *index += 1;
        }
    }

    /// One locked (inherited) layer row: dim, lock-prefixed, not editable in place. Clicking it
    /// enters master mode on the owning container and selects the element so it can be edited.
    fn locked_row(
        &self,
        label: String,
        container: Entity,
        e: Entity,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id(("layer", index))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .py_1()
            .pr_2()
            .pl(px(8.0))
            .text_sm()
            .text_color(rgb(0x808080))
            .rounded_md()
            .hover(|s| s.bg(rgb(0x2a2a2a)).text_color(rgb(0xcfcfcf)))
            .child("\u{1F512}")
            .child(Self::row_icon(self.layer_icon(e), 0x808080))
            .child(label)
            .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.edit_inherited(container, e);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Recursively emit layer rows for `entities` at nesting `level`. Shapes whose group path
    /// ends at this level are leaf rows; deeper paths produce a "Group N" header followed by a
    /// recursively-built subtree.
    #[allow(clippy::too_many_arguments)]
    fn build_rows(
        &self,
        entities: &[Entity],
        level: usize,
        labels: &HashMap<Entity, String>,
        group_no: &mut u32,
        index: &mut usize,
        out: &mut Vec<gpui::AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let mut seen: Vec<u64> = Vec::new();
        for &e in entities {
            let path = self.pres.world.groups.get(&e).cloned().unwrap_or_default();
            if path.len() <= level {
                let lbl = labels.get(&e).cloned().unwrap_or_default();
                out.push(self.layer_row(lbl, e, *index, level, cx));
                *index += 1;
            } else {
                let key = path[level];
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                *group_no += 1;
                let gno = *group_no;
                let members: Vec<Entity> = entities
                    .iter()
                    .copied()
                    .filter(|m| {
                        self.pres
                            .world
                            .groups
                            .get(m)
                            .map_or(false, |p| p.get(level) == Some(&key))
                    })
                    .collect();
                out.push(self.group_header_row(gno, level, &members, *index, cx));
                *index += 1;
                self.build_rows(&members, level + 1, labels, group_no, index, out, cx);
            }
        }
    }

    /// A "Group N" header row; clicking it selects exactly that (sub-)group's members.
    fn group_header_row(
        &self,
        group_no: u32,
        level: usize,
        members: &[Entity],
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = members
            .iter()
            .any(|m| self.selection == Some(*m) || self.also.contains(m));
        // Precompute owned 'static data for the (Fn) click listener.
        let first = members.first().copied();
        let also: Vec<Entity> = first
            .map(|f| members.iter().copied().filter(|&m| m != f).collect())
            .unwrap_or_default();
        let mut row = div()
            .id(("layer", index))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .py_1()
            .pr_2()
            .pl(px(8.0 + level as f32 * 16.0))
            .text_sm()
            .rounded_md()
            .hover(|s| s.bg(rgb(0x2f2f2f)))
            .child(Self::row_icon("folder", 0xb8b8b8))
            .child(format!("Group {group_no}"));
        if active {
            row = row.bg(rgb(0x2f2f2f));
        }
        row.on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            if let Some(f) = first {
                this.selection = Some(f);
                this.also = also.clone();
            }
            cx.notify();
        }))
        .into_any_element()
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
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .py_1()
            .pr_2()
            // Indentation conveys nesting depth (each level adds a step).
            .pl(px(8.0 + depth as f32 * 16.0))
            .text_sm()
            .rounded_md()
            .hover(|s| s.bg(rgb(0x2f2f2f)))
            .child(Self::row_icon(self.layer_icon(e), 0xb8b8b8))
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

    /// The embedded icon name (see `icons::svg_for`) for an object's kind, so a layer row shows at
    /// a glance whether it is text, a rectangle, an ellipse, a line/arrow, or an image.
    fn layer_icon(&self, e: Entity) -> &'static str {
        if self.pres.world.texts.contains_key(&e) {
            "type"
        } else if self.pres.world.pictures.contains_key(&e) {
            "image"
        } else {
            match self.pres.world.geometries.get(&e) {
                Some(Geometry::Ellipse) => "circle",
                Some(Geometry::Line { start, end })
                    if matches!(start, ArrowHead::Arrow) || matches!(end, ArrowHead::Arrow) =>
                {
                    "arrow"
                }
                Some(Geometry::Line { .. }) => "line",
                _ => "square",
            }
        }
    }

    /// A small kind icon for a layer row, tinted `color`.
    fn row_icon(name: &str, color: u32) -> impl IntoElement {
        svg()
            .path(format!("icons/{name}.svg"))
            .size(px(13.))
            .flex_none()
            .text_color(rgb(color))
    }

    /// Build a distinguishing label per object, numbering each kind in creation order
    /// (Rectangle 1, Rectangle 2, Ellipse 1, Image 1, ...). Text shows its content.
    fn numbered_labels(&self, children: &[Entity]) -> HashMap<Entity, String> {
        let mut labels = HashMap::new();
        let (mut rect, mut ell, mut img, mut txt) = (0u32, 0u32, 0u32, 0u32);
        let (mut line, mut arrow) = (0u32, 0u32);
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
                    // A line with an arrowhead on either end reads as an "Arrow".
                    Some(Geometry::Line { start, end })
                        if matches!(start, ArrowHead::Arrow) || matches!(end, ArrowHead::Arrow) =>
                    {
                        arrow += 1;
                        format!("Arrow {arrow}")
                    }
                    Some(Geometry::Line { .. }) => {
                        line += 1;
                        format!("Line {line}")
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
