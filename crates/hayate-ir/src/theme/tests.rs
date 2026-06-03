//! Unit tests for the parent module.

use super::*;
use crate::color::ColorXf;

#[test]
fn resolve_literal() {
    let t = Theme::default();
    assert_eq!(
        t.resolve_color(&Color::literal(Rgba::rgb(1, 2, 3))),
        Rgba::rgb(1, 2, 3)
    );
}

#[test]
fn resolve_token_with_transform() {
    let t = Theme::default();
    // accent1 darkened 50% via shade.
    let c = Color::Theme {
        token: ThemeColorToken::Accent1,
        xf: ColorXf {
            shade: Some(0.5),
            ..Default::default()
        },
    };
    let base = t.color_for(ThemeColorToken::Accent1);
    let expect = ColorXf {
        shade: Some(0.5),
        ..Default::default()
    }
    .apply(base);
    assert_eq!(t.resolve_color(&c), expect);
}

#[test]
fn font_picks_script_slot() {
    let t = Theme::default();
    let body = FontRef::Theme(ThemeFontSlot::Minor);
    assert_eq!(t.font_family(&body, Script::Latin), "DejaVu Sans");
    assert_eq!(t.font_family(&body, Script::Ea), "Noto Sans CJK JP");
    assert_eq!(
        t.font_family(&FontRef::Family("Mincho".into()), Script::Latin),
        "Mincho"
    );
}
