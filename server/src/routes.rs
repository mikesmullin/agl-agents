//! The JSON API the wasm frontend talks to.
//!
//! | route | what it does |
//! |---|---|
//! | `GET    /api/mail?filter=&q=&limit=` | the list, straight out of SQLite; `matched` is how many rows the filter + `q` match in the **whole mirror**, `rows` only the window |
//! | `GET    /api/mail-one?id=&full=1`    | one message; `full` fetches the body once and caches it |
//! | `POST   /api/queue`                  | **enqueue** an action for N messages (no network) |
//! | `DELETE /api/queue?message_id=`      | drop queued work (`all=1` for the whole outbox) |
//! | `POST   /api/pull`                   | Gmail → SQLite |
//! | `POST   /api/push`                   | queued mutations → Gmail |
//! | `GET    /api/labels`                 | the label table (cached; `?remote=1` re-reads Gmail) |
//! | `GET    /api/mail-status`            | auth, last pull, last push, counts |
//! | `GET    /api/mutations?limit=`       | recent outbox rows |
//! | `GET    /api/rules`                  | the `rules/*.yaml` set: what parsed, what did not, and whether the model answers |
//! | `POST   /api/rules/eval {ids}`       | ask the System One model about those emails → a recommended action each |
//! | `POST   /api/rules/new {ids,prompt}` | draft a rule from the user's narration + example emails → a new `rules/*.yaml` |
//! | `GET    /api/phrasers`               | the `phrasers/*.yaml` set: what parsed, what did not |
//! | `POST   /api/phrasers/eval {ids}`    | ask the text LLM to reword those emails → a replacement subject + summary each |
//! | `DELETE /api/phrasers`               | drop every stored rewording (rows go back to their own words) |
//!
//! The split is the point: **enqueue never touches the network**, so a click is
//! reflected the moment the row is re-read, and `push` is the one place that
//! talks to Gmail on the user's behalf.

use std::sync::Arc;

use rustex1_devserver::{Json, Request, Response};
use serde_json::{json, Value};

use crate::db::{Db, Mutation};
use crate::gmail::Gmail;
use crate::sync;

pub struct App {
    pub db: Arc<Db>,
    /// `None` when no OAuth client was found at startup. The app still runs —
    /// the mirror is browsable and the outbox still queues — but every Gmail
    /// call answers 503 with the reason instead of pretending.
    pub gmail: Option<Arc<Gmail>>,
    /// Project root: `rules/*.yaml` lives here (gitignored).
    pub root: std::path::PathBuf,
    /// The System One endpoint. Always constructed, health checked on demand:
    /// rules are useless without a model, but a missing model must not stop the
    /// mirror from browsing.
    pub model: Arc<crate::rules::SystemOne>,
    /// The text-LLM endpoint behind the phrasers. Same deal: constructed
    /// always (it defaults to the :1234 slot), answering only when a text
    /// model is actually serving there.
    pub llm: Arc<crate::phrasers::Llm>,
}

const NO_CREDS: &str =
    "no Gmail credentials found (gmail.json, or GMAIL_AUTH, or /workspace/cli/google-email)";

// ---------------------------------------------------------------------------
// GET /api/mail
// ---------------------------------------------------------------------------

