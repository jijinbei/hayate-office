//! HayateOffice command registry (DESIGN 6.5 / 6.13): the named, schema-tagged "verb"
//! surface that the command palette, scripts and AI call. A `Command` is a stable id plus a
//! parameter schema plus a handler; the handler reads typed args out of a JSON value, may
//! read current state from the [`World`], and produces a sequence of reversible
//! [`Operation`]s (DESIGN 6.10). The registry wraps those ops in a single [`Transaction`]
//! (one undo step) and can emit a `manifest` an AI/palette consumes.
//!
//! "1 schema, 3 roles" (DESIGN 6.13): one parameter schema feeds (1) the UI form, (2) the
//! AI/script call signature, and (3) the operation payload. For the MVP we use a minimal
//! `ParamSpec` list. The eventual *canonical* form is JSON Schema derived from Rust types via
//! `schemars` (so the manifest is literally an AI tool definition); the simple schema here is
//! a deliberate placeholder for that.

use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_ir::paint::Fill;
use hayate_ir::text::{HAlign, Paragraph, Run, TextBody};
use hayate_ir::units::{pt, EMU_PER_PT};
use hayate_ir::world::{CompKind, CompValue, Entity, World};
use hayate_model::align::{align, distribute, Align, Axis};
use hayate_model::edit;
use hayate_model::{Operation, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The type of a single command parameter. MVP stand-in for a JSON Schema property
/// (DESIGN 6.13); kept small and serializable so the manifest can be produced as-is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Entity,
    Int,
    Float,
    String,
    Color,
    Bool,
}

impl ParamType {
    /// Lowercase tag used in the manifest's `"type"` field.
    fn as_str(self) -> &'static str {
        match self {
            ParamType::Entity => "entity",
            ParamType::Int => "int",
            ParamType::Float => "float",
            ParamType::String => "string",
            ParamType::Color => "color",
            ParamType::Bool => "bool",
        }
    }
}

/// One named, typed parameter of a command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: String,
    pub ty: ParamType,
}

impl ParamSpec {
    pub fn new(name: impl Into<String>, ty: ParamType) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

/// Identifying / display metadata for a command (DESIGN 6.13 `CommandMeta`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMeta {
    /// Stable dotted id, e.g. `"shape.move"`. What scripts/AI/keymaps reference.
    pub id: String,
    /// Human-readable title; also used as the resulting transaction's undo label.
    pub title: String,
    /// Grouping for the palette, e.g. `"Shape"`.
    pub category: String,
}

impl CommandMeta {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            category: category.into(),
        }
    }
}

/// A command handler: reads typed args out of `args` (and may read current state from the
/// `World`, e.g. the current frame for a translate) and produces the reversible operations
/// to apply. Handlers are lenient: a missing/ill-typed field yields an empty op list rather
/// than an error (JSON Schema validation is the eventual gatekeeper, per DESIGN 6.13).
pub type Handler = Box<dyn Fn(&World, &Value) -> Vec<Operation>>;

/// A registered command: metadata, parameter schema, and handler.
struct Command {
    meta: CommandMeta,
    params: Vec<ParamSpec>,
    handler: Handler,
}

/// The set of available commands, keyed by id. The surface palette/scripts/AI invoke.
#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command. A later registration with the same id shadows the earlier one in
    /// lookups; both still appear in the manifest order they were added.
    pub fn register(
        &mut self,
        meta: CommandMeta,
        params: Vec<ParamSpec>,
        handler: impl Fn(&World, &Value) -> Vec<Operation> + 'static,
    ) {
        self.commands.push(Command {
            meta,
            params,
            handler: Box::new(handler),
        });
    }

    fn find(&self, id: &str) -> Option<&Command> {
        // Last registration wins (allows overriding builtins).
        self.commands.iter().rev().find(|c| c.meta.id == id)
    }

    /// Run the command's handler against `args` and wrap the resulting operations in a single
    /// `Transaction` labelled with the command title (one undo step, DESIGN 6.10). Returns
    /// `None` if `id` is unknown.
    pub fn build(&self, id: &str, args: &Value, world: &World) -> Option<Transaction> {
        let cmd = self.find(id)?;
        let ops = (cmd.handler)(world, args);
        Some(Transaction::new(cmd.meta.title.clone(), ops))
    }

    /// One entry per command, in registration order: the AI/palette tool catalogue
    /// (DESIGN 6.13 `registry.manifest()`).
    pub fn manifest(&self) -> Vec<Value> {
        self.commands
            .iter()
            .map(|c| {
                let params: Vec<Value> = c
                    .params
                    .iter()
                    .map(|p| json!({ "name": p.name, "type": p.ty.as_str() }))
                    .collect();
                json!({
                    "id": c.meta.id,
                    "title": c.meta.title,
                    "category": c.meta.category,
                    "params": params,
                })
            })
            .collect()
    }

    /// Return `(id, title)` pairs for commands whose id or title contains `query`
    /// (case-insensitive). An empty query matches every command. Used to drive the command
    /// palette's incremental search. Results are in registration order.
    pub fn filter(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        self.commands
            .iter()
            .filter(|c| {
                q.is_empty()
                    || c.meta.id.to_lowercase().contains(&q)
                    || c.meta.title.to_lowercase().contains(&q)
            })
            .map(|c| (c.meta.id.clone(), c.meta.title.clone()))
            .collect()
    }
}

// --- Arg parsing helpers (lenient) ---

/// Read an `Entity` from `args[key]` (a non-negative integer id).
fn arg_entity(args: &Value, key: &str) -> Option<Entity> {
    args.get(key)?.as_u64().map(Entity)
}

/// Read a list of `Entity`s from `args["entities"]` (a JSON array of non-negative integer
/// ids). Non-integer elements are skipped. A missing/non-array field yields an empty Vec, so
/// the align/distribute handlers stay lenient (they themselves return empty for too-few ids).
fn arg_entities(args: &Value) -> Vec<Entity> {
    match args.get("entities").and_then(Value::as_array) {
        Some(arr) => arr.iter().filter_map(Value::as_u64).map(Entity).collect(),
        None => vec![],
    }
}

/// Read an `i64` from `args[key]`, accepting integer or float JSON numbers.
fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key)?;
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// Read an `f64` from `args[key]`, accepting integer or float JSON numbers.
fn arg_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key)?.as_f64()
}

