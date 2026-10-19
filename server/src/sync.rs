//! The two directions this app moves data: **pull** (Gmail → SQLite) and
//! **push** (the queued mutations → Gmail).
//!
//! ## pull
//!
//! `users.messages.list` is cheap and returns *ids only*, so a sync is two
//! phases and the timing of each is reported separately:
//!
//! ```text
//! phase 1  list, paged at 500/page          ~1 request per 500 messages
//! phase 2  get, one per message, N workers  the part that actually costs
//! ```
//!
//! Phase 2 is where "how fast is it" is decided, so it runs `workers` threads
//! over one shared `ureq::Agent` (one TLS connection pool, keep-alive) pulling
//! ids off a shared cursor. `format=metadata` is the default: subject, from,
//! snippet and labelIds for every message, without dragging the bodies across.
//! `full` is opt-in and caches the bodies in SQLite so the detail view is then
//! a pure local read.
//!
//! ## push
//!
//! The outbox is drained in the order it was filled, but *grouped*: consecutive
//! queued mutations that add and remove the same labels go out as one
//! `batchModify`, so selecting 40 rows and pressing Archive is one request, not
//! 40. `trash` and `delete` have no batch verb, so those stay per-message.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};

use crate::db::{Db, Queued};
use crate::gmail::Gmail;

pub struct PullOpts {
    pub query: String,
    pub full: bool,
    /// For inbox pulls this is the metadata page size per pull (the listing
    /// itself is always complete); for other queries it caps the listing.
    pub max: u32,
    pub workers: usize,
    /// Requests per second the client is allowed to spend (see `gmail::Bucket`).
    pub rate: f64,
    /// `false` = list ids only. Useful for measuring the listing phase, and for
    /// a first pass that costs one request per 500 messages.
    pub fetch: bool,
}

impl Default for PullOpts {
    fn default() -> Self {
        PullOpts {
            query: "in:inbox".into(),
            full: false,
            max: 1000,
            workers: 12,
            rate: 20.0,
            fetch: true,
        }
    }
}