pub fn mail(app: &Arc<App>, req: &Request) -> Response {
    let filter = req.param("filter").unwrap_or_else(|| "inbox".into());
    let q = req.param("q").unwrap_or_default();
    let limit = req
        .param("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(400)
        .clamp(1, 2000);
    let (rows, counts, matched) = app.db.list(&filter, &q, limit);
    json!({
        "ok": true,
        "rows": rows,
        "counts": counts,
        // `rows` is the window, `matched` is the whole mirror: a search that hit
        // 1243 messages and returned 1000 has to say so, or the reader assumes
        // the list is the answer.
        "matched": matched,
        "limit": limit,
        "filter": filter,
        "q": q,
        "at": crate::db::now_ms(),
        "lastPull": app.db.meta_json("lastPull"),
        "lastPush": app.db.meta_json("lastPush"),
    })
    .into()
}

// ---------------------------------------------------------------------------
// GET /api/mail-one?id=…[&full=1]
// ---------------------------------------------------------------------------

pub fn mail_one(app: &Arc<App>, req: &Request) -> Response {
    let Some(id) = req.param("id") else {
        return Response::bad_request("id is required");
    };
    let want_full = req.param("full").as_deref() == Some("1");
    if want_full {
        if let Some(row) = app.db.message_row(&id) {
            if !row.get("hasBody").and_then(|v| v.as_bool()).unwrap_or(false) {
                let Some(gmail) = app.gmail.as_ref() else {
                    return Response::new(503, json!({ "ok": false, "error": NO_CREDS, "message": row }))
                };
                match gmail.get(&id, "full") {
                    Ok(doc) => app.db.apply_message(&id, &doc, true),
                    Err(e) => {
                        return Response::new(
                            502,
                            json!({ "ok": false, "error": e, "message": row }),
                        )
                    }
                }
            }
        }
    }
    match app.db.message_row(&id) {
        Some(row) => json!({ "ok": true, "message": row }).into(),
        None => Response::not_found(format!("no message {id} in the local mirror — pull first")),
    }
}

// ---------------------------------------------------------------------------
// POST /api/queue
// ---------------------------------------------------------------------------

/// The action vocabulary. One place turns a button into label ids, so the UI
/// cannot disagree with the outbox about what "Archive" means.
pub fn plan_action(op: &str, destination: &str, db: &Db) -> (String, Vec<String>, Vec<String>, Vec<String>) {
    match op {
        "archive" => ("modify".into(), vec![], vec!["INBOX".into()], vec![]),
        "spam" => ("modify".into(), vec!["SPAM".into()], vec!["INBOX".into()], vec![]),
        "delete" => ("trash".into(), vec![], vec![], vec![]),
        "purge" => ("delete".into(), vec![], vec![], vec![]),
        "read" => ("modify".into(), vec![], vec!["UNREAD".into()], vec![]),
        "unread" => ("modify".into(), vec!["UNREAD".into()], vec![], vec![]),
        "star" => ("modify".into(), vec!["STARRED".into()], vec![], vec![]),
        "unstar" => ("modify".into(), vec![], vec!["STARRED".into()], vec![]),
        "important" => ("modify".into(), vec!["IMPORTANT".into()], vec![], vec![]),
        "unimportant" => ("modify".into(), vec![], vec!["IMPORTANT".into()], vec![]),
        "move" => {
            match db.label_id_by_name(destination) {
                Some(id) => (
                    "modify".into(),
                    vec![id],
                    vec!["INBOX".into()],
                    vec![destination.to_string()],
                ),
                // Not cached (or does not exist yet): keep the *name* and let
                // push resolve or create it. Queuing must not need the network.
                None => ("move".into(), vec![], vec!["INBOX".into()], vec![destination.to_string()]),
            }
        }
        _ => ("modify".into(), vec![], vec![], vec![]),
    }
}

pub fn queue(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST a JSON body: {ids, op, destination, instruction, rationale}");
    }
    let Some(body) = req.json() else {
        return Response::bad_request("expected a JSON body");
    };
    let ids: Vec<String> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Response::bad_request("select at least one email first");
    }
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if op.is_empty() {
        return Response::bad_request("op is required");
    }
    let destination = body
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if op == "move" && destination.is_empty() {
        return Response::bad_request("move needs a destination folder");
    }
    let instruction = body
        .get("instruction")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let rationale = body
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let (real_op, add, remove, names) = plan_action(&op, &destination, &app.db);
    let mut queued = 0usize;
    for id in &ids {
        let m = Mutation {
            message_id: id.clone(),
            op: real_op.clone(),
            add: add.clone(),
            remove: remove.clone(),
            names: names.clone(),
            instruction: if instruction.is_empty() { op.clone() } else { instruction.clone() },
            rationale: rationale.clone(),
        };
        if app.db.queue(&m).is_ok() {
            queued += 1;
        }
    }
    // Hand the caller back the rows it just changed, already carrying their
    // pending state, so the UI repaints from the queue without a second read.
    let rows: Vec<Json> = ids
        .iter()
        .filter_map(|id| app.db.message_row(id))
        .collect();
    Response::created(json!({
        "ok": true,
        "queued": queued,
        "op": op,
        "rows": rows,
        "counts": app.db.counts(),
    }))
}

