//! Optional AI script authoring: turn a natural-language request into a Rhai script via the
//! Anthropic API. The self-repair loop runs in-app — HTTP on a background thread (it only moves
//! strings, so it stays `Send`), each candidate dry-run on the main thread (the script runtime's
//! `Rc`/engine are not `Send`), and the error fed back for the next round. The editor is fully
//! functional offline; this just needs `ANTHROPIC_API_KEY` when used.

use std::rc::Rc;

use gpui::{prelude::*, Context};
use serde_json::{json, Value};

use hayate_core::ScriptContext;

use crate::{HayateApp, ScriptPanel};

const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const MAX_ATTEMPTS: usize = 3;

/// Strip a Markdown code fence (``` or ```rhai … ```) from model output, returning just the code.
fn strip_code_fences(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // Drop the optional language tag on the fence's first line, then the closing fence.
        let after_lang = rest.splitn(2, '\n').nth(1).unwrap_or("");
        let body = after_lang.trim_end();
        return body.strip_suffix("```").unwrap_or(body).trim().to_string();
    }
    t.to_string()
}

/// One blocking call to the Anthropic Messages API. `prior` is (source, error) of a failed
/// attempt, appended so the model can repair it. Returns the (fence-stripped) script source.
fn anthropic_generate(
    api_key: &str,
    model: &str,
    system: &str,
    request: &str,
    prior: Option<&(String, String)>,
) -> Result<String, String> {
    let mut user = format!("Request: {request}");
    if let Some((src, err)) = prior {
        user.push_str(&format!(
            "\n\nYour previous script failed to run with this error:\n{err}\n\nPrevious script:\n\
             {src}\n\nFix it and output ONLY the corrected Rhai source."
        ));
    }
    let body = json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    let resp = ureq::post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(|e| e.to_string())?;
    let v: Value = resp.into_json().map_err(|e| e.to_string())?;
    let text = v["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("unexpected API response: {v}"))?;
    Ok(strip_code_fences(text))
}

impl HayateApp {
    /// Author a script from a natural-language `request` via the Anthropic API, with up to
    /// `MAX_ATTEMPTS` self-repair rounds. On success the working script is loaded into the console
    /// for the user to review and run. Requires `ANTHROPIC_API_KEY` (and optional
    /// `ANTHROPIC_MODEL`); offline use is unaffected.
    pub(crate) fn ai_author(&mut self, request: String, cx: &mut Context<Self>) {
        let Some(api_key) = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
        else {
            self.notice = Some("AI 機能には環境変数 ANTHROPIC_API_KEY が必要です".to_string());
            return;
        };
        let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let system = hayate_core::system_prompt(&self.registry);
        self.notice = Some("AI がスクリプトを生成中…".to_string());

        cx.spawn(async move |this, cx| {
            let mut prior: Option<(String, String)> = None;
            for _ in 0..MAX_ATTEMPTS {
                let (key, model, system, request, p) = (
                    api_key.clone(),
                    model.clone(),
                    system.clone(),
                    request.clone(),
                    prior.clone(),
                );
                // HTTP off the main thread (strings only -> Send).
                let generated = cx
                    .background_spawn(async move {
                        anthropic_generate(&key, &model, &system, &request, p.as_ref())
                    })
                    .await;
                let src = match generated {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = this.update(cx, |a, cx| {
                            a.notice = Some(format!("AI 生成エラー\n{e}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                // Dry-run on the main thread (run_script is not Send).
                let check = this.update(cx, |a, _| {
                    let ctx = ScriptContext {
                        current_slide: Some(a.slide),
                        selection: a.selected_all(),
                    };
                    hayate_core::run_script(Rc::clone(&a.registry), &a.pres, &ctx, &src)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
                match check {
                    Ok(Ok(())) => {
                        let _ = this.update(cx, |a, cx| {
                            a.script_panel = Some(ScriptPanel {
                                buf: src,
                                scroll: gpui::ScrollHandle::new(),
                            });
                            a.notice = Some(
                                "AI が生成しました。確認して Ctrl+Enter で実行してください"
                                    .to_string(),
                            );
                            cx.notify();
                        });
                        return;
                    }
                    Ok(Err(e)) => prior = Some((src, e)), // feed the error back and retry
                    Err(_) => return,                     // editor went away
                }
            }
            let _ = this.update(cx, |a, cx| {
                a.notice = Some("AI が有効なスクリプトを生成できませんでした".to_string());
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::strip_code_fences;

    #[test]
    fn strips_fences() {
        assert_eq!(
            strip_code_fences("```rhai\nshape_move(1, 2, 3);\n```"),
            "shape_move(1, 2, 3);"
        );
        assert_eq!(strip_code_fences("```\nx\n```"), "x");
        assert_eq!(
            strip_code_fences("shape_move(1, 2, 3);"),
            "shape_move(1, 2, 3);"
        );
    }
}
