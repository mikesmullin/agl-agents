//! Disk rules × a System One model: **the rules decide whether to ask, and what
//! to ask; the model decides which action the email deserves.**
//!
//! A rule is a YAML file in `rules/` (gitignored — a rule set is a personal
//! artifact, not source). It has two halves:
//!
//! ```yaml
//! name: gmail bulk mail
//! weight: 1.5              # default 1.0: how hard this rule's votes count
//! vision: true             # default false: attach a rendered screenshot
//! match:
//!   sender: .+@gmail\.com        # regex, tested against from_email *or* from_name
//!   subject: '(?i)newsletter|digest|weekly'
//! unless:
//!   label: Job/HSA               # a label name the message already carries
//! prompts:                       # one rule, as many questions as it wants
//!   - id: filing
//!     type: choice                # choice | noul | score (default choice)
//!     instructions: What should be done with this email?
//!     criteria:                   # the option names ARE mail1's action vocabulary
//!       archive: a bulk notification the reader does not need to keep
//!       spam: an unsolicited marketing blast selling something
//!       "move:Newsletters": a periodical the reader subscribed to and reads at leisure
//!       skip: needs the reader's own judgment
//!   - id: promo
//!     type: noul
//!     instructions: Is this a promotional blast trying to sell something?
//!   - id: urgency
//!     type: score
//!     instructions: How time-sensitive is this email?
//!     criteria: [ignore it, read this week, read today, act now]
//! ```
//!
//! The older single-question shape still parses — `ask:` becomes one `choice`
//! prompt (plus one `noul` prompt when `ask.noul` is present):
//!
//! ```yaml
//! ask:
//!   instructions: What should be done with this email?
//!   criteria: {archive: ..., spam: ...}
//!   noul: Is this a promotional blast?
//! ```
//!
//! ## Why the split is deterministic-then-model, not model-only
//!
//! The System One guidance is that the interesting work is *what goes in the
//! state and what you ask*, not prompting. A regex on the sender is a fact: it
//! either holds or it does not, it is reviewable, and it costs nothing. The
//! model's job is the judgment that a fact cannot carry — *which* of the actions
//! this particular email deserves. So `match` / `unless` are evaluated in Rust,
//! and only the emails that survive get a question.
//!
//! ## One call per email, one question per prompt
//!
//! Every matching rule contributes its prompts as questions in a **single**
//! request for that email. Parallel questions are the documented pattern
//! ("speculative fan-out": all questions in a request run in parallel, so adding
//! more typically does not add latency), and each rule keeps its own wording —
//! which is the point of writing rules at all: the prompt is specific to the
//! slice of mail the rule describes.
//!
//! ## Merging the answers
//!
//! Only `choice` prompts vote for the recommended action: each option's merged
//! score is its probability averaged — weighted by the owning rule's `weight:`
//! — over the prompts that offered it (a prompt that never offered `spam`
//! cannot vote against `spam`), then argmax. `noul` and `score` prompts are
//! reported as signals and never merged: a yes-ness of 0.88 is a fact to show,
//! not a vote to count.
//!
//! `confidence` is that merged mean — reported, not used as a gate: the
//! `laya-email` experiment measured confidence moving the *opposite* way to
//! accuracy (0.14 when right, 1.00 when wrong), so nothing here acts without a
//! press.
//!
//! ## The option names are the queue's vocabulary
//!
//! A choice criterion key is an action mail1 can already queue: `archive`,
//! `spam`, `delete`, `purge`, `read`, `unread`, `star`, `unstar`, `important`,
//! `unimportant`, `skip` (no-op: "leave it to me"), or `move:<Folder>` for a
//! folder. `move:Newsletters` is therefore both the model's answer and the
//! mutation the row will queue.
//!
//! ## Vision
//!
//! `vision: true` attaches a PNG screenshot of the rendered email (chromium
//! headless over the stored `bodyHtml`, cached under `data/.vision-cache/`).
//! The stock H2O shim is text-only and ignores the extra `images` field, so
//! this is forward plumbing: the screenshots are rendered, cached, reported
//! (`"images": n`), and sent — a vision-capable endpoint can read them, the
//! current one answers from the text alone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};
use ureq::Agent;