pub fn queue_cancel(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "DELETE" {
        return Response::bad_request("DELETE /api/queue[?message_id=… | ?all=1]");
    }
    let all = req.param("all").as_deref() == Some("1");
    let message_id = req.param("message_id");
    let removed = if all {
        app.db.cancel(None)
    } else {
        app.db.cancel(message_id.as_deref())
    };
    json!({ "ok": true, "removed": removed, "counts": app.db.counts() }).into()
}

// ---------------------------------------------------------------------------
// POST /api/pull · POST /api/push
// ---------------------------------------------------------------------------

pub fn pull(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST /api/pull");
    }
    if sync::busy(&sync::PULLING) {
        return Response::new(409, json!({ "ok": false, "error": "a pull is already running" }));
    }
    let body = req.json().unwrap_or(json!({}));
    let mut opts = sync::PullOpts {
        workers: sync::workers_hint().max(8).min(24),
        ..Default::default()
    };
    if let Some(q) = body.get("query").and_then(|v| v.as_str()) {
        if !q.trim().is_empty() {
            opts.query = q.trim().to_string();
        }
    }
    opts.full = body.get("full").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(r) = body.get("rate").and_then(|v| v.as_f64()) {
        opts.rate = r.clamp(1.0, 60.0);
    }
    opts.fetch = body.get("fetch").and_then(|v| v.as_bool()).unwrap_or(true);
    if let Some(max) = body.get("max").and_then(|v| v.as_u64()) {
        opts.max = max.clamp(1, 5000) as u32;
    }
    if let Some(w) = body.get("workers").and_then(|v| v.as_u64()) {
        opts.workers = w.clamp(1, 32) as usize;
    }
    let stats = match app.gmail.as_ref() {
        Some(gmail) => sync::pull(&app.db, gmail, &opts),
        None => json!({ "ok": false, "error": NO_CREDS, "listed": 0, "fetched": 0 }),
    };
    sync::idle(&sync::PULLING);
    Response::new(if stats["ok"].as_bool().unwrap_or(false) { 200 } else { 502 }, stats)
}

pub fn push(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST /api/push");
    }
    if sync::busy(&sync::PUSHING) {
        return Response::new(409, json!({ "ok": false, "error": "a push is already running" }));
    }
    let retry = req
        .json()
        .and_then(|b| b.get("retryFailed").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let report = match app.gmail.as_ref() {
        Some(gmail) => sync::push(&app.db, gmail, retry),
        None => json!({ "ok": false, "error": NO_CREDS, "applied": 0, "failed": 0, "queued": 0, "batches": 0, "ms": 0, "errors": [] }),
    };
    sync::idle(&sync::PUSHING);
    Response::new(if report["ok"].as_bool().unwrap_or(false) { 200 } else { 502 }, report)
}

// ---------------------------------------------------------------------------
// labels · status · recent outbox
// ---------------------------------------------------------------------------

pub fn labels(app: &Arc<App>, req: &Request) -> Response {
    let remote = req.param("remote").as_deref() == Some("1");
    if remote || app.db.labels().is_empty() {
        // A remote refresh is the only reason this route needs Gmail at all; the
        // cached table is good enough to fill the Move dropdown offline.
        if let Some(gmail) = app.gmail.as_ref() {
            match gmail.labels() {
                Ok(list) => db_labels(&app.db, &list),
                Err(e) if remote => {
                    return Response::new(502, json!({ "ok": false, "error": e }))
                }
                Err(_) => {}
            }
        } else if remote {
            return Response::new(503, json!({ "ok": false, "error": NO_CREDS }));
        }
    }
    let rows: Vec<Json> = app.db
        .labels()
        .into_iter()
        .filter(|(id, _, kind)| kind != "system" || id == "INBOX")
        .map(|(id, name, kind)| json!({ "id": id, "name": name, "kind": kind }))
        .collect();
    json!({ "ok": true, "labels": rows }).into()
}

