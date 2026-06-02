//! Small reusable UI widgets: toolbar buttons and context-menu rows/dividers.

use gpui::{div, prelude::*, px, rgb, ClickEvent, Context, SharedString, Window};

use crate::HayateApp;

/// A small clickable toolbar button.
pub(crate) fn tool_button(
    id: &'static str,
    label: impl Into<SharedString>,
    cx: &mut Context<HayateApp>,
    action: impl Fn(&mut HayateApp, &mut Window, &mut Context<HayateApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .bg(rgb(0x3a3a3a))
        .rounded_md()
        .hover(|s| s.bg(rgb(0x4a4a4a)))
        .child(label.into())
        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| action(this, window, cx)))
}

/// A thin horizontal separator between context-menu groups.
pub(crate) fn menu_divider() -> impl IntoElement {
    div().my_1().h(px(1.)).bg(rgb(0x555555))
}

/// A single PowerPoint-style context-menu row. Runs `action`, then closes the menu.
pub(crate) fn menu_item(
    id: &'static str,
    label: impl Into<SharedString>,
    cx: &mut Context<HayateApp>,
    action: impl Fn(&mut HayateApp, &mut Window, &mut Context<HayateApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .text_sm()
        .hover(|s| s.bg(rgb(0x094771)))
        .child(label.into())
        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
            action(this, window, cx);
            this.context_menu = None;
            cx.notify();
        }))
}
