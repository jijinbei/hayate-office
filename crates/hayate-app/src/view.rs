//! The main `Render` implementation: presentation mode, editing canvas (with caret, grid,
//! selection outlines, resize handles, alignment guides), slide sidebar, and the Format pane.

use gpui::{
    canvas, div, point, prelude::*, px, quad, rgb, rgba, size, Background, Bounds, ClickEvent,
    Context, ElementInputHandler, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, Render, SharedString, TextRun, Window,
};

use hayate_ir::color::ThemeColorToken;
use hayate_model::edit::LayoutPreset;
use hayate_render::scene::{Primitive, PxSize};
use hayate_render::{
    build_slide_scene, build_slide_scene_at, grid_lines, resize_handles, GuideKind,
};

use crate::icons::icon_button;
use crate::paint::paint_scene;
use crate::util::{hsla_of, prim_bounds, rotate_pt, run_font};
use crate::widgets::tool_button;
use crate::{DraggedSlide, FieldKind, HayateApp, LeftTab, MenuTarget, SlideDragPreview, SELECTION};

impl Render for HayateApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            window.focus(&self.focus, cx);
            self.focused_once = true;
        }
        // Scale the whole UI up a bit: rem drives text + spacing, so this enlarges the chrome
        // (toolbars, panels, dialogs) without touching the slide canvas geometry.
        window.set_rem_size(px(18.));

        // Home/start screen: shown at launch and via the Home button. Recents are built lazily.
        if self.home {
            self.load_home_recents(cx);
            return self.render_home(window, cx);
        }

        // Fullscreen presentation mode: the slide fit to the whole window, no panels.
        if self.present {
            let vp = window.viewport_size();
            let target = PxSize {
                w: f32::from(vp.width),
                h: f32::from(vp.height),
            };
            let pscene = build_slide_scene_at(&self.pres, self.slide, target, self.present_t);
            let pmedia = self.pres.media.clone();
            let indent_em = self.list_indent_em;
            let (pw, ph) = (pscene.size.w, pscene.size.h);
            let pcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| {
                    paint_scene(&pscene, b.origin, &pmedia, indent_em, window, cx)
                },
            )
            .size_full();
            return div()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
                .size_full()
                .bg(rgb(0x000000))
                .flex()
                .items_center()
                .justify_center()
                .child(div().w(px(pw)).h(px(ph)).child(pcanvas))
                .into_any_element();
        }

        // Refit the slide to the available area whenever the window is resized, so enlarging
        // the window enlarges the slide. Manual zoom (+/-) persists until the next resize.
        let vp = window.viewport_size();
        if self.last_viewport != Some(vp) {
            self.last_viewport = Some(vp);
            self.fit_zoom(window);
            self.rebuild();
        }

        // Document coordinates are absolute (points); on-screen size = slide_pt * zoom.
        let scene = self.scene.clone();
        let media = self.pres.media.clone();
        let selection = self.selection;
        // A locked placeholder shows a selection outline but no resize handles (its geometry is
        // fixed by the layout).
        let sel_locked = selection.is_some_and(|e| self.is_locked_placeholder(e));
        let also = self.also.clone();
        let guides = self.guides.clone();
        let show_grid = self.show_grid;
        let marquee = self.marquee;
        let list_indent_em = self.list_indent_em;
        // Caret/selection (editing entity + byte range) for drawing the cursor and highlight.
        let caret = self
            .text_edit
            .as_ref()
            .map(|te| (te.entity, te.selected.clone()));
        let origin_cell = self.canvas_origin.clone();
        let input_entity = cx.entity();
        let input_focus = self.focus.clone();
        let (sw, sh) = (scene.size.w, scene.size.h);
        // The interactive canvas area extends to cover shapes that overflow the slide's right/
        // bottom edges, so those shapes stay clickable (hit-testing happens within this region).
        // It is never smaller than the slide.
        let (area_w, area_h) = match scene.content_bounds() {
            Some(c) => ((c.x + c.w).max(sw), (c.y + c.h).max(sh)),
            None => (sw, sh),
        };

        let palette_panel = self.palette.as_ref().map(|p| {
            let list = self.palette_commands();
            let sel = p.sel;
            let mut col = div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(560.))
                .bg(rgb(0x2a2a2a))
                .border_1()
                .border_color(rgb(SELECTION));
            col = col.child(
                div()
                    .bg(rgb(0x111111))
                    .child(format!("\u{203a} {}", p.query)),
            );
            for (i, (_id, t)) in list.iter().enumerate() {
                let row = div().child(t.clone());
                let row = if i == sel {
                    row.bg(rgb(SELECTION)).text_color(rgb(0xffffff))
                } else {
                    row
                };
                col = col.child(row);
            }
            col
        });

        let slide_canvas = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let o = bounds.origin;
                origin_cell.set(o);
                // Register the IME/text input handler for this editing region.
                window.handle_input(
                    &input_focus,
                    ElementInputHandler::new(bounds, input_entity.clone()),
                    cx,
                );

                paint_scene(&scene, o, &media, list_indent_em, window, cx);

                // Text-edit caret + selection highlight. The edit buffer's byte offsets span all
                // lines (paragraphs joined by '\n'), so map an offset to its paragraph/line, then
                // place the caret on that row accounting for the bullet indent and glyph.
                if let Some((ent, sel)) = &caret {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(*ent)) {
                        if let Primitive::Text(tb) = &node.prim {
                            if !tb.paragraphs.is_empty() {
                                // (paragraph index, x in px, y top in px, line height) for a byte.
                                let pos_of = |byte: usize, window: &mut Window| {
                                    // Locate the paragraph and local byte offset.
                                    let mut start = 0usize;
                                    let mut pi = 0usize;
                                    let mut local = 0usize;
                                    for (i, para) in tb.paragraphs.iter().enumerate() {
                                        let len: usize =
                                            para.runs.iter().map(|r| r.text.len()).sum();
                                        pi = i;
                                        if byte <= start + len {
                                            local = byte - start;
                                            break;
                                        }
                                        local = len;
                                        start += len + 1; // + newline
                                    }
                                    let para = &tb.paragraphs[pi];
                                    let fs_of = |p: &hayate_render::scene::ResolvedParagraph| {
                                        p.runs
                                            .iter()
                                            .map(|r| r.size_px)
                                            .fold(0.0, f32::max)
                                            .max(1.0)
                                    };
                                    let font_size = px(fs_of(para));
                                    // y = sum of preceding paragraph line heights (one row each).
                                    let mut y = px(tb.bounds.y);
                                    for p in &tb.paragraphs[..pi] {
                                        y += px(fs_of(p) * 1.3);
                                    }
                                    let indent =
                                        font_size * (list_indent_em * para.bullet_level as f32);
                                    let style = para.runs.first();
                                    let font = style
                                        .map(run_font)
                                        .unwrap_or_else(|| gpui::font("sans-serif"));
                                    let color = hsla_of(
                                        style
                                            .map(|r| r.color)
                                            .unwrap_or(hayate_ir::color::Rgba::rgb(0, 0, 0)),
                                    );
                                    let shape_w = |s: &str, window: &mut Window| -> f32 {
                                        if s.is_empty() {
                                            return 0.0;
                                        }
                                        let trun = TextRun {
                                            len: s.len(),
                                            font: font.clone(),
                                            color,
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        f32::from(
                                            window
                                                .text_system()
                                                .shape_line(
                                                    SharedString::from(s),
                                                    font_size,
                                                    &[trun],
                                                    None,
                                                )
                                                .width,
                                        )
                                    };
                                    let bullet_w = if para.bullet_level > 0 {
                                        let glyph = match para.bullet_level {
                                            1 => "\u{2022} ",
                                            2 => "\u{25E6} ",
                                            _ => "\u{25AA} ",
                                        };
                                        shape_w(glyph, window)
                                    } else {
                                        0.0
                                    };
                                    let text_x = if para.runs.len() == 1 {
                                        let s = &para.runs[0].text;
                                        shape_w(&s[..local.min(s.len())], window)
                                    } else {
                                        let line_text: String =
                                            para.runs.iter().map(|r| r.text.as_str()).collect();
                                        shape_w(&line_text[..local.min(line_text.len())], window)
                                    };
                                    (
                                        pi,
                                        f32::from(indent) + bullet_w + text_x,
                                        y,
                                        font_size * 1.3,
                                    )
                                };

                                let (pi_e, x1, y_e, lh) = pos_of(sel.end, window);
                                let (pi_s, x0, _y_s, _) = pos_of(sel.start, window);
                                // Same-line selection highlight.
                                if pi_s == pi_e && (x1 - x0).abs() > 0.5 {
                                    window.paint_quad(quad(
                                        Bounds {
                                            origin: point(
                                                o.x + px(tb.bounds.x) + px(x0.min(x1)),
                                                o.y + y_e,
                                            ),
                                            size: size(px((x1 - x0).abs()), lh),
                                        },
                                        px(0.),
                                        Background::from(gpui::rgba(0x1166DD55)),
                                        px(0.),
                                        gpui::transparent_black(),
                                        Default::default(),
                                    ));
                                }
                                // Caret bar at the insertion point.
                                window.paint_quad(quad(
                                    Bounds {
                                        origin: point(o.x + px(tb.bounds.x) + px(x1), o.y + y_e),
                                        size: size(px(2.0), lh),
                                    },
                                    px(0.),
                                    Background::from(rgb(0x1166DD)),
                                    px(0.),
                                    gpui::transparent_black(),
                                    Default::default(),
                                ));
                            }
                        }
                    }
                }

                if show_grid {
                    let g = grid_lines(scene.size, scene.size.w / 16.0);
                    let gc = rgb(0xD0D0D0);
                    for x in g.vertical {
                        window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x + px(x), o.y),
                                size: size(px(1.0), px(scene.size.h)),
                            },
                            px(0.),
                            Background::from(gc),
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        ));
                    }
                    for y in g.horizontal {
                        window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x, o.y + px(y)),
                                size: size(px(scene.size.w), px(1.0)),
                            },
                            px(0.),
                            Background::from(gc),
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        ));
                    }
                }

                // Selection outline (drawn on top), rotated to match the shape.
                if let Some(sel) = selection {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
                        // A line/arrow is selected by its two endpoints, not a bounding box.
                        if let Primitive::Line { from, to, .. } = &node.prim {
                            let angle = node.rotation_deg.to_radians();
                            let (cx_, cy_) = ((from.0 + to.0) * 0.5, (from.1 + to.1) * 0.5);
                            let (ax, ay) = rotate_pt(from.0, from.1, cx_, cy_, angle);
                            let (bx, by) = rotate_pt(to.0, to.1, cx_, cy_, angle);
                            let mut sb = PathBuilder::stroke(px(1.5));
                            sb.move_to(point(o.x + px(ax), o.y + px(ay)));
                            sb.line_to(point(o.x + px(bx), o.y + px(by)));
                            if let Ok(path) = sb.build() {
                                window.paint_path(path, rgb(SELECTION));
                            }
                        } else {
                            let r = prim_bounds(&node.prim);
                            let angle = node.rotation_deg.to_radians();
                            let pad = 2.0;
                            if angle.abs() < 1e-3 {
                                let b = Bounds {
                                    origin: point(o.x + px(r.x - pad), o.y + px(r.y - pad)),
                                    size: size(px(r.w + 2.0 * pad), px(r.h + 2.0 * pad)),
                                };
                                window.paint_quad(quad(
                                    b,
                                    px(0.),
                                    gpui::transparent_black(),
                                    px(2.),
                                    rgb(SELECTION),
                                    Default::default(),
                                ));
                            } else {
                                let (cx_, cy_) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                                let corners = [
                                    (r.x - pad, r.y - pad),
                                    (r.x + r.w + pad, r.y - pad),
                                    (r.x + r.w + pad, r.y + r.h + pad),
                                    (r.x - pad, r.y + r.h + pad),
                                    (r.x - pad, r.y - pad), // close the loop for the stroke
                                ];
                                let mut sb = PathBuilder::stroke(px(2.));
                                for (i, (xx, yy)) in corners.iter().enumerate() {
                                    let (gx, gy) = rotate_pt(*xx, *yy, cx_, cy_, angle);
                                    let p = point(o.x + px(gx), o.y + px(gy));
                                    if i == 0 {
                                        sb.move_to(p);
                                    } else {
                                        sb.line_to(p);
                                    }
                                }
                                if let Ok(path) = sb.build() {
                                    window.paint_path(path, rgb(SELECTION));
                                }
                            }
                        }
                    }
                }
                // Additional multi-selection outlines.
                for ae in &also {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(*ae)) {
                        let r = prim_bounds(&node.prim);
                        let b = Bounds {
                            origin: point(o.x + px(r.x - 2.0), o.y + px(r.y - 2.0)),
                            size: size(px(r.w + 4.0), px(r.h + 4.0)),
                        };
                        window.paint_quad(quad(
                            b,
                            px(0.),
                            gpui::transparent_black(),
                            px(2.),
                            rgb(0x60A5FA),
                            Default::default(),
                        ));
                    }
                }

                // Combined bounding box for a group / multi-selection, so it reads as one object.
                if also.len() + usize::from(selection.is_some()) > 1 {
                    let mut union: Option<(f32, f32, f32, f32)> = None;
                    for e in selection.into_iter().chain(also.iter().copied()) {
                        if let Some(n) = scene.nodes.iter().find(|n| n.source == Some(e)) {
                            let r = prim_bounds(&n.prim);
                            let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
                            union = Some(match union {
                                None => (x0, y0, x1, y1),
                                Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                            });
                        }
                    }
                    if let Some((x0, y0, x1, y1)) = union {
                        let pad = 5.0;
                        window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x + px(x0 - pad), o.y + px(y0 - pad)),
                                size: size(px(x1 - x0 + 2.0 * pad), px(y1 - y0 + 2.0 * pad)),
                            },
                            px(0.),
                            gpui::transparent_black(),
                            px(1.5),
                            rgb(0x93C5FD),
                            Default::default(),
                        ));
                    }
                }

                // Resize handles on the selection: two endpoint handles for a line, the usual
                // eight bounding-box handles otherwise. Locked placeholders show none.
                if let Some(sel) = selection.filter(|_| !sel_locked) {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
                        let handle = |hx: f32, hy: f32, window: &mut Window| {
                            window.paint_quad(quad(
                                Bounds {
                                    origin: point(o.x + px(hx - 4.0), o.y + px(hy - 4.0)),
                                    size: size(px(8.0), px(8.0)),
                                },
                                px(1.0),
                                Background::from(rgb(0xffffff)),
                                px(1.0),
                                rgb(SELECTION),
                                Default::default(),
                            ));
                        };
                        if let Primitive::Line { from, to, .. } = &node.prim {
                            let angle = node.rotation_deg.to_radians();
                            let (cx_, cy_) = ((from.0 + to.0) * 0.5, (from.1 + to.1) * 0.5);
                            let (ax, ay) = rotate_pt(from.0, from.1, cx_, cy_, angle);
                            let (bx, by) = rotate_pt(to.0, to.1, cx_, cy_, angle);
                            handle(ax, ay, window);
                            handle(bx, by, window);
                        } else {
                            let r = prim_bounds(&node.prim);
                            for (hx, hy) in resize_handles(r, node.rotation_deg) {
                                handle(hx, hy, window);
                            }
                        }
                    }
                }
                // Smart alignment guides (drawn while dragging).
                for g in &guides {
                    let color = Background::from(rgb(0xFF3DAA));
                    match g.kind {
                        GuideKind::Vertical => window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x + px(g.pos - 0.5), o.y),
                                size: size(px(1.0), px(scene.size.h)),
                            },
                            px(0.),
                            color,
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        )),
                        GuideKind::Horizontal => window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x, o.y + px(g.pos - 0.5)),
                                size: size(px(scene.size.w), px(1.0)),
                            },
                            px(0.),
                            color,
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        )),
                    }
                }

                // Marquee (rubber-band) selection rectangle.
                if let Some((sx, sy, cx0, cy0)) = marquee {
                    let rx = sx.min(cx0);
                    let ry = sy.min(cy0);
                    let rw = (sx - cx0).abs();
                    let rh = (sy - cy0).abs();
                    let b = Bounds {
                        origin: point(o.x + px(rx), o.y + px(ry)),
                        size: size(px(rw), px(rh)),
                    };
                    window.paint_quad(quad(
                        b,
                        px(0.),
                        Background::from(gpui::rgba(0x3B82F622)),
                        px(1.),
                        rgb(SELECTION),
                        Default::default(),
                    ));
                }
            },
        )
        .size_full();

        // Left panel: a tab toggle between the slide list and the layers list, each scrollable.
        let slides = self.pres.slides();
        let current = self.slide;
        let left_tab = self.left_tab;
        let mut slide_list = div().flex().flex_col().gap_2();
        slide_list = slide_list.child(tool_button(
            "add_slide",
            "+ Add Slide \u{25be}",
            cx,
            |this, _w, cx| {
                this.add_slide_menu = !this.add_slide_menu;
                cx.notify();
            },
        ));
        // Layout picker: pick which layout the new slide uses (ONLYOFFICE-style).
        if self.add_slide_menu {
            let mut menu = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_1()
                .rounded_md()
                .bg(rgb(0x252525))
                .border_1()
                .border_color(rgb(0x444444));
            for layout in self.master_layouts() {
                let name = self
                    .pres
                    .world
                    .layout_info
                    .get(&layout)
                    .map(|li| li.name.clone())
                    .unwrap_or_else(|| "Layout".to_string());
                let ttheme = self
                    .pres
                    .container_theme(layout)
                    .cloned()
                    .unwrap_or_default();
                let tbg = self.pres.container_background(layout);
                let tctx: Vec<hayate_ir::world::Entity> =
                    self.pres.owning_master(layout).into_iter().collect();
                let tscene = hayate_render::build_container_scene(
                    &self.pres,
                    layout,
                    &ttheme,
                    tbg,
                    &tctx,
                    PxSize { w: 88.0, h: 50.0 },
                );
                let tmedia = self.pres.media.clone();
                let tindent = self.list_indent_em;
                let thumb = div()
                    .flex_none()
                    .w(px(88.))
                    .h(px(50.))
                    .border_1()
                    .border_color(rgb(0x444444))
                    .bg(rgb(0xffffff))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |b, _, window, cx| {
                                paint_scene(&tscene, b.origin, &tmedia, tindent, window, cx)
                            },
                        )
                        .size_full(),
                    );
                menu = menu.child(
                    div()
                        .id(("addslide", layout.0 as usize))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .p_1()
                        .rounded_md()
                        .hover(|s| s.bg(rgb(0x094771)))
                        .child(thumb)
                        .child(div().text_sm().truncate().child(name))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.add_slide_with_layout(layout);
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(
                div()
                    .id("addslide_dup")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(rgb(0x3a3a3a))
                    .hover(|s| s.bg(rgb(0x4a4a4a)))
                    .child("Duplicate slide")
                    .on_click(cx.listener(|this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.duplicate_slide();
                        this.add_slide_menu = false;
                        cx.notify();
                    })),
            );
            slide_list = slide_list.child(menu);
        }
        for (i, &s) in slides.iter().enumerate() {
            let tscene = build_slide_scene(&self.pres, s, PxSize { w: 176.0, h: 99.0 });
            let tmedia = self.pres.media.clone();
            let tindent = self.list_indent_em;
            let is_cur = s == current;
            let tcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| {
                    paint_scene(&tscene, b.origin, &tmedia, tindent, window, cx)
                },
            )
            .size_full();
            slide_list = slide_list.child(
                div()
                    .id(("slide", i))
                    .w(px(176.))
                    .h(px(99.))
                    .border_2()
                    .border_color(if is_cur {
                        rgb(SELECTION)
                    } else {
                        rgb(0x444444)
                    })
                    .child(tcanvas)
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                        // Selecting a slide leaves any master/layout edit mode.
                        this.slide = s;
                        this.scope = crate::EditScope::Slide(s);
                        this.selection = None;
                        this.rebuild();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _w, cx| {
                            this.slide = s;
                            this.scope = crate::EditScope::Slide(s);
                            this.selection = None;
                            this.also.clear();
                            this.rebuild();
                            this.open_menu(
                                f32::from(ev.position.x),
                                f32::from(ev.position.y),
                                MenuTarget::Slide,
                            );
                            cx.notify();
                        }),
                    )
                    // Drag a thumbnail onto another to reorder the deck.
                    .on_drag(DraggedSlide(s), |_, _offset, _window, cx| {
                        cx.new(|_| SlideDragPreview)
                    })
                    .drag_over::<DraggedSlide>(|style, _, _, _| style.border_color(rgb(0x9bbcff)))
                    .on_drop::<DraggedSlide>({
                        let view = cx.entity();
                        move |dragged, _window, cx| {
                            let dragged = dragged.0;
                            view.update(cx, |this, cx| {
                                this.reorder_slide(dragged, s);
                                cx.notify();
                            });
                        }
                    }),
            );
        }

        let master_mode = !self.scope.is_slide();

        // PRIMARY mode switcher — the first-class control for "what am I editing": the whole deck's
        // slides, or its master/layouts. Each half is a big segment; the active one is filled with
        // its scope color (slide = slate, master = purple). Switching flips the entire sidebar +
        // canvas, so there is exactly one place that decides the mode.
        let mode_seg = |id: &'static str,
                        icon: &'static str,
                        lbl: &'static str,
                        active: bool,
                        accent: u32,
                        cx: &mut Context<Self>,
                        on: Box<dyn Fn(&mut HayateApp)>| {
            let mut d = div()
                .id(id)
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_1()
                .py_2()
                .rounded_md()
                .text_sm();
            if active {
                d = d
                    .bg(rgb(accent))
                    .text_color(rgb(0xffffff))
                    .font_weight(gpui::FontWeight::SEMIBOLD);
            } else {
                d = d
                    .bg(rgb(0x2a2a2a))
                    .text_color(rgb(0x9a9a9a))
                    .hover(|s| s.bg(rgb(0x383838)).text_color(rgb(0xffffff)));
            }
            d.child(format!("{icon}  {lbl}")).on_click(cx.listener(
                move |t, _ev: &ClickEvent, window, cx| {
                    window.focus(&t.focus, cx);
                    on(t);
                    cx.notify();
                },
            ))
        };
        let mode_switcher = div()
            .flex()
            .flex_row()
            .gap_1()
            .p_1()
            .rounded_lg()
            .bg(rgb(0x1c1c1c))
            .child(mode_seg(
                "mode_slide",
                "📄",
                "スライド",
                !master_mode,
                crate::SCOPE_SLIDE,
                cx,
                Box::new(|t| t.exit_scope()),
            ))
            .child(mode_seg(
                "mode_master",
                "◆",
                "マスター",
                master_mode,
                crate::SCOPE_MASTER,
                cx,
                Box::new(|t| t.enter_master_mode()),
            ));

        // Secondary Slides | Layers sub-tabs (slide mode only).
        let subtab = |id: &'static str,
                      lbl: &'static str,
                      this_tab: LeftTab,
                      active: bool,
                      cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .px_2()
                .py_1()
                .rounded_md()
                .text_sm()
                .bg(if active { rgb(0x3a3a3a) } else { rgb(0x2a2a2a) })
                .hover(|s| s.bg(rgb(0x444444)))
                .child(lbl)
                .on_click(cx.listener(move |t, _ev: &ClickEvent, window, cx| {
                    window.focus(&t.focus, cx);
                    t.left_tab = this_tab;
                    cx.notify();
                }))
        };

        let mut sidebar = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .w(px(self.sidebar_w))
            .h_full()
            .p_2()
            .bg(rgb(0x252525))
            .child(mode_switcher);

        let content: gpui::AnyElement = if master_mode {
            self.master_panel(cx)
        } else {
            sidebar = sidebar.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(subtab(
                        "tab_slides",
                        "Slides",
                        LeftTab::Slides,
                        left_tab == LeftTab::Slides,
                        cx,
                    ))
                    .child(subtab(
                        "tab_layers",
                        "Layers",
                        LeftTab::Layers,
                        left_tab == LeftTab::Layers,
                        cx,
                    )),
            );
            match left_tab {
                LeftTab::Slides => slide_list.into_any_element(),
                LeftTab::Layers => self.layers_panel(cx),
            }
        };
        let sidebar = sidebar.child(
            div()
                .id("left_scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(content),
        );

        // Format (properties) pane for the selected shape — PowerPoint-style.
        let accents = [
            ThemeColorToken::Accent1,
            ThemeColorToken::Accent2,
            ThemeColorToken::Accent3,
            ThemeColorToken::Accent4,
            ThemeColorToken::Accent5,
            ThemeColorToken::Accent6,
        ];
        let theme = self
            .pres
            .container_theme(self.container())
            .cloned()
            .unwrap_or_default();
        let inspector = self.selection.map(|e| {
            let has_text = self.pres.world.texts.contains_key(&e);
            // A muted section label (Figma-style).
            let label = |s: &'static str| div().pt_1().text_sm().text_color(rgb(0x8a8a8a)).child(s);
            let mut swatches = div().flex().flex_row().gap_1();
            for (i, t) in accents.into_iter().enumerate() {
                let cu = crate::util::rgb_u32(theme.color_for(t));
                swatches = swatches.child(
                    div()
                        .id(("acc", i))
                        .w(px(22.))
                        .h(px(22.))
                        .rounded_md()
                        .bg(rgb(cu))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.set_fill_accent(t);
                            cx.notify();
                        })),
                );
            }
            // Stroke colour swatches (set the outline, not the fill).
            let mut stroke_swatches = div().flex().flex_row().gap_1();
            for (i, t) in accents.into_iter().enumerate() {
                let cu = crate::util::rgb_u32(theme.color_for(t));
                stroke_swatches = stroke_swatches.child(
                    div()
                        .id(("strk", i))
                        .w(px(22.))
                        .h(px(22.))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x777777))
                        .bg(rgb(cu))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.set_stroke_color(t);
                            cx.notify();
                        })),
                );
            }
            let line_heads = self.sel_line_heads();
            let mut pane = div()
                .id("inspector")
                .flex()
                .flex_col()
                .gap_1()
                .w(px(228.))
                .h_full()
                .overflow_y_scroll()
                .p_2()
                .bg(rgb(0x252525))
                .child(div().text_lg().pb_1().child("Format"))
                // Position / Size: editable numeric fields (click to type), no stepper buttons.
                .child(label("Position (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(self.num_field("f_x", FieldKind::PosX, cx))
                        .child(self.num_field("f_y", FieldKind::PosY, cx)),
                )
                .child(label("Size (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(self.num_field("f_w", FieldKind::SizeW, cx))
                        .child(self.num_field("f_h", FieldKind::SizeH, cx)),
                )
                .child(label("Rotation / Opacity"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(self.num_field("f_rot", FieldKind::Rotation, cx))
                        .child(self.num_field("f_op", FieldKind::Opacity, cx)),
                )
                .child(label("Fill"))
                .child(swatches)
                .child(label("Stroke"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .items_center()
                        .child(div().text_sm().text_color(rgb(0x8a8a8a)).child("W"))
                        .child(self.num_field("f_stroke_w", FieldKind::StrokeWidth, cx)),
                )
                .child(stroke_swatches);
            // Arrowhead start/end controls (only for line shapes).
            if let Some((start_on, end_on)) = line_heads {
                pane = pane.child(label("Arrowheads (start / end)")).child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(tool_button(
                            "ah_s",
                            if start_on {
                                "Start \u{25C0}"
                            } else {
                                "Start \u{2014}"
                            },
                            cx,
                            move |t, _w, cx| {
                                t.set_arrow_head(false, !start_on);
                                cx.notify();
                            },
                        ))
                        .child(tool_button(
                            "ah_e",
                            if end_on {
                                "End \u{25B6}"
                            } else {
                                "End \u{2014}"
                            },
                            cx,
                            move |t, _w, cx| {
                                t.set_arrow_head(true, !end_on);
                                cx.notify();
                            },
                        )),
                );
            }
            pane = pane.child(label("Arrange")).child(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(icon_button("front", "bring-front", cx, |t, _w, cx| {
                        t.run_on_selection("shape.bring_to_front");
                        cx.notify();
                    }))
                    .child(icon_button("back", "send-back", cx, |t, _w, cx| {
                        t.run_on_selection("shape.send_to_back");
                        cx.notify();
                    }))
                    .child(tool_button("al_l", "L", cx, |t, _w, cx| {
                        t.align("shapes.align_left");
                        cx.notify();
                    }))
                    .child(tool_button("al_c", "C", cx, |t, _w, cx| {
                        t.align("shapes.align_hcenter");
                        cx.notify();
                    }))
                    .child(tool_button("al_r", "R", cx, |t, _w, cx| {
                        t.align("shapes.align_right");
                        cx.notify();
                    }))
                    .child(tool_button("al_t", "T", cx, |t, _w, cx| {
                        t.align("shapes.align_top");
                        cx.notify();
                    }))
                    .child(tool_button("al_m", "M", cx, |t, _w, cx| {
                        t.align("shapes.align_vcenter");
                        cx.notify();
                    }))
                    .child(tool_button("al_b", "B", cx, |t, _w, cx| {
                        t.align("shapes.align_bottom");
                        cx.notify();
                    })),
            );
            // Text controls only when the selected shape carries text.
            if has_text {
                pane = pane
                    .child(label("Text"))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(icon_button("txt_bold", "bold", cx, |t, _w, cx| {
                                t.run_on_selection("shape.toggle_bold");
                                cx.notify();
                            }))
                            .child(icon_button("txt_italic", "italic", cx, |t, _w, cx| {
                                t.run_on_selection("shape.toggle_italic");
                                cx.notify();
                            }))
                            .child(icon_button(
                                "txt_underline",
                                "underline",
                                cx,
                                |t, _w, cx| {
                                    t.run_on_selection("shape.toggle_underline");
                                    cx.notify();
                                },
                            ))
                            .child(tool_button("txt_aminus", "A-", cx, |t, _w, cx| {
                                t.change_font_size(-4);
                                cx.notify();
                            }))
                            .child(tool_button("txt_aplus", "A+", cx, |t, _w, cx| {
                                t.change_font_size(4);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(tool_button("txt_al_l", "L", cx, |t, _w, cx| {
                                t.run_on_selection("shape.align_text_left");
                                cx.notify();
                            }))
                            .child(tool_button("txt_al_c", "C", cx, |t, _w, cx| {
                                t.run_on_selection("shape.align_text_center");
                                cx.notify();
                            }))
                            .child(tool_button("txt_al_r", "R", cx, |t, _w, cx| {
                                t.run_on_selection("shape.align_text_right");
                                cx.notify();
                            })),
                    )
                    .child(icon_button("txt_font", "type", cx, |t, _w, cx| {
                        t.font_picker = !t.font_picker;
                        cx.notify();
                    }))
                    // Bullet-list indent per level (adjustable).
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(label("List indent"))
                            .child(tool_button("li_minus", "-", cx, |t, _w, cx| {
                                t.list_indent_em = (t.list_indent_em - 0.25).max(0.0);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .min_w(px(32.))
                                    .text_sm()
                                    .text_color(rgb(0xdddddd))
                                    .child(format!("{:.2}", self.list_indent_em)),
                            )
                            .child(tool_button("li_plus", "+", cx, |t, _w, cx| {
                                t.list_indent_em = (t.list_indent_em + 0.25).min(3.0);
                                cx.notify();
                            })),
                    );
            }
            pane
        });

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            // While dragging the sidebar divider, the cursor tracks anywhere in the window.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| {
                if this.resizing_sidebar {
                    this.sidebar_w = f32::from(ev.position.x).clamp(140.0, 480.0);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseUpEvent, _w, cx| {
                    if this.resizing_sidebar {
                        this.resizing_sidebar = false;
                        cx.notify();
                    }
                }),
            )
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            // Window title bar (controls live here, ONLYOFFICE-style).
            .children(self.title_bar(cx))
            // Ribbon: a File/Home/Insert/Slideshow tab strip + the active tab's button row.
            .child(self.ribbon_strip(cx))
            .child(self.ribbon_row(cx))
            .children(palette_panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .gap_3()
                    .child(sidebar)
                    // Draggable divider: drag to resize the sidebar.
                    .child(
                        div()
                            .id("sidebar_divider")
                            .w(px(5.))
                            .h_full()
                            .flex_none()
                            .cursor_col_resize()
                            .bg(rgb(if self.resizing_sidebar {
                                SELECTION
                            } else {
                                0x333333
                            }))
                            .hover(|s| s.bg(rgb(SELECTION)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _ev: &MouseDownEvent, _w, cx| {
                                    this.resizing_sidebar = true;
                                    cx.notify();
                                }),
                            ),
                    )
                    // Canvas viewport: takes the remaining width and scrolls if the slide is
                    // larger than the area, so it never overlaps the Format pane.
                    .child(
                        div()
                            .id("canvas_viewport")
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .overflow_x_scroll()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .id("slide_canvas_area")
                                    .w(px(area_w))
                                    .h(px(area_h))
                                    // Frame tinted by the current edit scope (matches the
                                    // breadcrumb): slate for a slide, blue for a layout, purple for
                                    // the master — so the canvas itself signals what is being edited.
                                    .border_2()
                                    .border_color(rgb(self.scope.color()))
                                    // Drop image files onto the slide to insert them.
                                    .on_drop::<gpui::ExternalPaths>({
                                        let view = cx.entity();
                                        move |paths, _window, cx| {
                                            let paths = paths.paths().to_vec();
                                            view.update(cx, |this, cx| {
                                                for p in paths {
                                                    this.insert_image_file(p);
                                                }
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                            window.focus(&this.focus, cx);
                                            this.on_mouse_down(ev, cx);
                                        }),
                                    )
                                    .on_mouse_move(cx.listener(
                                        |this, ev: &MouseMoveEvent, _, cx| {
                                            this.on_mouse_move(ev, cx)
                                        },
                                    ))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                                            this.on_mouse_up(ev, cx)
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                            window.focus(&this.focus, cx);
                                            this.on_right_down(ev, cx);
                                        }),
                                    )
                                    .child(slide_canvas),
                            ),
                    )
                    .children(inspector),
            )
            .children(self.menu_overlay(cx))
            .children(self.font_overlay(window, cx))
            .children(self.save_overlay(cx))
            .children(self.script_overlay(cx))
            .children(self.ai_overlay(cx))
            .children(self.notice_overlay(cx))
            .into_any_element()
    }
}

