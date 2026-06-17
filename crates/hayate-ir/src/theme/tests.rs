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
    // Use distinct families per script so the slot-selection mechanism is exercised
    // independently of the default theme's font choice.
    let mut t = Theme::default();
    t.fonts.minor = ScriptFonts {
        latin: "LatinFace".into(),
        ea: "EaFace".into(),
        cs: "CsFace".into(),
    };
    let body = FontRef::Theme(ThemeFontSlot::Minor);
    assert_eq!(t.font_family(&body, Script::Latin), "LatinFace");
    assert_eq!(t.font_family(&body, Script::Ea), "EaFace");
    assert_eq!(t.font_family(&body, Script::Cs), "CsFace");
    assert_eq!(
        t.font_family(&FontRef::Family("Mincho".into()), Script::Latin),
        "Mincho"
    );
}

#[test]
fn default_uses_one_family_for_all_scripts() {
    // The default theme uses a single family across scripts so list bullets stay a consistent
    // size regardless of whether a line contains CJK.
    let t = Theme::default();
    let body = FontRef::Theme(ThemeFontSlot::Minor);
    let fam = super::default_sans_family();
    assert_eq!(t.font_family(&body, Script::Latin), fam);
    assert_eq!(t.font_family(&body, Script::Ea), fam);
}