/// The default endpoint: the System One slot (`:1237`), not the text-LLM slot
/// (`:1234`). Override with `SYSTEMONE_URL`.
pub const DEFAULT_URL: &str = "http://127.0.0.1:1237";
/// Body characters carried into the state. Subject and sender carry nearly all
/// the signal; a whole marketing email is noise the model has to wade through.
const BODY_CAP: usize = 240;

// ---------------------------------------------------------------------------
// the rule
// ---------------------------------------------------------------------------

/// One `match:` / `unless:` clause. Every field is optional; a clause with none
/// is a parse error rather than "matches everything".
#[derive(Default)]
pub(crate) struct Clause {
    sender: Option<Regex>,
    subject: Option<Regex>,
    snippet: Option<Regex>,
    label: Option<String>,
    unread: Option<bool>,
    starred: Option<bool>,
    inbox: Option<bool>,
}

/// One question a rule asks. The `type` decides the shape of `criteria` and of
/// the answer, following the System One contract: `choice` takes an
/// option→description map, `score` an ordered array of level descriptions,
/// `noul` an optional true/false description map (or nothing at all).
pub struct Prompt {
    pub id: String,
    pub qtype: String,
    pub instructions: String,
    pub criteria: Value,
    pub threshold: Option<f64>,
}

pub struct Rule {
    pub file: String,
    pub name: String,
    pub enabled: bool,
    pub weight: f64,
    pub vision: bool,
    pub matches: Clause,
    pub unless: Clause,
    pub prompts: Vec<Prompt>,
    pub body_cap: usize,
}

impl Rule {
    fn applies(&self, row: &Value) -> bool {
        // `unless` is a veto, not a second filter: any clause that hits disqualifies.
        clause_matches(&self.matches, row) && !clause_matches(&self.unless, row)
    }

    pub fn public(&self) -> Value {
        json!({
            "file": self.file,
            "name": self.name,
            "enabled": self.enabled,
            "weight": self.weight,
            "vision": self.vision,
            "instructions": self.prompts.iter()
                .find(|p| p.qtype == "choice")
                .map(|p| p.instructions.clone())
                .unwrap_or_default(),
            "criteria": self.prompts.iter()
                .filter(|p| p.qtype == "choice")
                .flat_map(|p| p.criteria.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default())
                .collect::<Vec<_>>(),
            "prompts": self.prompts.iter().map(|p| json!({
                "id": p.id, "type": p.qtype, "instructions": p.instructions,
            })).collect::<Vec<_>>(),
        })
    }
}

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

