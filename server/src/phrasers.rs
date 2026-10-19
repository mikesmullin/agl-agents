//! Phrasers: the PRESENTATION half of the old agent, rebuilt for mail1.
//!
//! A phraser matches with the **same deterministic syntax as a rule**
//! (`match:` / `unless:` over sender / subject / snippet / label / unread /
//! starred / inbox) but asks a **different model a different question**: not
//! the System One endpoint for a routing decision, but the text LLM for a
//! rewording. Whatever the LLM returns is shown **in place of the email's
//! subject and summary** in the list.
//!
//! ```yaml
//! name: nextdoor digest in one line
//! match:
//!   sender: .+@.*nextdoor\.com
//! prompt: |
//!   This is a neighborhood digest. One line, plain words, no emoji:
//!   SUBJECT: {{subject}}
//!   BODY: {{body}}
//! max_tokens: 120
//! ```
//!
//! `{{from}}`, `{{subject}}`, `{{snippet}}` and `{{body}}` are substituted
//! before sending. The reply is parsed for `SUBJECT:` / `SUMMARY:` lines —
//! first line becomes the subject, the rest the summary — and falls back to
//! the whole reply as the summary when the model ignores the format.
//!
//! Results persist in the `phrase` table (not in memory like recommendations),
//! so a rephrase survives refresh, re-poll and restart, and every row the list
//! returns already carries its phrase. Clearing is explicit: DELETE
//! /api/phrasers drops the whole table.
//!
//! ## The two-model workflow
//!
//! One GPU, two slots: the text LLM serves :1234 and the System One model
//! :1237, and they cannot run together. So the workflow is manual — stop the
//! jevlike, start the text model, press Phrase, swap back — and the backend
//! treats a missing LLM as a reportable fact: per-email `ok:false` with the
//! connection error, never a silent gap.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::rules::{clause_is_empty, clause_matches, clause_of, Clause};

/// The default endpoint: the text-LLM slot. Override with `LLM_URL`.
pub const DEFAULT_URL: &str = "http://127.0.0.1:1234";
/// Body characters offered to the prompt. A rewording needs more than the 240
/// the router sees, but not a whole marketing email.
const BODY_CAP: usize = 4000;

pub struct Phraser {
    pub file: String,
    pub name: String,
    pub enabled: bool,
    pub matches: Clause,
    pub unless: Clause,
    pub prompt: String,
    pub max_tokens: i64,
}

impl Phraser {
    fn applies(&self, row: &Value) -> bool {
        clause_matches(&self.matches, row) && !clause_matches(&self.unless, row)
    }

    pub fn public(&self) -> Value {
        json!({
            "file": self.file,
            "name": self.name,
            "enabled": self.enabled,
            "max_tokens": self.max_tokens,
        })
    }
}

/// Read `phrasers/*.yaml`, sorted by file name like the rules.
pub fn load(root: &Path) -> (Vec<Phraser>, Vec<String>) {
    let dir = root.join("phrasers");
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let Ok(reads) = std::fs::read_dir(&dir) else {
        return (out, errors);
    };
    let mut files: Vec<PathBuf> = reads
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "yaml" || e == "yml")
                .unwrap_or(false)
        })
        .collect();
    files.sort();

    for path in files {
        let file = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("{file}: {e}"));
                continue;
            }
        };
        let doc: Value = match serde_yaml::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{file}: not valid YAML: {e}"));
                continue;
            }
        };
        let Some(obj) = doc.as_object() else {
            errors.push(format!("{file}: expected a mapping"));
            continue;
        };
        let (matches, mut errs) = clause_of(obj.get("match"));
        if clause_is_empty(&matches) {
            errs.push(format!("{file}: `match` names nothing to match on"));
        }
        let (unless, unless_errs) = clause_of(obj.get("unless"));
        errs.extend(unless_errs);
        let prompt = obj
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if prompt.trim().is_empty() {
            errs.push(format!("{file}: `prompt` is required — it is the whole point of a phraser"));
        }
        if !errs.is_empty() {
            errors.extend(errs);
            continue;
        }
        out.push(Phraser {
            file: file.clone(),
            name: obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| file.trim_end_matches(".yaml"))
                .to_string(),
            enabled: obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            matches,
            unless,
            prompt,
            max_tokens: obj
                .get("max_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(150)
                .clamp(16, 2000),
        });
    }
    (out, errors)
}

