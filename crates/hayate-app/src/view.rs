//! The main `Render` implementation: presentation mode, editing canvas (with caret, grid,
//! selection outlines, resize handles, alignment guides), slide sidebar, and the Format pane.

use gpui::{
    canvas, div, point, prelude::*, px, quad, rgb, size, Background, Bounds, ClickEvent, Context,
    ElementInputHandler, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Render, SharedString, TextRun, Window,
};

use hayate_ir::color::ThemeColorToken;
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

        // Fullscreen presentation mode: the slide fit to the whole window, no panels.
        if self.present {
            let vp = window.viewport_size();
            let target = PxSize {
                w: f32::from(vp.width),
                h: f32::from(vp.height),
            };
            let pscene = build_slide_scene_at(&self.pres, self.slide, target, self.present_t);
            let pmedia = self.pres.media.clone();
            let (pw, ph) = (pscene.size.w, pscene.size.h);
            let pcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_scene(&pscene, b.origin, &pmedia, window, cx),
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
        let also = self.also.clone();
        let guides = self.guides.clone();
        let show_grid = self.show_grid;
        let marquee = self.marquee;
        // Caret/selection (editing entity + byte range) for drawing the cursor and highlight.
        let caret = self
            .text_edit
            .as_ref()
            .map(|te| (te.entity, te.selected.clone()));
        let origin_cell = self.canvas_origin.clone();
        let input_entity = cx.entity();
        let input_focus = self.focus.clone();
        let (sw, sh) = (scene.size.w, scene.size.h);
        let title: SharedString = "HayateOffice".into();

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

                paint_scene(&scene, o, &media, window, cx);

                // Text-edit caret + selection highlight.
                if let Some((ent, sel)) = &caret {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(*ent)) {
                        if let Primitive::Text(tb) = &node.prim {
                            if let Some((para, run0)) = tb
                                .paragraphs
                                .first()
                                .and_then(|p| p.runs.first().map(|r| (p, r)))
                            {
                                let font_size =
                                    px(para.runs.iter().map(|r| r.size_px).fold(0.0, f32::max));
                                let line_height = font_size * 1.3;
                                // Pixel x of a byte offset into the first run (shaped prefix width).
                                let x_at = |upto_byte: usize, window: &mut Window| -> f32 {
                                    let upto = upto_byte.min(run0.text.len());
                                    if upto == 0 {
                                        return 0.0;
                                    }
                                    let prefix = &run0.text[..upto];
                                    let trun = TextRun {
                                        len: prefix.len(),
                                        font: run_font(run0),
                                        color: hsla_of(run0.color),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    let shaped = window.text_system().shape_line(
                                        SharedString::from(prefix.to_string()),
                                        font_size,
                                        &[trun],
                                        None,
                                    );
                                    f32::from(shaped.width)
                                };
                                let x0 = x_at(sel.start, window);
                                let x1 = x_at(sel.end, window);
                                let top = o.y + px(tb.bounds.y);
                                // Selection highlight when the range is non-empty.
                                if (x1 - x0).abs() > 0.5 {
                                    window.paint_quad(quad(
                                        Bounds {
                                            origin: point(o.x + px(tb.bounds.x + x0.min(x1)), top),
                                            size: size(px((x1 - x0).abs()), line_height),
                                        },
                                        px(0.),
                                        Background::from(gpui::rgba(0x1166DD55)),
                                        px(0.),
                                        gpui::transparent_black(),
                                        Default::default(),
                                    ));
                                }
                                // Caret bar at the selection end (the insertion point).
                                window.paint_quad(quad(
                                    Bounds {
                                        origin: point(o.x + px(tb.bounds.x + x1), top),
                                        size: size(px(2.0), line_height),
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
                let sel_ents: Vec<_> = selection.into_iter().chain(also.iter().copied()).collect();
                if sel_ents.len() > 1 {
                    let mut union: Option<(f32, f32, f32, f32)> = None;
                    for e in &sel_ents {
                        if let Some(n) = scene.nodes.iter().find(|n| n.source == Some(*e)) {
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

                // Resize handles on the selection.
                if let Some(sel) = selection {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
                        let r = prim_bounds(&node.prim);
                        for (hx, hy) in resize_handles(r, node.rotation_deg) {
                            let b = Bounds {
                                origin: point(o.x + px(hx - 4.0), o.y + px(hy - 4.0)),
                                size: size(px(8.0), px(8.0)),
                            };
                            window.paint_quad(quad(
                                b,
                                px(1.0),
                                Background::from(rgb(0xffffff)),
                                px(1.0),
                                rgb(SELECTION),
                                Default::default(),
                            ));
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
        slide_list = slide_list.child(tool_button("add_slide", "+ Slide", cx, |this, _w, cx| {
            this.add_slide();
            cx.notify();
        }));
        for (i, &s) in slides.iter().enumerate() {
            let tscene = build_slide_scene(&self.pres, s, PxSize { w: 176.0, h: 99.0 });
            let tmedia = self.pres.media.clone();
            let is_cur = s == current;
            let tcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_scene(&tscene, b.origin, &tmedia, window, cx),
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
                        this.slide = s;
                        this.selection = None;
                        this.rebuild();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _w, cx| {
                            this.slide = s;
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

        // Tab row: Slides | Layers.
        let tab = |id: &'static str,
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
        let tab_row = div()
            .flex()
            .flex_row()
            .gap_1()
            .child(tab(
                "tab_slides",
                "Slides",
                LeftTab::Slides,
                left_tab == LeftTab::Slides,
                cx,
            ))
            .child(tab(
                "tab_layers",
                "Layers",
                LeftTab::Layers,
                left_tab == LeftTab::Layers,
                cx,
            ));
        let content: gpui::AnyElement = match left_tab {
            LeftTab::Slides => slide_list.into_any_element(),
            LeftTab::Layers => self.layers_panel(cx),
        };
        let sidebar = div()
            .flex()
            .flex_col()
            .gap_2()
            .w(px(208.))
            .h_full()
            .p_2()
            .bg(rgb(0x252525))
            .child(tab_row)
            .child(
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
        let theme = self.pres.theme_of(self.slide).cloned().unwrap_or_default();
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
                .child(label("Arrange"))
                .child(
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
                    }));
            }
            pane
        });

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().text_xl().child(title))
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
                            }))
                            .child(icon_button("min", "minus", cx, |_this, window, _cx| {
                                window.minimize_window();
                            }))
                            .child(icon_button("max", "square", cx, |_this, window, _cx| {
                                window.zoom_window();
                            }))
                            .child(icon_button("close", "x", cx, |_this, window, _cx| {
                                window.remove_window();
                            })),
                    ),
            )
            .children(palette_panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .gap_3()
                    .child(sidebar)
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
                                    .w(px(sw))
                                    .h(px(sh))
                                    .border_1()
                                    .border_color(rgb(0x555555))
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
            .into_any_element()
    }
}

impl HayateApp {
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
                        this.run_on_selection_with(
                            "shape.set_font",
                            serde_json::json!({ "family": fam.clone() }),
                        );
                        this.font_picker = false;
                        cx.notify();
                    })),
            );
        }
        Some(list.into_any_element())
    }
}
