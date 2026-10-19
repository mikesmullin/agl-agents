//! mail1 backend.
//!
//! The dev server, the HMR socket, the watcher and the wasm rebuild come from
//! `rustex1-devserver`; this file is the configuration, the SQLite handle, the
//! Gmail client, and the app's own JSON routes.

mod db;
mod gmail;
mod phrasers;
mod rules;
mod routes;
mod sync;

use std::path::Path;
use std::sync::Arc;

use rustex1_devserver::{args_to_config, Config, Server};

use db::Db;
use gmail::Gmail;
use routes::App;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let (config, rebuild) = args_to_config(&args, Config::discover(&start));

    let db_path = config.root.join("data/mail.db");
    let db = match Db::open(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("mail1: {e}");
            std::process::exit(1);
        }
    };
    let counts = db.counts();
    println!(
        "[    0.0s] db      sqlite at {} · {} messages, {} queued",
        db_path.display(),
        counts["total"].as_i64().unwrap_or(0),
        counts["queued"].as_i64().unwrap_or(0)
    );

    // Gmail is optional at boot on purpose: without credentials the mirror is
    // still browsable and the outbox still queues — the routes answer 503.
    let gmail = match gmail::Creds::load(&config.root) {
        Ok(creds) => {
            println!("[    0.0s] gmail   oauth client {}…", &creds.client_id[..12.min(creds.client_id.len())]);
            Some(Arc::new(Gmail::new(creds)))
        }
        Err(e) => {
            println!("[    0.0s] gmail   OFFLINE — {e}");
            None
        }
    };

    // The label table is the Move dropdown's data. Read it once at boot so the
    // first Move is already typed against real folder names.
    if let Some(gmail) = gmail.as_ref() {
        match gmail.labels() {
            Ok(list) => {
                let user = list.iter().filter(|(_, _, kind)| kind != "system").count();
                db.put_labels(&list);
                println!("[    0.0s] labels  {} total, {user} user folders", list.len());
            }
            Err(e) => println!("[    0.0s] labels  could not read Gmail labels: {e}"),
        }
    }

    let app = Arc::new(App {
        db,
        gmail,
        root: config.root.clone(),
        model: Arc::new(rules::SystemOne::new(
            &std::env::var("SYSTEMONE_URL")
                .unwrap_or_else(|_| rules::DEFAULT_URL.to_string()),
        )),
        llm: Arc::new(phrasers::Llm::new(
            &std::env::var("LLM_URL").unwrap_or_else(|_| phrasers::DEFAULT_URL.to_string()),
        )),
    });
    let a1 = Arc::clone(&app);
    let a2 = Arc::clone(&app);
    let a3 = Arc::clone(&app);
    let a4 = Arc::clone(&app);
    let a5 = Arc::clone(&app);
    let a6 = Arc::clone(&app);
    let a7 = Arc::clone(&app);
    let a8 = Arc::clone(&app);
    let a9 = Arc::clone(&app);
    let a10 = Arc::clone(&app);
    let a11 = Arc::clone(&app);
    let a12 = Arc::clone(&app);
    let a13 = Arc::clone(&app);
    let a14 = Arc::clone(&app);

    // Say what the rule engine has to work with: how many rules parsed, how many
    // did not, and whether anything answers /v1/systemone.
    let (rule_list, rule_errors) = rules::load(&app.root);
    println!(
        "[    0.0s] rules   {} rule(s) in rules/{}",
        rule_list.len(),
        if rule_errors.is_empty() {
            String::new()
        } else {
            format!(", {} rejected: {}", rule_errors.len(), rule_errors[0])
        }
    );
    println!(
        "[    0.0s] model   {} — {}",
        app.model.base,
        if app.model.healthy() { "answering /health" } else { "no answer (rules evaluate to an error)" }
    );
    // Phrasers live next door to rules and are listed the same way; the LLM
    // itself is only checked on demand, because the two-model workflow means
    // it is usually *not* running when the server boots.
    let (phraser_list, phraser_errors) = phrasers::load(&app.root);
    println!(
        "[    0.0s] phraser {} rule(s) in phrasers/{}",
        phraser_list.len(),
        if phraser_errors.is_empty() {
            String::new()
        } else {
            format!(", {} rejected: {}", phraser_errors.len(), phraser_errors[0])
        }
    );
    println!("[    0.0s] llm     {} (text slot — checked when phrasing)", app.llm.base);

    let mut server = Server::new(config)
        .api("/api/mail", move |_state, req| routes::mail(&a1, req))
        .api("/api/mail-one", move |_state, req| routes::mail_one(&a2, req))
        .api("/api/queue", move |_state, req| match req.method.as_str() {
            "POST" => routes::queue(&a3, req),
            _ => routes::queue_cancel(&a3, req),
        })
        .api("/api/pull", move |_state, req| routes::pull(&a4, req))
        .api("/api/push", move |_state, req| routes::push(&a5, req))
        .api("/api/labels", move |_state, req| routes::labels(&a6, req))
        .api("/api/mail-status", move |_state, req| routes::status(&a7, req))
        .api("/api/mutations", move |_state, req| routes::mutations(&a8, req))
        .api("/api/rules", move |_state, req| routes::rules(&a9, req))
        .api("/api/rules/eval", move |_state, req| routes::rules_eval(&a10, req))
        .api("/api/rules/new", move |_state, req| routes::rules_new(&a14, req))
        .api("/api/phrasers", move |_state, req| routes::phrasers(&a11, req))
        .api("/api/phrasers/eval", move |_state, req| routes::phrasers_eval(&a12, req))
        .api("/api/phrases", move |_state, req| routes::phrases_clear(&a13, req));

    if rebuild {
        server = server.rebuild_on_start();
    }
    server.run();
}
