//! Sandboxed Rhai scripting runtime (DESIGN 6.x extension system).
//!
//! Scripts drive the SAME [`CommandRegistry`] surface the palette and AI use. Every registered
//! command is exposed to Rhai as a *generated, typed function* whose name is the command id with
//! dots replaced by underscores (`shape.set_fill` -> `shape_set_fill`) and whose positional
//! arguments map, in schema order, to the command's parameters. Commands that create a shape
//! return the new entity id. So a script reads like:
//!
//! ```text
//! let s = current_slide();
//! let e = shape_add_rect(s, 100, 100, 200, 150);
//! shape_set_fill(e, "#ff0000");
//! for e in selection() { shape_move(e, 20, 0); }
//! ```
//!
//! Semantics:
//! - Each command call runs the registry handler against a *scratch* clone of the document, so
//!   later calls and queries observe the effects of earlier ones.
//! - Every operation produced during the run is accumulated; [`run_script`] returns them so the
//!   caller can commit the whole script as ONE undoable transaction (it does not touch the real
//!   document — running is effectively a dry run until the caller commits).
//! - Sandboxed: no file/network/system access, `eval` disabled, and operation/call-depth/size
//!   caps so a script cannot hang or exhaust memory.
//!
//! Read-only host helpers: `entities()`, `slides()`, `current_slide()`, `shapes(slide)`,
//! `selection()`, `frame(entity)` (`#{x,y,w,h}` in points), `text(entity)`.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use hayate_ir::presentation::Presentation;
use hayate_ir::world::{CompKind, CompValue, Entity};
use hayate_model::Operation;
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map};
use serde_json::{json, Value};

use crate::{CommandRegistry, ParamType};

/// Sandbox limits applied to every script engine.
const MAX_OPERATIONS: u64 = 5_000_000;
const MAX_CALL_LEVELS: usize = 64;
const MAX_STRING_SIZE: usize = 256 * 1024;
const MAX_ARRAY_SIZE: usize = 100_000;
const MAX_MAP_SIZE: usize = 100_000;

const EMU_PER_POINT: f64 = 12_700.0;

/// Editor context a script can read: the current slide and selection. The app fills this so
/// `current_slide()` / `selection()` reflect the live editor; defaults are empty.
#[derive(Debug, Default, Clone)]
pub struct ScriptContext {
    pub current_slide: Option<Entity>,
    pub selection: Vec<Entity>,
}

/// What a script run produced.
#[derive(Debug, Default)]
pub struct ScriptOutcome {
    /// Forward operations issued by the script, in order. Wrap these in one transaction to apply
    /// them to the real document as a single undo step.
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
    /// Scratch document the script mutates so calls observe each other's effects. Media bytes
    /// are intentionally NOT cloned (scripts don't touch them), avoiding a costly copy.
    pres: Presentation,
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

/// Map a manifest `"type"` tag back to a [`ParamType`] (defaults to String for an unknown tag,
/// which only affects argument coercion).
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
/// int/float mismatch is bridged; anything truly unconvertible becomes JSON null (handlers treat
/// a missing/ill-typed field as a no-op).
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
#[derive(Clone)]
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

/// Build an empty scratch presentation cloning structure but NOT media bytes.
fn scratch_of(pres: &Presentation) -> Presentation {
    Presentation {
        world: pres.world.clone(),
        slide_size: pres.slide_size,
        default_master: pres.default_master,
        media: BTreeMap::new(),
    }
}

/// Register every host function (commands + queries) on `engine`, wired to `state`/`ctx`.
fn register_all(
    engine: &mut Engine,
    registry: &Rc<CommandRegistry>,
    state: &Rc<RefCell<ScriptState>>,
    ctx: &ScriptContext,
) {
    // --- Read-only queries ---
    {
        let st = Rc::clone(state);
        engine.register_fn("entities", move || -> Array {
            st.borrow()
                .pres
                .world
                .iter()
                .map(|e| Dynamic::from(e.0 as i64))
                .collect()
        });
    }
    {
        let st = Rc::clone(state);
        engine.register_fn("slides", move || -> Array {
            st.borrow()
                .pres
                .slides()
                .into_iter()
                .map(|e| Dynamic::from(e.0 as i64))
                .collect()
        });
    }
    {
        let st = Rc::clone(state);
        engine.register_fn("shapes", move |slide: i64| -> Array {
            st.borrow()
                .pres
                .children(Entity(slide as u64))
                .into_iter()
                .map(|e| Dynamic::from(e.0 as i64))
                .collect()
        });
    }
    {
        let current = ctx.current_slide;
        engine.register_fn("current_slide", move || -> Dynamic {
            match current {
                Some(e) => Dynamic::from(e.0 as i64),
                None => Dynamic::UNIT,
            }
        });
    }
    {
        let sel: Vec<i64> = ctx.selection.iter().map(|e| e.0 as i64).collect();
        engine.register_fn("selection", move || -> Array {
            sel.iter().map(|&i| Dynamic::from(i)).collect()
        });
    }
    {
        let st = Rc::clone(state);
        engine.register_fn("frame", move |entity: i64| -> Dynamic {
            let st = st.borrow();
            match st.pres.world.get(Entity(entity as u64), CompKind::Frame) {
                Some(CompValue::Frame(r)) => {
                    let mut m = Map::new();
                    m.insert("x".into(), Dynamic::from(r.origin.x as f64 / EMU_PER_POINT));
                    m.insert("y".into(), Dynamic::from(r.origin.y as f64 / EMU_PER_POINT));
                    m.insert("w".into(), Dynamic::from(r.size.w as f64 / EMU_PER_POINT));
                    m.insert("h".into(), Dynamic::from(r.size.h as f64 / EMU_PER_POINT));
                    Dynamic::from_map(m)
                }
                _ => Dynamic::UNIT,
            }
        });
    }
    {
        let st = Rc::clone(state);
        engine.register_fn("text", move |entity: i64| -> String {
            let st = st.borrow();
            match st.pres.world.get(Entity(entity as u64), CompKind::Text) {
                Some(CompValue::Text(tb)) => tb
                    .paragraphs
                    .iter()
                    .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            }
        });
    }

    // --- One generated typed function per command ---
    for sig in command_sigs(registry) {
        let arg_types: Vec<TypeId> = vec![TypeId::of::<Dynamic>(); sig.params.len()];
        let reg = Rc::clone(registry);
        let st = Rc::clone(state);
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
                // Build against the scratch world (the immutable borrow ends when the owned
                // Transaction is returned), then accumulate + apply forward to the scratch.
                let result = match reg.build(&id, &args_json, &state.pres.world) {
                    Some(tx) => {
                        // A create command returns the id of the entity it spawned.
                        let spawned = tx.ops.iter().find_map(|op| match op {
                            Operation::Spawn { entity } => Some(entity.0 as i64),
                            _ => None,
                        });
                        state.ops.extend(tx.ops.iter().cloned());
                        let _ = tx.apply(&mut state.pres.world);
                        spawned.map(Dynamic::from).unwrap_or(Dynamic::UNIT)
                    }
                    None => Dynamic::UNIT,
                };
                Ok(result)
            },
        );
    }
}