impl HayateApp {
    /// One tab in the ribbon strip; filled when active, dimmed otherwise.
    fn ribbon_tab_button(
        &self,
        id: &'static str,
        label: &'static str,
        tab: crate::RibbonTab,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = self.ribbon_tab == tab;
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded_md()
            .text_sm()
            .text_color(if active { rgb(0xffffff) } else { rgb(0xaaaaaa) })
            .bg(if active {
                rgb(SELECTION)
            } else {
                rgb(0x2a2a2a)
            })
            .hover(|s| {
                s.bg(if active {
                    rgb(SELECTION)
                } else {
                    rgb(0x3a3a3a)
                })
            })
            .child(label)
            .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.ribbon_tab = tab;
                cx.notify();
            }))
            .into_any_element()
    }

    /// The ribbon tab strip (File / Home / Insert / Slideshow).
    fn ribbon_strip(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::RibbonTab::*;
        div()
            .flex()
            .flex_row()
            .gap_1()
            .px_2()
            .pt_1()
            .child(self.ribbon_tab_button("rb_file", "File", File, cx))
            .child(self.ribbon_tab_button("rb_home", "Home", Home, cx))
            .child(self.ribbon_tab_button("rb_insert", "Insert", Insert, cx))
            .child(self.ribbon_tab_button("rb_slideshow", "Slideshow", Slideshow, cx))
            .child(self.ribbon_tab_button("rb_tools", "Tools", Tools, cx))
            .into_any_element()
    }

    /// The active tab's button row, with the zoom controls always present on the right.
    fn ribbon_row(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let left = match self.ribbon_tab {
            crate::RibbonTab::File => self.ribbon_file(cx),
            crate::RibbonTab::Home => self.ribbon_home(cx),
            crate::RibbonTab::Insert => self.ribbon_insert(cx),
            crate::RibbonTab::Slideshow => self.ribbon_slideshow(cx),
            crate::RibbonTab::Tools => self.ribbon_tools(cx),
        };
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .child(left)
            // Zoom controls (always available).
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(icon_button("zoom_out", "minus", cx, |t, _w, cx| {
                        let z = t.zoom / 1.25;
                        t.set_zoom(z, cx);
                    }))
                    .child(div().child(format!("{}%", (self.zoom * 100.0).round() as i32)))
                    .child(icon_button("zoom_in", "plus", cx, |t, _w, cx| {
                        let z = t.zoom * 1.25;
                        t.set_zoom(z, cx);
                    }))
                    .child(icon_button("zoom_fit", "maximize", cx, |t, w, cx| {
                        t.fit_zoom(w);
                        t.rebuild();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// A small vertical divider between ribbon button groups.
    fn ribbon_sep() -> gpui::AnyElement {
        div()
            .w(px(1.))
            .h(px(20.))
            .bg(rgb(0x444444))
            .into_any_element()
    }

    /// File tab: new / open / save / export.
    fn ribbon_file(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .child(tool_button("rb_new", "New", cx, |t, _w, cx| {
                t.new_presentation();
                cx.notify();
            }))
            .child(tool_button("rb_open", "Open", cx, |t, _w, cx| {
                t.open();
                cx.notify();
            }))
            .child(tool_button("rb_save", "Save", cx, |t, _w, cx| {
                t.save();
                cx.notify();
            }))
            .child(tool_button(
                "rb_saveas",
                "Save As\u{2026}",
                cx,
                |t, _w, cx| {
                    t.open_save_dialog();
                    cx.notify();
                },
            ))
            .child(Self::ribbon_sep())
            .child(tool_button("rb_pdf", "Export PDF", cx, |t, _w, cx| {
                t.export_pdf();
                cx.notify();
            }))
            .child(tool_button("rb_pptx", "Export PPTX", cx, |t, _w, _cx| {
                t.export_pptx();
            }))
            .child(tool_button("rb_svg", "Export SVG", cx, |t, _w, _cx| {
                t.export_svg();
            }))
            .child(tool_button("rb_import", "Import PPTX", cx, |t, _w, cx| {
                t.import_pptx();
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(tool_button(
                "rb_home_screen",
                "\u{2302} Start",
                cx,
                |t, _w, cx| {
                    t.go_home();
                    cx.notify();
                },
            ))
            .into_any_element()
    }

    /// Home tab: slide ops, clipboard/history, and text formatting.
    fn ribbon_home(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .gap_1()
            .items_center()
            .child(tool_button("rb_add_slide", "+ Slide", cx, |t, _w, cx| {
                t.add_slide();
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(tool_button("rb_undo", "Undo", cx, |t, _w, cx| {
                t.undo();
                cx.notify();
            }))
            .child(tool_button("rb_redo", "Redo", cx, |t, _w, cx| {
                t.redo();
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(tool_button("rb_copy", "Copy", cx, |t, _w, _cx| {
                t.copy_selection();
            }))
            .child(tool_button("rb_paste", "Paste", cx, |t, _w, cx| {
                t.paste_clipboard();
                cx.notify();
            }))
            .child(tool_button("rb_dup", "Duplicate", cx, |t, _w, cx| {
                t.duplicate_selection();
                cx.notify();
            }))
            .child(tool_button("rb_del", "Delete", cx, |t, _w, cx| {
                t.delete_selection();
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(icon_button("rb_bold", "bold", cx, |t, _w, cx| {
                t.run_on_selection("shape.toggle_bold");
                cx.notify();
            }))
            .child(icon_button("rb_italic", "italic", cx, |t, _w, cx| {
                t.run_on_selection("shape.toggle_italic");
                cx.notify();
            }))
            .child(icon_button("rb_underline", "underline", cx, |t, _w, cx| {
                t.run_on_selection("shape.toggle_underline");
                cx.notify();
            }))
            .child(tool_button("rb_size_down", "A-", cx, |t, _w, cx| {
                t.change_font_size(-4);
                cx.notify();
            }))
            .child(tool_button("rb_size_up", "A+", cx, |t, _w, cx| {
                t.change_font_size(4);
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(icon_button("rb_align_l", "align-left", cx, |t, _w, cx| {
                t.run_on_selection("shape.align_text_left");
                cx.notify();
            }))
            .child(icon_button(
                "rb_align_c",
                "align-center",
                cx,
                |t, _w, cx| {
                    t.run_on_selection("shape.align_text_center");
                    cx.notify();
                },
            ))
            .child(icon_button("rb_align_r", "align-right", cx, |t, _w, cx| {
                t.run_on_selection("shape.align_text_right");
                cx.notify();
            }))
            .child(Self::ribbon_sep())
            .child(icon_button("rb_front", "bring-front", cx, |t, _w, cx| {
                t.run_on_selection("shape.bring_to_front");
                cx.notify();
            }))
            .child(icon_button("rb_back", "send-back", cx, |t, _w, cx| {
                t.run_on_selection("shape.send_to_back");
                cx.notify();
            }))
            .into_any_element()
    }

    /// Insert tab: shapes, text box, image.
    fn ribbon_insert(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .gap_1()
            .items_center()
            .child(icon_button("tool_rect", "square", cx, |t, _w, cx| {
                t.add_rect();
                cx.notify();
            }))
            .child(icon_button("tool_ellipse", "circle", cx, |t, _w, cx| {
                t.add_ellipse();
                cx.notify();
            }))
            .child(icon_button("tool_line", "line", cx, |t, _w, cx| {
                t.add_line(false);
                cx.notify();
            }))
            .child(icon_button("tool_arrow", "arrow", cx, |t, _w, cx| {
                t.add_line(true);
                cx.notify();
            }))
            .child(icon_button("tool_text", "type", cx, |t, _w, cx| {
                t.add_text_box();
                cx.notify();
            }))
            .child(icon_button("tool_image", "image", cx, |t, _w, cx| {
                t.insert_image(cx);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Slideshow tab: start the fullscreen presentation.
    fn ribbon_slideshow(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .child(tool_button(
                "rb_present",
                "\u{25b6} Start",
                cx,
                |t, _w, cx| {
                    t.start_present();
                    cx.notify();
                },
            ))
            .into_any_element()
    }

    /// Tools tab: the Rhai script console, AI authoring prompt, and command palette.
    fn ribbon_tools(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .child(tool_button("rb_script", "Script", cx, |t, _w, cx| {
                t.script_panel = Some(crate::ScriptPanel {
                    buf: String::new(),
                    scroll: gpui::ScrollHandle::new(),
                });
                cx.notify();
            }))
            .child(tool_button("rb_ai", "\u{2728} Ask AI", cx, |t, _w, cx| {
                t.ai_panel = Some(crate::AiPanel { buf: String::new() });
                cx.notify();
            }))
            .child(tool_button(
                "rb_palette",
                "Commands\u{2026}",
                cx,
                |t, _w, cx| {
                    t.palette = Some(crate::PaletteState {
                        query: String::new(),
                        sel: 0,
                    });
                    cx.notify();
                },
            ))
            .into_any_element()
    }

    /// Custom window title bar (the window is client-side decorated, so we draw our own controls).
    /// A custom title bar for platforms that draw their own window decoration: a draggable strip
    /// with the app name on the left and minimize / maximize / close controls on the right.
    ///
    /// Returns `None` on macOS, which provides a native titlebar (the window title is set on the
    /// native bar, and the traffic-light buttons handle the window controls), so no custom strip is
    /// drawn at all.
    fn title_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(target_os = "macos")]
        {
            let _ = cx;
            None
        }
        #[cfg(not(target_os = "macos"))]
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(34.))
                .w_full()
                .flex_none()
                .bg(rgb(0x171717))
                .border_b_1()
                .border_color(rgb(0x2a2a2a))
                // Draggable region: the title/logo area moves the window (controls are excluded).
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_t, _ev: &MouseDownEvent, window, _cx| {
                                window.start_window_move();
                            }),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0xdddddd))
                                .child("HayateOffice"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(icon_button("win_min", "minus", cx, |_t, window, _cx| {
                            window.minimize_window();
                        }))
                        .child(icon_button("win_max", "square", cx, |_t, window, _cx| {
                            window.zoom_window();
                        }))
                        .child(icon_button("win_close", "x", cx, |_t, window, _cx| {
                            window.remove_window();
                        })),
                )
                .into_any_element(),
        )
    }

    /// The home/start screen: a "New presentation" card plus a thumbnailed grid of recently
    /// opened files. Shown at launch and via the Home button; leaving it opens the editor.
    fn render_home(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        // The "New presentation" card opens a fresh template deck in master-edit mode.
        let new_card = div()
            .id("home_new")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .w(px(240.))
            .h(px(135.))
            .border_2()
            .border_color(rgb(SELECTION))
            .bg(rgb(0x2a2a2a))
            .cursor_pointer()
            .child(div().text_3xl().child("+"))
            .child(div().text_sm().child("New presentation"))
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.new_presentation();
                cx.notify();
            }));

        let mut grid = div().flex().flex_row().flex_wrap().gap_4().child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(new_card)
                .child(div().text_sm().text_color(rgb(0x8a8a8a)).child("Blank")),
        );

        for (i, thumb) in self.home_recents.iter().enumerate() {
            let scene = thumb.scene.clone();
            let media = thumb.media.clone();
            let path = thumb.path.clone();
            let name = thumb.name.clone();
            let (w, h) = (scene.size.w, scene.size.h);
            let tcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_scene(&scene, b.origin, &media, 0.5, window, cx),
            )
            .size_full();
            grid = grid.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(("recent", i))
                            .w(px(w.max(240.)))
                            .h(px(h.max(135.)))
                            .border_1()
                            .border_color(rgb(0x3a3a3a))
                            .bg(rgb(0xffffff))
                            .cursor_pointer()
                            .child(tcanvas)
                            .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                                this.open_recent(&path);
                                cx.notify();
                            })),
                    )
                    .child(div().text_sm().child(name)),
            );
        }

        let recents_section = if self.home_recents.is_empty() {
            div()
                .text_sm()
                .text_color(rgb(0x8a8a8a))
                .child("No recent presentations yet \u{2014} create one to get started.")
        } else {
            div()
                .text_sm()
                .text_color(rgb(0x8a8a8a))
                .child("Recent presentations")
        };

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .flex()
            .flex_col()
            // Window title bar with controls, then the padded home content.
            .children(self.title_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_6()
                    .p_8()
                    .child(div().text_2xl().child("HayateOffice"))
                    .child(recents_section)
                    .child(grid),
            )
            .into_any_element()
    }

    /// The font-picker overlay: a scrollable list of available font families. Clicking one sets
    /// the selected text shape's font via the `shape.set_font` command.
    fn font_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.font_picker {
            return None;
        }
        let mut names = window.text_system().all_font_names();
        names.sort();
        names.dedup();
        let mut list = div()
            .id("font_list")
            .absolute()
            .right(px(244.))
            .top(px(60.))
            .flex()
            .flex_col()
            .w(px(240.))
            .max_h(px(440.))
            .overflow_y_scroll()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_md()
            .shadow_lg()
            .text_color(rgb(0xffffff));
        for (i, name) in names.into_iter().enumerate() {
            let fam = name.clone();
            list = list.child(
                div()
                    .id(("font", i))
                    .px_3()
                    .py_1()
                    .text_sm()
                    .hover(|s| s.bg(rgb(0x094771)))
                    .child(name)
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        match this.font_target {
                            crate::FontTarget::Selection => this.run_on_selection_with(
                                "shape.set_font",
                                serde_json::json!({ "family": fam.clone() }),
                            ),
                            crate::FontTarget::ThemeMajor => this.set_theme_font(true, &fam),
                            crate::FontTarget::ThemeMinor => this.set_theme_font(false, &fam),
                        }
                        this.font_target = crate::FontTarget::Selection;
                        this.font_picker = false;
                        cx.notify();
                    })),
            );
        }
        Some(list.into_any_element())
    }

    /// The Master tab: pick the current slide's layout, create layouts, and add placeholders to
    /// the active layout. Placeholders added here render on every slide that uses that layout.
    fn master_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        use hayate_ir::doc::PlaceholderType as PT;
        let active = self.active_layout();

        // A small clickable button row.
        let btn =
            |id: &'static str, label: String, cx: &mut Context<Self>, f: fn(&mut HayateApp)| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(rgb(0x3a3a3a))
                    .hover(|s| s.bg(rgb(0x4a4a4a)))
                    .child(label)
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        f(this);
                        cx.notify();
                    }))
            };

        let mut col = div().flex().flex_col().gap_2().text_color(rgb(0xdddddd));

        // ONLYOFFICE/LibreOffice-style master view: a vertical list of large thumbnails — the
        // master at top, then each layout — clicking a thumbnail edits it on the canvas.
        let master = self.pres.master_of(self.slide);
        let master_active = matches!(self.scope, crate::EditScope::Master(_));
        if let Some(m) = master {
            let mtheme = self.pres.container_theme(m).cloned().unwrap_or_default();
            let mbg = self.pres.container_background(m);
            let mscene = hayate_render::build_container_scene(
                &self.pres,
                m,
                &mtheme,
                mbg,
                &[],
                PxSize { w: 176.0, h: 99.0 },
            );
            let mmedia = self.pres.media.clone();
            let mindent = self.list_indent_em;
            col = col.child(
                div()
                    .id("master_node")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .w(px(176.))
                            .h(px(99.))
                            .border_2()
                            .border_color(if master_active {
                                rgb(SELECTION)
                            } else {
                                rgb(0x666666)
                            })
                            .bg(rgb(0xffffff))
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |b, _, window, cx| {
                                        paint_scene(&mscene, b.origin, &mmedia, mindent, window, cx)
                                    },
                                )
                                .size_full(),
                            ),
                    )
                    .child(div().text_sm().child("Slide Master"))
                    .on_click(cx.listener(|this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        if let Some(m) = this.pres.master_of(this.slide) {
                            this.enter_master_scope(m);
                        }
                        cx.notify();
                    })),
            );
        }

        // Layouts nested under the master (indented with a tree rail).
        let mut layouts_col = div()
            .flex()
            .flex_col()
            .gap_2()
            .pl_3()
            .border_l_2()
            .border_color(rgb(0x3a3a3a));

        for layout in self.master_layouts() {
            let name = self
                .pres
                .world
                .layout_info
                .get(&layout)
                .map(|li| li.name.clone())
                .unwrap_or_else(|| "Layout".to_string());
            let is_active = active == Some(layout);
            // While renaming this layout, show the edit buffer instead of the name.
            let renaming = self
                .layout_rename
                .as_ref()
                .filter(|(l, _)| *l == layout)
                .map(|(_, b)| b.clone());
            let ttheme = self
                .pres
                .container_theme(layout)
                .cloned()
                .unwrap_or_default();
            let tbg = self.pres.container_background(layout);
            let tctx: Vec<hayate_ir::world::Entity> =
                self.pres.owning_master(layout).into_iter().collect();
            let tscene = hayate_render::build_container_scene(
                &self.pres,
                layout,
                &ttheme,
                tbg,
                &tctx,
                PxSize { w: 160.0, h: 90.0 },
            );
            let tmedia = self.pres.media.clone();
            let tindent = self.list_indent_em;
            let name_child = if let Some(buf) = renaming {
                div()
                    .text_sm()
                    .px_1()
                    .rounded_md()
                    .bg(rgb(0x1f1f1f))
                    .border_1()
                    .border_color(rgb(SELECTION))
                    .child(format!("{buf}|"))
                    .into_any_element()
            } else {
                div().text_sm().truncate().child(name).into_any_element()
            };
            layouts_col = layouts_col.child(
                div()
                    .id(("layout_card", layout.0 as usize))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    // Click the thumbnail to edit the layout on the canvas.
                    .child(
                        div()
                            .id(("layout_thumb", layout.0 as usize))
                            .w(px(160.))
                            .h(px(90.))
                            .border_2()
                            .border_color(if is_active {
                                rgb(SELECTION)
                            } else {
                                rgb(0x666666)
                            })
                            .bg(rgb(0xffffff))
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |b, _, window, cx| {
                                        paint_scene(&tscene, b.origin, &tmedia, tindent, window, cx)
                                    },
                                )
                                .size_full(),
                            )
                            .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                                window.focus(&this.focus, cx);
                                this.enter_layout_scope(layout);
                                cx.notify();
                            })),
                    )
                    .child(name_child)
                    // Right-click opens rename/duplicate/delete for this layout.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.open_menu(
                                f32::from(ev.position.x),
                                f32::from(ev.position.y),
                                MenuTarget::Layout(layout),
                            );
                            cx.notify();
                        }),
                    ),
            );
        }
        col = col.child(layouts_col);

        // Add a new (blank) layout under the master and edit it (LibreOffice "Add Layout").
        col = col.child(btn("add_layout", "+ Add Layout".to_string(), cx, |a| {
            if let Some(l) = a.add_layout_preset(LayoutPreset::Blank) {
                a.enter_layout_scope(l);
            }
        }));

        // Placeholders on the active layout, plus add buttons.
        col = col.child(
            div()
                .mt_2()
                .text_sm()
                .text_color(rgb(0x888888))
                .child("Placeholders on this layout"),
        );
        if let Some(layout) = active {
            for ph in self.pres.placeholder_shapes(layout) {
                if let Some(r) = self.pres.world.placeholders.get(&ph) {
                    col = col.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .text_color(rgb(0xbbbbbb))
                            .child(format!("{:?} #{}", r.ph_type, r.idx)),
                    );
                }
            }
        }
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap_1()
                .child(btn("ph_title", "+ Title".to_string(), cx, |a| {
                    a.add_layout_placeholder(PT::Title)
                }))
                .child(btn("ph_subtitle", "+ Subtitle".to_string(), cx, |a| {
                    a.add_layout_placeholder(PT::Subtitle)
                }))
                .child(btn("ph_body", "+ Body".to_string(), cx, |a| {
                    a.add_layout_placeholder(PT::Body)
                })),
        );

        // Theme: colour-scheme presets, an accent preview, and heading/body fonts. Edits the
        // current master's theme, so every slide updates at once.
        col = col.child(
            div()
                .mt_2()
                .text_sm()
                .text_color(rgb(0x888888))
                .child("Theme"),
        );
        let mut presets_row = div().flex().flex_row().flex_wrap().gap_1();
        for (i, pname) in hayate_ir::theme::theme_color_preset_names()
            .iter()
            .copied()
            .enumerate()
        {
            presets_row = presets_row.child(
                div()
                    .id(("cpreset", i))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(rgb(0x3a3a3a))
                    .hover(|s| s.bg(rgb(0x4a4a4a)))
                    .child(pname)
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.apply_color_preset(i);
                        cx.notify();
                    })),
            );
        }
        col = col.child(presets_row);
        if let Some(theme) = self.pres.container_theme(self.container()) {
            let accents = theme.colors.accent;
            let mut sw = div().flex().flex_row().gap_1();
            for (i, c) in accents.into_iter().enumerate() {
                sw = sw.child(
                    div()
                        .id(("accent", i))
                        .w(px(20.))
                        .h(px(20.))
                        .rounded_md()
                        .bg(rgb(crate::util::rgb_u32(c)))
                        .hover(|s| s.border_1().border_color(rgb(0xffffff)))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.cycle_theme_accent(i);
                            cx.notify();
                        })),
                );
            }
            col = col.child(sw);
        }
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap_1()
                .child(btn("theme_major", "Heading font".to_string(), cx, |a| {
                    a.font_target = crate::FontTarget::ThemeMajor;
                    a.font_picker = true;
                }))
                .child(btn("theme_minor", "Body font".to_string(), cx, |a| {
                    a.font_target = crate::FontTarget::ThemeMinor;
                    a.font_picker = true;
                })),
        );
        col.into_any_element()
    }

    /// A transient notice modal (e.g. "PDF exported …") over a dimmed backdrop. Dismissed by the
    /// OK button, clicking the backdrop, or Esc/Enter (handled in on_key_down).
    fn notice_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let msg = self.notice.clone()?;
        let dialog = div()
            .flex()
            .flex_col()
            .gap_3()
            .max_w(px(520.))
            .p_4()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_lg()
            .shadow_lg()
            .text_color(rgb(0xffffff))
            .child(div().text_sm().child(msg))
            .child(
                div()
                    .id("notice_ok")
                    .self_end()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x3b82f6))
                    .hover(|s| s.bg(rgb(0x2f6fd6)))
                    .child("OK")
                    .on_click(cx.listener(|this, _ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.notice = None;
                        cx.notify();
                    })),
            );
        Some(
            div()
                .id("notice_backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x00000088))
                .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                    this.notice = None;
                    cx.notify();
                }))
                .child(dialog)
                .into_any_element(),
        )
    }

    /// The "Save As" dialog: a centered modal over a dimmed backdrop with an editable filename.
    /// Enter saves, Esc cancels (both handled by `save_modal_key`); clicking the backdrop cancels.
    /// The script console overlay: an editable multi-line Rhai buffer. Lines are rendered
    /// individually (gpui does not break a single text child on `\n`); a caret marks the end.
    fn script_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let p = self.script_panel.as_ref()?;
        // Scrollable, syntax-highlighted source area with a fixed height so a long script no
        // longer pushes the dialog off-screen — the wheel scrolls it, and edits track the caret.
        let mut body = div()
            .id("script_body")
            .flex()
            .flex_col()
            .w_full()
            .h(px(360.))
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(0x1f1f1f))
            .border_1()
            .border_color(rgb(0x3b82f6))
            .text_color(rgb(0xeaeaea))
            .font_family("monospace")
            .overflow_y_scroll()
            .track_scroll(&p.scroll);
        let lines: Vec<&str> = p.buf.split('\n').collect();
        let last = lines.len().saturating_sub(1);
        for (i, line) in lines.iter().enumerate() {
            let mut row = div().flex().flex_row().child(
                // Right-aligned gutter line number.
                div()
                    .w(px(34.))
                    .pr_2()
                    .flex_none()
                    .text_color(rgb(0x5a5a5a))
                    .child(format!("{:>3}", i + 1)),
            );
            // Highlighted segments; keep empty lines from collapsing with a non-breaking space.
            let mut content = div().flex().flex_row().flex_wrap();
            if line.is_empty() && i != last {
                content = content.child(div().child("\u{00a0}"));
            }
            for (text, color) in highlight_rhai(line) {
                content = content.child(div().text_color(rgb(color)).child(text));
            }
            // Caret on the final line.
            if i == last {
                content = content.child(div().text_color(rgb(0x3b82f6)).child("|"));
            }
            body = body.child(row.child(content));
        }
        let footer = match hayate_core::check_script(&p.buf) {
            Some(err) => div()
                .text_xs()
                .text_color(rgb(0xff6b6b))
                .child(format!("⚠ {err}")),
            None => div().text_xs().text_color(rgb(0x888888)).child(
                "Ctrl/Cmd+Enter to run · Enter for newline · Ctrl/Cmd+V to paste · Esc to close",
            ),
        };
        let dialog = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(720.))
            .p_4()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_lg()
            .shadow_lg()
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xaaaaaa))
                    .child("Script Console (Rhai)"),
            )
            .child(body)
            .child(footer);
        let backdrop = div()
            .id("script_backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000088))
            .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                this.script_panel = None;
                cx.notify();
            }))
            .child(dialog);
        Some(backdrop.into_any_element())
    }

    /// The AI prompt overlay: a single-line natural-language request turned into a script.
    fn ai_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let p = self.ai_panel.as_ref()?;
        let field = div()
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(0x1f1f1f))
            .border_1()
            .border_color(rgb(0x3b82f6))
            .text_color(rgb(0xffffff))
            .child(format!("{}|", p.buf));
        let dialog = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(560.))
            .p_4()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_lg()
            .shadow_lg()
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xaaaaaa))
                    .child("Ask AI — describe an edit"),
            )
            .child(field)
            .child(div().text_xs().text_color(rgb(0x888888)).child(
                "Enter to generate · Esc to cancel — e.g. \u{201c}make a 3x3 grid of blue boxes\u{201d}",
            ));
        let backdrop = div()
            .id("ai_backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000088))
            .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                this.ai_panel = None;
                cx.notify();
            }))
            .child(dialog);
        Some(backdrop.into_any_element())
    }

    fn save_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let m = self.save_modal.as_ref()?;
        let field = div()
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(0x1f1f1f))
            .border_1()
            .border_color(rgb(0x3b82f6))
            .text_color(rgb(0xffffff))
            .child(format!("{}|", m.buf));
        let dialog = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(420.))
            .p_4()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_lg()
            .shadow_lg()
            .text_color(rgb(0xffffff))
            .child(div().text_sm().text_color(rgb(0xaaaaaa)).child("Save As"))
            .child(field)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x888888))
                    .child("Enter to save · Esc to cancel"),
            );
        let backdrop = div()
            .id("save_backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000088))
            .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                this.save_modal = None;
                cx.notify();
            }))
            .child(dialog);
        Some(backdrop.into_any_element())
    }
}

