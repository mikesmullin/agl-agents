//! SQLite storage for mail1: the local mirror of the Gmail inbox, plus the
//! queue of mutations waiting to be pushed back.
//!
//! Three tables, one idea each:
//!
//! * `message`  — one row per Gmail message. Everything the list and the detail
//!   view render comes from here; the network is never on the render path.
//! * `mutation` — the outbox. A UI action inserts rows here and the UI renders
//!   their *effect* immediately (see [`effective_labels`]); `POST /api/push`
//!   drains them to Gmail and stamps the result.
//! * `label`    — Gmail's label table, cached so a Move can show names without
//!   another round trip.
//!
//! Timestamps are epoch **milliseconds** in integer columns: sorting, comparing
//! and "was this row seen in that pull?" are all arithmetic, and the ISO string
//! is derived (`date_iso`) only for display.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS message (
  id            TEXT PRIMARY KEY,          -- Gmail message id
  thread_id     TEXT NOT NULL DEFAULT '',
  internal_date INTEGER NOT NULL DEFAULT 0, -- ms epoch, Gmail's own clock
  date_iso      TEXT NOT NULL DEFAULT '',   -- display string, derived
  from_name     TEXT NOT NULL DEFAULT '',
  from_email    TEXT NOT NULL DEFAULT '',
  from_raw      TEXT NOT NULL DEFAULT '',
  to_raw        TEXT NOT NULL DEFAULT '',
  subject       TEXT NOT NULL DEFAULT '',
  snippet       TEXT NOT NULL DEFAULT '',
  body_text     TEXT NOT NULL DEFAULT '',
  body_html     TEXT NOT NULL DEFAULT '',
  body_kind     TEXT NOT NULL DEFAULT '',   -- html | text | ''
  labels        TEXT NOT NULL DEFAULT '[]', -- JSON array of label ids
  size          INTEGER NOT NULL DEFAULT 0,
  history_id    TEXT NOT NULL DEFAULT '',
  got_meta      INTEGER NOT NULL DEFAULT 0, -- a messages.get has filled the row
  has_body      INTEGER NOT NULL DEFAULT 0, -- a format=full fetch has landed
  in_inbox      INTEGER NOT NULL DEFAULT 1, -- last inbox-scoped pull saw it
  first_seen_ms INTEGER NOT NULL DEFAULT 0,
  last_seen_ms  INTEGER NOT NULL DEFAULT 0,
  updated_ms    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS message_date  ON message(internal_date DESC);
CREATE INDEX IF NOT EXISTS message_inbox ON message(in_inbox);