/// Fill the template: `{{from}}`, `{{subject}}`, `{{snippet}}`, `{{body}}`.
/// Unknown `{{…}}` is left verbatim rather than blanked — a typo in a template
/// should show, not silently eat the prompt.
pub fn render(row: &Value, template: &str) -> String {
    let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let from = if s("fromName").is_empty() {
        s("fromEmail")
    } else {
        format!("{} <{}>", s("fromName"), s("fromEmail"))
    };
    let flat = |raw: String| {
        raw.chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let body = flat(s("bodyText"));
    let body: String = body.chars().take(BODY_CAP).collect();
    template
        .replace("{{from}}", &from)
        .replace("{{subject}}", &s("subject"))
        .replace("{{snippet}}", &flat(s("snippet")))
        .replace("{{body}}", &body)
}

/// Split the reply into the subject and summary the list will show. The prompt
/// asks for `SUBJECT:` / `SUMMARY:` lines; when the model ignores the format,
/// the first line becomes the subject and the rest the summary, and a one-line
/// reply keeps the original subject and replaces only the summary.
pub fn split_reply(text: &str, orig_subject: &str) -> (String, String) {
    let mut subject = String::new();
    let mut summary = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("SUBJECT:").or_else(|| t.strip_prefix("Subject:")) {
            subject = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("SUMMARY:").or_else(|| t.strip_prefix("Summary:")) {
            summary.push(rest.trim().to_string());
        } else {
            summary.push(t.to_string());
        }
    }
    if subject.is_empty() && !summary.is_empty() && text.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
        subject = summary.remove(0);
    }
    if subject.is_empty() {
        subject = orig_subject.to_string();
    }
    let mut out_summary = summary.join(" ");
    if out_summary.is_empty() {
        out_summary = text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    (subject, out_summary)
}

// ---------------------------------------------------------------------------
// the text LLM (OpenAI-compatible chat completions on the :1234 slot)
// ---------------------------------------------------------------------------

pub struct Llm {
    agent: ureq::Agent,
    pub base: String,
}

impl Llm {
    pub fn new(base: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(180))
                .build(),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn healthy(&self) -> bool {
        self.agent
            .get(&format!("{}/health", self.base))
            .call()
            .map(|r| r.status() < 400)
            .unwrap_or(false)
    }

    /// One chat completion, no streaming. Every profile on the :1234 slot
    /// serves model id `default` (the inference manager guarantees it), so the
    /// phraser never names a model.
    pub fn complete(&self, prompt: &str, max_tokens: i64) -> Result<String, String> {
        let body = json!({
            "model": "default",
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens,
            "stream": false,
        });
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    format!("LLM answered {code}: {}", r.into_string().unwrap_or_default())
                }
                other => format!(
                    "cannot reach the LLM at {} — start a text model on :1234 first ({other})",
                    self.base
                ),
            })?;
        let text = resp.into_string().unwrap_or_default();
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("LLM returned bad JSON: {e}"))?;
        v.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("LLM reply had no choices[0].message.content: {}", text.chars().take(200).collect::<String>()))
    }

    /// One chat completion with an explicit system + user message. Same slot and
    /// `default` model as [`complete`]; this is what rule generation uses. `system`
    /// carries the standing instructions; `user` the per-request data.
    pub fn ask(&self, system: &str, user: &str, max_tokens: i64) -> Result<String, String> {
        let body = json!({
            "model": "default",
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "max_tokens": max_tokens,
            "stream": false,
            // This call wants a YAML document, not a trace. Thinking on eats
            // the budget and leaves `content` empty (finish_reason: length).
            "chat_template_kwargs": { "enable_thinking": false },
        });
        self._chat(&body)
    }

    /// One chat completion, returning the assistant message as JSON. Tool calls
    /// live on that object (`tool_calls`); `content` may be null when the model
    /// called a tool instead of writing text.
    pub fn chat(&self, body: &Value) -> Result<Value, String> {
        let v = self.post_chat(body)?;
        v.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| "LLM reply had no choices[0].message".to_string())
    }

    fn post_chat(&self, body: &Value) -> Result<Value, String> {
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    format!("LLM answered {code}: {}", r.into_string().unwrap_or_default())
                }
                other => format!(
                    "cannot reach the LLM at {} -- start a text model on :1234 first ({other})",
                    self.base
                ),
            })?;
        let text = resp.into_string().unwrap_or_default();
        serde_json::from_str(&text).map_err(|e| format!("LLM returned bad JSON: {e}"))
    }

    /// POST the chat-completions body and return `message.content`. A reply
    /// that only thought, and wrote nothing, is an error the caller can show.
    fn _chat(&self, body: &Value) -> Result<String, String> {
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    format!("LLM answered {code}: {}", r.into_string().unwrap_or_default())
                }
                other => format!(
                    "cannot reach the LLM at {} -- start a text model on :1234 first ({other})",
                    self.base
                ),
            })?;
        let text = resp.into_string().unwrap_or_default();
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("LLM returned bad JSON: {e}"))?;
        let message = v
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"));
        let content = message
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(
                "the model did not return a rule — it spent the reply thinking instead of writing YAML. Try again."
                    .to_string(),
            );
        }
        Ok(content)
    }
}

/// Phrase one email: the first enabled matching phraser wins (deterministic
/// order, filename-sorted — overlapping phrasers resolve to the first file,
// like mail rules resolving to the first rule).
pub fn evaluate(llm: &Llm, phrasers: &[Phraser], row: &Value) -> Value {
    let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let hit = phrasers.iter().find(|p| p.enabled && p.applies(row));
    let Some(p) = hit else {
        return json!({ "id": id, "ok": true, "matched": 0, "note": "no phraser matched" });
    };
    let prompt = render(row, &p.prompt);
    let t0 = std::time::Instant::now();
    let reply = match llm.complete(&prompt, p.max_tokens) {
        Ok(t) => t,
        Err(e) => {
            return json!({ "id": id, "ok": false, "error": e, "matched": 1, "rule": p.name });
        }
    };
    let ms = t0.elapsed().as_millis() as i64;
    let orig = row.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let (subject, summary) = split_reply(&reply, orig);
    json!({
        "id": id, "ok": true, "matched": 1, "rule": p.name,
        "subject": subject, "summary": summary, "ms": ms,
        "promptChars": prompt.chars().count(),
    })
}
