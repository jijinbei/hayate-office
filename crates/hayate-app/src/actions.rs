//! Shape and document editing actions: selection, image insertion, alignment, animation,
//! transforms, grouping, clipboard, font sizing, zoom, and the transaction commit helpers.

use gpui::{Context, PathPromptOptions, Window};

use hayate_ir::color::{Color, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_ir::paint::Fill;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::{CompValue, Entity};
use hayate_model::{edit, Operation, Transaction};
use hayate_render::build_slide_scene;

use crate::{view_px, HayateApp, DOC_PATH};

impl HayateApp {
    /// All currently-selected entities (primary + additional).
    pub(crate) fn selected_all(&self) -> Vec<Entity> {
        let mut v: Vec<Entity> = self.selection.into_iter().collect();
        v.extend(self.also.iter().copied());
        v
    }

    /// File extensions accepted by the "Insert Image" picker.
    const IMAGE_EXTS: &'static [&'static str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

    /// Open a native file dialog and insert the chosen image on the current slide.
    ///
    /// gpui's file dialog is asynchronous: `App::prompt_for_paths` returns a
    /// `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>`. We drive it on the foreground
    /// executor with `cx.spawn(...)`, read the file bytes, then update the entity to add the
    /// Picture shape. If the dialog cannot be opened, we fall back to the `HAYATE_IMAGE`
    /// environment variable so insertion still functions.
    pub(crate) fn insert_image(&mut self, cx: &mut Context<Self>) {
        let options = PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Insert Image".into()),
        };
        let receiver = cx.prompt_for_paths(options);
        cx.spawn(async move |this, cx| {
            // Await the dialog result; on any error fall back to the env-var path.
            let chosen: Option<std::path::PathBuf> = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => std::env::var_os("HAYATE_IMAGE").map(std::path::PathBuf::from),
            };
            let Some(path) = chosen else { return };
            // Only accept recognized image extensions.
            let ok_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| Self::IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if !ok_ext {
                return;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.insert_image_bytes(bytes);
                cx.notify();
            });
        })
        .detach();
    }

    /// Add a Picture shape backed by `bytes` to the current slide at a default frame.
    pub(crate) fn insert_image_bytes(&mut self, bytes: Vec<u8>) {
        let key = self.pres.add_media(bytes);
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(3.0), inch_f(2.0), inch_f(3.0), inch_f(2.0));
        let pic = hayate_ir::image::PictureRef {
            media_key: key,
            natural: SizeEmu::new(inch_f(3.0), inch_f(2.0)),
        };
        let tx = Transaction::new(
            "insert image",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Parent(self.slide),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Order(order),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Picture(pic),
                },
            ],
        );
        self.commit_tx(tx);
        self.selection = Some(e);
        self.also.clear();
    }

    /// Align the current multi-selection using a registry command.
    pub(crate) fn align(&mut self, cmd_id: &str) {
        let ids: Vec<u64> = self.selected_all().iter().map(|e| e.0).collect();
        if ids.len() < 2 {
            return;
        }
        let args = serde_json::json!({ "entities": ids });
        if let Some(tx) = self.registry.build(cmd_id, &args, &self.pres.world) {
            self.commit_tx(tx);
        }
    }

    /// Add a fade-in entrance animation to the selected shape on the current slide.
    pub(crate) fn add_fade_in(&mut self) {
        if let Some(e) = self.selection {
            let tx = edit::add_entrance(
                &self.pres.world,
                self.slide,
                e,
                hayate_ir::anim::Effect::Fade,
                600,
            );
            self.commit_tx(tx);
        }
    }

    /// Add a new text box on the current slide and start editing it.
    pub(crate) fn add_text_box(&mut self) {
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(1.0), inch_f(3.2), inch_f(5.0), inch_f(1.0));
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: "Text".to_string(),
                font: FontRef::Theme(ThemeFontSlot::Minor),
                size: pt(24),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: false,
                italic: false,
                underline: false,
            }])],
            autofit: false,
        };
        let tx = Transaction::new(
            "add text",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Parent(self.slide),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Order(order),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Text(body),
                },
            ],
        );
        self.commit_tx(tx);
        self.selection = Some(e);
        self.begin_text_edit(e);
    }

    pub(crate) fn set_zoom(&mut self, z: f32, cx: &mut Context<Self>) {
        self.zoom = z.clamp(0.1, 8.0);
        self.rebuild();
        cx.notify();
    }

    /// Set zoom so the slide fits the current window's editing area.
    pub(crate) fn fit_zoom(&mut self, window: &Window) {
        let vp = window.viewport_size();
        let inspector_w = if self.selection.is_some() { 244.0 } else { 0.0 };
        let avail_w = (f32::from(vp.width) - 244.0 - inspector_w - 24.0).max(64.0);
        let avail_h = (f32::from(vp.height) - 96.0).max(64.0);
        let pt = |v: i64| (v as f32 / 12_700.0).max(1.0);
        let z = (avail_w / pt(self.pres.slide_size.w))
            .min(avail_h / pt(self.pres.slide_size.h))
            .clamp(0.1, 8.0);
        self.zoom = z;
    }

    pub(crate) fn rebuild(&mut self) {
        let target = view_px(&self.pres, self.zoom);
        self.scene = build_slide_scene(&self.pres, self.slide, target);
    }

    /// Rebuild + autosave after a document change (undo/redo, etc.).
    pub(crate) fn after_doc_change(&mut self) {
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, DOC_PATH);
    }

    /// Add a rectangle at the slide center as one undoable transaction, and select it.
    pub(crate) fn add_rect(&mut self) {
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(4.0), inch_f(3.5), inch_f(1.6), inch_f(1.6));
        let tx = edit::create_rect(
            e,
            self.slide,
            order,
            frame,
            Fill::Solid(Color::theme(ThemeColorToken::Accent5)),
        );
        self.commit_tx(tx);
        self.selection = Some(e);
    }

    /// Delete the selected shape (undoable: despawn captures components to restore).
    pub(crate) fn delete_selection(&mut self) {
        if let Some(e) = self.selection.take() {
            let tx = Transaction::new("delete shape", vec![Operation::Despawn { entity: e }]);
            self.commit_tx(tx);
        }
    }

    // --- inspector (Format pane) actions ---

    pub(crate) fn commit_tx(&mut self, tx: Transaction) {
        if !tx.ops.is_empty() {
            self.history.commit(&mut self.pres.world, tx);
            self.rebuild();
            let _ = hayate_format::autosave(&self.pres, DOC_PATH);
        }
    }

    pub(crate) fn sel_rotation(&self) -> f32 {
        self.selection
            .and_then(|e| self.pres.world.rotations.get(&e).copied())
            .unwrap_or(0.0)
    }

    pub(crate) fn set_rotation_abs(&mut self, deg: f32) {
        if let Some(e) = self.selection {
            let tx = edit::set_rotation(e, deg);
            self.commit_tx(tx);
        }
    }

    pub(crate) fn nudge(&mut self, dx: i64, dy: i64) {
        if let Some(e) = self.selection {
            let tx = edit::translate(&self.pres.world, e, dx, dy);
            self.commit_tx(tx);
        }
    }

    /// Group the current (multi-)selection under a fresh group key.
    pub(crate) fn group_selection(&mut self) {
        let members = self.selected_all();
        if members.len() < 2 {
            return;
        }
        // Mint a unique nonzero group key from a reserved entity id.
        let key = self.pres.world.reserve_id().0;
        let tx = edit::group(&members, key);
        self.commit_tx(tx);
    }

    /// Ungroup the group that the current selection belongs to.
    pub(crate) fn ungroup_selection(&mut self) {
        if let Some(sel) = self.selection {
            if let Some(&key) = self.pres.world.groups.get(&sel) {
                let tx = edit::ungroup(&self.pres.world, key);
                self.commit_tx(tx);
                self.also.clear();
            }
        }
    }

    pub(crate) fn set_fill_accent(&mut self, token: ThemeColorToken) {
        if let Some(e) = self.selection {
            let tx = edit::set_fill(e, Fill::Solid(Color::theme(token)));
            self.commit_tx(tx);
        }
    }

    pub(crate) fn run_on_selection(&mut self, id: &str) {
        if let Some(e) = self.selection {
            let args = serde_json::json!({ "entity": e.0 });
            if let Some(tx) = self.registry.build(id, &args, &self.pres.world) {
                self.commit_tx(tx);
            }
        }
    }

    /// Like `run_on_selection`, but merges extra named fields into the command args
    /// (e.g. `shape.set_font_size` needs a `pt` value alongside the `entity`).
    pub(crate) fn run_on_selection_with(&mut self, id: &str, extra: serde_json::Value) {
        if let Some(e) = self.selection {
            let mut args = serde_json::json!({ "entity": e.0 });
            if let (Some(obj), Some(extra)) = (args.as_object_mut(), extra.as_object()) {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
            if let Some(tx) = self.registry.build(id, &args, &self.pres.world) {
                self.commit_tx(tx);
            }
        }
    }

    /// Current font size (in points) of the selected shape's first run, if any.
    pub(crate) fn sel_font_size_pt(&self) -> Option<i64> {
        let e = self.selection?;
        self.pres
            .world
            .texts
            .get(&e)
            .and_then(|tb| tb.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.size / hayate_ir::units::EMU_PER_PT)
    }

    /// Adjust the selected shape's font size by `delta_pt` points via `shape.set_font_size`.
    /// Falls back to a fixed target size (32 up / 18 down) when the current size is unknown.
    pub(crate) fn change_font_size(&mut self, delta_pt: i64) {
        let target = match self.sel_font_size_pt() {
            Some(cur) => (cur + delta_pt).clamp(8, 200),
            None => {
                if delta_pt >= 0 {
                    32
                } else {
                    18
                }
            }
        };
        self.run_on_selection_with("shape.set_font_size", serde_json::json!({ "pt": target }));
    }

    pub(crate) fn duplicate_selection(&mut self) {
        if let Some(src) = self.selection {
            let ne = self.pres.world.reserve_id();
            let tx = edit::duplicate(&self.pres.world, src, ne);
            self.commit_tx(tx);
            self.selection = Some(ne);
        }
    }

    pub(crate) fn copy_selection(&mut self) {
        self.clipboard = self.selection.map(|e| self.pres.world.components_of(e));
    }

    pub(crate) fn paste_clipboard(&mut self) {
        let Some(comps) = self.clipboard.clone() else {
            return;
        };
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let ne = self.pres.world.reserve_id();
        let mut ops = vec![Operation::Spawn { entity: ne }];
        for comp in comps {
            let comp = match comp {
                CompValue::Frame(f) => CompValue::Frame(RectEmu {
                    origin: PointEmu::new(f.origin.x + 182_880, f.origin.y + 182_880),
                    size: f.size,
                }),
                other => other,
            };
            ops.push(Operation::SetComponent {
                entity: ne,
                value: comp,
            });
        }
        // Ensure it lands on the current slide, appended in order.
        ops.push(Operation::SetComponent {
            entity: ne,
            value: CompValue::Parent(self.slide),
        });
        ops.push(Operation::SetComponent {
            entity: ne,
            value: CompValue::Order(order),
        });
        self.commit_tx(Transaction::new("paste", ops));
        self.selection = Some(ne);
    }
}