pub(crate) fn regex_of(v: &Value) -> Option<Regex> {
    let raw = match v {
        Value::String(s) => s.clone(),
        // A bare `.+@gmail.com` in YAML is fine, but a value that parses as a
        // number or a bool is not a pattern someone meant to match mail with.
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Regex::new(&raw).ok()
}

/// Whether the clause names anything at all. An empty `match: {}` parses to no
/// fields and must be rejected by the loader — otherwise it would read as
/// "match everything". Shared with the phrasers' loader.
pub(crate) fn clause_is_empty(c: &Clause) -> bool {
    c.sender.is_none()
        && c.subject.is_none()
        && c.snippet.is_none()
        && c.label.is_none()
        && c.unread.is_none()
        && c.starred.is_none()
        && c.inbox.is_none()
}

/// One clause against one row: every named field must hold, and a clause that
/// names nothing matches nothing (never "everything"). Shared with the
/// phrasers, which match with the same syntax but ask a different model.
pub(crate) fn clause_matches(c: &Clause, row: &Value) -> bool {
    let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let from = format!("{} <{}>", s("fromName"), s("fromEmail"));
    let text = format!(
        "{} {}",
        s("snippet"),
        row.get("bodyText").and_then(|v| v.as_str()).unwrap_or("")
    );
    let names: Vec<String> = row
        .get("labelNames")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    let mut asked = false;
    let mut ok = true;
    if let Some(re) = &c.sender {
        asked = true;
        ok &= re.is_match(&s("fromEmail")) || re.is_match(&s("fromName")) || re.is_match(&from);
    }
    if let Some(re) = &c.subject {
        asked = true;
        ok &= re.is_match(&s("subject"));
    }
    if let Some(re) = &c.snippet {
        asked = true;
        ok &= re.is_match(&text);
    }
    if let Some(l) = &c.label {
        asked = true;
        let want = l.to_lowercase();
        ok &= names.iter().any(|n| n.to_lowercase() == want);
    }
    if let Some(want) = c.unread {
        asked = true;
        ok &= row.get("unread").and_then(|v| v.as_bool()).unwrap_or(false) == want;
    }
    if let Some(want) = c.starred {
        asked = true;
        ok &= row.get("starred").and_then(|v| v.as_bool()).unwrap_or(false) == want;
    }
    if let Some(want) = c.inbox {
        asked = true;
        ok &= row.get("inInbox").and_then(|v| v.as_bool()).unwrap_or(false) == want;
    }
    asked && ok
}

pub(crate) fn clause_of(v: Option<&Value>) -> (Clause, Vec<String>) {
    let mut c = Clause::default();
    let mut errs = Vec::new();
    let Some(obj) = v.and_then(|x| x.as_object()) else {
        return (c, errs);
    };
    for (k, val) in obj {
        match k.as_str() {
            "sender" => match regex_of(val) {
                Some(r) => c.sender = Some(r),
                None => errs.push(format!("match.sender is not a valid regex: {val}")),
            },
            "subject" => match regex_of(val) {
                Some(r) => c.subject = Some(r),
                None => errs.push(format!("match.subject is not a valid regex: {val}")),
            },
            "snippet" | "body" => match regex_of(val) {
                Some(r) => c.snippet = Some(r),
                None => errs.push(format!("match.{k} is not a valid regex: {val}")),
            },
            "label" => c.label = val.as_str().map(str::to_string),
            "unread" => c.unread = val.as_bool(),
            "starred" => c.starred = val.as_bool(),
            "inbox" => c.inbox = val.as_bool(),
            other => errs.push(format!("unknown match key `{other}`")),
        }
    }
    (c, errs)
}

fn parse_prompt(file: &str, idx: usize, v: &Value, errs: &mut Vec<String>) -> Option<Prompt> {
    let Some(obj) = v.as_object() else {
        errs.push(format!("{file}: prompts[{idx}] must be a mapping"));
        return None;
    };
    let qtype = obj
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("choice")
        .to_lowercase();
    if !["choice", "noul", "score"].contains(&qtype.as_str()) {
        errs.push(format!("{file}: prompts[{idx}] type `{qtype}` is not choice | noul | score"));
        return None;
    }
    let instructions = obj
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("What should be done with this email?")
        .to_string();
    let criteria = obj.get("criteria").cloned().unwrap_or(json!({}));
    match qtype.as_str() {
        "choice" => {
            let ok = criteria
                .as_object()
                .map(|o| !o.is_empty())
                .unwrap_or(false);
            if !ok {
                errs.push(format!("{file}: prompts[{idx}] choice criteria must be a non-empty option → description map"));
                return None;
            }
        }
        "score" => {
            let ok = criteria
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if !ok {
                errs.push(format!("{file}: prompts[{idx}] score criteria must be a non-empty ordered array of level descriptions"));
                return None;
            }
        }
        // `noul` needs no criteria at all; a {true,false} map is passed through
        // when the rule wants its own yes/no wording.
        _ => {}
    }
    Some(Prompt {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("p{idx}"))
            .to_string(),
        qtype,
        instructions,
        criteria,
        threshold: obj.get("threshold").and_then(|v| v.as_f64()),
    })
}

