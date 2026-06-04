//! Ergonomic editing helpers (DESIGN 6.10). Each helper builds a labelled `Transaction`
//! out of the uniform four-kind `Operation`, so callers express intent ("create a rect",
//! "translate") without hand-assembling `SetComponent`/`Spawn` vocabulary, while undo,
//! serialization and (later) CRDT keep operating on that closed vocabulary.

use crate::history::Transaction;
use crate::op::Operation;
use hayate_ir::anim::{Anim, AnimKind, AnimStep, Easing, Effect, SlideTimeline, Trigger};
use hayate_ir::color::Color;
use hayate_ir::color::ThemeColorToken;
use hayate_ir::doc::{PlaceholderRef, SlideInfo};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::{Fill, Stroke};
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::{ArrowHead, Geometry};
use hayate_ir::text::{HAlign, Paragraph, Run, TextBody};
use hayate_ir::units::pt;
use hayate_ir::world::{CompKind, CompValue, Entity, World};

/// A minimal default run carrying `text`: body theme font, 18pt, Dk1 theme color, no
/// bold/italic/underline. Used when a text helper must synthesize a run because none exists.
fn default_run(text: String) -> Run {
    Run {
        text,
        font: FontRef::Theme(ThemeFontSlot::Minor),
        size: pt(18),
        color: Color::theme(ThemeColorToken::Dk1),
        bold: false,
        italic: false,
        underline: false,
    }
}

/// Set (insert or replace) the `Frame` of `e`.
pub fn set_frame(e: Entity, frame: RectEmu) -> Transaction {
    Transaction::new(
        "set frame",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Frame(frame),
        }],
    )
}

/// Set (insert or replace) the `Fill` of `e`.
pub fn set_fill(e: Entity, fill: Fill) -> Transaction {
    Transaction::new(
        "set fill",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Fill(fill),
        }],
    )
}

/// Shift `e`'s `Frame` origin by `(dx, dy)`, keeping its size. Reads the current frame from
/// `world`; if `e` has no `Frame`, returns an empty transaction (nothing to move).
pub fn translate(world: &World, e: Entity, dx: i64, dy: i64) -> Transaction {
    match world.get(e, CompKind::Frame) {
        Some(CompValue::Frame(mut frame)) => {
            frame.origin.x += dx;
            frame.origin.y += dy;
            Transaction::new(
                "translate",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                }],
            )
        }
        // No frame (or some other component kind): nothing to translate.
        _ => Transaction::new("translate", vec![]),
    }
}

/// Create a rectangle shape on a reserved id: spawn it, then attach parent, sibling order,
/// frame, rectangular geometry and fill. `reserved` should come from [`World::reserve_id`]
/// so the spawn is fully captured by this (undoable) transaction.
pub fn create_rect(
    reserved: Entity,
    parent: Entity,
    order: FracIndex,
    frame: RectEmu,
    fill: Fill,
) -> Transaction {
    Transaction::new(
        "create rect",
        vec![
            Operation::Spawn { entity: reserved },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Parent(parent),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Order(order),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Frame(frame),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Geometry(Geometry::Rect),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Fill(fill),
            },
        ],
    )
}

/// Create a line (or arrow) shape on a reserved id: spawn it, then attach parent, sibling
/// order, frame, line geometry and a default stroke. The line is drawn along the frame's
/// diagonal from the START point (top-left) to the END point (bottom-right); each end carries
/// an independent [`ArrowHead`] (an arrowhead is drawn where it is `ArrowHead::Arrow`). Unlike
/// a rect, a line carries a `Stroke` (a dark 2pt outline) rather than a fill. `reserved` should
/// come from [`World::reserve_id`] so the spawn is fully captured by this (undoable) transaction.
pub fn create_line(
    reserved: Entity,
    parent: Entity,
    order: FracIndex,
    frame: RectEmu,
    start: ArrowHead,
    end: ArrowHead,
) -> Transaction {
    Transaction::new(
        "create line",
        vec![
            Operation::Spawn { entity: reserved },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Parent(parent),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Order(order),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Frame(frame),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Geometry(Geometry::Line { start, end }),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Stroke(Stroke::solid(Color::theme(ThemeColorToken::Dk1), pt(2))),
            },
        ],
    )
}

