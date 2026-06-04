//! AI script authoring with a self-repair loop (DESIGN 6.x flagship).
//!
//! The loop is: build a system prompt from the *same* command registry the scripts call (the
//! JSON-Schema tool catalogue + the bundled few-shot examples), ask a [`ScriptGenerator`] for
//! Rhai source, dry-run it through [`run_script`] (which never touches the real document), and if
//! it errors feed the error back and try again — up to `max_attempts`. On success the caller gets
//! working source plus its dry-run [`ScriptOutcome`] to commit.
//!
//! The generator is abstracted so the LLM call is a pluggable seam: a real implementation sends
//! the prompt to Claude (optional-online; see DESIGN's AI authoring section), while tests use a
//! deterministic mock. Everything here is pure and offline.

use std::rc::Rc;

use hayate_ir::presentation::Presentation;

use crate::{run_script, CommandRegistry, ScriptContext, ScriptOutcome};

/// One generation attempt: the source produced and the error it hit (empty if generation itself
/// failed). Fed back to the generator so it can repair its previous output.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub source: String,
    pub error: String,
}

/// Produces Rhai source for a natural-language request. A real implementation calls an LLM with
/// `system` (the tool catalogue + examples) and `request`; `prior` is the previous failed attempt
/// for self-repair. Returns the raw script source (or an error string if generation failed).
pub trait ScriptGenerator {
    fn generate(
        &self,
        request: &str,
        system: &str,
        prior: Option<&Attempt>,
    ) -> Result<String, String>;
}

/// The system prompt: how to write scripts, the callable functions (JSON-Schema catalogue from
/// the registry), and the bundled few-shot examples. Built from the registry so it always matches
/// what scripts can actually call.
pub fn system_prompt(registry: &CommandRegistry) -> String {
    let mut s = String::new();
    s.push_str(
        "You write short Rhai scripts that edit a HayateOffice presentation. Call ONLY the \
         functions listed below. Conventions: entity ids are integers; colors are strings like \
         \"#RRGGBB\" or a theme token such as \"accent1\"; positions/sizes are in points. Use \
         current_slide(), selection(), shapes(slide) and entities() to find shapes; create \
         functions (shape_add_rect/ellipse/text) return the new entity id. Output ONLY Rhai \
         source — no prose, no code fences.\n\n## Callable functions (JSON Schema)\n",
    );
    s.push_str(&serde_json::to_string_pretty(&registry.tool_schemas()).unwrap_or_default());
    s.push_str("\n\n## Example scripts\n");
    for (name, src) in crate::script_examples() {
        s.push_str(&format!("// {name}\n{}\n\n", src.trim()));
    }
    s
}

/// Drive generate -> dry-run -> repair-on-error up to `max_attempts`. Returns the first working
/// source and its (uncommitted) outcome, or every failed attempt if none worked.
pub fn author_script<G: ScriptGenerator>(
    generator: &G,
    registry: Rc<CommandRegistry>,
    pres: &Presentation,
    ctx: &ScriptContext,
    request: &str,
    max_attempts: usize,
) -> Result<(String, ScriptOutcome), Vec<Attempt>> {
    let system = system_prompt(&registry);
    let mut attempts: Vec<Attempt> = Vec::new();
    for _ in 0..max_attempts.max(1) {
        let prior = attempts.last();
        let source = match generator.generate(request, &system, prior) {
            Ok(s) => s,
            Err(e) => {
                attempts.push(Attempt {
                    source: String::new(),
                    error: e,
                });
                continue;
            }
        };
        match run_script(Rc::clone(&registry), pres, ctx, &source) {
            Ok(out) => return Ok((source, out)),
            Err(e) => attempts.push(Attempt {
                source,
                error: e.to_string(),
            }),
        }
    }
    Err(attempts)
}

#[cfg(test)]
mod tests;