/// Read `rules/*.yaml`. Sorted by file name, so `010-…` runs before `090-…` and
/// the order a reader sees is the order the rules were numbered.
pub fn load(root: &Path) -> (Vec<Rule>, Vec<String>) {
    let dir = root.join("rules");
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    let Ok(reads) = std::fs::read_dir(&dir) else {
        return (rules, errors);
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

        // The new shape: an explicit prompt list. The old shape (`ask:` with one
        // criteria map) becomes one choice prompt, plus a noul prompt when the
        // rule asked its yes/no question the old way.
        let mut prompts = Vec::new();
        if let Some(list) = obj.get("prompts").and_then(|p| p.as_array()) {
            for (i, p) in list.iter().enumerate() {
                if let Some(prompt) = parse_prompt(&file, i, p, &mut errs) {
                    prompts.push(prompt);
                }
            }
        } else if let Some(ask) = obj.get("ask").and_then(|a| a.as_object()) {
            let criteria = ask.get("criteria").cloned().unwrap_or(json!({}));
            if criteria.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                errs.push(format!("{file}: `ask.criteria` needs at least one action → description pair"));
            } else {
                prompts.push(Prompt {
                    id: "ask".to_string(),
                    qtype: "choice".to_string(),
                    instructions: ask
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .unwrap_or("What should be done with this email?")
                        .to_string(),
                    criteria,
                    threshold: None,
                });
                if let Some(n) = ask.get("noul").and_then(|v| v.as_str()) {
                    prompts.push(Prompt {
                        id: "ask-noul".to_string(),
                        qtype: "noul".to_string(),
                        instructions: n.to_string(),
                        criteria: json!({}),
                        threshold: None,
                    });
                }
            }
        } else {
            errs.push(format!("{file}: needs `prompts:` (or the older `ask:`) with at least one question"));
        }
        if !errs.is_empty() {
            errors.extend(errs);
            continue;
        }

        let weight = obj
            .get("weight")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        if !(weight > 0.0) {
            errors.push(format!("{file}: `weight` must be positive"));
            continue;
        }
        let ask_obj = obj.get("ask").and_then(|a| a.as_object());
        rules.push(Rule {
            file: file.clone(),
            name: obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| file.trim_end_matches(".yaml"))
                .to_string(),
            enabled: obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            weight,
            vision: obj.get("vision").and_then(|v| v.as_bool()).unwrap_or(false),
            matches,
            unless,
            prompts,
            body_cap: ask_obj
                .and_then(|a| a.get("max_body"))
                .or_else(|| obj.get("max_body"))
                .and_then(|v| v.as_i64())
                .map(|n| n.clamp(40, 4000) as usize)
                .unwrap_or(BODY_CAP),
        });
    }
    (rules, errors)
}

// ---------------------------------------------------------------------------
// the state: what the model is shown
// ---------------------------------------------------------------------------

/// The email as prose, with nothing in it that is the same on every email.
///
/// No date, no id, no size, no "email" boilerplate. On the `laya-email` run a
/// single constant line — the received timestamp — flipped four correct answers
/// to `archive` and took 61% of the whole run, because a date stamp reads as
/// routine correspondence and it was present on all 500 emails.
pub fn state_of(row: &Value, cap: usize) -> String {
    let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let from = if s("fromName").is_empty() {
        s("fromEmail")
    } else {
        format!("{} <{}>", s("fromName"), s("fromEmail"))
    };
    let body = {
        let raw = s("bodyText");
        let raw = if raw.is_empty() { s("snippet") } else { raw };
        let flat: String = raw.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
        flat.chars().take(cap).collect::<String>()
    };
    format!("From: {from}.\nSubject: {}.\nBody: {body}", s("subject"))
}

