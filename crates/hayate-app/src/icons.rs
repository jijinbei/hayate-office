//! Monochrome line icons (Lucide, ISC-licensed) embedded as SVG strings and served through a
//! gpui `AssetSource`, plus an `icon_button` widget. gpui tints the SVG with the element's
//! text color, so icons stay crisp at any zoom and match the dark UI.

use std::borrow::Cow;

use gpui::{
    div, prelude::*, px, rgb, svg, AssetSource, ClickEvent, Context, Result, SharedString, Window,
};

use crate::HayateApp;

/// Wrap Lucide path data in the standard 24x24 stroke SVG header.
macro_rules! lucide {
    ($body:expr) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
            $body,
            "</svg>"
        )
    };
}

const MINUS: &str = lucide!(r#"<path d="M5 12h14"/>"#);
const PLUS: &str = lucide!(r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#);
const MAXIMIZE: &str = lucide!(
    r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#
);
const X: &str = lucide!(r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#);
const SQUARE: &str = lucide!(r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#);
const BOLD: &str =
    lucide!(r#"<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/>"#);
const ITALIC: &str = lucide!(
    r#"<line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/>"#
);
const UNDERLINE: &str =
    lucide!(r#"<path d="M6 4v6a6 6 0 0 0 12 0V4"/><line x1="4" x2="20" y1="20" y2="20"/>"#);
const BRING_FRONT: &str =
    lucide!(r#"<path d="M5 3h14"/><path d="m18 13-6-6-6 6"/><path d="M12 7v14"/>"#);
const SEND_BACK: &str =
    lucide!(r#"<path d="M12 17V3"/><path d="m6 11 6 6 6-6"/><path d="M19 21H5"/>"#);
const TYPE: &str = lucide!(
    r#"<path d="M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2"/><path d="M9 20h6"/><path d="M12 4v16"/>"#
);
const ALIGN_LEFT: &str = lucide!(
    r#"<line x1="21" x2="3" y1="6" y2="6"/><line x1="15" x2="3" y1="12" y2="12"/><line x1="17" x2="3" y1="18" y2="18"/>"#
);
const ALIGN_CENTER: &str = lucide!(
    r#"<line x1="21" x2="3" y1="6" y2="6"/><line x1="17" x2="7" y1="12" y2="12"/><line x1="19" x2="5" y1="18" y2="18"/>"#
);
const ALIGN_RIGHT: &str = lucide!(
    r#"<line x1="21" x2="3" y1="6" y2="6"/><line x1="21" x2="9" y1="12" y2="12"/><line x1="21" x2="7" y1="18" y2="18"/>"#
);
const CIRCLE: &str = lucide!(r#"<circle cx="12" cy="12" r="10"/>"#);
const LINE: &str = lucide!(r#"<line x1="5" y1="19" x2="19" y2="5"/>"#);
const ARROW: &str = lucide!(r#"<path d="M7 7h10v10"/><path d="M7 17 17 7"/>"#);
const IMAGE: &str = lucide!(
    r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/>"#
);

/// Map an icon name (the `icons/<name>.svg` path) to its embedded SVG source.
fn svg_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "minus" => MINUS,
        "plus" => PLUS,
        "maximize" => MAXIMIZE,
        "x" => X,
        "square" => SQUARE,
        "bold" => BOLD,
        "italic" => ITALIC,
        "underline" => UNDERLINE,
        "bring-front" => BRING_FRONT,
        "send-back" => SEND_BACK,
        "type" => TYPE,
        "align-left" => ALIGN_LEFT,
        "align-center" => ALIGN_CENTER,
        "align-right" => ALIGN_RIGHT,
        "circle" => CIRCLE,
        "line" => LINE,
        "arrow" => ARROW,
        "image" => IMAGE,
        _ => return None,
    })
}

/// Serves the embedded icon SVGs to gpui's `svg()` element via `icons/<name>.svg` paths.
pub(crate) struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
            .unwrap_or(path);
        Ok(svg_for(name).map(|s| Cow::Borrowed(s.as_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

/// A small icon-only toolbar button. `name` is an embedded icon (see `svg_for`). Keeps the
/// editor focused on click so keyboard shortcuts keep working.
pub(crate) fn icon_button(
    id: &'static str,
    name: &'static str,
    cx: &mut Context<HayateApp>,
    action: impl Fn(&mut HayateApp, &mut Window, &mut Context<HayateApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded_md()
        .bg(rgb(0x3a3a3a))
        .hover(|s| s.bg(rgb(0x4a4a4a)))
        .child(
            svg()
                .path(format!("icons/{name}.svg"))
                .size(px(16.))
                .text_color(rgb(0xeaeaea)),
        )
        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            action(this, window, cx);
        }))
}