pub fn pull(db: &Db, gmail: &Gmail, opts: &PullOpts) -> Value {
    let started = Instant::now();
    gmail.set_rate(opts.rate);
    let began_ms = crate::db::now_ms();
    let workers = opts.workers.clamp(1, 32);

    // ---- phase 1 · list ----------------------------------------------------
    // Primary only: the Social and Promotions tabs are triaged in Gmail, not
    // here. (Updates/Forums stay: transaction alerts and statements arrive as
    // CATEGORY_UPDATES.) An explicit `category:` in the query is respected.
    let is_inbox = opts.query.contains("inbox");
    let query = if is_inbox && !opts.query.contains("category:") {
        format!("{} -category:{{social promotions}}", opts.query)
    } else {
        opts.query.clone()
    };
    let mut ids: Vec<(String, String)> = Vec::new();
    let mut page_token: Option<String> = None;
    let mut pages = 0usize;
    let mut complete = false;
    let mut errors: Vec<String> = Vec::new();
    loop {
        pages += 1;
        match gmail.list(&query, 500, page_token.as_deref()) {
            Ok((batch, next)) => {
                ids.extend(batch);
                page_token = next;
                if page_token.is_none() {
                    complete = true;
                    break;
                }
                // An inbox listing is the mirror's source of truth: page it
                // fully so the flags reconcile. Other queries keep the cap.
                if !is_inbox && ids.len() as u32 >= opts.max {
                    break;
                }
            }
            Err(e) => {
                errors.push(format!("list: {e}"));
                break;
            }
        }
    }
    if !is_inbox {
        ids.truncate(opts.max as usize);
    }
    let list_ms = started.elapsed().as_millis() as i64;

    // ---- phase 2 · get -----------------------------------------------------
    let listed = ids.len();
    let fresh = db.upsert_listed(&ids);
    let plain: Vec<String> = ids.iter().map(|(id, _)| id.clone()).collect();
    // Listing is the source of truth for "this id is in the inbox right now".
    // The fetch below only runs for rows missing metadata, so without this the
    // other 990 listed ids stay out of the Inbox chip.
    if is_inbox {
        db.mark_listed_inbox(&plain);
        db.backlog_merge(&plain);
    }
    // The read cursor: Gmail lists newest-first, so the backlog walks from
    // the old end, one `max`-sized page of metadata per pull. New arrivals
    // are appended after the oldest ord and picked up by the next pages.
    let todo = if opts.fetch {
        if is_inbox {
            db.needs_fetch(&db.backlog_batch(opts.max as usize), opts.full)
        } else {
            db.needs_fetch(&plain, opts.full)
        }
    } else {
        Vec::new()
    };
    let todo_n = todo.len();

    let queue = Mutex::new(VecDeque::from(todo));
    let done = AtomicUsize::new(0);
    let got_body = AtomicUsize::new(0);
    // A quota wall is not an error to keep hitting: the workers stop taking work
    // and the rows left behind are picked up by the next pull.
    let halted = AtomicBool::new(false);
    let fails: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let format = if opts.full { "full" } else { "metadata" };

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if halted.load(Ordering::Relaxed) {
                    return;
                }
                let id = match queue.lock().unwrap().pop_front() {
                    Some(id) => id,
                    None => return,
                };
                match gmail.get(&id, format) {
                    Ok(doc) => {
                        db.apply_message(&id, &doc, opts.full);
                        done.fetch_add(1, Ordering::Relaxed);
                        if opts.full {
                            got_body.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        if crate::gmail::hit_quota(&e) {
                            halted.store(true, Ordering::SeqCst);
                            fails
                                .lock()
                                .unwrap()
                                .push(format!("{id}: {}", &e[crate::gmail::QUOTA_MARK.len()..]));
                        } else {
                            fails.lock().unwrap().push(format!("{id}: {e}"));
                        }
                    }
                }
            });
        }
    });
    let fetch_ms = started.elapsed().as_millis() as i64 - list_ms;

    // An inbox-scoped pull *that enumerated everything* also says what is no
    // longer in the inbox. A partial pull (max cut the paging short) must not
    // claim that: it would mark every message past the cut as gone, which is
    // exactly the difference between a filtered view and a broken mirror.
    // Inbox listings are always complete now, so this runs every inbox pull —
    // including for the Social/Promotions rows already mirrored, which leave
    // the Inbox chip (they stay under All).
    let gone = if is_inbox && complete {
        db.mark_unseen(began_ms) as u64
    } else {
        0
    };
    let backlog = if is_inbox {
        db.backfill_advance();
        db.backfill_remaining()
    } else {
        0
    };

    let fetched = done.load(Ordering::Relaxed);
    let quota = halted.load(Ordering::Relaxed);
    let errors = {
        let mut e = fails.lock().unwrap().clone();
        e.truncate(8);
        e
    };
    let total_ms = started.elapsed().as_millis() as i64;
    let stats = json!({
        "ok": errors.is_empty(),
        "query": query,
        "format": format,
        "listed": listed,
        "pages": pages,
        "complete": complete,
        "new": fresh,
        "fetched": fetched,
        "skipped": todo_n.saturating_sub(fetched),
        "bodies": got_body.load(Ordering::Relaxed),
        "gone": gone,
        "backlog": backlog,
        "workers": workers,
        "listMs": list_ms,
        "fetchMs": fetch_ms,
        "totalMs": total_ms,
        "perSecond": if fetch_ms > 0 { (fetched as f64 * 1000.0 / fetch_ms as f64) as i64 } else { 0 },
        "quota": quota,
        "rate": opts.rate,
        "at": began_ms,
        "errors": errors,
    });
    db.set_meta("lastPull", &stats);
    stats
}