/// Rhai keywords we tint blue in the script console.
const RHAI_KEYWORDS: [&str; 16] = [
    "let", "const", "fn", "for", "in", "if", "else", "while", "loop", "return", "break",
    "continue", "switch", "true", "false", "throw",
];

/// Token colors for the editor (VS Code "Dark+"-ish).
const HL_DEFAULT: u32 = 0xeaeaea;
const HL_COMMENT: u32 = 0x6a9955;
const HL_STRING: u32 = 0xce9178;
const HL_NUMBER: u32 = 0xb5cea8;
const HL_KEYWORD: u32 = 0x569cd6;

/// Split a single source line into `(text, color)` segments for lightweight syntax highlighting.
/// Per-line only (no multi-line string tracking) — cheap and good enough for the console. Consec-
/// utive punctuation/whitespace is merged into one default-colored run to keep the element count
/// low.
fn highlight_rhai(line: &str) -> Vec<(String, u32)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<(String, u32)>| {
        if !plain.is_empty() {
            out.push((std::mem::take(plain), HL_DEFAULT));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Line comment: the rest of the line.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            flush(&mut plain, &mut out);
            out.push((chars[i..].iter().collect(), HL_COMMENT));
            break;
        }
        // String literal up to the closing quote (or end of line).
        if c == '"' {
            flush(&mut plain, &mut out);
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push((chars[start..i].iter().collect(), HL_STRING));
            continue;
        }
        // Identifier / keyword / number word.
        if c.is_alphanumeric() || c == '_' {
            flush(&mut plain, &mut out);
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let color = if word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                HL_NUMBER
            } else if RHAI_KEYWORDS.contains(&word.as_str()) {
                HL_KEYWORD
            } else {
                HL_DEFAULT
            };
            out.push((word, color));
            continue;
        }
        // Punctuation / whitespace: accumulate into the default run.
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