/// Read a string from `args[key]`.
fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(str::to_owned)
}

/// A plain text run with the MVP default formatting: body (minor) theme font, 18pt, the
/// theme's primary dark text color (Dk1). Used to seed a new text body or paragraph/run.
fn default_run(text: impl Into<String>) -> Run {
    Run {
        text: text.into(),
        font: FontRef::Theme(ThemeFontSlot::Minor),
        size: pt(18),
        color: Color::theme(ThemeColorToken::Dk1),
        bold: false,
        italic: false,
        underline: false,
    }
}

/// Convert a measurement in points to EMU (rounding to the nearest EMU).
fn pt_to_emu(pt: f64) -> hayate_ir::units::Emu {
    (pt * EMU_PER_PT as f64).round() as hayate_ir::units::Emu
}

/// Parse a color from `args[key]`. Accepts either an `{ "r":.., "g":.., "b":.., "a"?:.. }`
/// object or a hex string (`"#rrggbb"` / `"#rrggbbaa"`, leading `#` optional). Returns a
/// literal-color `Fill::Solid`.
fn arg_fill(args: &Value, key: &str) -> Option<Fill> {
    let v = args.get(key)?;
    let rgba = match v {
        Value::String(s) => parse_hex(s)?,
        Value::Object(_) => {
            let comp = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as u8);
            let r = comp("r")?;
            let g = comp("g")?;
            let b = comp("b")?;
            let a = comp("a").unwrap_or(255);
            Rgba::rgba(r, g, b, a)
        }
        _ => return None,
    };
    Some(Fill::Solid(Color::Literal(rgba)))
}