pub fn push(db: &Db, gmail: &Gmail, retry_failed: bool) -> Value {
    let started = Instant::now();
    if retry_failed {
        db.requeue_failed();
    }
    let queued = db.queued();
    if queued.is_empty() {
        return json!({ "ok": true, "applied": 0, "failed": 0, "batches": 0, "ms": 0, "errors": [] });
    }

    let mut errors: Vec<String> = Vec::new();
    let mut applied = 0usize;
    let mut failed = 0usize;
    let mut batches = 0usize;

    // Resolve any Move destination to a label id first: a Move typed before the
    // label table was cached (or naming a folder that does not exist yet) is a
    // label *name* in `names` and nothing in `add`.
    let raw_labels = gmail.labels().unwrap_or_default();
    if !raw_labels.is_empty() {
        db.put_labels(&raw_labels);
    }
    let mut label_cache: Vec<(String, String)> = raw_labels
        .into_iter()
        .map(|(id, name, _kind)| (id, name))
        .collect();
    let resolve = |name: &str, cache: &mut Vec<(String, String)>, gmail: &Gmail| -> Option<String> {
        if let Some((id, _)) = cache.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)) {
            return Some(id.clone());
        }
        match gmail.create_label(name) {
            Ok((id, canonical)) => {
                cache.push((id.clone(), canonical));
                Some(id)
            }
            Err(_) => None,
        }
    };

    // Group: op + exact add/remove signature → one batchModify.
    let mut groups: Vec<(Vec<String>, Vec<i64>, Vec<String>, Vec<String>)> = Vec::new(); // (message ids, mutation ids, add, remove)
    let mut singles: Vec<&Queued> = Vec::new();

    for m in &queued {
        if m.op == "trash" || m.op == "delete" {
            singles.push(m);
            continue;
        }
        let mut add = m.add.clone();
        let names = m.names.clone();
        // Move destinations that were queued as names, not ids.
        if m.op == "move" {
            for name in names.clone() {
                match resolve(&name, &mut label_cache, gmail) {
                    Some(id) => {
                        if !add.iter().any(|x| x == &id) {
                            add.push(id);
                        }
                    }
                    None => {
                        errors.push(format!("label “{name}” could not be created"));
                    }
                }
            }
        }
        if let Some(g) = groups
            .iter_mut()
            .find(|(_, _, ga, gr)| *ga == add && *gr == m.remove)
        {
            g.0.push(m.message_id.clone());
            g.1.push(m.id);
        } else {
            groups.push((vec![m.message_id.clone()], vec![m.id], add, m.remove.clone()));
        }
    }

    // ---- batched modifies --------------------------------------------------
    for (ids, mids, add, remove) in groups {
        for chunk in ids.chunks(50) {
            batches += 1;
            let mids_here: Vec<i64> = mids
                .iter()
                .enumerate()
                .filter(|(i, _)| *i < chunk.len())
                .map(|(_, id)| *id)
                .collect();
            let result = if chunk.len() == 1 {
                gmail.modify(&chunk[0], &add, &remove)
            } else {
                gmail.batch_modify(chunk, &add, &remove)
            };
            match result {
                Ok(()) => {
                    db.mark(&mids_here, "applied", "");
                    applied += mids_here.len();
                    // The local mirror should agree with what we just told Gmail.
                    for id in chunk {
                        apply_locally(db, id, &add, &remove);
                    }
                }
                Err(e) => {
                    db.mark(&mids_here, "failed", &e);
                    failed += mids_here.len();
                    errors.push(e);
                }
            }
        }
    }

    // ---- per-message verbs -------------------------------------------------
    for m in singles {
        let result = match m.op.as_str() {
            "trash" => gmail.trash(&m.message_id),
            "delete" => gmail.delete(&m.message_id),
            other => Err(format!("unknown op {other}")),
        };
        match result {
            Ok(()) => {
                db.mark(&[m.id], "applied", "");
                applied += 1;
                if m.op == "trash" {
                    apply_locally(db, &m.message_id, &["TRASH".into()], &["INBOX".into(), "UNREAD".into()]);
                } else {
                    let c = db.conn.lock().unwrap();
                    let _ = c.execute("DELETE FROM message WHERE id = ?1", rusqlite::params![m.message_id]);
                }
            }
            Err(e) => {
                db.mark(&[m.id], "failed", &e.to_string());
                failed += 1;
                errors.push(e.to_string());
            }
        }
    }

    let ms = started.elapsed().as_millis() as i64;
    errors.truncate(8);
    let report = json!({
        "ok": failed == 0,
        "queued": queued.len(),
        "applied": applied,
        "failed": failed,
        "batches": batches,
        "ms": ms,
        "at": crate::db::now_ms(),
        "errors": errors,
    });
    db.set_meta("lastPush", &report);
    report
}

/// Write the mutation's effect into the mirror as soon as Gmail accepted it, so
/// the next read does not have to wait for a pull to show the truth.
fn apply_locally(db: &Db, id: &str, add: &[String], remove: &[String]) {
    let c = db.conn.lock().unwrap();
    let labels: String = c
        .query_row("SELECT labels FROM message WHERE id = ?1", rusqlite::params![id], |r| r.get(0))
        .unwrap_or_else(|_| "[]".into());
    let mut current: Vec<String> =
        serde_json::from_str(&labels).unwrap_or_else(|_| Vec::<String>::new());
    for a in add {
        if !current.iter().any(|x| x == a) {
            current.push(a.clone());
        }
    }
    for r in remove {
        current.retain(|x| x != r);
    }
    let inbox = current.iter().any(|l| l == "INBOX");
    let _ = c.execute(
        "UPDATE message SET labels = ?2, in_inbox = ?3, updated_ms = ?4 WHERE id = ?1",
        rusqlite::params![id, json!(current).to_string(), inbox, crate::db::now_ms()],
    );
}

/// A monotonic counter used by the routes to reject a second concurrent pull.
pub static PULLING: AtomicI64 = AtomicI64::new(0);
pub static PUSHING: AtomicI64 = AtomicI64::new(0);

pub fn busy(flag: &AtomicI64) -> bool {
    flag.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err()
}

pub fn idle(flag: &AtomicI64) {
    flag.store(0, Ordering::SeqCst);
}

pub fn workers_hint() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}