fn db_labels(db: &Db, list: &[(String, String, String)]) {
    db.put_labels(list);
}

// ---------------------------------------------------------------------------
// rules: GET /api/rules · POST /api/rules/eval
// ---------------------------------------------------------------------------

/// The rule set as the UI sees it: which files parsed, which did not and why,
/// which actions each rule offers, and whether the model answers at all.
pub fn rules(app: &Arc<App>, _req: &Request) -> Response {
    let (rules, errors) = crate::rules::load(&app.root);
    json!({
        "ok": true,
        "rules": rules.iter().map(|r| r.public()).collect::<Vec<_>>(),
        "errors": errors,
        "model": { "url": app.model.base, "healthy": app.model.healthy() },
    })
    .into()
}

/// `POST {ids:[…]}` → one recommendation per email.
///
/// The rules are re-read from disk on every call: a rule set is something you
/// edit while looking at mail, and a restart for a wording change would make
/// the whole exercise feel like programming.
pub fn rules_eval(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST a JSON body: {ids: […]}");
    }
    let Some(body) = req.json() else {
        return Response::bad_request("expected a JSON body");
    };
    let ids: Vec<String> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Response::bad_request("no ids");
    }

    let (rules, errors) = crate::rules::load(&app.root);
    if rules.is_empty() {
        let why = if errors.is_empty() {
            "no rules/*.yaml on disk".to_string()
        } else {
            "no rule parsed".to_string()
        };
        return json!({ "ok": false, "error": why, "errors": errors }).into();
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for id in &ids {
        match app.db.message_row(id) {
            Some(r) => rows.push(r),
            None => missing.push(id.clone()),
        }
    }

    // One request per email, all matching rules' questions inside it. Emails are
    // independent, so a handful of threads keep a batched evaluation from being
    // `ids × latency`.
    let results: std::sync::Mutex<Vec<Value>> = std::sync::Mutex::new(vec![json!(null); rows.len()]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = rows.len().min(6).max(1);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= rows.len() {
                    break;
                }
                let r = crate::rules::evaluate(&app.model, &app.root, &rules, &rows[i]);
                results.lock().unwrap()[i] = r;
            });
        }
    });
    let results = results.into_inner().unwrap();
    let failed = results.iter().filter(|r| r.get("ok").and_then(|v| v.as_bool()) == Some(false)).count();

    json!({
        "ok": failed == 0,
        "results": results,
        "rules": rules.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        "errors": errors,
        "missing": missing,
        "model": { "url": app.model.base, "healthy": app.model.healthy() },
    })
    .into()
}

// ---------------------------------------------------------------------------
// POST /api/rules/new — draft a rule from the user's narration + example emails
// ---------------------------------------------------------------------------

/// The system prompt that turns a narration into one `rules/*.yaml` file.
///
/// The LLM only ever sees the schema and the example emails; the file it emits
/// is parsed by the same loader the evaluator uses, so anything it gets wrong
/// is rejected here rather than written to disk.
const RULE_SYSTEM: &str = r#"You create one mail1 routing rule by calling tools. Do not print the YAML as your final answer.

Tools:
- make_rule(yaml): write the rule. The filename is already chosen from the sender domain. You do not pick it. Calling make_rule again overwrites that same file. If the user message includes an existing rule for this domain, that is the file you are editing: revise it, and do not drop behavior it already got right unless the user is changing that behavior.
- check_rule(): run every rule on disk, including the file you just wrote, against the selected emails. Same evaluation as the Rules button. It takes no arguments and returns each email's recommended action.

Loop until the recommendation is what the user asked for:
1. Call make_rule with a complete rule YAML.
2. Call check_rule.
3. If the action is wrong, or no rule matched, fix the YAML, call make_rule again, then check_rule again.
4. Stop when check_rule shows the outcome the user wanted. A short confirmation is enough.

A rule matches emails by sender/subject/snippet and asks a decision model which
action the email deserves. Shape of the YAML you pass to make_rule:

name: <short kebab-case id>
weight: 1.0
match:
  sender: <regex matched against the from address or name>
  subject: <regex matched against the subject>
  snippet: <regex matched against the snippet and body>
