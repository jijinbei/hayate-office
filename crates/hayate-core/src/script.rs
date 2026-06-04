//! Sandboxed Rhai scripting runtime (DESIGN 6.x extension system).
//!
//! Scripts drive the SAME [`CommandRegistry`] surface the palette and AI use. Every registered
//! command is exposed to Rhai as a *generated, typed function* whose name is the command id with
//! dots replaced by underscores (`shape.set_fill` -> `shape_set_fill`) and whose positional
//! arguments map, in schema order, to the command's parameters. So a script reads like:
//!
//! ```text
//! for e in entities() {
//!     shape_set_fill(e, "#ff0000");
//!     shape_move(e, 100, 0);
//! }
//! ```
//!
//! Semantics:
//! - Each command call runs the registry handler against a *scratch* clone of the document
//!   [`World`], so later calls and queries observe the effects of earlier ones.
//! - Every operation produced during the run is accumulated; [`run_script`] returns them so the
//!   caller can commit the whole script as ONE undoable transaction.
//! - The runtime is sandboxed: no file/network/system access is exposed, `eval` is disabled, and
//!   operation/among other limits are capped so a script cannot hang or exhaust memory.
//!
//! Read-only host helpers: `entities()` returns the ids of all live entities.

use std::any::TypeId;
use std::cell::RefCell;
use std::rc::Rc;

use hayate_ir::world::World;
use hayate_model::Operation;
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use serde_json::{json, Value};

use crate::{CommandRegistry, ParamType};

/// Sandbox limits applied to every script engine.
const MAX_OPERATIONS: u64 = 5_000_000;
const MAX_CALL_LEVELS: usize = 64;
const MAX_STRING_SIZE: usize = 256 * 1024;
const MAX_ARRAY_SIZE: usize = 100_000;
const MAX_MAP_SIZE: usize = 100_000;

/// What a script run produced.
#[derive(Debug, Default)]
pub struct ScriptOutcome {
    /// Forward operations issued by the script, in order. Wrap these in one [`Transaction`] to
    /// apply them to the real document as a single undo step.
    pub ops: Vec<Operation>,
    /// Anything the script printed via `print`/`debug`.
    pub log: Vec<String>,
}

