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

mod script;
pub use script::{run_script, ScriptError, ScriptOutcome};

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
    /// One-line description of what the command does. Feeds the generated scripting-API
    /// reference and the AI tool schema (DESIGN 6.13). Empty when unset; the docs then fall
    /// back to the title.
    pub description: String,
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
            description: String::new(),
        }
    }

    /// Attach a one-line description (chainable). Used by the generated API docs / AI schema.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
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
                    "description": c.meta.description,
                    "params": params,
                })
            })
            .collect()
    }

    /// One JSON-Schema tool definition per command (Anthropic-tool shape: name + description +
    /// input_schema), the machine-readable surface scripts/AI call. The input schema is JSON
    /// Schema (draft 2020-12) built from each command's `ParamSpec` list. `schemars::Schema`
    /// validates/normalizes the object so the output is canonical (DESIGN 6.13).
    pub fn tool_schemas(&self) -> Vec<Value> {
        self.commands
            .iter()
            .map(|c| {
                let mut properties = serde_json::Map::new();
                let mut required: Vec<Value> = Vec::new();
                for p in &c.params {
                    properties.insert(p.name.clone(), param_json_schema(p.ty));
                    required.push(Value::from(p.name.clone()));
                }
                let input = json!({
                    "type": "object",
                    "properties": Value::Object(properties),
                    "required": required,
                    "additionalProperties": false,
                });
                let schema = schemars::Schema::try_from(input).expect("well-formed JSON Schema");
                let description = if c.meta.description.is_empty() {
                    c.meta.title.clone()
                } else {
                    c.meta.description.clone()
                };
                json!({
                    "name": c.meta.id,
                    "description": description,
                    "input_schema": schema,
                })
            })
            .collect()
    }

    /// Return `(id, title)` pairs for commands whose id or title contains `query`
    /// (case-insensitive). An empty query matches every command. Used to drive the command
    /// palette's incremental search. Results are in registration order.
    pub fn filter(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_ascii_lowercase();
        self.commands
            .iter()
            .filter(|c| contains_ci(&c.meta.id, &q) || contains_ci(&c.meta.title, &q))
            .map(|c| (c.meta.id.clone(), c.meta.title.clone()))
            .collect()
    }

    /// Count commands matching `query` (same predicate as `filter`), without building any
    /// intermediate Vec or the manifest JSON.
    pub fn filter_count(&self, query: &str) -> usize {
        let q = query.to_ascii_lowercase();
        self.commands
            .iter()
            .filter(|c| contains_ci(&c.meta.id, &q) || contains_ci(&c.meta.title, &q))
            .count()
    }

    /// Return the id of the `n`th command matching `query` (same predicate as `filter`),
    /// without building any intermediate Vec or the manifest JSON.
    pub fn nth_matching_id(&self, query: &str, n: usize) -> Option<String> {
        let q = query.to_ascii_lowercase();
        self.commands
            .iter()
            .filter(|c| contains_ci(&c.meta.id, &q) || contains_ci(&c.meta.title, &q))
            .nth(n)
            .map(|c| c.meta.id.clone())
    }
}

/// Allocation-free ASCII case-insensitive substring test. `needle_lower` is assumed to be
/// already ASCII-lowercased; an empty needle matches everything.
fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle_lower.len())
        .any(|w| w.eq_ignore_ascii_case(needle_lower.as_bytes()))
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

