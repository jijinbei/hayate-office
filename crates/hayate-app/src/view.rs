//! The main `Render` implementation: presentation mode, editing canvas (with caret, grid,
//! selection outlines, resize handles, alignment guides), slide sidebar, and the Format pane.

use gpui::{
    canvas, div, point, prelude::*, px, quad, rgb, size, Background, Bounds, ClickEvent, Context,
    ElementInputHandler, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Render, SharedString, TextRun, Window,
};

use hayate_ir::color::ThemeColorToken;
use hayate_render::scene::{Primitive, PxSize};
use hayate_render::{build_slide_scene, build_slide_scene_at, grid_lines, resize_handles, GuideKind};

use crate::paint::paint_scene;
use crate::util::{hsla_of, prim_bounds, rotate_pt, run_font};
use crate::widgets::tool_button;
use crate::{DraggedSlide, FieldKind, HayateApp, MenuTarget, SlideDragPreview, SELECTION};

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

        // Document coordinates are absolute (points); on-screen size = slide_pt * zoom.
        // Window resizing does not change zoom (use the zoom controls / Fit).
        let scene = self.scene.clone();
        let media = self.pres.media.clone();
        let selection = self.selection;
        let also = self.also.clone();
        let guides = self.guides.clone();
        let show_grid = self.show_grid;
        // Caret (editing entity + byte offset) for drawing the text-insertion cursor.
        let caret = self
            .text_edit
            .as_ref()
            .map(|te| (te.entity, te.selected.start));
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

                // Text-edit caret: a thin vertical bar at the insertion point.
                if let Some((ent, caret_byte)) = caret {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(ent)) {
                        if let Primitive::Text(tb) = &node.prim {
                            if let Some((para, run0)) = tb
                                .paragraphs
                                .first()
                                .and_then(|p| p.runs.first().map(|r| (p, r)))
                            {
                                let font_size =
                                    px(para.runs.iter().map(|r| r.size_px).fold(0.0, f32::max));
                                let line_height = font_size * 1.3;
                                let upto = caret_byte.min(run0.text.len());
                                let prefix = &run0.text[..upto];
                                let caret_x = if prefix.is_empty() {
                                    0.0
                                } else {
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
                                let left = o.x + px(tb.bounds.x + caret_x);
                                let top = o.y + px(tb.bounds.y);
                                window.paint_quad(quad(
                                    Bounds {
                                        origin: point(left, top),
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
            },
        )
        .size_full();

        // Slide-list sidebar: a clickable thumbnail per slide + an "add slide" button.
        let slides = self.pres.slides();
        let current = self.slide;
        let mut sidebar = div()
            .flex()
            .flex_col()
            .gap_2()
            .w(px(208.))
            .p_2()
            .bg(rgb(0x252525));
        sidebar = sidebar.child(tool_button("add_slide", "+ Slide", cx, |this, _w, cx| {
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
            sidebar = sidebar.child(
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
        let inspector = self.selection.map(|_e| {
            let mut swatches = div().flex().flex_row().gap_1();
            for (i, t) in accents.into_iter().enumerate() {
                let cu = crate::util::rgb_u32(theme.color_for(t));
                swatches = swatches.child(
                    div()
                        .id(("acc", i))
                        .w(px(28.))
                        .h(px(28.))
                        .rounded_md()
                        .bg(rgb(cu))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                            this.set_fill_accent(t);
                            cx.notify();
                        })),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap_2()
                .w(px(228.))
                .p_2()
                .bg(rgb(0x252525))
                .child(div().text_xl().child("Format"))
                .child(div().child("Rotation (click to type 0-360)"))
                .child(self.num_field("f_rot", FieldKind::Rotation, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("rot_m", "\u{27F2}-15", cx, |t, _w, cx| {
                            t.rotate_by(-15.0);
                            cx.notify();
                        }))
                        .child(tool_button("rot_p", "\u{27F3}+15", cx, |t, _w, cx| {
                            t.rotate_by(15.0);
                            cx.notify();
                        }))
                        .child(tool_button("rot_0", "0\u{00B0}", cx, |t, _w, cx| {
                            t.set_rotation_abs(0.0);
                            cx.notify();
                        })),
                )
                .child(div().child("Position (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(self.num_field("f_x", FieldKind::PosX, cx))
                        .child(self.num_field("f_y", FieldKind::PosY, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("x_m", "X-", cx, |t, _w, cx| {
                            t.nudge(-91_440, 0);
                            cx.notify();
                        }))
                        .child(tool_button("x_p", "X+", cx, |t, _w, cx| {
                            t.nudge(91_440, 0);
                            cx.notify();
                        }))
                        .child(tool_button("y_m", "Y-", cx, |t, _w, cx| {
                            t.nudge(0, -91_440);
                            cx.notify();
                        }))
                        .child(tool_button("y_p", "Y+", cx, |t, _w, cx| {
                            t.nudge(0, 91_440);
                            cx.notify();
                        })),
                )
                .child(div().child("Size (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(self.num_field("f_w", FieldKind::SizeW, cx))
                        .child(self.num_field("f_h", FieldKind::SizeH, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("w_m", "W-", cx, |t, _w, cx| {
                            t.resize_by(-182_880, 0);
                            cx.notify();
                        }))
                        .child(tool_button("w_p", "W+", cx, |t, _w, cx| {
                            t.resize_by(182_880, 0);
                            cx.notify();
                        }))
                        .child(tool_button("h_m", "H-", cx, |t, _w, cx| {
                            t.resize_by(0, -182_880);
                            cx.notify();
                        }))
                        .child(tool_button("h_p", "H+", cx, |t, _w, cx| {
                            t.resize_by(0, 182_880);
                            cx.notify();
                        })),
                )
                .child(div().child("Fill"))
                .child(swatches)
                .child(div().child("Opacity (%)"))
                .child(self.num_field("f_op", FieldKind::Opacity, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("front", "Front", cx, |t, _w, cx| {
                            t.run_on_selection("shape.bring_to_front");
                            cx.notify();
                        }))
                        .child(tool_button("back", "Back", cx, |t, _w, cx| {
                            t.run_on_selection("shape.send_to_back");
                            cx.notify();
                        })),
                )
                .child(div().child("Align (Shift-click for multi)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
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
                )
                .child(div().child("Text"))
                .child(
                    // Character styling. These build registry commands by string id; they
                    // simply no-op until the text-formatting commands are registered.
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(tool_button("txt_bold", "B", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_bold");
                            cx.notify();
                        }))
                        .child(tool_button("txt_italic", "I", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_italic");
                            cx.notify();
                        }))
                        .child(tool_button("txt_underline", "U", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_underline");
                            cx.notify();
                        }))
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
                        .child(tool_button("txt_al_l", "Left", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_left");
                            cx.notify();
                        }))
                        .child(tool_button("txt_al_c", "Center", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_center");
                            cx.notify();
                        }))
                        .child(tool_button("txt_al_r", "Right", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_right");
                            cx.notify();
                        })),
                )
                .child(tool_button(
                    "anim_fade",
                    "Animate: Fade In",
                    cx,
                    |t, _w, cx| {
                        t.add_fade_in();
                        cx.notify();
                    },
                ))
                .child(tool_button(
                    "edit_text",
                    "Edit Text (F2)",
                    cx,
                    |t, _w, cx| {
                        if let Some(e) = t.selection {
                            t.begin_text_edit(e);
                        }
                        cx.notify();
                    },
                ))
                .child(tool_button("del", "Delete", cx, |t, _w, cx| {
                    t.delete_selection();
                    t.rebuild();
                    cx.notify();
                }))
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
                            .child(tool_button("zoom_out", "\u{2212}", cx, |t, _w, cx| {
                                let z = t.zoom / 1.25;
                                t.set_zoom(z, cx);
                            }))
                            .child(div().child(format!("{}%", (self.zoom * 100.0).round() as i32)))
                            .child(tool_button("zoom_in", "+", cx, |t, _w, cx| {
                                let z = t.zoom * 1.25;
                                t.set_zoom(z, cx);
                            }))
                            .child(tool_button("zoom_fit", "Fit", cx, |t, w, cx| {
                                t.fit_zoom(w);
                                t.rebuild();
                                cx.notify();
                            }))
                            .child(tool_button("min", "\u{2013}", cx, |_this, window, _cx| {
                                window.minimize_window();
                            }))
                            .child(tool_button("max", "\u{25A1}", cx, |_this, window, _cx| {
                                window.zoom_window();
                            }))
                            .child(tool_button(
                                "close",
                                "\u{00D7}",
                                cx,
                                |_this, window, _cx| {
                                    window.remove_window();
                                },
                            )),
                    ),
            )
            .children(palette_panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(sidebar)
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
                            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                                this.on_mouse_move(ev, cx)
                            }))
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
                    )
                    .children(inspector),
            )
            .children(self.menu_overlay(cx))
            .into_any_element()
    }
}