/// Create a placeholder shape on a reserved id: spawn it, then attach parent, sibling
/// order, frame and its `PlaceholderRef`, plus a `Text` body when `text` is provided. This
/// is the editable, slide/layout/master-level definition of a placeholder; resolution of
/// inherited fields is done by `Presentation::ph_*`. `reserved` should come from
/// [`World::reserve_id`] so the spawn is fully captured by this (undoable) transaction.
pub fn create_placeholder(
    reserved: Entity,
    parent: Entity,
    order: FracIndex,
    ph: PlaceholderRef,
    frame: RectEmu,
    text: Option<TextBody>,
) -> Transaction {
    let mut ops = vec![
        Operation::Spawn { entity: reserved },
        Operation::SetComponent {
            entity: reserved,
            value: CompValue::Parent(parent),
        },
        Operation::SetComponent {
            entity: reserved,
            value: CompValue::Order(order),
        },
        Operation::SetComponent {
            entity: reserved,
            value: CompValue::Frame(frame),
        },
        Operation::SetComponent {
            entity: reserved,
            value: CompValue::Placeholder(ph),
        },
    ];
    if let Some(body) = text {
        ops.push(Operation::SetComponent {
            entity: reserved,
            value: CompValue::Text(body),
        });
    }
    Transaction::new("create placeholder", ops)
}

/// A standard slide layout, mirroring the common PowerPoint/OnlyOffice set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPreset {
    TitleSlide,
    TitleAndContent,
    SectionHeader,
    TwoContent,
    TitleOnly,
    Blank,
}

/// One placeholder a preset defines: where it sits and how its prompt text looks.
#[derive(Clone, Copy, Debug)]
pub struct PlaceholderSpec {
    pub ph: PlaceholderRef,
    pub frame: RectEmu,
    pub label: &'static str,
    pub slot: ThemeFontSlot,
    pub size_pt: i64,
    pub bold: bool,
}

impl LayoutPreset {
    /// A human-friendly default name for the preset.
    pub fn name(self) -> &'static str {
        match self {
            LayoutPreset::TitleSlide => "Title Slide",
            LayoutPreset::TitleAndContent => "Title and Content",
            LayoutPreset::SectionHeader => "Section Header",
            LayoutPreset::TwoContent => "Two Content",
            LayoutPreset::TitleOnly => "Title Only",
            LayoutPreset::Blank => "Blank",
        }
    }
}

/// The placeholders a preset defines, with frames scaled to `slide_size` so presets adapt to
/// any aspect ratio. Frames use fractions of the slide so 4:3 and 16:9 both look reasonable.
pub fn preset_placeholders(
    preset: LayoutPreset,
    slide_size: hayate_ir::geom::SizeEmu,
) -> Vec<PlaceholderSpec> {
    use hayate_ir::doc::PlaceholderType as PT;
    let (w, h) = (slide_size.w as f64, slide_size.h as f64);
    // Rect from fractional (x, y, w, h) of the slide.
    let r = |fx: f64, fy: f64, fw: f64, fh: f64| {
        RectEmu::new(
            (w * fx) as i64,
            (h * fy) as i64,
            (w * fw) as i64,
            (h * fh) as i64,
        )
    };
    let title = |idx, frame, label, size, bold| PlaceholderSpec {
        ph: PlaceholderRef {
            ph_type: PT::Title,
            idx,
        },
        frame,
        label,
        slot: ThemeFontSlot::Major,
        size_pt: size,
        bold,
    };
    let ctitle = |frame, label, size| PlaceholderSpec {
        ph: PlaceholderRef {
            ph_type: PT::CenteredTitle,
            idx: 0,
        },
        frame,
        label,
        slot: ThemeFontSlot::Major,
        size_pt: size,
        bold: true,
    };
    let sub = |frame, label| PlaceholderSpec {
        ph: PlaceholderRef {
            ph_type: PT::Subtitle,
            idx: 0,
        },
        frame,
        label,
        slot: ThemeFontSlot::Minor,
        size_pt: 24,
        bold: false,
    };
    let body = |idx, frame| PlaceholderSpec {
        ph: PlaceholderRef {
            ph_type: PT::Body,
            idx,
        },
        frame,
        label: "Click to add text",
        slot: ThemeFontSlot::Minor,
        size_pt: 24,
        bold: false,
    };
    match preset {
        LayoutPreset::TitleSlide => vec![
            ctitle(r(0.08, 0.35, 0.84, 0.18), "Click to add title", 44),
            sub(r(0.08, 0.56, 0.84, 0.12), "Click to add subtitle"),
        ],
        LayoutPreset::TitleAndContent => vec![
            title(0, r(0.05, 0.04, 0.90, 0.15), "Click to add title", 40, true),
            body(0, r(0.05, 0.22, 0.90, 0.72)),
        ],
        LayoutPreset::SectionHeader => vec![
            ctitle(r(0.08, 0.40, 0.84, 0.16), "Click to add section title", 40),
            sub(r(0.08, 0.58, 0.84, 0.10), "Click to add text"),
        ],
        LayoutPreset::TwoContent => vec![
            title(0, r(0.05, 0.04, 0.90, 0.15), "Click to add title", 40, true),
            body(0, r(0.05, 0.22, 0.43, 0.72)),
            body(1, r(0.52, 0.22, 0.43, 0.72)),
        ],
        LayoutPreset::TitleOnly => vec![title(
            0,
            r(0.05, 0.04, 0.90, 0.15),
            "Click to add title",
            40,
            true,
        )],
        LayoutPreset::Blank => Vec::new(),
    }
}