prompts:
  - id: <short id>
    type: choice
    instructions: <one question the decision model answers about the email>
    criteria:
      <action>: <short phrase describing when this action is right>
      <action>: <short phrase>

Rules for the output:
- `match` must name at least one of sender, subject, snippet. Use a regex that
  matches the example email(s) and the same kind of mail, not just this one.
- A `sender` regex is tested against three strings **separately**: the address,
  the display name, and `Display Name <address>`. It matches if **any one** of
  those three strings matches. It is not one combined string you can span.
  `^Grok xAI.*@stripe\.com$` matches none of them: the address does not start
  with the name, the name has no `@`, and the combined form has `<` `>` in
  between. Prefer a fragment that appears inside the address alone, such as
  `.*@stripe\.com`, and do not anchor with `^` and `$` unless one of the three
  lines matches that pattern in full.
- Escape regex metacharacters with a single backslash (`\.` for a dot).
- Every `criteria` key MUST be one of: archive, spam, delete, purge, read,
  unread, star, unstar, important, unimportant, skip, or move:<Folder>.
  `move:<Folder>` must use a folder name from the provided folder list.
- `skip` means "leave it to me". Include it when the narration leaves some mail
  for the user to judge.
- `instructions` and each criteria phrase are shown to a decision model, so make
  them concrete and mutually distinct.
- Do not output the YAML in a message. The only way to save it is make_rule."#;

/// `POST {ids:[...], prompt:"..."}` → ask the text LLM to draft a rule, validate
/// it with the rule loader, and write it to `rules/<nnn>-<slug>.yaml`.
///
/// The next number is one past the highest existing `NNN-` prefix, so the new
/// rule sorts after everything already on disk.
pub fn rules_new(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST a JSON body: {ids: [...], prompt: \"...\"}");
    }
    let Some(body) = req.json() else {
        return Response::bad_request("expected a JSON body");
    };
    let ids: Vec<String> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Response::bad_request("select at least one email first");
    }
    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if prompt.is_empty() {
        return Response::bad_request("describe what the rule should do");
    }

    let mut rows: Vec<Value> = Vec::new();
    for id in &ids {
        if let Some(r) = app.db.message_row(id) {
            rows.push(r);
        }
    }
    if rows.is_empty() {
        return Response::bad_request("none of the selected emails are in the mirror");
    }

    let dir = app.root.join("rules");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return json!({ "ok": false, "error": format!("cannot create rules dir: {e}") }).into();
    }
    // One file for this sender domain. If one already exists, make_rule
    // overwrites it and the model is shown its current contents.
    let domain = sender_domain(
        rows.first()
            .and_then(|r| r.get("fromEmail"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );
    let existing = domain_rule_files(&dir, &domain);
    let dest = existing.last().cloned().unwrap_or_else(|| {
        dir.join(format!("{:03}-{domain}.yaml", next_rule_number(&dir)))
    });
    let file = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("new.yaml")
        .to_string();
    let mut user = rule_user_prompt(&rows, &prompt, &app.db);
    if existing.is_empty() {
        user.push_str("\nNo rule file exists yet for this sender domain.\n");
    } else {
        user.push_str(
            "\nA rule file already exists for this sender domain. make_rule overwrites it. Read it and revise it.\n",
        );
        for path in &existing {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            let text = std::fs::read_to_string(path).unwrap_or_default();
            user.push_str(&format!("\n--- {name} ---\n{text}\n"));
        }
    }
    match draft_rule(app, &rows, &user, &dest, &file) {
        Ok(yaml) => json!({ "ok": true, "file": file, "yaml": yaml }).into(),
        Err(e) => json!({ "ok": false, "error": e, "file": file }).into(),
    }
}

/// `amazon.com` -> `amazon-com`. A leading mailbox subdomain (`info.`,
/// `notification.`) is dropped so `info.rockymountainpower.net` files as
/// `rockymountainpower-net`.
fn sender_domain(email: &str) -> String {
    let host = email.split('@').nth(1).unwrap_or("mail");
    let parts: Vec<&str> = host.split('.').filter(|p| !p.is_empty()).collect();
    let host = if parts.len() >= 3
        && matches!(
            parts[0],
            "info" | "notification" | "notifications" | "mail" | "email" | "e" | "news" | "e1"
        )
    {
        parts[1..].join(".")
    } else {
        host.to_string()
    };
    let slug: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "mail".to_string() } else { slug }
}