/// Parse `#rrggbb` or `#rrggbbaa` (the `#` is optional) into an `Rgba`.
fn parse_hex(s: &str) -> Option<Rgba> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        6 => Some(Rgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Rgba::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// The built-in command set for the MVP. Each handler maps typed JSON args to the editing
/// helpers in `hayate_model::edit` (or directly to `Operation`s).
pub fn builtins() -> CommandRegistry {
    let mut reg = CommandRegistry::new();

    // shape.delete — despawn the target entity (DESIGN 6.10 Despawn op).
    reg.register(
        CommandMeta::new("shape.delete", "Delete Shape", "Shape"),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |_world, args| match arg_entity(args, "entity") {
            Some(entity) => vec![Operation::Despawn { entity }],
            None => vec![],
        },
    );

    // shape.set_fill — replace the entity's Fill with a solid color.
    reg.register(
        CommandMeta::new("shape.set_fill", "Set Fill", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("color", ParamType::Color),
        ],
        |_world, args| match (arg_entity(args, "entity"), arg_fill(args, "color")) {
            (Some(entity), Some(fill)) => edit::set_fill(entity, fill).ops,
            _ => vec![],
        },
    );

    // shape.move — translate the entity's frame by (dx, dy); reads the current frame.
    reg.register(
        CommandMeta::new("shape.move", "Move Shape", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("dx", ParamType::Int),
            ParamSpec::new("dy", ParamType::Int),
        ],
        |world, args| match (
            arg_entity(args, "entity"),
            arg_i64(args, "dx"),
            arg_i64(args, "dy"),
        ) {
            (Some(entity), Some(dx), Some(dy)) => edit::translate(world, entity, dx, dy).ops,
            _ => vec![],
        },
    );

    // shape.set_position — set the entity's frame origin to an absolute point (in points,
    // converted to EMU), keeping its current size. Reads the current Frame from the World; an
    // entity with no Frame yields no ops. Built directly as a SetComponent op.
    reg.register(
        CommandMeta::new("shape.set_position", "Set Position", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("x", ParamType::Float),
            ParamSpec::new("y", ParamType::Float),
        ],
        |world, args| {
            match (
                arg_entity(args, "entity"),
                arg_f64(args, "x"),
                arg_f64(args, "y"),
            ) {
                (Some(entity), Some(x), Some(y)) => match world.get(entity, CompKind::Frame) {
                    Some(CompValue::Frame(frame)) => {
                        let origin_x = pt_to_emu(x);
                        let origin_y = pt_to_emu(y);
                        let rect = RectEmu {
                            origin: PointEmu::new(origin_x, origin_y),
                            size: frame.size,
                        };
                        vec![Operation::SetComponent {
                            entity,
                            value: CompValue::Frame(rect),
                        }]
                    }
                    // No Frame to position: lenient no-op.
                    _ => vec![],
                },
                _ => vec![],
            }
        },
    );

    // shape.set_size — set the entity's frame size to an absolute size (in points, converted
    // to EMU; each dimension is floored at 1pt), keeping its current origin. Reads the current
    // Frame from the World; an entity with no Frame yields no ops.
    reg.register(
        CommandMeta::new("shape.set_size", "Set Size", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("w", ParamType::Float),
            ParamSpec::new("h", ParamType::Float),
        ],
        |world, args| {
            match (
                arg_entity(args, "entity"),
                arg_f64(args, "w"),
                arg_f64(args, "h"),
            ) {
                (Some(entity), Some(w), Some(h)) => match world.get(entity, CompKind::Frame) {
                    Some(CompValue::Frame(frame)) => {
                        // Convert to EMU and clamp each dimension to a minimum of 1pt.
                        let min = EMU_PER_PT;
                        let width = pt_to_emu(w).max(min);
                        let height = pt_to_emu(h).max(min);
                        let rect = RectEmu {
                            origin: frame.origin,
                            size: SizeEmu::new(width, height),
                        };
                        vec![Operation::SetComponent {
                            entity,
                            value: CompValue::Frame(rect),
                        }]
                    }
                    // No Frame to resize: lenient no-op.
                    _ => vec![],
                },
                _ => vec![],
            }
        },
    );

    // shape.bring_to_front — move the entity to the front (largest Order) among its
    // siblings (entities sharing the same parent). Reads current order state from the World.
    reg.register(
        CommandMeta::new("shape.bring_to_front", "Bring to Front", "Shape"),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |world, args| match arg_entity(args, "entity") {
            Some(entity) => reorder_to_edge(world, entity, Edge::Front),
            None => vec![],
        },
    );

    // shape.send_to_back — symmetric: move the entity to the back (smallest Order) among
    // its siblings.
    reg.register(
        CommandMeta::new("shape.send_to_back", "Send to Back", "Shape"),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |world, args| match arg_entity(args, "entity") {
            Some(entity) => reorder_to_edge(world, entity, Edge::Back),
            None => vec![],
        },
    );

    // shape.set_rotation — set the entity's Rotation (degrees clockwise). Built directly as a
    // SetComponent op so this does not depend on the (separately evolving) edit::set_rotation.
    reg.register(
        CommandMeta::new("shape.set_rotation", "Set Rotation", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("degrees", ParamType::Float),
        ],
        |_world, args| match (arg_entity(args, "entity"), arg_f64(args, "degrees")) {
            (Some(entity), Some(degrees)) => vec![Operation::SetComponent {
                entity,
                value: CompValue::Rotation(degrees as f32),
            }],
            _ => vec![],
        },
    );

    // shape.set_opacity — set the entity's Opacity. The value is clamped to 0.0..=1.0 and
    // stored as f32. Built directly as a SetComponent op (no edit-helper dependency).
    reg.register(
        CommandMeta::new("shape.set_opacity", "Set Opacity", "Style"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("value", ParamType::Float),
        ],
        |_world, args| match (arg_entity(args, "entity"), arg_f64(args, "value")) {
            (Some(entity), Some(value)) => {
                let value = value.clamp(0.0, 1.0);
                vec![Operation::SetComponent {
                    entity,
                    value: CompValue::Opacity(value as f32),
                }]
            }
            _ => vec![],
        },
    );

    // shape.reset_rotation — clear any rotation by setting it back to 0 degrees.
    reg.register(
        CommandMeta::new("shape.reset_rotation", "Reset Rotation", "Shape"),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |_world, args| match arg_entity(args, "entity") {
            Some(entity) => vec![Operation::SetComponent {
                entity,
                value: CompValue::Rotation(0.0),
            }],
            None => vec![],
        },
    );

    // shape.rotate_by — add `degrees` to the entity's current Rotation (default 0.0 when the
    // entity has no Rotation yet). Reads current state from the World.
    reg.register(
        CommandMeta::new("shape.rotate_by", "Rotate By", "Shape"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("degrees", ParamType::Float),
        ],
        |world, args| match (arg_entity(args, "entity"), arg_f64(args, "degrees")) {
            (Some(entity), Some(degrees)) => {
                let current = match world.get(entity, CompKind::Rotation) {
                    Some(CompValue::Rotation(r)) => r,
                    _ => 0.0,
                };
                let sum = current + degrees as f32;
                vec![Operation::SetComponent {
                    entity,
                    value: CompValue::Rotation(sum),
                }]
            }
            _ => vec![],
        },
    );

    // shape.fill_accent1 .. shape.fill_accent6 — set the shape fill to a theme accent color.
    // Theme references (rather than literals) let a palette change propagate everywhere.
    for (n, token) in [
        ThemeColorToken::Accent1,
        ThemeColorToken::Accent2,
        ThemeColorToken::Accent3,
        ThemeColorToken::Accent4,
        ThemeColorToken::Accent5,
        ThemeColorToken::Accent6,
    ]
    .into_iter()
    .enumerate()
    {
        let n = n + 1;
        reg.register(
            CommandMeta::new(
                format!("shape.fill_accent{n}"),
                format!("Fill: Accent {n}"),
                "Style",
            ),
            vec![ParamSpec::new("entity", ParamType::Entity)],
            move |_world, args| match arg_entity(args, "entity") {
                Some(entity) => vec![Operation::SetComponent {
                    entity,
                    value: CompValue::Fill(Fill::Solid(Color::theme(token))),
                }],
                None => vec![],
            },
        );
    }

    // shape.set_text — set the entity's text to a single string, preserving the existing
    // first run's formatting where possible. If the entity already has a TextBody, clone it
    // and overwrite the first paragraph's first run text (creating a default paragraph/run if
    // either is missing). If there is no TextBody yet, create one with a single default run.
    reg.register(
        CommandMeta::new("shape.set_text", "Set Text", "Text"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("text", ParamType::String),
        ],
        |world, args| match (arg_entity(args, "entity"), arg_str(args, "text")) {
            (Some(entity), Some(text)) => {
                let body = match world.texts.get(&entity) {
                    Some(existing) => {
                        // Preserve existing formatting; only replace the leading run's text.
                        let mut body = existing.clone();
                        match body.paragraphs.first_mut() {
                            Some(para) => match para.runs.first_mut() {
                                Some(run) => run.text = text,
                                None => para.runs.push(default_run(text)),
                            },
                            None => body
                                .paragraphs
                                .push(Paragraph::new(vec![default_run(text)])),
                        }
                        body
                    }
                    None => TextBody {
                        paragraphs: vec![Paragraph::new(vec![default_run(text)])],
                        autofit: false,
                    },
                };
                vec![Operation::SetComponent {
                    entity,
                    value: CompValue::Text(body),
                }]
            }
            _ => vec![],
        },
    );

    // shape.set_font_size — set EVERY run's size to `pt` points (clamped to a minimum of 1pt),
    // preserving all other formatting. Reads the current TextBody from the World; an entity with
    // no text yields no ops. Built directly as a SetComponent op.
    reg.register(
        CommandMeta::new("shape.set_font_size", "Set Font Size", "Text"),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("pt", ParamType::Float),
        ],
        |world, args| match (arg_entity(args, "entity"), arg_i64(args, "pt")) {
            (Some(entity), Some(points)) => match world.texts.get(&entity) {
                Some(existing) => {
                    // Clamp to a minimum of 1pt, then apply to every run.
                    let size = pt(points.max(1));
                    let mut body = existing.clone();
                    for para in &mut body.paragraphs {
                        for run in &mut para.runs {
                            run.size = size;
                        }
                    }
                    vec![Operation::SetComponent {
                        entity,
                        value: CompValue::Text(body),
                    }]
                }
                // No text to size: lenient no-op.
                None => vec![],
            },
            _ => vec![],
        },
    );

    // shape.toggle_bold / shape.toggle_italic / shape.toggle_underline — flip the named run
    // attribute across the whole text box. To keep the box consistent, the new value is the
    // negation of the FIRST run's current value, then applied to every run. Reads the current
    // TextBody from the World; an entity with no text yields no ops.
    for (id, title, attr) in [
        ("shape.toggle_bold", "Toggle Bold", RunAttr::Bold),
        ("shape.toggle_italic", "Toggle Italic", RunAttr::Italic),
        (
            "shape.toggle_underline",
            "Toggle Underline",
            RunAttr::Underline,
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Text"),
            vec![ParamSpec::new("entity", ParamType::Entity)],
            move |world, args| match arg_entity(args, "entity") {
                Some(entity) => match world.texts.get(&entity) {
                    Some(existing) => {
                        let mut body = existing.clone();
                        // Negate the first run's current value (default false when no run exists).
                        let first = body
                            .paragraphs
                            .first()
                            .and_then(|p| p.runs.first())
                            .map(|r| attr.get(r))
                            .unwrap_or(false);
                        let next = !first;
                        for para in &mut body.paragraphs {
                            for run in &mut para.runs {
                                attr.set(run, next);
                            }
                        }
                        vec![Operation::SetComponent {
                            entity,
                            value: CompValue::Text(body),
                        }]
                    }
                    // No text to toggle: lenient no-op.
                    None => vec![],
                },
                None => vec![],
            },
        );
    }

    // shape.align_text_left / _center / _right — set EVERY paragraph's horizontal alignment.
    // Reads the current TextBody from the World; an entity with no text yields no ops.
    for (id, title, halign) in [
        ("shape.align_text_left", "Align Text Left", HAlign::Left),
        (
            "shape.align_text_center",
            "Align Text Center",
            HAlign::Center,
        ),
        ("shape.align_text_right", "Align Text Right", HAlign::Right),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Text"),
            vec![ParamSpec::new("entity", ParamType::Entity)],
            move |world, args| match arg_entity(args, "entity") {
                Some(entity) => match world.texts.get(&entity) {
                    Some(existing) => {
                        let mut body = existing.clone();
                        for para in &mut body.paragraphs {
                            para.align = halign;
                        }
                        vec![Operation::SetComponent {
                            entity,
                            value: CompValue::Text(body),
                        }]
                    }
                    // No text to align: lenient no-op.
                    None => vec![],
                },
                None => vec![],
            },
        );
    }

    // shape.fill_black / shape.fill_white — set the shape fill to a literal black/white. Unlike
    // the accent fills these use literals (not theme tokens), so they stay fixed across themes.
    for (id, title, rgba) in [
        ("shape.fill_black", "Fill: Black", Rgba::BLACK),
        ("shape.fill_white", "Fill: White", Rgba::WHITE),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Style"),
            vec![ParamSpec::new("entity", ParamType::Entity)],
            move |_world, args| match arg_entity(args, "entity") {
                Some(entity) => vec![Operation::SetComponent {
                    entity,
                    value: CompValue::Fill(Fill::Solid(Color::Literal(rgba))),
                }],
                None => vec![],
            },
        );
    }

    // shapes.align_* — align a multi-entity selection to the group bounding box. The selection
    // is passed as `entities` (a JSON array of u64 ids). Each delegates to
    // `hayate_model::align::align`, returning its frame-setting ops (empty for < 2 framed ids).
    for (id, title, how) in [
        ("shapes.align_left", "Align Left", Align::Left),
        (
            "shapes.align_hcenter",
            "Align Center (Horizontal)",
            Align::HCenter,
        ),
        ("shapes.align_right", "Align Right", Align::Right),
        ("shapes.align_top", "Align Top", Align::Top),
        (
            "shapes.align_vcenter",
            "Align Middle (Vertical)",
            Align::VCenter,
        ),
        ("shapes.align_bottom", "Align Bottom", Align::Bottom),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Arrange"),
            vec![ParamSpec::new("entities", ParamType::Entity)],
            move |world, args| {
                let ids = arg_entities(args);
                align(world, &ids, how).ops
            },
        );
    }

    // shapes.distribute_* — evenly distribute a multi-entity selection along an axis. The
    // selection is passed as `entities` (a JSON array of u64 ids). Each delegates to
    // `hayate_model::align::distribute`, returning its ops (empty for < 3 framed ids).
    for (id, title, axis) in [
        (
            "shapes.distribute_h",
            "Distribute Horizontally",
            Axis::Horizontal,
        ),
        (
            "shapes.distribute_v",
            "Distribute Vertically",
            Axis::Vertical,
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Arrange"),
            vec![ParamSpec::new("entities", ParamType::Entity)],
            move |world, args| {
                let ids = arg_entities(args);
                distribute(world, &ids, axis).ops
            },
        );
    }

    reg
}

