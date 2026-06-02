//! Keyboard, text-edit, numeric-field, and command-palette input handling, plus the
//! platform/IME `EntityInputHandler` implementation.

use std::ops::Range;

use gpui::{
    div, prelude::*, rgb, Bounds, ClickEvent, Context, EntityInputHandler, KeyDownEvent, Pixels,
    Point, UTF16Selection, Window,
};

use hayate_model::edit;

use crate::util::{
    next_char_boundary, prev_char_boundary, range_from_utf16, range_to_utf16, utf16_to_byte,
};
use crate::{pt_to_emu, FieldEdit, FieldKind, HayateApp, PaletteState, TextEdit};

use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue, Entity};
use hayate_model::{Operation, Transaction};

impl HayateApp {
    pub(crate) fn first_run_text(&self, e: Entity) -> String {
        self.pres
            .world
            .texts
            .get(&e)
            .and_then(|tb| tb.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.text.clone())
            .unwrap_or_default()
    }

    pub(crate) fn begin_text_edit(&mut self, e: Entity) {
        let original = self.first_run_text(e);
        let caret = original.len();
        self.text_edit = Some(TextEdit {
            entity: e,
            buf: original.clone(),
            original,
            selected: caret..caret,
            marked: None,
        });
    }

    /// Apply text to an entity's first run without recording history (live preview).
    pub(crate) fn live_set_text(&mut self, e: Entity, text: String) {
        let tx = edit::set_run_text(&self.pres.world, e, text);
        for op in tx.ops {
            op.apply(&mut self.pres.world);
        }
        self.rebuild();
    }