CREATE TABLE IF NOT EXISTS mutation (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  message_id   TEXT NOT NULL,
  op           TEXT NOT NULL,               -- modify | trash | delete
  add_labels   TEXT NOT NULL DEFAULT '[]',  -- JSON arrays of label *ids*
  remove_labels TEXT NOT NULL DEFAULT '[]',
  label_names  TEXT NOT NULL DEFAULT '[]',  -- display names, for the chip
  instruction  TEXT NOT NULL DEFAULT '',
  rationale    TEXT NOT NULL DEFAULT '',
  status       TEXT NOT NULL DEFAULT 'queued', -- queued | applied | failed
  error        TEXT NOT NULL DEFAULT '',
  attempts     INTEGER NOT NULL DEFAULT 0,
  created_ms   INTEGER NOT NULL DEFAULT 0,
  applied_ms   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS mutation_status ON mutation(status);
CREATE INDEX IF NOT EXISTS mutation_msg   ON mutation(message_id);

CREATE TABLE IF NOT EXISTS label (
  id      TEXT PRIMARY KEY,
  name    TEXT NOT NULL,
  kind    TEXT NOT NULL DEFAULT 'user',
  seen_ms INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS phrase (
  message_id TEXT PRIMARY KEY,           -- Gmail message id
  subject    TEXT NOT NULL DEFAULT '',  -- the rewording, shown in place of the row's own
  summary    TEXT NOT NULL DEFAULT '',
  rule       TEXT NOT NULL DEFAULT '',  -- which phraser wrote it
  created_ms INTEGER NOT NULL DEFAULT 0
);
-- The oldest-first read cursor. Gmail lists newest-first, so ord 0 is the
-- newest id seen and larger ords are older. `backfill_ord` (in meta) is the
-- largest ord fully fetched; each inbox pull fetches the next page of ords
-- above it. Ids that leave the inbox are deleted by the merge below.
CREATE TABLE IF NOT EXISTS backlog (
  message_id TEXT PRIMARY KEY,
  ord        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS backlog_ord ON backlog(ord);
"#;

/// `got_meta` arrived after the first rows were pulled, so an existing database
/// needs it added rather than deleted.
fn migrate(conn: &Connection) -> Result<(), String> {
    let mut have = false;
    if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(message)") {
        if let Ok(it) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            for name in it.filter_map(|x| x.ok()) {
                if name == "got_meta" {
                    have = true;
                }
            }
        }
    }
    if !have {
        conn.execute(
            "ALTER TABLE message ADD COLUMN got_meta INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("migration failed: {e}"))?;
    }
    Ok(())
}

pub struct Db {
    pub conn: Mutex<Connection>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(path: &Path) -> Result<Db, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("cannot open sqlite: {e}"))?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("cannot create schema: {e}"))?;
        migrate(&conn)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    // ---------------------------------------------------------------- meta

    pub fn meta(&self, key: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    pub fn meta_json(&self, key: &str) -> Value {
        self.meta(key)
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or(Value::Null)
    }

    pub fn set_meta(&self, key: &str, value: &Value) {
        let c = self.conn.lock().unwrap();
        let text = value.to_string();
        let _ = c.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, text],
        );
    }

    // ---------------------------------------------------------------- labels

    pub fn labels(&self) -> Vec<(String, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id,name,kind FROM label ORDER BY (kind='system') ASC, name COLLATE NOCASE ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn label_name(&self, id: &str) -> String {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT name FROM label WHERE id = ?1", params![id], |r| {
            r.get::<_, String>(0)
        })
        .unwrap_or_else(|_| id.to_string())
    }

    pub fn label_id_by_name(&self, name: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT id FROM label WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    pub fn put_labels(&self, list: &[(String, String, String)]) {
        let now = now_ms();
        let c = self.conn.lock().unwrap();
        for (id, name, kind) in list {
            let _ = c.execute(
                "INSERT INTO label(id,name,kind,seen_ms) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind, seen_ms=excluded.seen_ms",
                params![id, name, kind, now],
            );
        }
    }

    // ---------------------------------------------------------------- messages

    /// Insert the ids a `messages.list` returned. Returns how many were new.
    ///
    /// A list response carries only `id` + `threadId`, so the row starts empty
    /// and the per-message fetch fills it — that is why `has_body` and
    /// `updated_ms` exist: "listed" and "known" are different facts.
    pub fn upsert_listed(&self, ids: &[(String, String)]) -> usize {
        let now = now_ms();
        let c = self.conn.lock().unwrap();
        // `changes()` reports 1 for the ON CONFLICT branch too, so "how many
        // were new" is measured as the row count moving, not per statement.
        let before: i64 = c
            .query_row("SELECT COUNT(*) FROM message", [], |r| r.get(0))
            .unwrap_or(0);
        let mut fresh = 0usize;
        for (id, thread) in ids {
            let _changed = c
                .execute(
                    // updated_ms stays 0: "listed" and "fetched" are different
                    // facts, and `needs_fetch` reads the second one.
                    "INSERT INTO message(id,thread_id,first_seen_ms,last_seen_ms,updated_ms)
                     VALUES(?1,?2,?3,?3,0)
                     ON CONFLICT(id) DO UPDATE SET
                       thread_id = excluded.thread_id,
                       last_seen_ms = excluded.last_seen_ms",
                    params![id, thread, now],
                )
                .unwrap_or(0);
        }
        let after: i64 = c
            .query_row("SELECT COUNT(*) FROM message", [], |r| r.get(0))
            .unwrap_or(0);
        fresh = (after - before).max(0) as usize;
        fresh
    }

    /// Ids just returned by an inbox listing are in the inbox, even when we
    /// already have their metadata and skip the per-message fetch. Without
    /// this, a pull of 1000 known messages updates `last_seen_ms` and leaves
    /// `in_inbox` at 0, so the Inbox chip does not change.
    pub fn mark_listed_inbox(&self, ids: &[String]) -> usize {
        let c = self.conn.lock().unwrap();
        let mut n = 0usize;
        for id in ids {
            n += c
                .execute(
                    "UPDATE message SET in_inbox = 1 WHERE id = ?1 AND in_inbox = 0",
                    params![id],
                )
                .unwrap_or(0);
        }
        n
    }

    /// Merge a complete newest-first inbox listing into the backlog: drop ids
    /// that left the inbox, append new arrivals after the oldest ord so the
    /// next page picks them up promptly.
    pub fn backlog_merge(&self, ids: &[String]) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "CREATE TEMP TABLE IF NOT EXISTS fresh_inbox(id TEXT PRIMARY KEY)",
            [],
        );
        let _ = c.execute("DELETE FROM fresh_inbox", []);
        {
            let mut ins = match c.prepare("INSERT OR IGNORE INTO fresh_inbox(id) VALUES(?1)") {
                Ok(s) => s,
                Err(_) => return,
            };
            for id in ids {
                let _ = ins.execute(params![id]);
            }
        }
        let _ = c.execute(
            "DELETE FROM backlog WHERE message_id NOT IN (SELECT id FROM fresh_inbox)",
            [],
        );
        let mut ord: i64 = c
            .query_row("SELECT COALESCE(MAX(ord), -1) FROM backlog", [], |r| r.get(0))
            .unwrap_or(-1)
            + 1;
        for id in ids {
            let n = c
                .execute(
                    "INSERT OR IGNORE INTO backlog(message_id, ord) VALUES(?1, ?2)",
                    params![id, ord],
                )
                .unwrap_or(0);
            if n == 1 {
                ord += 1;
            }
        }
        let _ = c.execute("DROP TABLE fresh_inbox", []);
    }

    /// The next page from the old end: the `limit` largest ords above the
    /// cursor, i.e. the oldest inbox ids not yet read.
    pub fn backlog_batch(&self, limit: usize) -> Vec<String> {
        let cursor: i64 = self
            .meta("backfill_ord")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        let c = self.conn.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = c.prepare(
            "SELECT message_id FROM backlog WHERE ord > ?1 ORDER BY ord DESC LIMIT ?2",
        ) {
            if let Ok(rows) = stmt.query_map(params![cursor, limit as i64], |r| r.get(0)) {
                for r in rows.filter_map(|x| x.ok()) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// Move the cursor past every ord whose row is fully fetched. Self-healing:
    /// quota-skipped or failed ids stay above the cursor and are retried, and
    /// a row that left the inbox is deleted by the merge instead of blocking.
    pub fn backfill_advance(&self) {
        let min_open: Option<i64> = {
            let c = self.conn.lock().unwrap();
            c.query_row(
                "SELECT MIN(backlog.ord) FROM backlog LEFT JOIN message ON message.id = backlog.message_id
                 WHERE message.got_meta IS NULL OR message.got_meta = 0",
                [],
                |r| r.get(0),
            )
            .unwrap_or(None)
        };
        let cursor: i64 = match min_open {
            Some(o) => o - 1,
            None => {
                let c = self.conn.lock().unwrap();
                c.query_row("SELECT COALESCE(MAX(ord), -1) FROM backlog", [], |r| r.get(0))
                    .unwrap_or(-1)
            }
        };
        self.set_meta("backfill_ord", &json!(cursor));
    }

    /// Backlog rows above the cursor: inbox ids not yet read.
    pub fn backfill_remaining(&self) -> usize {
        let cursor: i64 = self
            .meta("backfill_ord")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM backlog WHERE ord > ?1",
            params![cursor],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Rows that still need a per-message fetch: never fetched, or fetched for
    /// metadata only when this pull asked for bodies.
    pub fn needs_fetch(&self, ids: &[String], full: bool) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let mut out = Vec::new();
        for id in ids {
            let (got_meta, has_body) = c
                .query_row(
                    "SELECT got_meta, has_body FROM message WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )
                .unwrap_or((0, 0));
            if got_meta == 0 || (full && has_body == 0) {
                out.push(id.clone());
            }
        }
        out
    }

    /// Apply one `messages.get` response to its row.
    pub fn apply_message(&self, id: &str, doc: &Value, full: bool) {
        let now = now_ms();
        let internal = doc
            .get("internalDate")
            .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")).map(|s| s.to_string()))
            .unwrap_or_default();
        let internal_ms: i64 = internal.parse().unwrap_or(0);
        let headers = doc
            .get("payload")
            .and_then(|p| p.get("headers"))
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();
        let header = |name: &str| -> String {
            headers
                .iter()
                .find(|h| h.get("name").and_then(|n| n.as_str()) == Some(name))
                .and_then(|h| h.get("value").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string()
        };
        let (from_name, from_email) = split_from(&header("From"));
        let labels: Vec<String> = doc
            .get("labelIds")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let snippet = doc
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let (text, html) = if full {
            crate::gmail::extract_body(doc)
        } else {
            (String::new(), String::new())
        };
        let has_body = full && (!text.is_empty() || !html.is_empty());
        let c = self.conn.lock().unwrap();
        match c.execute(
            "UPDATE message SET
               thread_id=?2, internal_date=?3, date_iso=?4,
               from_name=?5, from_email=?6, from_raw=?7, to_raw=?8,
               subject=?9, snippet=?10,
               body_text=CASE WHEN ?11='' THEN body_text ELSE ?11 END,
               body_html=CASE WHEN ?12='' THEN body_html ELSE ?12 END,
               body_kind=CASE WHEN ?13='' THEN body_kind ELSE ?13 END,
               labels=?14, size=?15, history_id=?16,
               got_meta=1, has_body=MAX(has_body, ?17),
               in_inbox=?18,
               last_seen_ms=?19, updated_ms=?19
             WHERE id=?1",
            params![
                id,
                doc.get("threadId").and_then(|v| v.as_str()).unwrap_or(""),
                internal_ms,
                iso_from_ms(internal_ms),
                from_name,
                from_email,
                header("From"),
                header("To"),
                header("Subject"),
                snippet,
                text,
                html,
                if html.is_empty() {
                    if text.is_empty() {
                        ""
                    } else {
                        "text"
                    }
                } else {
                    "html"
                },
                json!(labels).to_string(),
                doc.get("sizeEstimate").and_then(|v| v.as_i64()).unwrap_or(0),
                doc.get("historyId").and_then(|v| v.as_str()).unwrap_or(""),
                has_body as i64,
                labels.iter().any(|l| l == "INBOX"),
                now,
            ],
        ) {
            Ok(_) => {}
            Err(e) => println!("[apply] {id} failed: {e}"),
        }
    }

    /// An inbox-scoped pull is also a statement about what is *not* in the
    /// inbox any more: every row it did not report gets `in_inbox = 0`.
    pub fn mark_unseen(&self, since_ms: i64) -> usize {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE message SET in_inbox = 0 WHERE in_inbox = 1 AND last_seen_ms < ?1",
            params![since_ms],
        )
        .unwrap_or(0)
    }

    /// The `WHERE` a filter + a search means, spelled **once** so the window the
    /// UI renders and the total it reports cannot disagree about what "matches"
    /// is.
    fn list_where(filter: &str, q: &str) -> String {
        let mut sql = String::from("WHERE 1=1");
        match filter {
            // got_meta = 0 is an id listed but not yet fetched: an empty
            // sender and subject, which reads as a blank row.
            "inbox" => sql.push_str(" AND in_inbox = 1 AND got_meta = 1"),
            "unread" => sql.push_str(" AND in_inbox = 1 AND got_meta = 1 AND instr(labels,'\"UNREAD\"') > 0"),
            "starred" => sql.push_str(" AND instr(labels,'\"STARRED\"') > 0"),
            "pending" => sql.push_str(" AND EXISTS (SELECT 1 FROM mutation m WHERE m.message_id = message.id AND m.status='queued')"),
            "all" => {}
            _ => {}
        }
        if !q.is_empty() {
            sql.push_str(" AND (subject LIKE ?1 COLLATE NOCASE OR from_name LIKE ?1 COLLATE NOCASE \
                           OR from_email LIKE ?1 COLLATE NOCASE OR snippet LIKE ?1 COLLATE NOCASE)");
        }
        sql
    }

    /// How many rows of the **whole mirror** match filter + q, with no window
    /// limit. Without this the UI can only ever say "the 1000 rows I hold".
    pub fn count_matching(&self, filter: &str, q: &str) -> i64 {
        let where_sql = Self::list_where(filter, q);
        let c = self.conn.lock().unwrap();
        let sql = format!("SELECT COUNT(*) FROM message {where_sql}");
        let Ok(mut stmt) = c.prepare(&sql) else {
            return 0;
        };
        if q.is_empty() {
            // The statement has no `?1`; binding one anyway is a parameter-count
            // error, not a no-op.
            stmt.query_row([], |r| r.get(0)).unwrap_or(0)
        } else {
            stmt.query_row(params![format!("%{q}%")], |r| r.get(0))
                .unwrap_or(0)
        }
    }

    /// The list the UI renders, the counts for its chips, and how many rows the
    /// current filter + search match in total.
    pub fn list(&self, filter: &str, q: &str, limit: i64) -> (Vec<Value>, Value, i64) {
        let where_sql = Self::list_where(filter, q);
        let mut sql = format!(
            "SELECT id,thread_id,internal_date,date_iso,from_name,from_email,from_raw,to_raw,\
             subject,snippet,labels,size,history_id,has_body,in_inbox,body_kind \
             FROM message {where_sql}"
        );
        sql.push_str(" ORDER BY internal_date DESC LIMIT ?2");
        let like = format!("%{q}%");

        // The guard and the statement live in one block: `Statement<'_>` borrows
        // the connection, so both must die before `attach_pending` and `counts`
        // take the lock again.
        let rows: Vec<Value> = {
            let c = self.conn.lock().unwrap();
            let mut out: Vec<Value> = Vec::new();
            let prepared = c.prepare(&sql);
            if let Ok(mut stmt) = prepared {
                if let Ok(it) = stmt.query_map(params![like, limit], |r| {
                        Ok(json!({
                            "id": r.get::<_, String>(0)?,
                            "threadId": r.get::<_, String>(1)?,
                            "dateMs": r.get::<_, i64>(2)?,
                            "dateIso": r.get::<_, String>(3)?,
                            "fromName": r.get::<_, String>(4)?,
                            "fromEmail": r.get::<_, String>(5)?,
                            "fromRaw": r.get::<_, String>(6)?,
                            "toRaw": r.get::<_, String>(7)?,
                            "subject": r.get::<_, String>(8)?,
                            "snippet": r.get::<_, String>(9)?,
                            "labels": serde_json::from_str::<Value>(&r.get::<_, String>(10)?).unwrap_or(json!([])),
                            "size": r.get::<_, i64>(11)?,
                            "historyId": r.get::<_, String>(12)?,
                            "hasBody": r.get::<_, i64>(13)? != 0,
                            "inInbox": r.get::<_, i64>(14)? != 0,
                            "bodyKind": r.get::<_, String>(15)?,
                        }))
                }) {
                    out = it.filter_map(|x| x.ok()).collect();
                }
            }
            out
        };

        let rows = self.attach_pending(rows);
        let rows = self.attach_phrases(rows);
        let counts = self.counts();
        let matched = self.count_matching(filter, q);
        (rows, counts, matched)
    }

    /// Per-row rephrasings, in one query rather than N. A row with no phrase
    /// carries `"phrase": null` so the UI can tell "never phrased" from
    /// "phrased to nothing".
    fn attach_phrases(&self, rows: Vec<Value>) -> Vec<Value> {
        let by_id: std::collections::HashMap<String, Value> = {
            let c = self.conn.lock().unwrap();
            let mut map = std::collections::HashMap::new();
            if let Ok(mut stmt) = c.prepare(
                "SELECT message_id,subject,summary,rule,created_ms FROM phrase",
            ) {
                if let Ok(it) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        json!({
                            "subject": r.get::<_, String>(1)?,
                            "summary": r.get::<_, String>(2)?,
                            "rule": r.get::<_, String>(3)?,
                            "at": r.get::<_, i64>(4)?,
                        }),
                    ))
                }) {
                    for (id, phrase) in it.filter_map(|x| x.ok()) {
                        map.insert(id, phrase);
                    }
                }
            }
            map
        };
        rows.into_iter()
            .map(|mut row| {
                let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let phrase = by_id.get(&id).cloned().unwrap_or(Value::Null);
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("phrase".into(), phrase);
                }
                row
            })
            .collect()
    }

    /// Store one rephrasing. Re-running a phraser overwrites — a phrase is the
    /// latest word on that email, not a history.
    pub fn upsert_phrase(&self, id: &str, subject: &str, summary: &str, rule: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO phrase(message_id,subject,summary,rule,created_ms) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(message_id) DO UPDATE SET subject=excluded.subject, summary=excluded.summary,
             rule=excluded.rule, created_ms=excluded.created_ms",
            rusqlite::params![id, subject, summary, rule, now_ms()],
        );
    }

    /// Drop every rephrasing. The rows go back to their own words on the next
    /// poll; nothing about the mirror itself changes.
    pub fn clear_phrases(&self) -> usize {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM phrase", [])
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// Per-row pending state, in one query rather than N.
    fn attach_pending(&self, rows: Vec<Value>) -> Vec<Value> {
        let mut by_id: std::collections::HashMap<String, Vec<Value>> = {
            let c = self.conn.lock().unwrap();
            let mut map: std::collections::HashMap<String, Vec<Value>> =
                std::collections::HashMap::new();
            if let Ok(mut stmt) = c.prepare(
                "SELECT message_id,op,add_labels,remove_labels,label_names,instruction,rationale,status \
                 FROM mutation WHERE status='queued' ORDER BY id ASC",
            ) {
                if let Ok(it) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        json!({
                            "op": r.get::<_, String>(1)?,
                            "add": serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or(json!([])),
                            "remove": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(json!([])),
                            "names": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(json!([])),
                            "instruction": r.get::<_, String>(5)?,
                            "rationale": r.get::<_, String>(6)?,
                        }),
                    ))
                }) {
                    for row in it.filter_map(|x| x.ok()) {
                        map.entry(row.0).or_default().push(row.1);
                    }
                }
            }
            map
        };

        rows.into_iter()
            .map(|mut row| {
                let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let pend = by_id.remove(&id).unwrap_or_default();
                let labels: Vec<String> = row
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let mut eff = labels.clone();
                for p in &pend {
                    for add in p.get("add").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                        let s = add.as_str().unwrap_or("");
                        if !s.is_empty() && !eff.iter().any(|x| x == s) {
                            eff.push(s.to_string());
                        }
                    }
                    for rem in p.get("remove").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                        let s = rem.as_str().unwrap_or("");
                        eff.retain(|x| x != s);
                    }
                }
                let names: Vec<String> = eff.iter().map(|l| self.label_name(l)).collect();
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("labelNames".into(), json!(names));
                    obj.insert("pending".into(), json!(pend));
                    obj.insert("pendingCount".into(), json!(pend.len()));
                    obj.insert("effLabels".into(), json!(eff));
                    obj.insert(
                        "unread".into(),
                        json!(eff.iter().any(|l| l == "UNREAD")),
                    );
                    obj.insert("starred".into(), json!(eff.iter().any(|l| l == "STARRED")));
                    obj.insert("important".into(), json!(eff.iter().any(|l| l == "IMPORTANT")));
                }
                row
            })
            .collect()
    }

    pub fn counts(&self) -> Value {
        let c = self.conn.lock().unwrap();
        let one = |sql: &str| -> i64 {
            c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0)
        };
        json!({
            "total": one("SELECT COUNT(*) FROM message"),
            "inbox": one("SELECT COUNT(*) FROM message WHERE in_inbox=1 AND got_meta=1"),
            "unread": one("SELECT COUNT(*) FROM message WHERE in_inbox=1 AND got_meta=1 AND instr(labels,'\"UNREAD\"')>0"),
            "starred": one("SELECT COUNT(*) FROM message WHERE instr(labels,'\"STARRED\"')>0"),
            "withBody": one("SELECT COUNT(*) FROM message WHERE has_body=1"),
            "queued": one("SELECT COUNT(*) FROM mutation WHERE status='queued'"),
            "applied": one("SELECT COUNT(*) FROM mutation WHERE status='applied'"),
            "failed": one("SELECT COUNT(*) FROM mutation WHERE status='failed'"),
        })
    }

    pub fn message_row(&self, id: &str) -> Option<Value> {
        let c = self.conn.lock().unwrap();
        let row = c.query_row(
            "SELECT id,thread_id,internal_date,date_iso,from_name,from_email,from_raw,to_raw,\
             subject,snippet,body_text,body_html,body_kind,labels,size,history_id,has_body,in_inbox \
             FROM message WHERE id = ?1 OR id LIKE ?2 LIMIT 2",
            params![id, format!("{id}%")],
            |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "threadId": r.get::<_, String>(1)?,
                    "dateMs": r.get::<_, i64>(2)?,
                    "dateIso": r.get::<_, String>(3)?,
                    "fromName": r.get::<_, String>(4)?,
                    "fromEmail": r.get::<_, String>(5)?,
                    "fromRaw": r.get::<_, String>(6)?,
                    "toRaw": r.get::<_, String>(7)?,
                    "subject": r.get::<_, String>(8)?,
                    "snippet": r.get::<_, String>(9)?,
                    "bodyText": r.get::<_, String>(10)?,
                    "bodyHtml": r.get::<_, String>(11)?,
                    "bodyKind": r.get::<_, String>(12)?,
                    "labels": serde_json::from_str::<Value>(&r.get::<_, String>(13)?).unwrap_or(json!([])),
                    "size": r.get::<_, i64>(14)?,
                    "historyId": r.get::<_, String>(15)?,
                    "hasBody": r.get::<_, i64>(16)? != 0,
                    "inInbox": r.get::<_, i64>(17)? != 0,
                }))
            },
        );
        drop(c);
        match row {
            Ok(v) => self
                .attach_phrases(self.attach_pending(vec![v]))
                .into_iter()
                .next(),
            Err(_) => None,
        }
    }

    // ---------------------------------------------------------------- mutations

    /// Queue one action for one message. Returns the mutation id.
    pub fn queue(&self, m: &Mutation) -> Result<i64, String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO mutation(message_id,op,add_labels,remove_labels,label_names,instruction,rationale,created_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                m.message_id,
                m.op,
                json!(m.add).to_string(),
                json!(m.remove).to_string(),
                json!(m.names).to_string(),
                m.instruction,
                m.rationale,
                now_ms()
            ],
        )
        .map(|_| c.last_insert_rowid())
        .map_err(|e| e.to_string())
    }

    pub fn cancel(&self, message_id: Option<&str>) -> usize {
        let c = self.conn.lock().unwrap();
        match message_id {
            Some(id) => c
                .execute(
                    "DELETE FROM mutation WHERE status='queued' AND message_id = ?1",
                    params![id],
                )
                .unwrap_or(0),
            None => c
                .execute("DELETE FROM mutation WHERE status='queued'", [])
                .unwrap_or(0),
        }
    }

    pub fn queued(&self) -> Vec<Queued> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id,message_id,op,add_labels,remove_labels,label_names,instruction,rationale,attempts \
             FROM mutation WHERE status='queued' ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let arr = stmt.query_map([], |r| {
            Ok(Queued {
                id: r.get(0)?,
                message_id: r.get(1)?,
                op: r.get(2)?,
                add: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                remove: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                names: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                instruction: r.get(6)?,
                rationale: r.get(7)?,
                attempts: r.get(8)?,
            })
        });
        match arr {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Put every failed row back in line — the Push button's "retry failed" option.
    pub fn requeue_failed(&self) -> usize {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE mutation SET status='queued', error='' WHERE status='failed'",
            [],
        )
        .unwrap_or(0)
    }

    pub fn mark(&self, ids: &[i64], status: &str, error: &str) {
        let c = self.conn.lock().unwrap();
        for id in ids {
            let _ = c.execute(
                "UPDATE mutation SET status=?2, error=?3, attempts=attempts+1, applied_ms=?4 WHERE id=?1",
                params![id, status, error, now_ms()],
            );
        }
    }

    pub fn recent_mutations(&self, limit: i64) -> Vec<Value> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id,message_id,op,label_names,status,error,created_ms,applied_ms,instruction \
             FROM mutation ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let it = stmt.query_map(params![limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "messageId": r.get::<_, String>(1)?,
                "op": r.get::<_, String>(2)?,
                "names": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(json!([])),
                "status": r.get::<_, String>(4)?,
                "error": r.get::<_, String>(5)?,
                "createdMs": r.get::<_, i64>(6)?,
                "appliedMs": r.get::<_, i64>(7)?,
                "instruction": r.get::<_, String>(8)?,
            }))
        });
        match it {
            Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

pub struct Mutation {
    pub message_id: String,
    pub op: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub names: Vec<String>,
    pub instruction: String,
    pub rationale: String,
}

pub struct Queued {
    pub id: i64,
    pub message_id: String,
    pub op: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub names: Vec<String>,
    pub instruction: String,
    pub rationale: String,
    pub attempts: i64,
}

// ---------------------------------------------------------------- helpers

/// `"Mike Smullin" <msmullin@example.com>` → (`Mike Smullin`, `msmullin@example.com`).
pub fn split_from(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some((name, rest)) = raw.split_once('<') {
        let email = rest.split('>').next().unwrap_or("").trim().to_string();
        let name = name.trim().trim_matches('"').trim().to_string();
        return (name, email);
    }
    if raw.contains('@') {
        return (raw.to_string(), raw.to_string());
    }
    (raw.to_string(), String::new())
}

/// ms epoch → `2026-02-14 09:41` (UTC), derived by hand.
///
/// No chrono: this is the only calendar arithmetic the app does, and Howard
/// Hinnant's `civil_from_days` is twenty lines with no dependency and no
/// timezone database to get wrong.
pub fn iso_from_ms(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoy = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoy + era * 400;
    let doy = doe - (365 * yoy + yoy / 4 - yoy / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if y <= 0 { y + 1 } else { y }, m, d)
}
