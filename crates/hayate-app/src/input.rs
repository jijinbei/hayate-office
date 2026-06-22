//! Keyboard, text-edit, numeric-field, and command-palette input handling, plus the
//! platform/IME `EntityInputHandler` implementation.

use std::ops::Range;

use gpui::{
    div, prelude::*, rgb, Bounds, ClickEvent, Context, EntityInputHandler, KeyDownEvent, Pixels,
    Point, UTF16Selection, Window,
};

use hayate_model::edit;

use crate::util::{
    caret_line_move, next_char_boundary, prev_char_boundary, range_from_utf16, range_to_utf16,
    utf16_to_byte,
};
use crate::{
    pt_to_emu, AiPanel, FieldEdit, FieldKind, HayateApp, PaletteState, ScriptPanel, TextEdit,
};

use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue, Entity};
use hayate_model::{Operation, Transaction};

impl HayateApp {
    pub(crate) fn begin_text_edit(&mut self, e: Entity) {
        let buf = self
            .pres
            .world
            .texts
            .get(&e)
            .map(typst_source_of)
            .unwrap_or_default();
        let caret = buf.len();
        self.text_edit = Some(TextEdit {
            entity: e,
            original: buf.clone(),
            buf,
            selected: caret..caret,
            marked: None,
        });
        // Rebuild so the box now renders as its raw Typst source (not typeset), and the caret can
        // be placed on the plain-text node.
        self.rebuild();
    }

    /// Apply the current edit buffer as the box's Typst source live (no history), so the in-canvas
    /// preview shows it. While editing, the scene builder renders this box as raw source (the box
    /// is the `raw_text_entity`), so the caret tracks the literal characters.
    pub(crate) fn apply_edit_live(&mut self) {
        let Some(te) = self.text_edit.as_ref() else {
            return;
        };
        let (e, src) = (te.entity, te.buf.clone());
        let tx = edit::set_typst_source(&self.pres.world, e, src);
        for op in tx.ops {
            op.apply(&mut self.pres.world);
        }
        self.rebuild();
    }

    /// Revert an entity's text to the given Typst source (used to undo the live preview before a
    /// commit or on Esc), without recording history.
    pub(crate) fn revert_text_live(&mut self, e: Entity, source: &str) {
        let tx = edit::set_typst_source(&self.pres.world, e, source.to_string());
        for op in tx.ops {
            op.apply(&mut self.pres.world);
        }
        self.rebuild();
    }

    pub(crate) fn text_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
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
        let mut live = false;
        match key {
            "escape" => {
                if let Some(te) = self.text_edit.take() {
                    let (e, src) = (te.entity, te.original);
                    self.revert_text_live(e, &src);
                }
            }
            "tab" => {
                // Typst source editing: Tab inserts two spaces (lists use `-`/`+` typed literally).
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.replace_range(te.selected.clone(), "  ");
                    let c = te.selected.start + 2;
                    te.selected = c..c;
                    te.marked = None;
                    live = true;
                }
            }
            "enter" => {
                // Insert a newline (Typst paragraphs/lines are plain `\n` in the source).
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.replace_range(te.selected.clone(), "\n");
                    let c = te.selected.start + 1;
                    te.selected = c..c;
                    te.marked = None;
                    live = true;
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
                    live = true;
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
                    live = true;
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
            "up" => {
                if let Some(te) = self.text_edit.as_mut() {
                    let c = caret_line_move(&te.buf, te.selected.start, -1);
                    te.selected = c..c;
                }
            }
            "down" => {
                if let Some(te) = self.text_edit.as_mut() {
                    let c = caret_line_move(&te.buf, te.selected.end, 1);
                    te.selected = c..c;
                }
            }
            _ => {}
        }
        if live {
            self.apply_edit_live();
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
        {
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
            // No markdown auto-bullet: `-`/`+` stay literal in the Typst source.
        }
        self.apply_edit_live();
    }