    pub(crate) fn text_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        // Character input (including the space key and IME composition) is delivered through the
        // platform text-input handler (replace_text_in_range); handling it here too would double
        // it. text_key only covers control keys (commit/cancel/erase/caret motion).
        // Ctrl/Cmd+A selects all text in the box.
        let mods = ev.keystroke.modifiers;
        if (mods.control || mods.platform) && key == "a" {
            if let Some(te) = self.text_edit.as_mut() {
                te.selected = 0..te.buf.len();
                te.marked = None;
            }
            cx.notify();
            return;
        }
        let mut live: Option<(Entity, String)> = None;
        match key.as_str() {
            "escape" => {
                if let Some(te) = self.text_edit.take() {
                    self.live_set_text(te.entity, te.original);
                }
            }
            "enter" => {
                // Enter inserts a newline (multi-line text). Commit happens on click-away (or
                // Esc cancels). Shaping treats '\n' as a hard line break.
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.replace_range(te.selected.clone(), "\n");
                    let c = te.selected.start + 1;
                    te.selected = c..c;
                    te.marked = None;
                    live = Some((te.entity, te.buf.clone()));
                }
            }
            "backspace" => {
                if let Some(te) = self.text_edit.as_mut() {
                    if te.selected.start != te.selected.end {
                        te.buf.replace_range(te.selected.clone(), "");
                        let c = te.selected.start;
                        te.selected = c..c;
                    } else if te.selected.start > 0 {
                        let p = prev_char_boundary(&te.buf, te.selected.start);
                        te.buf.replace_range(p..te.selected.start, "");
                        te.selected = p..p;
                    }
                    te.marked = None;
                    live = Some((te.entity, te.buf.clone()));
                }
            }
            "delete" => {
                if let Some(te) = self.text_edit.as_mut() {
                    if te.selected.start != te.selected.end {
                        te.buf.replace_range(te.selected.clone(), "");
                        let c = te.selected.start;
                        te.selected = c..c;
                    } else if te.selected.end < te.buf.len() {
                        let n = next_char_boundary(&te.buf, te.selected.end);
                        te.buf.replace_range(te.selected.end..n, "");
                        // Caret stays at the deletion point.
                    }
                    te.marked = None;
                    live = Some((te.entity, te.buf.clone()));
                }
            }
            "left" => {
                if let Some(te) = self.text_edit.as_mut() {
                    let c = if te.selected.start > 0 {
                        prev_char_boundary(&te.buf, te.selected.start)
                    } else {
                        0
                    };
                    te.selected = c..c;
                }
            }
            "right" => {
                if let Some(te) = self.text_edit.as_mut() {
                    let c = next_char_boundary(&te.buf, te.selected.end.min(te.buf.len()));
                    te.selected = c..c;
                }
            }
            _ => {}
        }
        if let Some((e, buf)) = live {
            self.live_set_text(e, buf);
        }
        cx.notify();
    }

    /// Splice `new_text` into the edit buffer, replacing `range_utf16` (or the marked range, or
    /// the current selection), then move the caret to the end of the inserted text. When `mark`
    /// is set the inserted text becomes the IME composing (marked) region.
    pub(crate) fn apply_ime(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        mark: bool,
    ) {
        let (e, buf) = {
            let te = match self.text_edit.as_mut() {
                Some(t) => t,
                None => return,
            };
            let range = range_utf16
                .map(|r| range_from_utf16(&te.buf, &r))
                .or_else(|| te.marked.clone())
                .unwrap_or_else(|| te.selected.clone());
            te.buf.replace_range(range.clone(), new_text);
            let new_end = range.start + new_text.len();
            te.marked = if mark && !new_text.is_empty() {
                Some(range.start..new_end)
            } else {
                None
            };
            te.selected = new_end..new_end;
            (te.entity, te.buf.clone())
        };
        self.live_set_text(e, buf);
    }

    /// Handle a key while a numeric field is being edited (digits / . / -).
    /// Handle a key while renaming a layer. Enter commits the name (empty clears it back to the
    /// auto label), Escape cancels. Character input is taken directly (no IME) since there is no
    /// platform input handler over the Layers panel.
    pub(crate) fn rename_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        match key.as_str() {
            "escape" => self.renaming = None,
            "enter" => {
                if let Some((e, buf)) = self.renaming.take() {
                    let trimmed = buf.trim().to_string();
                    let op = if trimmed.is_empty() {
                        Operation::RemoveComponent {
                            entity: e,
                            kind: CompKind::Name,
                        }
                    } else {
                        Operation::SetComponent {
                            entity: e,
                            value: CompValue::Name(trimmed),
                        }
                    };
                    self.commit_tx(Transaction::new("rename layer", vec![op]));
                }
            }
            "backspace" => {
                if let Some((_, buf)) = self.renaming.as_mut() {
                    buf.pop();
                }
            }
            "space" => {
                if let Some((_, buf)) = self.renaming.as_mut() {
                    buf.push(' ');
                }
            }
            s if s.chars().count() == 1 => {
                if let Some((_, buf)) = self.renaming.as_mut() {
                    buf.push_str(s);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn field_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        match key.as_str() {
            "escape" => self.field_edit = None,
            "enter" => {
                if let Some(fe) = self.field_edit.take() {
                    if let Ok(v) = fe.buf.trim().parse::<f32>() {
                        match fe.kind {
                            FieldKind::Rotation => self.set_rotation_abs(v.rem_euclid(360.0)),
                            FieldKind::PosX => self.set_frame_field(|f| f.origin.x = pt_to_emu(v)),
                            FieldKind::PosY => self.set_frame_field(|f| f.origin.y = pt_to_emu(v)),
                            FieldKind::SizeW => {
                                self.set_frame_field(|f| f.size.w = pt_to_emu(v).max(12_700))
                            }
                            FieldKind::SizeH => {
                                self.set_frame_field(|f| f.size.h = pt_to_emu(v).max(12_700))
                            }
                            FieldKind::Opacity => self.set_opacity_pct(v),
                            FieldKind::StrokeWidth => self.set_stroke_width(v.round() as i64),
                        }
                    }
                }
            }
            "backspace" => {
                if let Some(fe) = self.field_edit.as_mut() {
                    fe.buf.pop();
                }
            }
            s if s.len() == 1
                && (s.chars().all(|c| c.is_ascii_digit()) || s == "." || s == "-") =>
            {
                if let Some(fe) = self.field_edit.as_mut() {
                    if fe.buf.len() < 8 {
                        fe.buf.push_str(s);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn set_frame_field(&mut self, f: impl FnOnce(&mut RectEmu)) {
        if let Some(e) = self.selection {
            if let Some(mut fr) = self.pres.world.frames.get(&e).copied() {
                f(&mut fr);
                let tx = edit::set_frame(e, fr);
                self.commit_tx(tx);
            }
        }
    }

    pub(crate) fn set_opacity_pct(&mut self, pct: f32) {
        if let Some(e) = self.selection {
            let v = (pct / 100.0).clamp(0.0, 1.0);
            let tx = Transaction::new(
                "set opacity",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Opacity(v),
                }],
            );
            self.commit_tx(tx);
        }
    }

    pub(crate) fn field_current(&self, kind: FieldKind) -> String {
        let frame = self
            .selection
            .and_then(|e| self.pres.world.frames.get(&e).copied());
        let to_pt = |v: i64| (v as f32 / 12_700.0).round() as i32;
        match kind {
            FieldKind::Rotation => format!("{}", self.sel_rotation().round() as i32),
            FieldKind::PosX => frame.map(|f| to_pt(f.origin.x)).unwrap_or(0).to_string(),
            FieldKind::PosY => frame.map(|f| to_pt(f.origin.y)).unwrap_or(0).to_string(),
            FieldKind::SizeW => frame.map(|f| to_pt(f.size.w)).unwrap_or(0).to_string(),
            FieldKind::SizeH => frame.map(|f| to_pt(f.size.h)).unwrap_or(0).to_string(),
            FieldKind::Opacity => {
                let o = self
                    .selection
                    .and_then(|e| self.pres.world.opacity.get(&e).copied())
                    .unwrap_or(1.0);
                format!("{}", (o * 100.0).round() as i32)
            }
            FieldKind::StrokeWidth => self.sel_stroke_pt().unwrap_or(0).to_string(),
        }
    }

    pub(crate) fn begin_field_edit(&mut self, kind: FieldKind) {
        self.field_edit = Some(FieldEdit {
            kind,
            buf: self.field_current(kind),
        });
    }

    /// A clickable numeric field (click to type; shows the current value otherwise).
    pub(crate) fn num_field(
        &self,
        id: &'static str,
        kind: FieldKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let editing = matches!(&self.field_edit, Some(fe) if fe.kind == kind);
        let shown = if editing {
            format!(
                "{}|",
                self.field_edit
                    .as_ref()
                    .map(|f| f.buf.clone())
                    .unwrap_or_default()
            )
        } else {
            self.field_current(kind)
        };
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_md()
            .bg(if editing {
                rgb(0x1f3a5f)
            } else {
                rgb(0x3a3a3a)
            })
            .child(shown)
            .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.begin_field_edit(kind);
                cx.notify();
            }))
    }

    pub(crate) fn on_key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        // Esc dismisses an open context menu before anything else handles the key.
        if self.context_menu.is_some() && ev.keystroke.key.as_str() == "escape" {
            self.context_menu = None;
            cx.notify();
            return;
        }
        if self.renaming.is_some() {
            self.rename_key(ev, cx);
            return;
        }
        if self.palette.is_some() {
            self.palette_key(ev, cx);
            return;
        }
        if self.field_edit.is_some() {
            self.field_key(ev, cx);
            return;
        }
        if self.text_edit.is_some() {
            self.text_key(ev, cx);
            return;
        }
        if self.present {
            match ev.keystroke.key.as_str() {
                "escape" => {
                    self.present = false;
                    cx.notify();
                }
                "space" => {
                    // Advance entrance animations on the current slide.
                    self.present_t = self.present_t.saturating_add(700);
                    cx.notify();
                }
                "right" | "down" => {
                    self.next_slide(1);
                    self.present_t = 0;
                }
                "left" | "up" => {
                    self.next_slide(-1);
                    self.present_t = 0;
                }
                _ => {}
            }
            return;
        }
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "p" if cmd => {
                self.palette = Some(PaletteState {
                    query: String::new(),
                    sel: 0,
                });
                cx.notify();
            }
            "e" if cmd && k.modifiers.shift => self.export_pptx(),
            "e" if cmd => self.export_svg(),
            "f5" => {
                self.present = true;
                self.present_t = 0;
                cx.notify();
            }
            "o" if cmd && k.modifiers.shift => {
                self.import_pptx();
                cx.notify();
            }
            "i" if !cmd => {
                self.insert_image(cx);
                cx.notify();
            }
            "z" if cmd && k.modifiers.shift => {
                self.history.redo(&mut self.pres.world);
                self.after_doc_change();
                cx.notify();
            }
            "z" if cmd => {
                self.history.undo(&mut self.pres.world);
                self.after_doc_change();
                cx.notify();
            }
            "s" if cmd => self.save(),
            "o" if cmd => {
                self.open();
                cx.notify();
            }
            "g" if cmd && k.modifiers.shift => {
                self.ungroup_selection();
                cx.notify();
            }
            "g" if cmd => {
                self.group_selection();
                cx.notify();
            }
            "g" if !cmd => {
                self.show_grid = !self.show_grid;
                cx.notify();
            }
            "r" if !cmd => {
                self.add_rect();
                cx.notify();
            }
            "t" if !cmd => {
                self.add_text_box();
                cx.notify();
            }
            "f2" => {
                if let Some(e) = self.selection {
                    self.begin_text_edit(e);
                    cx.notify();
                }
            }
            "d" if cmd => {
                self.duplicate_selection();
                cx.notify();
            }
            "c" if cmd => self.copy_selection(),
            "v" if cmd => {
                self.paste_clipboard();
                cx.notify();
            }
            "escape" => {
                // Cancel the current selection.
                self.selection = None;
                self.also.clear();
                self.marquee = None;
                cx.notify();
            }
            "delete" | "backspace" if !cmd => {
                self.delete_selection();
                cx.notify();
            }
            // Arrow keys nudge the selected shape (0.1in step).
            "left" if !cmd => {
                self.nudge(-91_440, 0);
                cx.notify();
            }
            "right" if !cmd => {
                self.nudge(91_440, 0);
                cx.notify();
            }
            "up" if !cmd => {
                self.nudge(0, -91_440);
                cx.notify();
            }
            "down" if !cmd => {
                self.nudge(0, 91_440);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Commands matching the palette query, as (id, title).
    pub(crate) fn palette_commands(&self) -> Vec<(String, String)> {
        let q = self
            .palette
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        self.registry
            .manifest()
            .into_iter()
            .filter_map(|v| {
                let id = v.get("id")?.as_str()?.to_string();
                let title = v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| id.clone());
                if q.is_empty()
                    || id.to_lowercase().contains(&q)
                    || title.to_lowercase().contains(&q)
                {
                    Some((id, title))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Run a registered command, supplying the current selection + sensible defaults as args.
    pub(crate) fn run_command(&mut self, id: &str) {
        let args = serde_json::json!({
            "entity": self.selection.map(|e| e.0),
            "dx": 200,
            "dy": 0,
            "color": "#e11d48",
        });
        if let Some(tx) = self.registry.build(id, &args, &self.pres.world) {
            self.history.commit(&mut self.pres.world, tx);
        }
    }

    pub(crate) fn palette_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        match key.as_str() {
            "escape" => self.palette = None,
            "enter" => {
                let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
                let chosen = self.palette_commands().get(sel).map(|(id, _)| id.clone());
                self.palette = None;
                if let Some(id) = chosen {
                    self.run_command(&id);
                    self.rebuild();
                }
            }
            "backspace" => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.pop();
                    p.sel = 0;
                }
            }
            "up" => {
                if let Some(p) = self.palette.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                }
            }
            "down" => {
                let n = self.palette_commands().len();
                if let Some(p) = self.palette.as_mut() {
                    if p.sel + 1 < n {
                        p.sel += 1;
                    }
                }
            }
            "space" => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push(' ');
                    p.sel = 0;
                }
            }
            s if s.chars().count() == 1 => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push_str(s);
                    p.sel = 0;
                }
            }
            _ => {}
        }
        cx.notify();
    }
}

/// IME / platform text input. Active only while a text box is being edited
/// (`accepts_text_input`), so single-key shortcuts still work otherwise.
impl EntityInputHandler for HayateApp {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let te = self.text_edit.as_ref()?;
        let s = utf16_to_byte(&te.buf, range.start);
        let e = utf16_to_byte(&te.buf, range.end);
        te.buf.get(s..e).map(|x| x.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let te = self.text_edit.as_ref()?;
        Some(UTF16Selection {
            range: range_to_utf16(&te.buf, &te.selected),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.text_edit
            .as_ref()
            .and_then(|te| te.marked.as_ref().map(|m| range_to_utf16(&te.buf, m)))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(te) = self.text_edit.as_mut() {
            te.marked = None;
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_ime(range, text, false);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_ime(range, new_text, true);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Approximate: place the IME candidate window over the editing area.
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.text_edit.is_some()
    }
}