/// Rule files already named for this sender domain, oldest first.
fn domain_rule_files(dir: &std::path::Path, domain: &str) -> Vec<std::path::PathBuf> {
    let suffix = format!("-{domain}.yaml");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.ends_with(&suffix) || n == format!("{domain}.yaml"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

fn rule_tools() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "make_rule",
                "description": "Write the rule YAML to the already-chosen file. Calling this again overwrites that same file. You do not choose the filename.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "yaml": {"type": "string", "description": "The complete rule YAML document"}
                    },
                    "required": ["yaml"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "check_rule",
                "description": "Run every rule on disk against the selected emails, the same way the Rules button does. Returns each email's recommended action. Takes no arguments.",
                "parameters": {"type": "object", "properties": {}}
            }
        }
    ])
}

fn tool_args(call: &Value) -> Value {
    match call.get("function").and_then(|f| f.get("arguments")) {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
        Some(other) => other.clone(),
        None => json!({}),
    }
}

fn write_made_rule(dest: &std::path::Path, file: &str, root: &std::path::Path, yaml: &str) -> (Value, String) {
    let yaml = normalize_regexes(&strip_fence(yaml));
    if let Err(e) = std::fs::write(dest, &yaml) {
        return (json!({"ok": false, "error": e.to_string()}), yaml);
    }
    let (parsed, errors) = crate::rules::load(root);
    if parsed.iter().any(|r| r.file == file) {
        (json!({"ok": true, "file": file, "parsed": true}), yaml)
    } else {
        let why = errors
            .iter()
            .find(|e| e.starts_with(file))
            .cloned()
            .unwrap_or_else(|| "rule did not parse".to_string());
        (json!({"ok": false, "file": file, "error": why}), yaml)
    }
}

/// The Rules button, for the emails this dialog was opened on.
fn check_made_rule(app: &App, rows: &[Value]) -> Value {
    let (rules, errors) = crate::rules::load(&app.root);
    let results: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut r = crate::rules::evaluate(&app.model, &app.root, &rules, row);
            if let Some(obj) = r.as_object_mut() {
                obj.insert("subject".into(), row.get("subject").cloned().unwrap_or(json!("")));
                obj.insert("from".into(), row.get("fromEmail").cloned().unwrap_or(json!("")));
            }
            r
        })
        .collect();
    json!({"results": results, "load_errors": errors})
}

/// Tool loop: make_rule writes one file, check_rule evaluates it, repeat.
fn draft_rule(
    app: &App,
    rows: &[Value],
    user: &str,
    dest: &std::path::Path,
    file: &str,
) -> Result<String, String> {
    let mut messages = vec![
        json!({"role": "system", "content": RULE_SYSTEM}),
        json!({"role": "user", "content": user}),
    ];
    let mut wrote = false;
    let mut checked_after_write = false;
    let mut yaml = String::new();
    let mut last_check = json!(null);

    for round in 0..8 {
        let body = json!({
            "model": "default",
            "messages": messages,
            "tools": rule_tools(),
            "tool_choice": "auto",
            "max_tokens": 4096,
            "stream": false,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let message = app.llm.chat(&body)?;
        let calls = message
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if calls.is_empty() {
            if wrote && checked_after_write {
                println!("[rules/new] done after {round} round(s) file={file}");
                return Ok(yaml);
            }
            messages.push(message);
            messages.push(json!({
                "role": "user",
                "content": "Do not finish yet. Call make_rule with the YAML, then check_rule, and repeat until the recommended action is what the user asked for."
            }));
            continue;
        }
        messages.push(message);
        for call in &calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = match name {
                "make_rule" => {
                    let args = tool_args(call);
                    let raw = args.get("yaml").and_then(|v| v.as_str()).unwrap_or("");
                    if raw.trim().is_empty() {
                        json!({"ok": false, "error": "yaml is required"})
                    } else {
                        let (report, saved) = write_made_rule(dest, file, &app.root, raw);
                        if report.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                            wrote = true;
                            checked_after_write = false;
                            yaml = saved;
                        }
                        println!("[rules/new] make_rule file={file} ok={}", report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
                        report
                    }
                }
                "check_rule" => {
                    let report = check_made_rule(app, rows);
                    if wrote {
                        checked_after_write = true;
                    }
                    last_check = report.clone();
                    println!("[rules/new] check_rule {last_check}");
                    report
                }
                other => json!({"ok": false, "error": format!("unknown tool {other}")}),
            };
            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": result.to_string(),
            }));
        }
    }
    if wrote && checked_after_write {
        return Ok(yaml);
    }
    Err(format!(
        "stopped before the rule was verified. last check: {last_check}"
    ))
}