/// Replace a master's whole theme in one undoable op. `MasterInfo` is a single component, so the
/// `SetComponent` undo captures the prior theme exactly — no special history handling needed.
pub fn set_master_theme(master: Entity, theme: hayate_ir::theme::Theme) -> Transaction {
    Transaction::new(
        "set theme",
        vec![Operation::SetComponent {
            entity: master,
            value: CompValue::Master(hayate_ir::doc::MasterInfo { theme }),
        }],
    )
}

/// Rebase `slide` onto a different `layout` by setting its `SlideInfo`.
pub fn set_slide_layout(slide: Entity, layout: Entity) -> Transaction {
    Transaction::new(
        "set slide layout",
        vec![Operation::SetComponent {
            entity: slide,
            value: CompValue::Slide(SlideInfo { layout }),
        }],
    )
}

/// Materialize an editable slide-level override of an inherited placeholder: spawn a NEW
/// shape `reserved` parented to `slide` at `order`, carrying the same `PlaceholderRef` and
/// copying the inherited resolved frame (via [`Presentation::ph_frame`]) and text (via
/// [`Presentation::ph_text`]). This new shape overrides the inherited placeholder because it
/// lives most-derived (on the slide) in the resolution chain. Returns `None` when there is no
/// inherited frame to copy. `reserved` should come from [`World::reserve_id`].
pub fn promote_placeholder(
    p: &Presentation,
    slide: Entity,
    ph: PlaceholderRef,
    reserved: Entity,
    order: FracIndex,
) -> Option<Transaction> {
    let frame = p.ph_frame(slide, ph)?;
    let text = p.ph_text(slide, ph).cloned();
    Some(create_placeholder(reserved, slide, order, ph, frame, text))
}

/// EMU per inch (914400) times 0.2, the offset applied to a duplicate's frame so it does
/// not sit exactly on top of its source.
const DUPLICATE_OFFSET_EMU: i64 = 182880; // 0.2 inch

/// Resize `e`'s `Frame` to `(w, h)`, keeping its origin. Reads the current frame from
/// `world`; if `e` has no `Frame`, returns an empty transaction (nothing to resize).
pub fn resize(world: &World, e: Entity, w: i64, h: i64) -> Transaction {
    match world.get(e, CompKind::Frame) {
        Some(CompValue::Frame(mut frame)) => {
            frame.size.w = w;
            frame.size.h = h;
            Transaction::new(
                "resize",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                }],
            )
        }
        // No frame (or some other component kind): nothing to resize.
        _ => Transaction::new("resize", vec![]),
    }
}

/// Set (insert or replace) the `Rotation` of `e`, in degrees clockwise.
pub fn set_rotation(e: Entity, degrees: f32) -> Transaction {
    Transaction::new(
        "set rotation",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Rotation(degrees),
        }],
    )
}