// ---------------------------------------------------------------------------
// vision: a rendered screenshot of the email
// ---------------------------------------------------------------------------

/// Render the stored `bodyHtml` to a PNG data URL with headless chromium.
/// Cached under `data/.vision-cache/<id>.png` keyed by a hash of the HTML, so a
/// re-evaluation costs a file read, not a browser launch. Returns `None` when
/// there is nothing renderable (no HTML body) or no chromium on PATH — vision
/// is best-effort, never fatal.
pub fn screenshot_of(root: &Path, row: &Value) -> Option<String> {
    if row.get("bodyKind").and_then(|v| v.as_str()) != Some("html") {
        return None;
    }
    let html = row.get("bodyHtml").and_then(|v| v.as_str()).unwrap_or("");
    if html.trim().is_empty() {
        return None;
    }
    let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    // A hash, not the HTML, names the cache entry: the same email re-rendered
    // is the same file, and a changed body re-renders.
    let mut h: u64 = 1469598103934665603;
    for b in html.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(1099511628211);
    }
    let dir = root.join("data").join(".vision-cache");
    let png = dir.join(format!("{id}-{h:016x}.png"));
    let cached = std::fs::read(&png).ok();
    let bytes = match cached {
        Some(b) if !b.is_empty() => b,
        _ => {
            let _ = std::fs::create_dir_all(&dir);
            let doc = format!(
                "<!doctype html><html><head><meta charset=\"utf-8\">\
                 <style>body{{margin:0 auto;max-width:1100px;padding:24px;font-family:sans-serif}}</style>\
                 </head><body>{html}</body></html>"
            );
            let src = dir.join(format!(".{id}-{h:016x}.html"));
            std::fs::write(&src, doc).ok()?;
            let out = std::process::Command::new("chromium")
                .args([
                    "--headless=new",
                    "--no-sandbox",
                    "--disable-gpu",
                    "--hide-scrollbars",
                    "--window-size=1100,1600",
                    &format!("--screenshot={}", png.display()),
                    &format!("file://{}", src.display()),
                ])
                .output()
                .ok()?;
            let _ = std::fs::remove_file(&src);
            if !out.status.success() {
                return None;
            }
            std::fs::read(&png).ok()?
        }
    };
    // A full-height marketing email can be megabytes; the state already carries
    // the words, so the picture is allowed to be a sketch, not a poster.
    if bytes.len() > 1_800_000 {
        return None;
    }
    Some(format!("data:image/png;base64,{}", base64_encode(&bytes)))
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
    for chunk in bytes.chunks(3) {
        let mut n: u32 = 0;
        for (i, b) in chunk.iter().enumerate() {
            n |= (*b as u32) << (8 * (2 - i));
        }
        let pad = 3 - chunk.len();
        for i in 0..4 - pad {
            out.push(ALPH[((n >> (6 * (3 - i))) & 63) as usize] as char);
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the model
// ---------------------------------------------------------------------------

pub struct SystemOne {
    agent: Agent,
    pub base: String,
}

impl SystemOne {
    pub fn new(base: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(60))
                .build(),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// `GET /health` on the shim. Not every build answers it, so a failure here
    /// is reported, never fatal: the mirror is browsable without a model.
    pub fn healthy(&self) -> bool {
        self.agent
            .get(&format!("{}/health", self.base))
            .call()
            .map(|r| r.status() < 400)
            .unwrap_or(false)
    }

    /// One request, many questions. `questions` is `{name: {type, …}}`; `images`
    /// rides alongside for endpoints that can see (the stock shim ignores it).
    fn ask(&self, state: &str, questions: &Value, images: &[String]) -> Result<Value, String> {
        let mut body = json!({ "state": state, "questions": questions });
        if !images.is_empty() {
            body.as_object_mut().unwrap().insert("images".to_string(), json!(images));
        }
        let resp = self
            .agent
            .post(&format!("{}/v1/systemone", self.base))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    format!("model answered {code}: {}", r.into_string().unwrap_or_default())
                }
                other => format!("cannot reach the model at {}/v1/systemone: {other}", self.base),
            })?;
        let text = resp.into_string().unwrap_or_default();
        serde_json::from_str(&text).map_err(|e| format!("model returned bad JSON: {e}"))
    }
}

