//! Right-click context menu: opening it, handling the right mouse press, and building the
//! floating overlay element.

use gpui::{div, prelude::*, px, rgb, ClickEvent, Context, MouseDownEvent};

use hayate_render::hit_test;

use crate::widgets::{menu_divider, menu_item};
use crate::{ContextMenu, HayateApp, MenuTarget};

impl HayateApp {
    /// Open the right-click context menu at window position (x,y) for `target`.
    pub(crate) fn open_menu(&mut self, x: f32, y: f32, target: MenuTarget) {
        self.context_menu = Some(ContextMenu { x, y, target });
    }

    /// Right mouse press: select what is under the cursor (if any) and open a context menu.
    pub(crate) fn on_right_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let o = self.canvas_origin.get();
        let x = f32::from(ev.position.x - o.x);
        let y = f32::from(ev.position.y - o.y);
        let target = if let Some(e) = hit_test(&self.scene, x, y) {
            // Keep an existing multi-selection if right-clicking within it (so Group works);
            // otherwise select the shape (expanding to its group).
            if !self.selected_all().contains(&e) {
                self.selection = Some(e);
                let members = hayate_model::edit::group_members(&self.pres.world, e);
                self.also = members.into_iter().filter(|&m| m != e).collect();
            }
            MenuTarget::Shape
        } else {
            MenuTarget::Canvas
        };
        self.open_menu(f32::from(ev.position.x), f32::from(ev.position.y), target);
        cx.notify();
    }

    /// Build the floating context-menu overlay (PowerPoint-style), if one is open.
    pub(crate) fn menu_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let cm = self.context_menu.as_ref()?;
        let mut menu = div()
            .absolute()
            .left(px(cm.x))
            .top(px(cm.y))
            .flex()
            .flex_col()
            .min_w(px(190.))
            .py_1()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .rounded_md()
            .shadow_lg()
            .text_color(rgb(0xffffff));

        match cm.target {
            MenuTarget::Shape => {
                // Text-formatting items only make sense when the shape carries text.
                let has_text = self
                    .selection
                    .map_or(false, |e| self.pres.world.texts.contains_key(&e));
                menu = menu
                    .child(menu_item("m_edit_text", "Edit Text", cx, |t, _w, cx| {
                        if let Some(e) = t.selection {
                            t.begin_text_edit(e);
                            cx.notify();
                        }
                    }))
                    .child(menu_item("m_cut", "Cut", cx, |t, _w, cx| {
                        t.copy_selection();
                        t.delete_selection();
                        cx.notify();
                    }))
                    .child(menu_item("m_copy", "Copy", cx, |t, _w, _cx| {
                        t.copy_selection();
                    }))
                    .child(menu_item("m_dup", "Duplicate", cx, |t, _w, cx| {
                        t.duplicate_selection();
                        cx.notify();
                    }));
                if has_text {
                    menu = menu
                        .child(menu_divider())
                        .child(menu_item("m_bold", "Bold", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_bold");
                            cx.notify();
                        }))
                        .child(menu_item("m_italic", "Italic", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_italic");
                            cx.notify();
                        }))
                        .child(menu_item("m_underline", "Underline", cx, |t, _w, cx| {
                            t.run_on_selection("shape.toggle_underline");
                            cx.notify();
                        }))
                        .child(menu_item("m_align_l", "Align Left", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_left");
                            cx.notify();
                        }))
                        .child(menu_item("m_align_c", "Align Center", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_center");
                            cx.notify();
                        }))
                        .child(menu_item("m_align_r", "Align Right", cx, |t, _w, cx| {
                            t.run_on_selection("shape.align_text_right");
                            cx.notify();
                        }));
                }
                menu = menu
                    .child(menu_divider())
                    .child(menu_item("m_group", "Group", cx, |t, _w, cx| {
                        t.group_selection();
                        cx.notify();
                    }))
                    .child(menu_item("m_ungroup", "Ungroup", cx, |t, _w, cx| {
                        t.ungroup_selection();
                        cx.notify();
                    }))
                    .child(menu_divider())
                    .child(menu_item("m_gradient", "Gradient Fill", cx, |t, _w, cx| {
                        t.run_on_selection_with(
                            "shape.fill_gradient",
                            serde_json::json!({ "from": "#4F86C6", "to": "#E91E63", "angle": 0.0 }),
                        );
                        cx.notify();
                    }))
                    .child(menu_divider())
                    .child(menu_item("m_front", "Bring to Front", cx, |t, _w, cx| {
                        t.run_on_selection("shape.bring_to_front");
                        cx.notify();
                    }))
                    .child(menu_item("m_back", "Send to Back", cx, |t, _w, cx| {
                        t.run_on_selection("shape.send_to_back");
                        cx.notify();
                    }))
                    .child(menu_item(
                        "m_anim_fade",
                        "Animate: Fade In",
                        cx,
                        |t, _w, cx| {
                            t.add_fade_in();
                            cx.notify();
                        },
                    ))
                    .child(menu_divider());
                if self.selection_is_slide_placeholder() {
                    menu = menu.child(menu_item(
                        "m_reset_ph",
                        "Reset to Layout",
                        cx,
                        |t, _w, cx| {
                            t.reset_selected_placeholder();
                            cx.notify();
                        },
                    ));
                }
                menu = menu.child(menu_item("m_delete", "Delete", cx, |t, _w, cx| {
                    t.delete_selection();
                    cx.notify();
                }));
            }
            MenuTarget::Slide => {
                menu = menu
                    .child(menu_item("m_new_slide", "New Slide", cx, |t, _w, cx| {
                        t.add_slide();
                        cx.notify();
                    }))
                    .child(menu_item(
                        "m_dup_slide",
                        "Duplicate Slide",
                        cx,
                        |t, _w, cx| {
                            t.duplicate_slide();
                            cx.notify();
                        },
                    ))
                    .child(menu_divider())
                    .child(menu_item("m_del_slide", "Delete Slide", cx, |t, _w, cx| {
                        t.delete_slide();
                        cx.notify();
                    }));
            }
            MenuTarget::Canvas => {
                menu = menu
                    .child(menu_item("m_paste", "Paste", cx, |t, _w, cx| {
                        t.paste_clipboard();
                        cx.notify();
                    }))
                    .child(menu_divider())
                    .child(menu_item("m_add_rect", "Add Rectangle", cx, |t, _w, cx| {
                        t.add_rect();
                        cx.notify();
                    }))
                    .child(menu_item("m_add_text", "Add Text Box", cx, |t, _w, cx| {
                        t.add_text_box();
                        cx.notify();
                    }))
                    .child(menu_item("m_add_image", "Insert Image", cx, |t, _w, cx| {
                        t.insert_image(cx);
                        cx.notify();
                    }))
                    .child(menu_divider())
                    .child(menu_item("m_new_slide_c", "New Slide", cx, |t, _w, cx| {
                        t.add_slide();
                        cx.notify();
                    }));
            }
            MenuTarget::Layout(layout) => {
                menu = menu
                    .child(menu_item(
                        "m_layout_edit",
                        "Edit Layout",
                        cx,
                        move |t, _w, cx| {
                            t.enter_layout_scope(layout);
                            cx.notify();
                        },
                    ))
                    .child(menu_item(
                        "m_layout_rename",
                        "Rename",
                        cx,
                        move |t, _w, cx| {
                            let name = t
                                .pres
                                .world
                                .layout_info
                                .get(&layout)
                                .map(|li| li.name.clone())
                                .unwrap_or_default();
                            t.layout_rename = Some((layout, name));
                            cx.notify();
                        },
                    ))
                    .child(menu_item(
                        "m_layout_dup",
                        "Duplicate",
                        cx,
                        move |t, _w, cx| {
                            t.duplicate_layout(layout);
                            cx.notify();
                        },
                    ))
                    .child(menu_divider())
                    .child(menu_item("m_layout_del", "Delete", cx, move |t, _w, cx| {
                        t.delete_layout(layout);
                        cx.notify();
                    }));
            }
        }
        // A full-window backdrop captures clicks outside the menu to dismiss it. The menu sits
        // on top, so clicks on its items reach the items (topmost hitbox wins); clicks elsewhere
        // hit the backdrop and close the menu.
        let backdrop = div()
            .id("menu_backdrop")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
                this.context_menu = None;
                cx.notify();
            }))
            .child(menu);
        Some(backdrop.into_any_element())
    }
}