/// A boolean run-formatting attribute that the toggle commands flip uniformly across a text
/// box. Lets the three toggles share one handler body via [`RunAttr::get`]/[`RunAttr::set`].
#[derive(Clone, Copy)]
enum RunAttr {
    Bold,
    Italic,
    Underline,
}

impl RunAttr {
    /// Read this attribute's current value off a run.
    fn get(self, run: &Run) -> bool {
        match self {
            RunAttr::Bold => run.bold,
            RunAttr::Italic => run.italic,
            RunAttr::Underline => run.underline,
        }
    }

    /// Set this attribute on a run.
    fn set(self, run: &mut Run, value: bool) {
        match self {
            RunAttr::Bold => run.bold = value,
            RunAttr::Italic => run.italic = value,
            RunAttr::Underline => run.underline = value,
        }
    }
}

/// Which sibling extreme to move an entity to.
enum Edge {
    /// Largest Order key (drawn last / on top).
    Front,
    /// Smallest Order key (drawn first / behind).
    Back,
}

/// Compute the single `SetComponent { Order }` operation that moves `e` to the front or back
/// of its sibling group. Siblings are the live entities sharing `e`'s parent (`None` parent
/// means root-level siblings), excluding `e` itself. The new key is generated just past the
/// current max (front) or min (back) sibling order via fractional indexing, so no other
/// entity's key changes. With no siblings we fall back to re-keying relative to `e`'s own
/// current order (still a valid, idempotent op).
fn reorder_to_edge(world: &World, e: Entity, edge: Edge) -> Vec<Operation> {
    if !world.is_alive(e) {
        return vec![];
    }
    let parent = world.parent.get(&e).copied();

    // Collect siblings' order keys (live entities with the same parent, excluding e).
    let sibling_orders = world
        .iter()
        .filter(|&s| s != e)
        .filter(|&s| world.parent.get(&s).copied() == parent);

    let new_order = match edge {
        Edge::Front => {
            let max = sibling_orders.filter_map(|s| world.order.get(&s)).max();
            // Fall back to e's own current order when there are no siblings to compare with.
            let anchor = max.or_else(|| world.order.get(&e));
            FracIndex::after(anchor)
        }
        Edge::Back => {
            let min = sibling_orders.filter_map(|s| world.order.get(&s)).min();
            let anchor = min.or_else(|| world.order.get(&e));
            FracIndex::before(anchor)
        }
    };

    vec![Operation::SetComponent {
        entity: e,
        value: CompValue::Order(new_order),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayate_ir::geom::RectEmu;
    use hayate_ir::world::{CompKind, CompValue};
    use hayate_model::History;

    /// Build a world with a single entity carrying a Frame; return (world, entity).
    fn world_with_framed_shape() -> (World, Entity) {
        let mut w = World::new();
        let e = w.spawn();
        w.set(e, CompValue::Frame(RectEmu::new(10, 20, 100, 50)));
        (w, e)
    }

    #[test]
    fn move_command_shifts_frame() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.move",
                &json!({ "entity": e.0, "dx": 100, "dy": 0 }),
                &w,
            )
            .expect("shape.move is registered");
        assert_eq!(tx.label, "Move Shape");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(110, 20, 100, 50))),
            "origin shifted by dx, size unchanged"
        );

        // And it is undoable as one step.
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
        );
    }

    #[test]
    fn delete_command_removes_entity() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.delete", &json!({ "entity": e.0 }), &w)
            .expect("shape.delete is registered");
        h.commit(&mut w, tx);

        assert!(!w.is_alive(e), "entity is gone after delete");

        // Undo restores it.
        assert!(h.undo(&mut w));
        assert!(w.is_alive(e));
        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
        );
    }

    #[test]
    fn set_fill_command_applies_color() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        // Object form.
        let tx = reg
            .build(
                "shape.set_fill",
                &json!({ "entity": e.0, "color": { "r": 255, "g": 0, "b": 0 } }),
                &w,
            )
            .unwrap();
        h.commit(&mut w, tx);
        assert_eq!(
            w.get(e, CompKind::Fill),
            Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(
                255, 0, 0
            )))))
        );

        // Hex string form.
        let tx = reg
            .build(
                "shape.set_fill",
                &json!({ "entity": e.0, "color": "#00ff00" }),
                &w,
            )
            .unwrap();
        h.commit(&mut w, tx);
        assert_eq!(
            w.get(e, CompKind::Fill),
            Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(
                0, 255, 0
            )))))
        );
    }

    #[test]
    fn unknown_command_returns_none() {
        let reg = builtins();
        let w = World::new();
        assert!(reg.build("shape.nope", &json!({}), &w).is_none());
    }

    #[test]
    fn missing_args_yield_empty_transaction() {
        let reg = builtins();
        let w = World::new();
        let tx = reg.build("shape.move", &json!({}), &w).unwrap();
        assert!(tx.ops.is_empty(), "lenient: missing fields => no ops");
    }

    /// Build a parent with three ordered children. Returns (world, parent, [c0, c1, c2])
    /// where the children's current Order keys are strictly increasing (c0 back .. c2 front).
    fn world_with_three_children() -> (World, Entity, [Entity; 3]) {
        let mut w = World::new();
        let parent = w.spawn();

        let mut children = [Entity(0); 3];
        let mut last: Option<FracIndex> = None;
        for child in &mut children {
            let e = w.spawn();
            w.set(e, CompValue::Parent(parent));
            let order = FracIndex::after(last.as_ref());
            w.set(e, CompValue::Order(order.clone()));
            last = Some(order);
            *child = e;
        }
        (w, parent, children)
    }

    /// Read an entity's Order key out of the world.
    fn order_of(w: &World, e: Entity) -> FracIndex {
        match w.get(e, CompKind::Order) {
            Some(CompValue::Order(o)) => o,
            other => panic!("expected an Order on {e:?}, got {other:?}"),
        }
    }

    /// The three children re-sorted by their current Order key (back -> front).
    fn children_by_order(w: &World, children: &[Entity; 3]) -> Vec<Entity> {
        let mut sorted = children.to_vec();
        sorted.sort_by_key(|&e| order_of(w, e));
        sorted
    }

    #[test]
    fn bring_to_front_moves_child_last() {
        let (mut w, _parent, children) = world_with_three_children();
        let reg = builtins();
        let mut h = History::new();

        // c0 starts at the back; bring it to the front.
        let target = children[0];
        let tx = reg
            .build("shape.bring_to_front", &json!({ "entity": target.0 }), &w)
            .expect("shape.bring_to_front is registered");
        assert_eq!(tx.label, "Bring to Front");
        h.commit(&mut w, tx);

        let sorted = children_by_order(&w, &children);
        assert_eq!(
            *sorted.last().unwrap(),
            target,
            "bring_to_front puts the child last in Order; got {sorted:?}"
        );

        // One undo step restores the original ordering (c0 back).
        assert!(h.undo(&mut w));
        let restored = children_by_order(&w, &children);
        assert_eq!(restored, children.to_vec());
    }

    #[test]
    fn send_to_back_moves_child_first() {
        let (mut w, _parent, children) = world_with_three_children();
        let reg = builtins();
        let mut h = History::new();

        // c2 starts at the front; send it to the back.
        let target = children[2];
        let tx = reg
            .build("shape.send_to_back", &json!({ "entity": target.0 }), &w)
            .expect("shape.send_to_back is registered");
        assert_eq!(tx.label, "Send to Back");
        h.commit(&mut w, tx);

        let sorted = children_by_order(&w, &children);
        assert_eq!(
            *sorted.first().unwrap(),
            target,
            "send_to_back puts the child first in Order; got {sorted:?}"
        );

        assert!(h.undo(&mut w));
        let restored = children_by_order(&w, &children);
        assert_eq!(restored, children.to_vec());
    }

    #[test]
    fn z_order_missing_entity_is_lenient() {
        let reg = builtins();
        let w = World::new();
        let front = reg.build("shape.bring_to_front", &json!({}), &w).unwrap();
        let back = reg.build("shape.send_to_back", &json!({}), &w).unwrap();
        assert!(front.ops.is_empty());
        assert!(back.ops.is_empty());
    }

    #[test]
    fn manifest_lists_builtin_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"shape.delete"));
        assert!(ids.contains(&"shape.set_fill"));
        assert!(ids.contains(&"shape.move"));

        // Each entry carries the documented shape.
        let mv = manifest.iter().find(|c| c["id"] == "shape.move").unwrap();
        assert_eq!(mv["title"], "Move Shape");
        assert_eq!(mv["category"], "Shape");
        let params = mv["params"].as_array().unwrap();
        assert_eq!(params[0]["name"], "entity");
        assert_eq!(params[0]["type"], "entity");
    }

    #[test]
    fn set_rotation_command_sets_rotation() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.set_rotation",
                &json!({ "entity": e.0, "degrees": 45 }),
                &w,
            )
            .expect("shape.set_rotation is registered");
        assert_eq!(tx.label, "Set Rotation");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(45.0))
        );

        // Undoable as one step (the rotation was previously absent).
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Rotation), None);
    }

    #[test]
    fn fill_accent_command_sets_theme_fill() {
        use hayate_ir::color::{Color, ThemeColorToken};

        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.fill_accent3", &json!({ "entity": e.0 }), &w)
            .expect("shape.fill_accent3 is registered");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Fill),
            Some(CompValue::Fill(Fill::Solid(Color::theme(
                ThemeColorToken::Accent3
            ))))
        );
    }

    #[test]
    fn filter_finds_accent_commands() {
        let reg = builtins();
        let hits = reg.filter("accent");
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids.len(), 6, "exactly the six accent fills, got {ids:?}");
        for n in 1..=6 {
            let id = format!("shape.fill_accent{n}");
            assert!(ids.contains(&id.as_str()), "missing {id}");
        }
        // Titles come through alongside ids.
        assert!(hits.iter().any(|(_, title)| title == "Fill: Accent 1"));
    }

    #[test]
    fn set_opacity_command_sets_and_clamps() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        // A normal in-range value is stored as-is.
        let tx = reg
            .build(
                "shape.set_opacity",
                &json!({ "entity": e.0, "value": 0.5 }),
                &w,
            )
            .expect("shape.set_opacity is registered");
        assert_eq!(tx.label, "Set Opacity");
        h.commit(&mut w, tx);
        assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(0.5)));

        // An out-of-range value is clamped to 1.0.
        let tx = reg
            .build(
                "shape.set_opacity",
                &json!({ "entity": e.0, "value": 2.5 }),
                &w,
            )
            .unwrap();
        h.commit(&mut w, tx);
        assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(1.0)));

        // Undoable as one step (back to 0.5).
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(0.5)));
    }

    #[test]
    fn reset_rotation_command_sets_zero() {
        let (mut w, e) = world_with_framed_shape();
        w.set(e, CompValue::Rotation(33.0));
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.reset_rotation", &json!({ "entity": e.0 }), &w)
            .expect("shape.reset_rotation is registered");
        assert_eq!(tx.label, "Reset Rotation");
        h.commit(&mut w, tx);
        assert_eq!(w.get(e, CompKind::Rotation), Some(CompValue::Rotation(0.0)));

        // Undoable as one step (back to the prior rotation).
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(33.0))
        );
    }

    #[test]
    fn rotate_by_command_adds_to_current() {
        let (mut w, e) = world_with_framed_shape();
        w.set(e, CompValue::Rotation(10.0));
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.rotate_by",
                &json!({ "entity": e.0, "degrees": 35 }),
                &w,
            )
            .expect("shape.rotate_by is registered");
        assert_eq!(tx.label, "Rotate By");
        h.commit(&mut w, tx);
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(45.0))
        );

        // Undoable as one step (back to 10.0).
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(10.0))
        );
    }

    #[test]
    fn manifest_includes_new_style_and_shape_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"shape.set_opacity"));
        assert!(ids.contains(&"shape.reset_rotation"));
        assert!(ids.contains(&"shape.rotate_by"));
    }

    #[test]
    fn manifest_includes_set_rotation() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"shape.set_rotation"));
    }

    #[test]
    fn set_position_command_sets_origin_keeping_size() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        // x=100pt, y=50pt -> (1_270_000, 635_000) EMU; size (100, 50) preserved.
        let tx = reg
            .build(
                "shape.set_position",
                &json!({ "entity": e.0, "x": 100, "y": 50 }),
                &w,
            )
            .expect("shape.set_position is registered");
        assert_eq!(tx.label, "Set Position");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(1_270_000, 635_000, 100, 50))),
            "origin set in EMU, size unchanged"
        );

        // Undoable as one step (back to the original frame).
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
        );
    }

    #[test]
    fn set_size_command_sets_size_keeping_origin() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        // w=200pt, h=100pt -> (2_540_000, 1_270_000) EMU; origin (10, 20) preserved.
        let tx = reg
            .build(
                "shape.set_size",
                &json!({ "entity": e.0, "w": 200, "h": 100 }),
                &w,
            )
            .expect("shape.set_size is registered");
        assert_eq!(tx.label, "Set Size");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 2_540_000, 1_270_000))),
            "size set in EMU, origin unchanged"
        );

        // Undoable as one step (back to the original frame).
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
        );
    }

    #[test]
    fn set_size_floors_to_one_point() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        // Zero-ish dimensions are clamped to a minimum of 1pt (= EMU_PER_PT) each.
        let tx = reg
            .build(
                "shape.set_size",
                &json!({ "entity": e.0, "w": 0, "h": 0 }),
                &w,
            )
            .unwrap();
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(10, 20, 12_700, 12_700))),
            "each dimension floored at 1pt, origin unchanged"
        );
    }

    #[test]
    fn set_position_missing_entity_is_lenient() {
        let reg = builtins();
        let w = World::new();
        let tx = reg
            .build("shape.set_position", &json!({ "x": 100, "y": 50 }), &w)
            .unwrap();
        assert!(tx.ops.is_empty(), "no entity => no ops");
    }

    #[test]
    fn set_position_no_frame_is_lenient() {
        // An entity that exists but carries no Frame yields an empty transaction.
        let mut w = World::new();
        let e = w.spawn();
        let reg = builtins();
        let tx = reg
            .build(
                "shape.set_position",
                &json!({ "entity": e.0, "x": 100, "y": 50 }),
                &w,
            )
            .unwrap();
        assert!(tx.ops.is_empty(), "no Frame => no ops");
    }

    #[test]
    fn set_size_no_frame_is_lenient() {
        let mut w = World::new();
        let e = w.spawn();
        let reg = builtins();
        let tx = reg
            .build(
                "shape.set_size",
                &json!({ "entity": e.0, "w": 200, "h": 100 }),
                &w,
            )
            .unwrap();
        assert!(tx.ops.is_empty(), "no Frame => no ops");
    }

    #[test]
    fn manifest_includes_set_position_and_set_size() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"shape.set_position"));
        assert!(ids.contains(&"shape.set_size"));
    }

    #[test]
    fn set_text_command_creates_body_with_run_text() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.set_text",
                &json!({ "entity": e.0, "text": "Hello" }),
                &w,
            )
            .expect("shape.set_text is registered");
        assert_eq!(tx.label, "Set Text");
        h.commit(&mut w, tx);

        match w.get(e, CompKind::Text) {
            Some(CompValue::Text(body)) => {
                assert_eq!(body.paragraphs[0].runs[0].text, "Hello");
                assert!(!body.autofit);
            }
            other => panic!("expected a Text component, got {other:?}"),
        }

        // Undoable as one step (the text was previously absent).
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Text), None);
    }

    #[test]
    fn fill_black_command_sets_literal_black() {
        let (mut w, e) = world_with_framed_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.fill_black", &json!({ "entity": e.0 }), &w)
            .expect("shape.fill_black is registered");
        h.commit(&mut w, tx);

        assert_eq!(
            w.get(e, CompKind::Fill),
            Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::BLACK))))
        );
    }

    #[test]
    fn manifest_includes_text_and_fill_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"shape.set_text"));
        assert!(ids.contains(&"shape.fill_black"));
        assert!(ids.contains(&"shape.fill_white"));
    }

    /// Spawn three framed entities under a common parent with increasing order keys; return
    /// (world, [e0, e1, e2]). Their frames have differing x positions so alignment is visible.
    fn world_with_three_framed() -> (World, [Entity; 3]) {
        let mut w = World::new();
        let parent = w.spawn();
        let frames = [
            RectEmu::new(10, 0, 100, 50),
            RectEmu::new(30, 100, 40, 50),
            RectEmu::new(70, 200, 60, 50),
        ];
        let mut entities = [Entity(0); 3];
        let mut last: Option<FracIndex> = None;
        for (slot, frame) in entities.iter_mut().zip(frames) {
            let e = w.spawn();
            w.set(e, CompValue::Parent(parent));
            let order = FracIndex::after(last.as_ref());
            w.set(e, CompValue::Order(order.clone()));
            last = Some(order);
            w.set(e, CompValue::Frame(frame));
            *slot = e;
        }
        (w, entities)
    }

    #[test]
    fn align_left_command_shares_min_x() {
        let (mut w, es) = world_with_three_framed();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shapes.align_left",
                &json!({ "entities": [es[0].0, es[1].0, es[2].0] }),
                &w,
            )
            .expect("shapes.align_left is registered");
        assert_eq!(tx.label, "Align Left");
        h.commit(&mut w, tx);

        // All three left edges now sit at the group minimum x (10).
        let x_of = |w: &World, e: Entity| match w.get(e, CompKind::Frame) {
            Some(CompValue::Frame(f)) => f.origin.x,
            other => panic!("expected a Frame, got {other:?}"),
        };
        assert_eq!(x_of(&w, es[0]), 10);
        assert_eq!(x_of(&w, es[1]), 10);
        assert_eq!(x_of(&w, es[2]), 10);
    }

    #[test]
    fn distribute_h_command_equalizes_gaps() {
        let (mut w, es) = world_with_three_framed();
        // Lay the items out so distribution has something to do.
        w.set(es[0], CompValue::Frame(RectEmu::new(0, 0, 100, 50)));
        w.set(es[1], CompValue::Frame(RectEmu::new(120, 0, 40, 50)));
        w.set(es[2], CompValue::Frame(RectEmu::new(240, 0, 60, 50)));
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shapes.distribute_h",
                &json!({ "entities": [es[0].0, es[1].0, es[2].0] }),
                &w,
            )
            .expect("shapes.distribute_h is registered");
        assert_eq!(tx.label, "Distribute Horizontally");
        h.commit(&mut w, tx);

        // The middle item is repositioned so gaps are equal (gap = 50): x = 100 + 50 = 150.
        assert_eq!(
            w.get(es[1], CompKind::Frame),
            Some(CompValue::Frame(RectEmu::new(150, 0, 40, 50)))
        );
    }

    #[test]
    fn align_distribute_missing_entities_is_lenient() {
        let reg = builtins();
        let w = World::new();
        // Missing `entities` -> no ids -> align/distribute return empty.
        assert!(reg
            .build("shapes.align_left", &json!({}), &w)
            .unwrap()
            .ops
            .is_empty());
        assert!(reg
            .build("shapes.distribute_v", &json!({ "entities": [] }), &w)
            .unwrap()
            .ops
            .is_empty());
    }

    #[test]
    fn manifest_includes_arrange_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        for id in [
            "shapes.align_left",
            "shapes.align_hcenter",
            "shapes.align_right",
            "shapes.align_top",
            "shapes.align_vcenter",
            "shapes.align_bottom",
            "shapes.distribute_h",
            "shapes.distribute_v",
        ] {
            assert!(ids.contains(&id), "manifest missing {id}");
        }

        // The new commands are grouped under "Arrange".
        let al = manifest
            .iter()
            .find(|c| c["id"] == "shapes.align_left")
            .unwrap();
        assert_eq!(al["category"], "Arrange");
    }

    /// Build a world with a single entity carrying a Frame and a TextBody with two runs in one
    /// paragraph (so "all runs" behaviour is observable). Returns (world, entity).
    fn world_with_text_shape() -> (World, Entity) {
        use hayate_ir::text::{Paragraph, Run};
        let (mut w, e) = world_with_framed_shape();
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![
                default_run("Hello"),
                default_run("World"),
            ])],
            autofit: false,
        };
        w.set(e, CompValue::Text(body));
        (w, e)
    }

    /// Read the entity's TextBody out of the world.
    fn text_of(w: &World, e: Entity) -> TextBody {
        match w.get(e, CompKind::Text) {
            Some(CompValue::Text(body)) => body,
            other => panic!("expected a Text component, got {other:?}"),
        }
    }

    #[test]
    fn set_font_size_command_sets_all_runs() {
        let (mut w, e) = world_with_text_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.set_font_size",
                &json!({ "entity": e.0, "pt": 32 }),
                &w,
            )
            .expect("shape.set_font_size is registered");
        assert_eq!(tx.label, "Set Font Size");
        h.commit(&mut w, tx);

        let body = text_of(&w, e);
        for run in &body.paragraphs[0].runs {
            assert_eq!(run.size, pt(32), "every run sized to 32pt");
        }

        // Undoable as one step (back to the seeded 18pt).
        assert!(h.undo(&mut w));
        let body = text_of(&w, e);
        assert_eq!(body.paragraphs[0].runs[0].size, pt(18));
    }

    #[test]
    fn set_font_size_command_clamps_to_one_point() {
        let (mut w, e) = world_with_text_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(
                "shape.set_font_size",
                &json!({ "entity": e.0, "pt": 0 }),
                &w,
            )
            .unwrap();
        h.commit(&mut w, tx);

        let body = text_of(&w, e);
        assert_eq!(body.paragraphs[0].runs[0].size, pt(1), "floored at 1pt");
    }

    #[test]
    fn toggle_bold_command_flips_all_runs() {
        let (mut w, e) = world_with_text_shape();
        let reg = builtins();
        let mut h = History::new();

        // First toggle: false -> true on every run.
        let tx = reg
            .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
            .expect("shape.toggle_bold is registered");
        assert_eq!(tx.label, "Toggle Bold");
        h.commit(&mut w, tx);
        let body = text_of(&w, e);
        assert!(body.paragraphs[0].runs.iter().all(|r| r.bold));

        // Second toggle: back to false on every run (based on first run's value).
        let tx = reg
            .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
            .unwrap();
        h.commit(&mut w, tx);
        let body = text_of(&w, e);
        assert!(body.paragraphs[0].runs.iter().all(|r| !r.bold));
    }

    #[test]
    fn toggle_italic_command_flips_all_runs() {
        let (mut w, e) = world_with_text_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.toggle_italic", &json!({ "entity": e.0 }), &w)
            .expect("shape.toggle_italic is registered");
        assert_eq!(tx.label, "Toggle Italic");
        h.commit(&mut w, tx);
        let body = text_of(&w, e);
        assert!(body.paragraphs[0].runs.iter().all(|r| r.italic));
    }

    #[test]
    fn toggle_underline_command_flips_all_runs() {
        let (mut w, e) = world_with_text_shape();
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.toggle_underline", &json!({ "entity": e.0 }), &w)
            .expect("shape.toggle_underline is registered");
        assert_eq!(tx.label, "Toggle Underline");
        h.commit(&mut w, tx);
        let body = text_of(&w, e);
        assert!(body.paragraphs[0].runs.iter().all(|r| r.underline));
    }

    #[test]
    fn toggle_bold_consistent_when_runs_differ() {
        // When runs disagree, the whole box follows the FIRST run: first=false -> all true.
        let (mut w, e) = world_with_text_shape();
        let mut body = text_of(&w, e);
        body.paragraphs[0].runs[1].bold = true; // second run already bold
        w.set(e, CompValue::Text(body));
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
            .unwrap();
        h.commit(&mut w, tx);
        let body = text_of(&w, e);
        assert!(
            body.paragraphs[0].runs.iter().all(|r| r.bold),
            "first run was false, so the whole box becomes bold"
        );
    }

    #[test]
    fn align_text_commands_set_every_paragraph() {
        for (id, expected) in [
            ("shape.align_text_left", HAlign::Left),
            ("shape.align_text_center", HAlign::Center),
            ("shape.align_text_right", HAlign::Right),
        ] {
            let (mut w, e) = world_with_text_shape();
            // Seed a second paragraph so "every paragraph" is observable.
            let mut body = text_of(&w, e);
            body.paragraphs
                .push(Paragraph::new(vec![default_run("Second")]));
            w.set(e, CompValue::Text(body));
            let reg = builtins();
            let mut h = History::new();

            let tx = reg
                .build(id, &json!({ "entity": e.0 }), &w)
                .unwrap_or_else(|| panic!("{id} is registered"));
            h.commit(&mut w, tx);

            let body = text_of(&w, e);
            for para in &body.paragraphs {
                assert_eq!(para.align, expected, "{id} sets every paragraph");
            }
        }
    }

    #[test]
    fn text_commands_no_text_is_lenient() {
        // An entity that exists but carries no TextBody yields an empty transaction.
        let (w, e) = world_with_framed_shape();
        let reg = builtins();
        for id in [
            "shape.toggle_bold",
            "shape.toggle_italic",
            "shape.toggle_underline",
            "shape.align_text_left",
            "shape.align_text_center",
            "shape.align_text_right",
        ] {
            let tx = reg.build(id, &json!({ "entity": e.0 }), &w).unwrap();
            assert!(tx.ops.is_empty(), "{id}: no text => no ops");
        }
        let tx = reg
            .build(
                "shape.set_font_size",
                &json!({ "entity": e.0, "pt": 24 }),
                &w,
            )
            .unwrap();
        assert!(tx.ops.is_empty(), "set_font_size: no text => no ops");
    }

    #[test]
    fn manifest_includes_text_formatting_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
        for id in [
            "shape.set_font_size",
            "shape.toggle_bold",
            "shape.toggle_italic",
            "shape.toggle_underline",
            "shape.align_text_left",
            "shape.align_text_center",
            "shape.align_text_right",
        ] {
            assert!(ids.contains(&id), "manifest missing {id}");
        }
    }
}