/// Duplicate `src` onto the reserved id `new_id`: spawn the new entity, then copy every
/// component currently on `src`. If the duplicate has a `Frame`, its origin is offset by
/// (+0.2 inch, +0.2 inch) so the copy is visible rather than hidden behind the original.
/// `new_id` must come from [`World::reserve_id`] so the spawn is fully captured by this
/// (undoable) transaction.
pub fn duplicate(world: &World, src: Entity, new_id: Entity) -> Transaction {
    let mut ops = vec![Operation::Spawn { entity: new_id }];
    for comp in world.components_of(src) {
        let value = match comp {
            CompValue::Frame(mut frame) => {
                frame.origin.x += DUPLICATE_OFFSET_EMU;
                frame.origin.y += DUPLICATE_OFFSET_EMU;
                CompValue::Frame(frame)
            }
            other => other,
        };
        ops.push(Operation::SetComponent {
            entity: new_id,
            value,
        });
    }
    Transaction::new("duplicate", ops)
}

/// Set the text of the entity's first run, preserving that run's formatting. Reads the
/// current `TextBody` from `world`: if present, clones it and replaces the first paragraph's
/// first run's `text` (keeping its font/size/color/bold/...). If the body exists but has no
/// paragraphs/runs, a minimal default-run paragraph carrying `text` is inserted. If the
/// entity has no `TextBody` at all, a new one is created. Always emits `SetComponent(Text)`.
pub fn set_run_text(world: &World, e: Entity, text: String) -> Transaction {
    // The in-canvas editor treats a text box as a single string buffer, so the resulting body
    // is a single paragraph with a single run. Any extra runs/paragraphs are collapsed — this
    // also prevents stale leftover runs from rendering alongside the edited text. The first
    // run's character formatting and the first paragraph's alignment are preserved.
    let existing = world.texts.get(&e);
    let mut run = existing
        .and_then(|b| b.paragraphs.first())
        .and_then(|p| p.runs.first())
        .cloned()
        .unwrap_or_else(|| default_run(String::new()));
    run.text = text;
    let mut para = Paragraph::new(vec![run]);
    if let Some(first) = existing.and_then(|b| b.paragraphs.first()) {
        para.align = first.align;
    }
    let new_body = TextBody {
        paragraphs: vec![para],
        autofit: existing.map(|b| b.autofit).unwrap_or(false),
    };
    Transaction::new(
        "set run text",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(new_body),
        }],
    )
}

