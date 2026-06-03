//! Shape and document editing actions: selection, image insertion, alignment, animation,
//! transforms, grouping, clipboard, font sizing, zoom, and the transaction commit helpers.

use gpui::{ClipboardEntry, Context, PathPromptOptions, Window};

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

use crate::{view_px, EditScope, HayateApp};

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
            let kids = self.pres.children(self.container());
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        // Size the frame to the image's real aspect ratio. The natural size maps pixels to EMU
        // at 96 DPI (9525 EMU/px); the on-slide frame scales that down to fit a sensible box
        // while preserving the ratio. Unknown headers fall back to a 3x2 inch frame.
        let (nat_w, nat_h, frame_w, frame_h) = match crate::paint::image_dimensions(
            self.pres.media.get(&key).map(Vec::as_slice).unwrap_or(&[]),
        ) {
            Some((pw, ph)) if pw > 0 && ph > 0 => {
                const EMU_PER_PX: f64 = 9525.0;
                let nat_w = (pw as f64 * EMU_PER_PX) as i64;
                let nat_h = (ph as f64 * EMU_PER_PX) as i64;
                // Fit within 6x4.5 inches, never upscaling past the natural size.
                let max_w = inch_f(6.0) as f64;
                let max_h = inch_f(4.5) as f64;
                let scale = (max_w / nat_w as f64).min(max_h / nat_h as f64).min(1.0);
                (
                    nat_w,
                    nat_h,
                    (nat_w as f64 * scale) as i64,
                    (nat_h as f64 * scale) as i64,
                )
            }
            _ => (inch_f(3.0), inch_f(2.0), inch_f(3.0), inch_f(2.0)),
        };
        let frame = RectEmu::new(inch_f(1.5), inch_f(1.0), frame_w, frame_h);
        let pic = hayate_ir::image::PictureRef {
            media_key: key,
            natural: SizeEmu::new(nat_w, nat_h),
        };
        let tx = Transaction::new(
            "insert image",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Parent(self.container()),
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

    /// The layout the Master tab is editing: the explicit `master_layout`, else the current
    /// slide's layout.
    pub(crate) fn active_layout(&self) -> Option<Entity> {
        self.master_layout
            .or_else(|| self.pres.layout_of(self.slide))
    }

    /// All layouts belonging to the current slide's master, in id order.
    pub(crate) fn master_layouts(&self) -> Vec<Entity> {
        let Some(master) = self.pres.master_of(self.slide) else {
            return Vec::new();
        };
        let mut v: Vec<Entity> = self
            .pres
            .world
            .iter()
            .filter(|e| {
                self.pres
                    .world
                    .layout_info
                    .get(e)
                    .is_some_and(|li| li.master == master)
            })
            .collect();
        v.sort_by_key(|e| e.0);
        v
    }

    /// Create a layout pre-populated from a standard preset (Title Slide, Title and Content, …)
    /// under the current master, select it for editing, and return it. Placeholders are one
    /// undoable transaction.
    pub(crate) fn add_layout_preset(&mut self, preset: edit::LayoutPreset) -> Option<Entity> {
        let master = self.pres.master_of(self.slide)?;
        let n = self.master_layouts().len() + 1;
        let layout = self
            .pres
            .add_layout(master, format!("{} {}", preset.name(), n));
        self.fill_layout_preset(layout, preset);
        self.master_layout = Some(layout);
        Some(layout)
    }

    /// Populate `layout` with a preset's placeholders (Title/Body/…) as one undoable transaction.
    pub(crate) fn fill_layout_preset(&mut self, layout: Entity, preset: edit::LayoutPreset) {
        let specs = edit::preset_placeholders(preset, self.pres.slide_size);
        let mut ops = Vec::new();
        let mut order = FracIndex::after(None);
        for spec in specs {
            let e = self.pres.world.reserve_id();
            let body = TextBody {
                paragraphs: vec![Paragraph::new(vec![Run {
                    text: spec.label.to_string(),
                    font: FontRef::Theme(spec.slot),
                    size: pt(spec.size_pt),
                    color: Color::theme(ThemeColorToken::Dk1),
                    bold: spec.bold,
                    italic: false,
                    underline: false,
                }])],
                autofit: false,
            };
            let tx =
                edit::create_placeholder(e, layout, order.clone(), spec.ph, spec.frame, Some(body));
            ops.extend(tx.ops);
            order = FracIndex::after(Some(&order));
        }
        if !ops.is_empty() {
            self.commit_tx(Transaction::new("add layout preset", ops));
        }
    }

    /// Point the current slide at `layout` (undoable). Inherited placeholders update immediately.
    pub(crate) fn set_current_slide_layout(&mut self, layout: Entity) {
        let tx = edit::set_slide_layout(self.slide, layout);
        self.commit_tx(tx);
    }

    /// Rename a layout (undoable; updates its `LayoutInfo.name`).
    pub(crate) fn rename_layout(&mut self, layout: Entity, name: String) {
        let Some(li) = self.pres.world.layout_info.get(&layout).cloned() else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let tx = Transaction::new(
            "rename layout",
            vec![Operation::SetComponent {
                entity: layout,
                value: CompValue::Layout(hayate_ir::doc::LayoutInfo {
                    master: li.master,
                    name: name.to_string(),
                }),
            }],
        );
        self.commit_tx(tx);
    }

    /// Duplicate a layout and its placeholder shapes; select the copy for editing.
    pub(crate) fn duplicate_layout(&mut self, layout: Entity) {
        let Some(li) = self.pres.world.layout_info.get(&layout).cloned() else {
            return;
        };
        let copy = self.pres.add_layout(li.master, format!("{} copy", li.name));
        let mut ops = Vec::new();
        let mut order = FracIndex::after(None);
        for child in self.pres.placeholder_shapes(layout) {
            let ne = self.pres.world.reserve_id();
            ops.push(Operation::Spawn { entity: ne });
            for comp in self.pres.world.components_of(child) {
                // Re-parent the copy to the new layout; keep all other components.
                let comp = match comp {
                    CompValue::Parent(_) => CompValue::Parent(copy),
                    CompValue::Order(_) => CompValue::Order(order.clone()),
                    other => other,
                };
                ops.push(Operation::SetComponent {
                    entity: ne,
                    value: comp,
                });
            }
            order = FracIndex::after(Some(&order));
        }
        if !ops.is_empty() {
            self.commit_tx(Transaction::new("duplicate layout", ops));
        }
        self.master_layout = Some(copy);
    }

    /// Delete a layout and its placeholders, unless a slide still uses it (then no-op).
    pub(crate) fn delete_layout(&mut self, layout: Entity) {
        let in_use = self
            .pres
            .slides()
            .iter()
            .any(|s| self.pres.layout_of(*s) == Some(layout));
        if in_use {
            return; // refuse to delete a layout slides depend on
        }
        let mut ops = vec![Operation::Despawn { entity: layout }];
        for child in self.pres.placeholder_shapes(layout) {
            ops.push(Operation::Despawn { entity: child });
        }
        self.commit_tx(Transaction::new("delete layout", ops));
        if self.master_layout == Some(layout) {
            self.master_layout = None;
        }
        if self.scope == EditScope::Layout(layout) {
            self.exit_scope();
        }
    }

    /// Add a placeholder of `ph_type` to the layout the Master tab is editing, with a sensible
    /// default frame and prompt text. It then renders on every slide using that layout.
    pub(crate) fn add_layout_placeholder(&mut self, ph_type: hayate_ir::doc::PlaceholderType) {
        use hayate_ir::doc::{PlaceholderRef, PlaceholderType as PT};
        let Some(layout) = self.active_layout() else {
            return;
        };
        // Next free idx for this type on the layout.
        let idx = self
            .pres
            .placeholder_shapes(layout)
            .iter()
            .filter_map(|e| self.pres.world.placeholders.get(e))
            .filter(|p| p.ph_type == ph_type)
            .map(|p| p.idx + 1)
            .max()
            .unwrap_or(0);
        let (frame, label, size_pt, slot) = match ph_type {
            PT::Title | PT::CenteredTitle => (
                RectEmu::new(inch_f(0.5), inch_f(0.3), inch_f(12.33), inch_f(1.2)),
                "Title",
                40,
                ThemeFontSlot::Major,
            ),
            PT::Subtitle => (
                RectEmu::new(inch_f(0.5), inch_f(1.6), inch_f(12.33), inch_f(1.0)),
                "Subtitle",
                28,
                ThemeFontSlot::Minor,
            ),
            PT::Body => (
                RectEmu::new(inch_f(0.5), inch_f(1.8), inch_f(12.33), inch_f(5.0)),
                "Body text",
                24,
                ThemeFontSlot::Minor,
            ),
            _ => (
                RectEmu::new(inch_f(1.0), inch_f(1.0), inch_f(4.0), inch_f(1.0)),
                "Placeholder",
                24,
                ThemeFontSlot::Minor,
            ),
        };
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: label.to_string(),
                font: FontRef::Theme(slot),
                size: pt(size_pt),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: matches!(ph_type, PT::Title | PT::CenteredTitle),
                italic: false,
                underline: false,
            }])],
            autofit: false,
        };
        let order = {
            let kids = self.pres.children(layout);
            FracIndex::after(kids.last().and_then(|e| self.pres.world.order.get(e)))
        };
        let reserved = self.pres.world.reserve_id();
        let tx = edit::create_placeholder(
            reserved,
            layout,
            order,
            PlaceholderRef { ph_type, idx },
            frame,
            Some(body),
        );
        self.commit_tx(tx);
    }

    /// Promote an inherited placeholder to an editable slide-level override and start editing it.
    /// Used when the user clicks an inherited placeholder/prompt on a slide.
    pub(crate) fn promote_and_edit(&mut self, ph: hayate_ir::doc::PlaceholderRef) {
        let reserved = self.pres.world.reserve_id();
        let order = self.append_order();
        if let Some(tx) = edit::promote_placeholder(&self.pres, self.slide, ph, reserved, order) {
            self.commit_tx(tx);
            self.selection = Some(reserved);
            self.also.clear();
            self.begin_text_edit(reserved);
        }
    }

    /// Whether the selected entity is a slide-level placeholder override (eligible for reset).
    pub(crate) fn selection_is_slide_placeholder(&self) -> bool {
        self.selection.is_some_and(|e| {
            self.pres.world.placeholders.contains_key(&e)
                && self.pres.world.parent.get(&e) == Some(&self.slide)
        })
    }

    /// Remove the selected slide-level placeholder override so it falls back to the layout/master.
    pub(crate) fn reset_selected_placeholder(&mut self) {
        if let Some(e) = self.selection {
            if self.selection_is_slide_placeholder() {
                self.commit_tx(Transaction::new(
                    "reset placeholder",
                    vec![Operation::Despawn { entity: e }],
                ));
                self.selection = None;
            }
        }
    }

    /// The master whose theme the editor targets (the scope's owning master).
    pub(crate) fn current_master(&self) -> Option<Entity> {
        self.pres.owning_master(self.container())
    }

    /// Clone the current master's theme, let `f` mutate it, and commit it as one undoable change.
    fn edit_theme(&mut self, f: impl FnOnce(&mut hayate_ir::theme::Theme)) {
        let Some(master) = self.current_master() else {
            return;
        };
        let Some(mut theme) = self.pres.container_theme(master).cloned() else {
            return;
        };
        f(&mut theme);
        self.commit_tx(edit::set_master_theme(master, theme));
    }

    pub(crate) fn set_theme_accent(&mut self, i: usize, rgba: hayate_ir::color::Rgba) {
        self.edit_theme(|t| {
            if i < 6 {
                t.colors.accent[i] = rgba;
            }
        });
    }

    /// Cycle accent `i` through the built-in palettes' colour for that slot (a quick recolour
    /// without a full colour picker).
    pub(crate) fn cycle_theme_accent(&mut self, i: usize) {
        if i >= 6 {
            return;
        }
        let candidates: Vec<hayate_ir::color::Rgba> = hayate_ir::theme::theme_color_presets()
            .into_iter()
            .map(|(_, c)| c.accent[i])
            .collect();
        if candidates.is_empty() {
            return;
        }
        let current = self
            .current_master()
            .and_then(|m| self.pres.container_theme(m))
            .map(|t| t.colors.accent[i]);
        let next = current
            .and_then(|cur| candidates.iter().position(|c| *c == cur))
            .map(|p| (p + 1) % candidates.len())
            .unwrap_or(0);
        self.set_theme_accent(i, candidates[next]);
    }

    pub(crate) fn apply_color_preset(&mut self, idx: usize) {
        let presets = hayate_ir::theme::theme_color_presets();
        if let Some((_, colors)) = presets.get(idx) {
            let colors = colors.clone();
            self.edit_theme(|t| t.colors = colors);
        }
    }

    pub(crate) fn set_theme_font(&mut self, major: bool, family: String) {
        self.edit_theme(|t| {
            let slot = if major {
                &mut t.fonts.major
            } else {
                &mut t.fonts.minor
            };
            slot.latin = family.clone();
            slot.ea = family.clone();
            slot.cs = family;
        });
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
            let kids = self.pres.children(self.container());
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
                    value: CompValue::Parent(self.container()),
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

    /// The container the canvas currently edits (the slide, or a layout/master in master mode).
    pub(crate) fn container(&self) -> Entity {
        self.scope.container()
    }

    pub(crate) fn rebuild(&mut self) {
        let target = view_px(&self.pres, self.zoom);
        let c = self.container();
        self.scene = match self.scope {
            EditScope::Slide(_) => build_slide_scene(&self.pres, c, target),
            EditScope::Layout(_) => {
                let theme = self.pres.container_theme(c).cloned().unwrap_or_default();
                let bg = self.pres.container_background(c);
                let context: Vec<Entity> = self.pres.owning_master(c).into_iter().collect();
                hayate_render::build_container_scene(&self.pres, c, &theme, bg, &context, target)
            }
            EditScope::Master(_) => {
                let theme = self.pres.container_theme(c).cloned().unwrap_or_default();
                let bg = self.pres.container_background(c);
                hayate_render::build_container_scene(&self.pres, c, &theme, bg, &[], target)
            }
        };
    }

    /// Enter master edit mode for a layout (its master renders as dimmed context).
    pub(crate) fn enter_layout_scope(&mut self, layout: Entity) {
        self.scope = EditScope::Layout(layout);
        self.master_layout = Some(layout);
        self.selection = None;
        self.also.clear();
        self.rebuild();
    }

    /// Enter master edit mode for a master.
    pub(crate) fn enter_master_scope(&mut self, master: Entity) {
        self.scope = EditScope::Master(master);
        self.selection = None;
        self.also.clear();
        self.rebuild();
    }

    /// Leave master edit mode, returning to the current slide.
    pub(crate) fn exit_scope(&mut self) {
        self.scope = EditScope::Slide(self.slide);
        self.selection = None;
        self.also.clear();
        self.rebuild();
    }

    /// Rebuild + autosave after a document change (undo/redo, etc.).
    pub(crate) fn after_doc_change(&mut self) {
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, &self.doc_path);
    }

    /// Add a rectangle at the slide center as one undoable transaction, and select it.
    pub(crate) fn add_rect(&mut self) {
        let order = {
            let kids = self.pres.children(self.container());
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(4.0), inch_f(3.5), inch_f(1.6), inch_f(1.6));
        let tx = edit::create_rect(
            e,
            self.container(),
            order,
            frame,
            Fill::Solid(Color::theme(ThemeColorToken::Accent5)),
        );
        self.commit_tx(tx);
        self.selection = Some(e);
    }

    /// Sibling order key that appends after the current shapes on the slide.
    fn append_order(&self) -> FracIndex {
        let kids = self.pres.children(self.container());
        let last = kids.last().and_then(|e| self.pres.world.order.get(e));
        FracIndex::after(last)
    }

    /// Add an ellipse at the slide center and select it.
    pub(crate) fn add_ellipse(&mut self) {
        let order = self.append_order();
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(4.0), inch_f(3.5), inch_f(2.0), inch_f(1.4));
        let tx = Transaction::new(
            "create ellipse",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Parent(self.container()),
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
                    value: CompValue::Geometry(hayate_ir::shape::Geometry::Ellipse),
                },
                Operation::SetComponent {
                    entity: e,
                    value: CompValue::Fill(Fill::Solid(Color::theme(ThemeColorToken::Accent6))),
                },
            ],
        );
        self.commit_tx(tx);
        self.selection = Some(e);
    }

    /// Add a line (or arrow) across the slide center and select it. The arrow tool puts a head
    /// on the end point; the plain line tool leaves both ends bare.
    pub(crate) fn add_line(&mut self, arrow: bool) {
        use hayate_ir::shape::ArrowHead;
        let order = self.append_order();
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(3.5), inch_f(3.5), inch_f(2.5), inch_f(1.5));
        let end = if arrow {
            ArrowHead::Arrow
        } else {
            ArrowHead::None
        };
        let tx = edit::create_line(e, self.container(), order, frame, ArrowHead::None, end);
        self.commit_tx(tx);
        self.selection = Some(e);
    }

    /// Set the selected shape's stroke width (points), keeping its colour.
    pub(crate) fn set_stroke_width(&mut self, pt_val: i64) {
        use hayate_ir::paint::Stroke;
        if let Some(e) = self.selection {
            let color = self
                .pres
                .world
                .strokes
                .get(&e)
                .map(|s| s.color)
                .unwrap_or_else(|| Color::theme(ThemeColorToken::Dk1));
            let tx = Transaction::new(
                "set stroke width",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Stroke(Stroke::solid(color, pt(pt_val.max(1)))),
                }],
            );
            self.commit_tx(tx);
        }
    }

    /// Set the selected shape's stroke colour (a theme accent), keeping its width.
    pub(crate) fn set_stroke_color(&mut self, token: ThemeColorToken) {
        use hayate_ir::paint::Stroke;
        if let Some(e) = self.selection {
            let width = self
                .pres
                .world
                .strokes
                .get(&e)
                .map(|s| s.width)
                .unwrap_or_else(|| pt(2));
            let tx = Transaction::new(
                "set stroke color",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Stroke(Stroke::solid(Color::theme(token), width)),
                }],
            );
            self.commit_tx(tx);
        }
    }

    /// Current stroke width of the selection in points, if it has a stroke.
    pub(crate) fn sel_stroke_pt(&self) -> Option<i64> {
        let e = self.selection?;
        self.pres
            .world
            .strokes
            .get(&e)
            .map(|s| (s.width / 12_700).max(1))
    }

    /// Toggle a line's start/end arrowhead. `which` is true for the END, false for the START.
    pub(crate) fn set_arrow_head(&mut self, which_end: bool, on: bool) {
        use hayate_ir::shape::{ArrowHead, Geometry};
        if let Some(e) = self.selection {
            if let Some(Geometry::Line { start, end }) = self.pres.world.geometries.get(&e).copied()
            {
                let head = if on {
                    ArrowHead::Arrow
                } else {
                    ArrowHead::None
                };
                let (start, end) = if which_end {
                    (start, head)
                } else {
                    (head, end)
                };
                let tx = Transaction::new(
                    "set arrow head",
                    vec![Operation::SetComponent {
                        entity: e,
                        value: CompValue::Geometry(Geometry::Line { start, end }),
                    }],
                );
                self.commit_tx(tx);
            }
        }
    }

    /// Whether the selection is a Line, and its (start_is_arrow, end_is_arrow).
    pub(crate) fn sel_line_heads(&self) -> Option<(bool, bool)> {
        use hayate_ir::shape::{ArrowHead, Geometry};
        let e = self.selection?;
        match self.pres.world.geometries.get(&e).copied()? {
            Geometry::Line { start, end } => Some((
                matches!(start, ArrowHead::Arrow),
                matches!(end, ArrowHead::Arrow),
            )),
            _ => None,
        }
    }

    /// Insert an image from a file path (used by drag-and-drop). Reads the bytes if the file has
    /// a supported image extension; otherwise does nothing.
    pub(crate) fn insert_image_file(&mut self, path: std::path::PathBuf) {
        const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
        let ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
        if ok {
            if let Ok(bytes) = std::fs::read(&path) {
                self.insert_image_bytes(bytes);
            }
        }
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
            let _ = hayate_format::autosave(&self.pres, &self.doc_path);
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
        let tx = edit::group(&self.pres.world, &members, key);
        self.commit_tx(tx);
    }

    /// Ungroup the outermost group the current selection belongs to (un-nests one level).
    pub(crate) fn ungroup_selection(&mut self) {
        if let Some(sel) = self.selection {
            if let Some(key) = edit::outer_group(&self.pres.world, sel) {
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

    /// Copy every selected shape (primary + multi-selection / group) to the in-app clipboard.
    pub(crate) fn copy_selection(&mut self) {
        let sel = self.selected_all();
        self.clipboard = if sel.is_empty() {
            None
        } else {
            Some(
                sel.iter()
                    .map(|e| self.pres.world.components_of(*e))
                    .collect(),
            )
        };
    }

    /// Try to paste an image from the system clipboard. If the clipboard holds an image entry,
    /// insert it as a Picture shape (via `insert_image_bytes`) and return `true` to signal the
    /// paste was handled. Returns `false` when there is no clipboard image, so the caller can
    /// fall back to the internal shape paste.
    pub(crate) fn paste_clipboard_image(&mut self, cx: &mut Context<Self>) -> bool {
        // 1) gpui's own clipboard read: a decoded image, or file paths / a file URI pointing at
        //    an image. On Wayland gpui returns text in preference to an image when the source
        //    offers both, so this can miss screenshots — hence the external-tool fallback below.
        if let Some(item) = cx.read_from_clipboard() {
            for entry in item.entries() {
                match entry {
                    ClipboardEntry::Image(img) => {
                        self.insert_image_bytes(img.bytes.clone());
                        return true;
                    }
                    ClipboardEntry::ExternalPaths(paths) => {
                        if let Some(p) = paths.paths().first() {
                            let before = self.pres.children(self.container()).len();
                            self.insert_image_file(p.clone());
                            if self.pres.children(self.container()).len() > before {
                                return true;
                            }
                        }
                    }
                    ClipboardEntry::String(s) => {
                        if let Some(path) = clipboard_text_image_path(s.text()) {
                            let before = self.pres.children(self.container()).len();
                            self.insert_image_file(path);
                            if self.pres.children(self.container()).len() > before {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // 2) Fallback: ask the system clipboard for raw image bytes directly. This recovers
        //    screenshots on Wayland/X11 that gpui skipped in favour of accompanying text.
        if let Some(bytes) = read_clipboard_image_via_tool() {
            self.insert_image_bytes(bytes);
            return true;
        }
        false
    }

    pub(crate) fn paste_clipboard(&mut self) {
        let Some(shapes) = self.clipboard.clone() else {
            return;
        };
        if shapes.is_empty() {
            return;
        }
        const OFFSET: i64 = 182_880; // ~0.2 inch, so the copy is visibly nudged
        let mut order = {
            let kids = self.pres.children(self.container());
            FracIndex::after(kids.last().and_then(|e| self.pres.world.order.get(e)))
        };
        let mut ops = Vec::new();
        let mut new_ids: Vec<Entity> = Vec::new();
        for comps in shapes {
            let ne = self.pres.world.reserve_id();
            new_ids.push(ne);
            ops.push(Operation::Spawn { entity: ne });
            for comp in comps {
                match comp {
                    // Parent/Order assigned explicitly below; Group is dropped so the copies are
                    // independent (no new group is formed on paste).
                    CompValue::Parent(_) | CompValue::Order(_) | CompValue::Group(_) => {}
                    CompValue::Frame(f) => ops.push(Operation::SetComponent {
                        entity: ne,
                        value: CompValue::Frame(RectEmu {
                            origin: PointEmu::new(f.origin.x + OFFSET, f.origin.y + OFFSET),
                            size: f.size,
                        }),
                    }),
                    other => ops.push(Operation::SetComponent {
                        entity: ne,
                        value: other,
                    }),
                }
            }
            ops.push(Operation::SetComponent {
                entity: ne,
                value: CompValue::Parent(self.container()),
            });
            ops.push(Operation::SetComponent {
                entity: ne,
                value: CompValue::Order(order.clone()),
            });
            order = FracIndex::after(Some(&order));
        }
        self.commit_tx(Transaction::new("paste", ops));
        self.selection = new_ids.first().copied();
        self.also = new_ids.into_iter().skip(1).collect();
    }
}

const CLIPBOARD_IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Interpret clipboard text as a path to an image file. Accepts an absolute path or a `file://`
/// URI (as file managers place on the clipboard when copying a file), and only when the target
/// has a supported image extension and exists. Returns `None` for ordinary text.
fn clipboard_text_image_path(text: &str) -> Option<std::path::PathBuf> {
    let first = text.lines().next()?.trim();
    let raw = first.strip_prefix("file://").unwrap_or(first);
    // Decode the handful of percent-escapes that show up in file URIs (spaces, etc.).
    let decoded = percent_decode(raw);
    let path = std::path::PathBuf::from(decoded);
    let ok_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| CLIPBOARD_IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    (ok_ext && path.is_file()).then_some(path)
}

/// Minimal percent-decoding for `file://` URIs (e.g. `%20` -> space). Leaves malformed escapes
/// untouched.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Fetch raw image bytes from the system clipboard using an external helper, recovering images
/// that gpui's reader skips (on Wayland it prefers accompanying text over the image). Tries
/// `wl-paste` first when a Wayland session is present, then `xclip`. Returns the bytes only when
/// they look like a supported image.
fn read_clipboard_image_via_tool() -> Option<Vec<u8>> {
    use std::process::Command;
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let attempts: &[(&str, &[&str])] = if wayland {
        &[
            ("wl-paste", &["--no-newline", "--type", "image/png"]),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
        ]
    } else {
        &[
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
            ("wl-paste", &["--no-newline", "--type", "image/png"]),
        ]
    };
    for (cmd, args) in attempts {
        if let Ok(out) = Command::new(cmd).args(*args).output() {
            if out.status.success() && crate::paint::guess_image_format(&out.stdout).is_some() {
                return Some(out.stdout);
            }
        }
    }
    None
}