// ---------------------------------------------------------------------------
// evaluate
// ---------------------------------------------------------------------------

/// Ask about one email: every matching rule contributes its prompts as
/// questions in the same call, the choice answers are merged (weighted by each
/// rule's `weight:`), and the argmax is the recommendation. `noul` / `score`
/// answers are signals — reported, never merged.
pub fn evaluate(model: &SystemOne, root: &Path, rules: &[Rule], row: &Value) -> Value {
    let live: Vec<&Rule> = rules.iter().filter(|r| r.enabled && r.applies(row)).collect();
    let base: Vec<String> = live.iter().map(|r| r.name.clone()).collect();
    let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if live.is_empty() {
        return json!({ "id": id, "ok": true, "action": null, "confidence": 0.0, "matched": 0,
                       "note": "no rule matched" });
    }

    let cap = live.iter().map(|r| r.body_cap).max().unwrap_or(BODY_CAP);
    let state = state_of(row, cap);

    // Vision is per-rule: only emails that matched a `vision: true` rule pay
    // for a render, and only when there is HTML to render.
    let mut images = Vec::new();
    if live.iter().any(|r| r.vision) {
        if let Some(url) = screenshot_of(root, row) {
            images.push(url);
        }
    }

    // One question per prompt, keyed r{rule}p{prompt} so the answers come back
    // attributable.
    let mut questions = serde_json::Map::new();
    let mut order: Vec<(usize, usize)> = Vec::new();
    for (i, r) in live.iter().enumerate() {
        for (j, p) in r.prompts.iter().enumerate() {
            let key = format!("r{i}p{j}");
            let mut q = json!({
                "type": p.qtype,
                "instructions": p.instructions,
                "criteria": p.criteria,
            });
            if let Some(t) = p.threshold {
                q.as_object_mut().unwrap().insert("threshold".to_string(), json!(t));
            }
            questions.insert(key, q);
            order.push((i, j));
        }
    }

    let t0 = std::time::Instant::now();
    let answer = match model.ask(&state, &Value::Object(questions), &images) {
        Ok(v) => v,
        Err(e) => {
            return json!({ "id": id, "ok": false, "error": e, "matched": live.len(), "rules": base });
        }
    };
    let ms = t0.elapsed().as_millis() as i64;
    let answers = answer.get("answers").cloned().unwrap_or(Value::Null);

    // Merge: each option's weighted mean over the choice prompts that offered
    // it. A rule's `weight:` multiplies every vote it casts.
    let mut wsum: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut wtot: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut per_prompt = Vec::new();
    let mut signals = serde_json::Map::new();

    for (i, j) in &order {
        let r = live[*i];
        let p = &r.prompts[*j];
        let a = answers
            .get(&format!("r{i}p{j}"))
            .cloned()
            .unwrap_or(Value::Null);
        let conf = a.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.0);
        match p.qtype.as_str() {
            "choice" => {
                let mut voted = false;
                if let Some(obj) = a.get("probabilities").and_then(|x| x.as_object()) {
                    for (k, v) in obj {
                        let pr = v.as_f64().unwrap_or(0.0);
                        *wsum.entry(k.clone()).or_insert(0.0) += pr * r.weight;
                        *wtot.entry(k.clone()).or_insert(0.0) += r.weight;
                        voted = true;
                    }
                }
                let choice = a.get("choice").and_then(|c| c.as_str()).unwrap_or("");
                if !voted && !choice.is_empty() {
                    // A model that returns only the winner still votes for it.
                    *wsum.entry(choice.to_string()).or_insert(0.0) += conf.max(0.0) * r.weight;
                    *wtot.entry(choice.to_string()).or_insert(0.0) += r.weight;
                }
                per_prompt.push(json!({
                    "rule": r.name, "prompt": p.id, "type": "choice",
                    "choice": choice, "confidence": conf, "weight": r.weight,
                }));
            }
            "noul" => {
                let v = a
                    .get("noul")
                    .and_then(|x| x.as_f64())
                    .or_else(|| a.as_f64())
                    .unwrap_or(0.0);
                signals.insert(format!("{}/{}", r.name, p.id), json!(v));
                per_prompt.push(json!({
                    "rule": r.name, "prompt": p.id, "type": "noul",
                    "answer": format!("{v:.2}"), "confidence": conf, "weight": r.weight,
                }));
            }
            _ => {
                let v = a.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0);
                signals.insert(format!("{}/{}", r.name, p.id), json!(v));
                per_prompt.push(json!({
                    "rule": r.name, "prompt": p.id, "type": "score",
                    "answer": format!("{v:.2}"), "confidence": conf, "weight": r.weight,
                }));
            }
        }
    }

    let avg: std::collections::BTreeMap<String, f64> = wsum
        .iter()
        .map(|(k, s)| (k.clone(), s / wtot.get(k).cloned().unwrap_or(1.0).max(f64::EPSILON)))
        .collect();

    // argmax, ties broken alphabetically (BTreeMap order) — deterministic.
    let mut best: Option<(&String, f64)> = None;
    for (k, p) in &avg {
        if best.map(|(_, bp)| *p > bp).unwrap_or(true) {
            best = Some((k, *p));
        }
    }
    let (key, confidence) = match best {
        Some((k, p)) => (k.clone(), p),
        None => (String::new(), 0.0),
    };

    let (op, folder) = match key.split_once(':') {
        Some(("move", f)) => ("move".to_string(), Some(f.to_string())),
        Some(("purge", f)) => ("purge".to_string(), Some(f.to_string())),
        _ => (key.clone(), None),
    };

    // The detail view lists one line per prompt; keep the old per-rule shape
    // readable by collapsing each rule's choice vote to its own line.
    let mut rule_lines = Vec::new();
    for r in &live {
        let mine: Vec<&Value> = per_prompt
            .iter()
            .filter(|p| p.get("rule").and_then(|x| x.as_str()) == Some(r.name.as_str()))
            .collect();
        for m in mine {
            let t = m.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let line = match t {
                "choice" => format!(
                    "{} / {} → {} ({:.2})",
                    m.get("rule").and_then(|x| x.as_str()).unwrap_or(""),
                    m.get("prompt").and_then(|x| x.as_str()).unwrap_or(""),
                    m.get("choice").and_then(|x| x.as_str()).unwrap_or(""),
                    m.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0),
                ),
                _ => format!(
                    "{} / {} = {}",
                    m.get("rule").and_then(|x| x.as_str()).unwrap_or(""),
                    m.get("prompt").and_then(|x| x.as_str()).unwrap_or(""),
                    m.get("answer").and_then(|x| x.as_str()).unwrap_or(""),
                ),
            };
            rule_lines.push(line);
        }
    }

    json!({
        "id": id,
        "ok": true,
        "key": key,
        "action": if key.is_empty() { Value::Null } else { json!(op) },
        "folder": folder,
        "label": if key.is_empty() { Value::Null } else if folder.is_some() {
            json!(format!("{op}→{}", folder.clone().unwrap_or_default()))
        } else { json!(op.clone()) },
        "confidence": (confidence * 1000.0).round() / 1000.0,
        "probs": avg,
        "matched": live.len(),
        "rules": base,
        "answers": rule_lines,
        "prompts": per_prompt,
        "signals": signals,
        "images": images.len(),
        "ms": ms,
        "stateChars": state.chars().count(),
    })
}