/// The user message: the narration, then each example email as prose, then the
/// folder names a `move:` action is allowed to name.
fn rule_user_prompt(rows: &[Value], narration: &str, db: &Db) -> String {
    let s = |row: &Value, k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut out = format!("The user wants this rule:\n{narration}\n\nExample email(s) it must match:\n");
    for (i, row) in rows.iter().enumerate() {
        let from = if s(row, "fromName").is_empty() {
            s(row, "fromEmail")
        } else {
            format!("{} <{}>", s(row, "fromName"), s(row, "fromEmail"))
        };
        let body = {
            let raw = s(row, "bodyText");
            let raw = if raw.is_empty() { s(row, "snippet") } else { raw };
            let flat: String = raw.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            flat.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(600).collect::<String>()
        };
        out.push_str(&format!(
            "\n[{}] Subject: {}\n    Body: {body}\n    sender regex is tested against each of these lines separately, and must match at least one:\n      address: {}\n      name: {}\n      combined: {from}\n",
            i + 1,
            s(row, "subject"),
            s(row, "fromEmail"),
            s(row, "fromName"),
        ));
    }
    let folders: Vec<String> = db
        .labels()
        .into_iter()
        .filter(|(_, _, kind)| kind != "system")
        .map(|(_, name, _)| name)
        .collect();
    if !folders.is_empty() {
        out.push_str(&format!("\nFolders a move: action may name: {}\n", folders.join(", ")));
    }
    out
}

/// Drop a ```yaml fence if the model wrapped its answer in one.
fn strip_fence(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("```yaml").or_else(|| t.strip_prefix("```yml")).or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().to_string()
}