/// Replace the entity's text with one paragraph per `(text, bullet_level)` line, preserving the
/// first existing run's character formatting and the first paragraph's alignment. This is the
/// multi-paragraph counterpart of [`set_run_text`], used by the in-canvas list editor so each
/// line carries its own bullet level. An empty `lines` yields a single empty paragraph.
pub fn set_paragraphs(world: &World, e: Entity, lines: &[(String, u8)]) -> Transaction {
    let existing = world.texts.get(&e);
    let template = existing
        .and_then(|b| b.paragraphs.first())
        .and_then(|p| p.runs.first())
        .cloned()
        .unwrap_or_else(|| default_run(String::new()));
    let align = existing
        .and_then(|b| b.paragraphs.first())
        .map(|p| p.align)
        .unwrap_or(HAlign::Left);
    let mut paragraphs: Vec<Paragraph> = lines
        .iter()
        .map(|(text, level)| {
            let mut run = template.clone();
            run.text = text.clone();
            let mut para = Paragraph::new(vec![run]);
            para.align = align;
            para.bullet_level = *level;
            para
        })
        .collect();
    if paragraphs.is_empty() {
        let mut run = template.clone();
        run.text = String::new();
        let mut para = Paragraph::new(vec![run]);
        para.align = align;
        paragraphs.push(para);
    }
    let body = TextBody {
        paragraphs,
        autofit: existing.map(|b| b.autofit).unwrap_or(false),
    };
    Transaction::new(
        "set paragraphs",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Append a new paragraph (a single minimal default run carrying `text`) to the entity's
/// `TextBody`, creating an empty body first if the entity has none. Emits
/// `SetComponent(Text)`.
pub fn append_paragraph(world: &World, e: Entity, text: String) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => TextBody {
            paragraphs: Vec::new(),
            autofit: false,
        },
    };
    body.paragraphs
        .push(Paragraph::new(vec![default_run(text)]));
    Transaction::new(
        "append paragraph",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Set the horizontal alignment of *every* paragraph in the entity's `TextBody` to `align`.
/// Reads the current body from `world`; if the entity has no `TextBody`, returns an empty
/// transaction (nothing to align). Otherwise emits `SetComponent(Text)`.
pub fn set_paragraph_align(world: &World, e: Entity, align: HAlign) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => return Transaction::new("set paragraph align", vec![]),
    };
    for para in &mut body.paragraphs {
        para.align = align;
    }
    Transaction::new(
        "set paragraph align",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Toggle bullets across all paragraphs of the entity's `TextBody`. If any paragraph
/// currently has `bullet_level == 0` (i.e. bullets are not uniformly on), turn bullets on by
/// setting every paragraph's `bullet_level` to 1; otherwise turn them all off (set to 0).
/// Reads the current body from `world`; if the entity has no `TextBody`, returns an empty
/// transaction. Otherwise emits `SetComponent(Text)`.
pub fn toggle_bullets(world: &World, e: Entity) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => return Transaction::new("toggle bullets", vec![]),
    };
    // Any paragraph without a bullet means "not fully bulleted": turn bullets on.
    let turn_on = body.paragraphs.iter().any(|p| p.bullet_level == 0);
    let new_level = if turn_on { 1 } else { 0 };
    for para in &mut body.paragraphs {
        para.bullet_level = new_level;
    }
    Transaction::new(
        "toggle bullets",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Collect `e`'s siblings (entities sharing `e`'s parent; a `None` parent means root-level
/// siblings) paired with their `Order` key, sorted ascending by that key. `e` itself is
/// excluded, and siblings without an `Order` are skipped (they cannot be positioned
/// relative to `e`). The parent of an entity is read from `world.parent`.
fn ordered_siblings(world: &World, e: Entity) -> Vec<(Entity, FracIndex)> {
    let target_parent = world.parent.get(&e).copied();
    let mut siblings: Vec<(Entity, FracIndex)> = world
        .iter()
        .filter(|&other| other != e)
        .filter(|other| world.parent.get(other).copied() == target_parent)
        .filter_map(|other| world.order.get(&other).map(|o| (other, o.clone())))
        .collect();
    siblings.sort_by(|a, b| a.1.cmp(&b.1));
    siblings
}

/// Read `e`'s own `Order` key, if any.
fn order_of(world: &World, e: Entity) -> Option<FracIndex> {
    world.order.get(&e).cloned()
}

/// Build a transaction that sets `e`'s `Order` to `order`.
fn set_order_tx(label: &str, e: Entity, order: FracIndex) -> Transaction {
    Transaction::new(
        label,
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Order(order),
        }],
    )
}

/// Move `e` one step forward (toward the front / later in sibling order) among its siblings.
/// Siblings are those sharing `e`'s parent (a `None` parent means root-level), sorted by
/// their `Order` key. If `e` has a next sibling `n` and a sibling-after-next `n2`, `e`'s new
/// `Order` is placed strictly between them; if `n` is the last sibling, `e`'s new `Order` is
/// placed after `n`. If `e` is already last (no next sibling) or has no `Order`, this is a
/// no-op (empty transaction).
pub fn move_forward(world: &World, e: Entity) -> Transaction {
    let empty = Transaction::new("move forward", vec![]);
    let my_order = match order_of(world, e) {
        Some(o) => o,
        None => return empty,
    };
    let siblings = ordered_siblings(world, e);
    // Index of the first sibling whose order is strictly after `e`'s order: that is the
    // "next" sibling in front of `e`.
    let next_pos = siblings.iter().position(|(_, o)| *o > my_order);
    let next_pos = match next_pos {
        Some(p) => p,
        None => return empty, // Already last: nothing in front.
    };
    let n_order = &siblings[next_pos].1;
    let new_order = match siblings.get(next_pos + 1) {
        Some((_, n2_order)) => FracIndex::between(Some(n_order), Some(n2_order)),
        None => FracIndex::after(Some(n_order)),
    };
    set_order_tx("move forward", e, new_order)
}