/// A script that failed to compile or run.
#[derive(Debug, Clone)]
pub struct ScriptError(pub String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ScriptError {}

/// Mutable state shared by every host function during a single run.
struct ScriptState {
    /// Scratch document the script mutates so calls observe each other's effects.
    world: World,
    /// Accumulated forward operations (the script's net effect).
    ops: Vec<Operation>,
    /// `print`/`debug` output.
    log: Vec<String>,
}

/// The Rhai-callable name for a command id: dots (and any non-identifier char) become `_`.
fn fn_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Map a manifest `"type"` tag back to a [`ParamType`] (defaults to String on the off chance of
/// an unknown tag, which only affects argument coercion).
fn param_type(tag: &str) -> ParamType {
    match tag {
        "entity" => ParamType::Entity,
        "int" => ParamType::Int,
        "float" => ParamType::Float,
        "color" => ParamType::Color,
        "bool" => ParamType::Bool,
        _ => ParamType::String,
    }
}

/// Coerce one Rhai argument into the JSON value the command handler expects. Lenient: an
/// int/float mismatch is bridged, anything truly unconvertible becomes JSON null (the handlers
/// treat a missing/ill-typed field as a no-op).
fn arg_to_json(d: &Dynamic, ty: ParamType) -> Value {
    match ty {
        ParamType::Entity | ParamType::Int => {
            if let Ok(i) = d.as_int() {
                json!(i)
            } else if let Ok(f) = d.as_float() {
                json!(f as i64)
            } else {
                Value::Null
            }
        }
        ParamType::Float => {
            if let Ok(f) = d.as_float() {
                json!(f)
            } else if let Ok(i) = d.as_int() {
                json!(i as f64)
            } else {
                Value::Null
            }
        }
        ParamType::String | ParamType::Color => {
            if d.is_string() {
                json!(d.clone().into_string().unwrap_or_default())
            } else {
                Value::Null
            }
        }
        ParamType::Bool => match d.as_bool() {
            Ok(b) => json!(b),
            Err(_) => Value::Null,
        },
    }
}

/// One command's calling shape, captured for its generated Rhai function.
struct CommandSig {
    id: String,
    name: String,
    params: Vec<(String, ParamType)>,
}

/// Read each command's id + parameter (name, type) list out of the registry manifest.
fn command_sigs(registry: &CommandRegistry) -> Vec<CommandSig> {
    registry
        .manifest()
        .into_iter()
        .filter_map(|c| {
            let id = c.get("id")?.as_str()?.to_string();
            let params = c
                .get("params")
                .and_then(Value::as_array)
                .map(|ps| {
                    ps.iter()
                        .filter_map(|p| {
                            let name = p.get("name")?.as_str()?.to_string();
                            let ty = param_type(p.get("type")?.as_str()?);
                            Some((name, ty))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(CommandSig {
                name: fn_name(&id),
                id,
                params,
            })
        })
        .collect()
}

/// Run `src` against a clone of `world`, exposing every command in `registry` as a generated
/// typed function plus the `entities()` query. Returns the operations the script issued (to be
/// committed as one transaction) and its print log, or a [`ScriptError`] on compile/run failure.
pub fn run_script(
    registry: Rc<CommandRegistry>,
    world: &World,
    src: &str,
) -> Result<ScriptOutcome, ScriptError> {
    let state = Rc::new(RefCell::new(ScriptState {
        world: world.clone(),
        ops: Vec::new(),
        log: Vec::new(),
    }));

    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    engine.set_max_string_size(MAX_STRING_SIZE);
    engine.set_max_array_size(MAX_ARRAY_SIZE);
    engine.set_max_map_size(MAX_MAP_SIZE);
    engine.disable_symbol("eval");

    // print/debug -> log.
    {
        let st = Rc::clone(&state);
        engine.on_print(move |s| st.borrow_mut().log.push(s.to_string()));
    }
    {
        let st = Rc::clone(&state);
        engine.on_debug(move |s, _src, _pos| st.borrow_mut().log.push(s.to_string()));
    }

    // entities() -> array of live entity ids.
    {
        let st = Rc::clone(&state);
        engine.register_fn("entities", move || -> Array {
            st.borrow()
                .world
                .iter()
                .map(|e| Dynamic::from(e.0 as i64))
                .collect()
        });
    }

    // One generated typed function per command.
    for sig in command_sigs(&registry) {
        let arg_types: Vec<TypeId> = vec![TypeId::of::<Dynamic>(); sig.params.len()];
        let reg = Rc::clone(&registry);
        let st = Rc::clone(&state);
        let id = sig.id.clone();
        let params = sig.params.clone();
        engine.register_raw_fn(
            &sig.name,
            &arg_types,
            move |_ctx, args| -> Result<Dynamic, Box<EvalAltResult>> {
                let mut map = serde_json::Map::new();
                for (i, (pname, pty)) in params.iter().enumerate() {
                    map.insert(pname.clone(), arg_to_json(&args[i].clone(), *pty));
                }
                let args_json = Value::Object(map);
                let mut state = st.borrow_mut();
                // Build against the scratch world (immutable borrow ends when the owned
                // Transaction is returned), then accumulate + apply forward to the scratch.
                if let Some(tx) = reg.build(&id, &args_json, &state.world) {
                    state.ops.extend(tx.ops.iter().cloned());
                    let _ = tx.apply(&mut state.world);
                }
                Ok(Dynamic::UNIT)
            },
        );
    }

    engine.run(src).map_err(|e| ScriptError(e.to_string()))?;

    // Drop the engine so its captured `Rc`s release, leaving us the only owner of `state`.
    drop(engine);
    let state = Rc::try_unwrap(state)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| {
            let s = rc.borrow();
            ScriptState {
                world: s.world.clone(),
                ops: s.ops.clone(),
                log: s.log.clone(),
            }
        });
    Ok(ScriptOutcome {
        ops: state.ops,
        log: state.log,
    })
}

#[cfg(test)]
mod tests;