    /// Handle a key while a numeric field is being edited (digits / . / -).
    /// Handle a key while renaming a layer. Enter commits the name (empty clears it back to the
    /// auto label), Escape cancels. Character input is taken directly (no IME) since there is no
    /// platform input handler over the Layers panel.
    pub(crate) fn rename_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        match key {
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

    /// Key handling for inline layout renaming in the Master tab.
    pub(crate) fn layout_rename_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => self.layout_rename = None,
            "enter" => {
                if let Some((layout, buf)) = self.layout_rename.take() {
                    self.rename_layout(layout, &buf);
                }
            }
            "backspace" => {
                if let Some((_, buf)) = self.layout_rename.as_mut() {
                    buf.pop();
                }
            }
            "space" => {
                if let Some((_, buf)) = self.layout_rename.as_mut() {
                    buf.push(' ');
                }
            }
            s if s.chars().count() == 1 => {
                if let Some((_, buf)) = self.layout_rename.as_mut() {
                    buf.push_str(s);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Key handling for the "Save As" dialog: edit the filename, Enter saves, Esc cancels.
    pub(crate) fn save_modal_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        match key {
            "escape" => self.save_modal = None,
            "enter" => {
                if let Some(m) = self.save_modal.take() {
                    let name = m.buf.trim();
                    if !name.is_empty() {
                        self.doc_path = name.to_string();
                        self.save();
                    }
                }
            }
            "backspace" => {
                if let Some(m) = self.save_modal.as_mut() {
                    m.buf.pop();
                }
            }
            "space" => {
                if let Some(m) = self.save_modal.as_mut() {
                    m.buf.push(' ');
                }
            }
            s if s.chars().count() == 1 => {
                if let Some(m) = self.save_modal.as_mut() {
                    m.buf.push_str(s);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Key handling for the script console. Ctrl/Cmd+Enter runs the buffer (and closes the
    /// panel); plain Enter inserts a newline; Esc closes. Editing mirrors the save dialog.
    pub(crate) fn script_panel_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "escape" => self.script_panel = None,
            "enter" if cmd => {
                if let Some(p) = self.script_panel.take() {
                    self.run_script_src(&p.buf);
                }
            }
            "v" if cmd => {
                if let Some(text) = self.read_clipboard_text(cx) {
                    if let Some(p) = self.script_panel.as_mut() {
                        p.buf.push_str(&text);
                    }
                }
            }
            "enter" => {
                if let Some(p) = self.script_panel.as_mut() {
                    p.buf.push('\n');
                }
            }
            "backspace" => {
                if let Some(p) = self.script_panel.as_mut() {
                    p.buf.pop();
                }
            }
            "space" => {
                if let Some(p) = self.script_panel.as_mut() {
                    p.buf.push(' ');
                }
            }
            "tab" => {
                if let Some(p) = self.script_panel.as_mut() {
                    p.buf.push_str("  ");
                }
            }
            // Plain typed character. Ignore modifier combos (e.g. Cmd+C) so they don't insert
            // literal letters into the buffer.
            s if s.chars().count() == 1 && !cmd => {
                if let Some(p) = self.script_panel.as_mut() {
                    p.buf.push_str(s);
                }
            }
            _ => {}
        }
        // The caret lives on the last line; keep it in view as the buffer grows (e.g. after a
        // long paste). Reading without typing still leaves wheel-scrolling free.
        if let Some(p) = self.script_panel.as_ref() {
            p.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// Key handling for the AI prompt. Enter submits the natural-language request to the AI
    /// authoring loop (and closes the prompt); Esc cancels. Single-line editing like the dialog.
    pub(crate) fn ai_panel_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "escape" => self.ai_panel = None,
            "v" if cmd => {
                if let Some(text) = self.read_clipboard_text(cx) {
                    if let Some(p) = self.ai_panel.as_mut() {
                        p.buf.push_str(&text);
                    }
                }
            }
            "enter" => {
                if let Some(p) = self.ai_panel.take() {
                    let req = p.buf.trim().to_string();
                    if !req.is_empty() {
                        self.ai_author(req, cx);
                    }
                }
            }
            "backspace" => {
                if let Some(p) = self.ai_panel.as_mut() {
                    p.buf.pop();
                }
            }
            "space" => {
                if let Some(p) = self.ai_panel.as_mut() {
                    p.buf.push(' ');
                }
            }
            s if s.chars().count() == 1 && !cmd => {
                if let Some(p) = self.ai_panel.as_mut() {
                    p.buf.push_str(s);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn field_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        match key {
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
        // Use the resolved frame so a locked placeholder shows its inherited position/size.
        let frame = self.selection.and_then(|e| self.resolved_frame(e));
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
                    .map(|f| f.buf.as_str())
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
        // Esc (or Enter) dismisses a transient notice modal.
        if self.notice.is_some() && matches!(ev.keystroke.key.as_str(), "escape" | "enter") {
            self.notice = None;
            cx.notify();
            return;
        }
        if self.save_modal.is_some() {
            self.save_modal_key(ev, cx);
            return;
        }
        if self.layout_rename.is_some() {
            self.layout_rename_key(ev, cx);
            return;
        }
        if self.renaming.is_some() {
            self.rename_key(ev, cx);
            return;
        }
        if self.ai_panel.is_some() {
            self.ai_panel_key(ev, cx);
            return;
        }
        if self.script_panel.is_some() {
            self.script_panel_key(ev, cx);
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
                    // The editor scene wasn't kept in sync during the slideshow (see next_slide);
                    // rebuild it for the slide we landed on so the editor shows the right one.
                    self.rebuild();
                    cx.notify();
                }
                "space" => {
                    // Advance entrance animations on the current slide.
                    self.present_t = self.present_t.saturating_add(700);
                    cx.notify();
                }
                "right" | "down" => {
                    // Advancing past the last slide ends the slideshow (rather than wrapping).
                    let slides = self.pres.slides();
                    if slides.last() == Some(&self.slide) {
                        self.present = false;
                        // The editor scene wasn't kept in sync during the show; resync it.
                        self.rebuild();
                    } else {
                        self.next_slide(1);
                        self.present_t = 0;
                    }
                    cx.notify();
                }
                "left" | "up" => {
                    self.next_slide(-1);
                    self.present_t = 0;
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            // Ctrl/Cmd+Shift+P exports a PDF (P = PDF). Must come before the plain Ctrl/Cmd+P
            // palette arm, which would otherwise also match the shift chord.
            "p" if cmd && k.modifiers.shift => {
                self.export_pdf();
                cx.notify();
            }
            "p" if cmd => {
                self.palette = Some(PaletteState {
                    query: String::new(),
                    sel: 0,
                });
                cx.notify();
            }
            // Ctrl/Cmd+Shift+R opens the script console (R = Run script).
            "r" if cmd && k.modifiers.shift => {
                self.script_panel = Some(ScriptPanel {
                    buf: String::new(),
                    scroll: gpui::ScrollHandle::new(),
                });
                cx.notify();
            }
            // Ctrl/Cmd+Shift+A opens the AI prompt (A = Ask AI).
            "a" if cmd && k.modifiers.shift => {
                self.ai_panel = Some(AiPanel { buf: String::new() });
                cx.notify();
            }
            "e" if cmd && k.modifiers.shift => self.export_pptx(),
            "e" if cmd => self.export_svg(),
            "f5" => {
                self.start_present();
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
                self.redo();
                cx.notify();
            }
            "z" if cmd => {
                self.undo();
                cx.notify();
            }
            "s" if cmd => {
                self.open_save_dialog();
                cx.notify();
            }
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
                // Prefer an in-app copied shape so Ctrl+C/Ctrl+V of objects is reliable and is
                // never hijacked by a stale system-clipboard image (or a slow wl-paste call).
                // Only reach for a system-clipboard image when nothing was copied in-app.
                if self.clipboard.is_some() {
                    self.paste_clipboard();
                } else if !self.paste_clipboard_image(cx) {
                    self.paste_clipboard();
                }
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
            .chain(self.script_commands.iter().filter_map(|c| {
                // Script-registered commands carry a `script:` prefix so dispatch can tell them
                // apart from builtin command ids.
                if q.is_empty()
                    || c.id.to_lowercase().contains(&q)
                    || c.title.to_lowercase().contains(&q)
                {
                    Some((format!("script:{}", c.id), format!("\u{25b6} {}", c.title)))
                } else {
                    None
                }
            }))
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
        let key = ev.keystroke.key.as_str();
        match key {
            "escape" => self.palette = None,
            "enter" => {
                let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
                // Index the unified list (builtins + script-registered commands).
                let chosen = self
                    .palette_commands()
                    .into_iter()
                    .nth(sel)
                    .map(|(id, _)| id);
                self.palette = None;
                if let Some(id) = chosen {
                    if let Some(script_id) = id.strip_prefix("script:") {
                        self.run_script_command(script_id);
                    } else {
                        self.run_command(&id);
                    }
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

/// The Typst source to edit for a text box: its `typst_source` if present, else a plain-text
/// migration of the paragraphs (one line per paragraph) for a legacy box being edited for the
/// first time.
pub(crate) fn typst_source_of(tb: &hayate_ir::text::TextBody) -> String {
    if let Some(src) = &tb.typst_source {
        return src.clone();
    }
    let lines: Vec<String> = tb
        .paragraphs
        .iter()
        .map(|para| {
            para.runs
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>()
        })
        .collect();
    lines.join("\n")
}