#[cfg(test)]
mod highlight_tests {
    use super::{highlight_rhai, HL_COMMENT, HL_KEYWORD, HL_NUMBER, HL_STRING};

    fn color_of<'a>(segs: &'a [(String, u32)], needle: &str) -> Option<u32> {
        segs.iter().find(|(t, _)| t == needle).map(|(_, c)| *c)
    }

    #[test]
    fn keywords_strings_numbers_and_comments_are_tinted() {
        let segs = highlight_rhai(r#"let x = 42; // note"#);
        assert_eq!(color_of(&segs, "let"), Some(HL_KEYWORD));
        assert_eq!(color_of(&segs, "x"), Some(super::HL_DEFAULT));
        assert_eq!(color_of(&segs, "42"), Some(HL_NUMBER));
        assert_eq!(
            segs.iter()
                .find(|(t, _)| t.starts_with("//"))
                .map(|(_, c)| *c),
            Some(HL_COMMENT)
        );

        let segs = highlight_rhai(r##"shape_set_fill(e, "#ff0000");"##);
        assert_eq!(color_of(&segs, "\"#ff0000\""), Some(HL_STRING));
        // A `//` inside a string is part of the string, not a comment.
        let segs = highlight_rhai(r#"let u = "http://x";"#);
        assert_eq!(color_of(&segs, "\"http://x\""), Some(HL_STRING));
    }
}