/// A sandboxed engine with the given limits, but no host functions yet.
fn sandboxed_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    engine.set_max_string_size(MAX_STRING_SIZE);
    engine.set_max_array_size(MAX_ARRAY_SIZE);
    engine.set_max_map_size(MAX_MAP_SIZE);
    engine.disable_symbol("eval");
    engine
}

/// Run `src` against a scratch clone of `pres`, exposing every command as a generated typed
/// function plus the read-only query helpers. Returns the operations the script issued (commit
/// them as one transaction) and its print log, or a [`ScriptError`] on compile/run failure.
pub fn run_script(
    registry: Rc<CommandRegistry>,
    pres: &Presentation,
    ctx: &ScriptContext,
    src: &str,
) -> Result<ScriptOutcome, ScriptError> {
    let state = Rc::new(RefCell::new(ScriptState {
        pres: scratch_of(pres),
        ops: Vec::new(),
        log: Vec::new(),
    }));

    let mut engine = sandboxed_engine();
    {
        let st = Rc::clone(&state);
        engine.on_print(move |s| st.borrow_mut().log.push(s.to_string()));
    }
    {
        let st = Rc::clone(&state);
        engine.on_debug(move |s, _src, _pos| st.borrow_mut().log.push(s.to_string()));
    }
    register_all(&mut engine, &registry, &state, ctx);

    engine.run(src).map_err(|e| ScriptError(e.to_string()))?;

    // Drop the engine so its captured `Rc`s release, leaving us the sole owner of `state`.
    drop(engine);
    let state = Rc::try_unwrap(state)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| {
            let s = rc.borrow();
            ScriptState {
                pres: scratch_of(&s.pres),
                ops: s.ops.clone(),
                log: s.log.clone(),
            }
        });
    Ok(ScriptOutcome {
        ops: state.ops,
        log: state.log,
    })
}

/// The Rhai-callable surface as JSON (function signatures), for editor autocomplete / AI
/// context. Built from the same registry, so it matches what scripts can actually call.
pub fn script_api_metadata(registry: Rc<CommandRegistry>) -> String {
    let state = Rc::new(RefCell::new(ScriptState {
        pres: Presentation::new(),
        ops: Vec::new(),
        log: Vec::new(),
    }));
    let mut engine = sandboxed_engine();
    register_all(&mut engine, &registry, &state, &ScriptContext::default());
    engine.gen_fn_metadata_to_json(false).unwrap_or_default()
}

#[cfg(test)]
mod tests;