/// Models routinely double-escape regex backslashes (`\\.` for a literal dot).
/// Collapse runs of backslashes to one on the regex-bearing lines, so the
/// pattern the loader compiles is the one the model meant.
fn normalize_regexes(yaml_text: &str) -> String {
    yaml_text
        .lines()
        .map(|l| {
            let key = l.trim_start();
            if key.starts_with("sender:") || key.starts_with("subject:") || key.starts_with("snippet:") {
                let (lead, rest) = l.split_at(l.len() - key.len());
                format!("{lead}{}", collapse_backslashes(rest))
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_backslashes(s: &str) -> String {
    let mut out = String::new();
    let mut prev_bs = false;
    for c in s.chars() {
        if c == '\\' {
            if !prev_bs {
                out.push('\\');
            }
            prev_bs = true;
        } else {
            prev_bs = false;
            out.push(c);
        }
    }
    out
}

/// The next `NNN` prefix: one past the highest existing rule file's number.
fn next_rule_number(dir: &std::path::Path) -> u32 {
    let mut max = 0u32;
    if let Ok(reads) = std::fs::read_dir(dir) {
        for e in reads.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(n) = name.split('-').next().and_then(|s| s.parse::<u32>().ok()) {
                max = max.max(n);
            }
        }
    }
    max + 10
}

/// A filename slug from the rule's `name:`, falling back to a generic one.
fn slug_of(yaml_text: &str) -> String {
    let name = yaml_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("name:"))
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .unwrap_or_default();
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "rule".to_string() } else { slug }
}

// ---------------------------------------------------------------------------
// phrasers: GET /api/phrasers · POST /api/phrasers/eval · DELETE /api/phrasers
// ---------------------------------------------------------------------------

/// The phraser set as the UI sees it: which files parsed and which did not.
/// The LLM itself is not health-checked here — the two-model workflow means
/// it is usually down, and that is normal, not an error.
pub fn phrasers(app: &Arc<App>, _req: &Request) -> Response {
    let (phrasers, errors) = crate::phrasers::load(&app.root);
    json!({
        "ok": true,
        "phrasers": phrasers.iter().map(|p| p.public()).collect::<Vec<_>>(),
        "errors": errors,
        "llm": { "url": app.llm.base },
    })
    .into()
}

/// `POST {ids:[…]}` → one rewording per email, stored in the `phrase` table
/// (so the rows the list returns already carry them) and echoed back.
///
/// LLM calls take seconds, not milliseconds, so this runs fewer threads than
/// the rules evaluator — and every per-email failure carries the connection
/// error, because "the text model is not running" is the normal state of
/// the two-model workflow, not an exception.
pub fn phrasers_eval(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "POST" {
        return Response::bad_request("POST a JSON body: {ids: […]}");
    }
    let Some(body) = req.json() else {
        return Response::bad_request("expected a JSON body");
    };
    let ids: Vec<String> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Response::bad_request("no ids");
    }

    let (phrasers, errors) = crate::phrasers::load(&app.root);
    if phrasers.is_empty() {
        return json!({ "ok": false, "error": "no phrasers/*.yaml on disk", "errors": errors }).into();
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for id in &ids {
        match app.db.message_row(id) {
            Some(r) => rows.push(r),
            None => missing.push(id.clone()),
        }
    }

    let results: std::sync::Mutex<Vec<Value>> = std::sync::Mutex::new(vec![json!(null); rows.len()]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = rows.len().min(4).max(1);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= rows.len() {
                    break;
                }
                let r = crate::phrasers::evaluate(&app.llm, &phrasers, &rows[i]);
                // A rewording is stored the moment it arrives, so a later
                // email's failure cannot take down an earlier email's phrase.
                if r.get("ok").and_then(|v| v.as_bool()) == Some(true)
                    && r.get("matched").and_then(|v| v.as_i64()) == Some(1)
                {
                    app.db.upsert_phrase(
                        &r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        r.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("summary").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("rule").and_then(|v| v.as_str()).unwrap_or(""),
                    );
                }
                results.lock().unwrap()[i] = r;
            });
        }
    });
    let results = results.into_inner().unwrap();
    let phrased = results
        .iter()
        .filter(|r| {
            r.get("ok").and_then(|v| v.as_bool()) == Some(true)
                && r.get("matched").and_then(|v| v.as_i64()) == Some(1)
        })
        .count();
    let failed = results
        .iter()
        .filter(|r| r.get("ok").and_then(|v| v.as_bool()) == Some(false))
        .count();

    json!({
        "ok": failed == 0,
        "results": results,
        "phrased": phrased,
        "rules": phrasers.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "errors": errors,
        "missing": missing,
        "llm": { "url": app.llm.base, "healthy": app.llm.healthy() },
    })
    .into()
}

/// Drop every stored rewording. The next poll shows the rows' own words again.
pub fn phrases_clear(app: &Arc<App>, req: &Request) -> Response {
    if req.method != "DELETE" {
        return Response::bad_request("DELETE to clear all stored phrases");
    }
    let removed = app.db.clear_phrases();
    json!({ "ok": true, "removed": removed }).into()
}

pub fn status(app: &Arc<App>, _req: &Request) -> Response {
    json!({
        "ok": true,
        "counts": app.db.counts(),
        "lastPull": app.db.meta_json("lastPull"),
        "lastPush": app.db.meta_json("lastPush"),
        "pulling": sync::PULLING.load(std::sync::atomic::Ordering::SeqCst) != 0,
        "pushing": sync::PUSHING.load(std::sync::atomic::Ordering::SeqCst) != 0,
        "workers": sync::workers_hint(),
    })
    .into()
}

pub fn mutations(app: &Arc<App>, req: &Request) -> Response {
    let limit = req
        .param("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(40)
        .clamp(1, 200);
    json!({ "ok": true, "mutations": app.db.recent_mutations(limit) }).into()
}
