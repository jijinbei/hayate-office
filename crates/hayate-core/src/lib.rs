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

use hayate_ir::color::{Color, Rgba};
use hayate_ir::paint::Fill;
use hayate_ir::world::{Entity, World};
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
    pub fn new(id: impl Into<String>, title: impl Into<String>, category: impl Into<String>) -> Self {
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
}

// --- Arg parsing helpers (lenient) ---

/// Read an `Entity` from `args[key]` (a non-negative integer id).
fn arg_entity(args: &Value, key: &str) -> Option<Entity> {
    args.get(key)?.as_u64().map(Entity)
}

/// Read an `i64` from `args[key]`, accepting integer or float JSON numbers.
fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key)?;
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
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
        |world, args| {
            match (
                arg_entity(args, "entity"),
                arg_i64(args, "dx"),
                arg_i64(args, "dy"),
            ) {
                (Some(entity), Some(dx), Some(dy)) => edit::translate(world, entity, dx, dy).ops,
                _ => vec![],
            }
        },
    );

    reg
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
            .build("shape.move", &json!({ "entity": e.0, "dx": 100, "dy": 0 }), &w)
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
            Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(255, 0, 0)))))
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
            Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(0, 255, 0)))))
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

    #[test]
    fn manifest_lists_builtin_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(ids.contains(&"shape.delete"));
        assert!(ids.contains(&"shape.set_fill"));
        assert!(ids.contains(&"shape.move"));

        // Each entry carries the documented shape.
        let mv = manifest
            .iter()
            .find(|c| c["id"] == "shape.move")
            .unwrap();
        assert_eq!(mv["title"], "Move Shape");
        assert_eq!(mv["category"], "Shape");
        let params = mv["params"].as_array().unwrap();
        assert_eq!(params[0]["name"], "entity");
        assert_eq!(params[0]["type"], "entity");
    }
}
