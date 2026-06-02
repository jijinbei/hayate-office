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
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_ir::paint::Fill;
use hayate_ir::units::EMU_PER_PT;
use hayate_ir::world::{CompKind, CompValue, Entity, World};
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

/// Read an `i64` from `args[key]`, accepting integer or float JSON numbers.
fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key)?;
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// Read an `f64` from `args[key]`, accepting integer or float JSON numbers.
fn arg_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key)?.as_f64()
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

    reg
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
    let sibling_orders = world.iter().filter(|&s| s != e).filter(|&s| {
        world.parent.get(&s).copied() == parent
    });

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
        assert!(hits
            .iter()
            .any(|(_, title)| title == "Fill: Accent 1"));
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
        assert_eq!(w.get(e, CompKind::Rotation), Some(CompValue::Rotation(33.0)));
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
        assert_eq!(w.get(e, CompKind::Rotation), Some(CompValue::Rotation(45.0)));

        // Undoable as one step (back to 10.0).
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Rotation), Some(CompValue::Rotation(10.0)));
    }

    #[test]
    fn manifest_includes_new_style_and_shape_commands() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(ids.contains(&"shape.set_opacity"));
        assert!(ids.contains(&"shape.reset_rotation"));
        assert!(ids.contains(&"shape.rotate_by"));
    }

    #[test]
    fn manifest_includes_set_rotation() {
        let reg = builtins();
        let manifest = reg.manifest();
        let ids: Vec<&str> = manifest
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
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
        let ids: Vec<&str> = manifest
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(ids.contains(&"shape.set_position"));
        assert!(ids.contains(&"shape.set_size"));
    }
}