/// Parse a color argument into a bare `Color` (reuses [`arg_fill`]'s hex/object parsing).
fn arg_color(args: &Value, key: &str) -> Option<Color> {
    match arg_fill(args, key)? {
        Fill::Solid(c) => Some(c),
        _ => None,
    }
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
        CommandMeta::new("shape.delete", "Delete Shape", "Shape")
            .describe("Delete the target shape (despawn the entity)."),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |_world, args| match arg_entity(args, "entity") {
            Some(entity) => vec![Operation::Despawn { entity }],
            None => vec![],
        },
    );

    // shape.set_fill — replace the entity's Fill with a solid color.
    reg.register(
        CommandMeta::new("shape.set_fill", "Set Fill", "Shape")
            .describe("Set the shape fill to a solid color."),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("color", ParamType::Color),
        ],
        |_world, args| match (arg_entity(args, "entity"), arg_fill(args, "color")) {
            (Some(entity), Some(fill)) => edit::set_fill(entity, fill).ops,
            _ => vec![],
        },
    );

    // shape.fill_gradient — replace the entity's Fill with a two-stop linear gradient.
    reg.register(
        CommandMeta::new("shape.fill_gradient", "Gradient Fill", "Shape")
            .describe("Set the shape fill to a two-stop linear gradient."),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("from", ParamType::Color),
            ParamSpec::new("to", ParamType::Color),
            ParamSpec::new("angle", ParamType::Float),
        ],
        |_world, args| match (
            arg_entity(args, "entity"),
            arg_color(args, "from"),
            arg_color(args, "to"),
        ) {
            (Some(entity), Some(from), Some(to)) => {
                let angle_deg = args.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                edit::set_fill(
                    entity,
                    Fill::Linear {
                        from,
                        to,
                        angle_deg,
                    },
                )
                .ops
            }
            _ => vec![],
        },
    );

    // shape.move — translate the entity's frame by (dx, dy); reads the current frame.
    reg.register(
        CommandMeta::new("shape.move", "Move Shape", "Shape")
            .describe("Translate the shape's frame by (dx, dy)."),
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
        CommandMeta::new("shape.set_position", "Set Position", "Shape").describe(
            "Set the shape's frame origin to an absolute point (in points), keeping its size.",
        ),
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
        CommandMeta::new("shape.set_size", "Set Size", "Shape").describe(
            "Set the shape's frame size to an absolute size (in points), keeping its origin.",
        ),
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
        CommandMeta::new("shape.bring_to_front", "Bring to Front", "Shape")
            .describe("Move the shape to the front (largest Order) among its siblings."),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |world, args| match arg_entity(args, "entity") {
            Some(entity) => reorder_to_edge(world, entity, Edge::Front),
            None => vec![],
        },
    );

    // shape.send_to_back — symmetric: move the entity to the back (smallest Order) among
    // its siblings.
    reg.register(
        CommandMeta::new("shape.send_to_back", "Send to Back", "Shape")
            .describe("Move the shape to the back (smallest Order) among its siblings."),
        vec![ParamSpec::new("entity", ParamType::Entity)],
        |world, args| match arg_entity(args, "entity") {
            Some(entity) => reorder_to_edge(world, entity, Edge::Back),
            None => vec![],
        },
    );

    // shape.set_rotation — set the entity's Rotation (degrees clockwise). Built directly as a
    // SetComponent op so this does not depend on the (separately evolving) edit::set_rotation.
    reg.register(
        CommandMeta::new("shape.set_rotation", "Set Rotation", "Shape")
            .describe("Set the shape's rotation (degrees clockwise)."),
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
        CommandMeta::new("shape.set_opacity", "Set Opacity", "Style")
            .describe("Set the shape's opacity (clamped to 0.0..=1.0)."),
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
        CommandMeta::new("shape.reset_rotation", "Reset Rotation", "Shape")
            .describe("Clear any rotation by setting it back to 0 degrees."),
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
        CommandMeta::new("shape.rotate_by", "Rotate By", "Shape")
            .describe("Add the given degrees to the shape's current rotation."),
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
            )
            .describe(format!("Set the shape fill to theme accent color {n}.")),
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
        CommandMeta::new("shape.set_text", "Set Text", "Text").describe(
            "Set the shape's text to a single string, preserving the first run's formatting.",
        ),
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
        CommandMeta::new("shape.set_font_size", "Set Font Size", "Text")
            .describe("Set every run's font size to the given points (minimum 1pt)."),
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

    // shape.set_font — set every run's font family to the given name.
    reg.register(
        CommandMeta::new("shape.set_font", "Set Font", "Text")
            .describe("Set every run's font family to the given name."),
        vec![
            ParamSpec::new("entity", ParamType::Entity),
            ParamSpec::new("family", ParamType::String),
        ],
        |world, args| {
            let family = args.get("family").and_then(Value::as_str);
            match (arg_entity(args, "entity"), family) {
                (Some(entity), Some(family)) => match world.texts.get(&entity) {
                    Some(existing) => {
                        let mut body = existing.clone();
                        for para in &mut body.paragraphs {
                            for run in &mut para.runs {
                                run.font = FontRef::Family(family.to_string());
                            }
                        }
                        vec![Operation::SetComponent {
                            entity,
                            value: CompValue::Text(body),
                        }]
                    }
                    None => vec![],
                },
                _ => vec![],
            }
        },
    );

    // shape.toggle_bold / shape.toggle_italic / shape.toggle_underline — flip the named run
    // attribute across the whole text box. To keep the box consistent, the new value is the
    // negation of the FIRST run's current value, then applied to every run. Reads the current
    // TextBody from the World; an entity with no text yields no ops.
    for (id, title, attr, desc) in [
        (
            "shape.toggle_bold",
            "Toggle Bold",
            RunAttr::Bold,
            "Toggle bold across the whole text box.",
        ),
        (
            "shape.toggle_italic",
            "Toggle Italic",
            RunAttr::Italic,
            "Toggle italic across the whole text box.",
        ),
        (
            "shape.toggle_underline",
            "Toggle Underline",
            RunAttr::Underline,
            "Toggle underline across the whole text box.",
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Text").describe(desc),
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
    for (id, title, halign, desc) in [
        (
            "shape.align_text_left",
            "Align Text Left",
            HAlign::Left,
            "Set every paragraph's horizontal alignment to left.",
        ),
        (
            "shape.align_text_center",
            "Align Text Center",
            HAlign::Center,
            "Set every paragraph's horizontal alignment to center.",
        ),
        (
            "shape.align_text_right",
            "Align Text Right",
            HAlign::Right,
            "Set every paragraph's horizontal alignment to right.",
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Text").describe(desc),
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
    for (id, title, rgba, desc) in [
        (
            "shape.fill_black",
            "Fill: Black",
            Rgba::BLACK,
            "Set the shape fill to literal black.",
        ),
        (
            "shape.fill_white",
            "Fill: White",
            Rgba::WHITE,
            "Set the shape fill to literal white.",
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Style").describe(desc),
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
    for (id, title, how, desc) in [
        (
            "shapes.align_left",
            "Align Left",
            Align::Left,
            "Align the selected shapes' left edges to the group bounding box.",
        ),
        (
            "shapes.align_hcenter",
            "Align Center (Horizontal)",
            Align::HCenter,
            "Align the selected shapes' horizontal centers to the group bounding box.",
        ),
        (
            "shapes.align_right",
            "Align Right",
            Align::Right,
            "Align the selected shapes' right edges to the group bounding box.",
        ),
        (
            "shapes.align_top",
            "Align Top",
            Align::Top,
            "Align the selected shapes' top edges to the group bounding box.",
        ),
        (
            "shapes.align_vcenter",
            "Align Middle (Vertical)",
            Align::VCenter,
            "Align the selected shapes' vertical centers to the group bounding box.",
        ),
        (
            "shapes.align_bottom",
            "Align Bottom",
            Align::Bottom,
            "Align the selected shapes' bottom edges to the group bounding box.",
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Arrange").describe(desc),
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
    for (id, title, axis, desc) in [
        (
            "shapes.distribute_h",
            "Distribute Horizontally",
            Axis::Horizontal,
            "Evenly distribute the selected shapes along the horizontal axis.",
        ),
        (
            "shapes.distribute_v",
            "Distribute Vertically",
            Axis::Vertical,
            "Evenly distribute the selected shapes along the vertical axis.",
        ),
    ] {
        reg.register(
            CommandMeta::new(id, title, "Arrange").describe(desc),
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

/// Map a command parameter type to a JSON Schema property.
fn param_json_schema(ty: ParamType) -> Value {
    match ty {
        ParamType::Entity => json!({ "type": "integer", "minimum": 0, "description": "entity id" }),
        ParamType::Int => json!({ "type": "integer" }),
        ParamType::Float => json!({ "type": "number" }),
        ParamType::String => json!({ "type": "string" }),
        ParamType::Color => {
            json!({ "type": "string", "description": "#RRGGBB hex or a theme token (e.g. accent1)" })
        }
        ParamType::Bool => json!({ "type": "boolean" }),
    }
}

/// Render the human-readable scripting-API reference (Markdown) from a registry: commands
/// grouped by category, each with id, params (name: type), and description (falling back to the
/// title). Generated from the registry so it never drifts (see the golden test).
pub fn render_scripting_api_markdown(reg: &CommandRegistry) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    out.push_str("# HayateOffice scripting-API reference\n\n");
    out.push_str(
        "GENERATED FILE — do not edit by hand. Regenerate with \
         `REGEN_DOCS=1 cargo test -p hayate-core scripting_api`.\n\n",
    );
    out.push_str(
        "Each command is exposed to Rhai scripts as the listed function (positional arguments \
         in the shown order). Argument types: `entity` = integer id, `color` = string \
         (`\"#RRGGBB\"` or a theme token like `\"accent1\"`), `int`/`float` = number, `bool`, \
         `string`. The read-only helper `entities()` returns the ids of all shapes. A whole \
         script is applied as one undoable change.\n",
    );

    let manifest = reg.manifest();

    // Categories in first-appearance (registration) order.
    let mut categories: Vec<&str> = Vec::new();
    for v in &manifest {
        let cat = v["category"].as_str().unwrap_or_default();
        if !categories.contains(&cat) {
            categories.push(cat);
        }
    }

    for cat in categories {
        let _ = write!(out, "\n## {cat}\n\n");
        out.push_str("| Script call | Command id | Description |\n");
        out.push_str("| --- | --- | --- |\n");
        for v in manifest.iter().filter(|v| v["category"] == cat) {
            let id = v["id"].as_str().unwrap_or_default();
            let title = v["title"].as_str().unwrap_or_default();
            let desc = match v["description"].as_str().unwrap_or_default() {
                "" => title,
                d => d,
            };
            // The Rhai function name is the id with non-identifier chars turned into `_`.
            let func: String = id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let arg_names: Vec<&str> = v["params"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|p| p["name"].as_str()).collect())
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "| `{func}({})` | `{id}` | {desc} |",
                arg_names.join(", ")
            );
        }
    }

    out
}

/// Render the tool-schema catalogue as pretty JSON (one JSON-Schema tool def per command).
pub fn render_tool_schemas_json(reg: &CommandRegistry) -> String {
    serde_json::to_string_pretty(&reg.tool_schemas()).unwrap() + "\n"
}

#[cfg(test)]
mod tests;