/// Move `e` one step backward (toward the back / earlier in sibling order) among its
/// siblings. Symmetric to [`move_forward`]: if `e` has a previous sibling `p` and a
/// sibling-before-that `p2`, `e`'s new `Order` is placed strictly between them; if `p` is the
/// first sibling, `e`'s new `Order` is placed before `p`. If `e` is already first (no
/// previous sibling) or has no `Order`, this is a no-op (empty transaction).
pub fn move_backward(world: &World, e: Entity) -> Transaction {
    let empty = Transaction::new("move backward", vec![]);
    let my_order = match order_of(world, e) {
        Some(o) => o,
        None => return empty,
    };
    let siblings = ordered_siblings(world, e);
    // Index of the last sibling whose order is strictly before `e`'s order: that is the
    // "previous" sibling behind `e`.
    let prev_pos = siblings.iter().rposition(|(_, o)| *o < my_order);
    let prev_pos = match prev_pos {
        Some(p) => p,
        None => return empty, // Already first: nothing behind.
    };
    let p_order = &siblings[prev_pos].1;
    let new_order = if prev_pos == 0 {
        FracIndex::before(Some(p_order))
    } else {
        let p2_order = &siblings[prev_pos - 1].1;
        FracIndex::between(Some(p2_order), Some(p_order))
    };
    set_order_tx("move backward", e, new_order)
}

/// Add an entrance animation for `target` to `slide`'s animation timeline. Reads the slide's
/// existing `SlideTimeline` from `world` (or starts an empty one) and appends a new
/// `AnimStep` that fires after the previous step (no delay) and runs a single
/// `Entrance(effect)` anim on `target` for `duration_ms` milliseconds with `EaseOut`. The new
/// timeline is written back via `SetComponent(Timeline)` on the SLIDE entity, so undo restores
/// the prior timeline (or removes it if the slide had none).
pub fn add_entrance(
    world: &World,
    slide: Entity,
    target: Entity,
    effect: Effect,
    duration_ms: u32,
) -> Transaction {
    let mut timeline = match world.timelines.get(&slide) {
        Some(existing) => existing.clone(),
        None => SlideTimeline::default(),
    };
    timeline.steps.push(AnimStep {
        trigger: Trigger::AfterPrev { delay: 0 },
        anims: vec![Anim {
            target,
            kind: AnimKind::Entrance(effect),
            duration: duration_ms,
            delay: 0,
            easing: Easing::EaseOut,
        }],
    });
    Transaction::new(
        "add entrance",
        vec![Operation::SetComponent {
            entity: slide,
            value: CompValue::Timeline(timeline),
        }],
    )
}

/// Assign group membership `key` to every entity in `members`, so they select, move, and
/// delete as a unit. Callers mint a fresh nonzero `key` per group.
/// Wrap `members` in a new outer group `key`: prepend `key` to each member's group path, so any
/// existing (inner) grouping is preserved and the new group nests around it.
pub fn group(world: &World, members: &[Entity], key: u64) -> Transaction {
    let ops = members
        .iter()
        .map(|&entity| {
            let mut path = world.groups.get(&entity).cloned().unwrap_or_default();
            path.insert(0, key);
            Operation::SetComponent {
                entity,
                value: CompValue::Group(path),
            }
        })
        .collect();
    Transaction::new("group", ops)
}

/// Remove group `key` from every shape's path (un-nesting one level). When a path becomes
/// empty the component is removed entirely.
pub fn ungroup(world: &World, key: u64) -> Transaction {
    let ops = world
        .iter()
        .filter_map(|entity| {
            let path = world.groups.get(&entity)?;
            if !path.contains(&key) {
                return None;
            }
            let rest: Vec<u64> = path.iter().copied().filter(|&k| k != key).collect();
            Some(if rest.is_empty() {
                Operation::RemoveComponent {
                    entity,
                    kind: CompKind::Group,
                }
            } else {
                Operation::SetComponent {
                    entity,
                    value: CompValue::Group(rest),
                }
            })
        })
        .collect();
    Transaction::new("ungroup", ops)
}

/// The outermost group key of `e`, if any (the top-level group it belongs to).
pub fn outer_group(world: &World, e: Entity) -> Option<u64> {
    world.groups.get(&e).and_then(|p| p.first().copied())
}

/// All entities sharing `e`'s OUTERMOST group (including `e`); just `[e]` if it is ungrouped.
pub fn group_members(world: &World, e: Entity) -> Vec<Entity> {
    match outer_group(world, e) {
        Some(key) => world
            .iter()
            .filter(|x| outer_group(world, *x) == Some(key))
            .collect(),
        None => vec![e],
    }
}

#[cfg(test)]
mod tests;
