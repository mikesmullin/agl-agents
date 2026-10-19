//! The `/__m_hmr` websocket hub.
//!
//! Wire protocol is the one m-js speaks (see `docs/index.md` → "Adding HMR to a
//! server you already have"):
//!
//! ```jsonc
//! { "type": "connected", "bootId": "…" }        // server -> client, on open
//! { "type": "change",  "path": "/styles.css" }  // server -> client, per file
//! ```
//!
//! plus three messages this prototype adds (a stock m-js client ignores them):
//!
//! ```jsonc
//! { "type": "build",   "phase": "start" | "ok" | "error", … }
//! { "type": "restart", "target": "server" }
//! ```

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::{Error, Message};

use crate::state::log;

const POLL: Duration = Duration::from_millis(20);
const HEARTBEAT: Duration = Duration::from_secs(30);

#[derive(Default, Clone)]
pub struct Hub(Arc<Mutex<HashMap<u64, Sender<String>>>>);

static SEQ: AtomicU64 = AtomicU64::new(1);

impl Hub {
    fn register(&self, tx: Sender<String>) -> u64 {
        let id = SEQ.fetch_add(1, Ordering::SeqCst);
        self.0.lock().unwrap().insert(id, tx);
        id
    }

    fn remove(&self, id: u64) {
        self.0.lock().unwrap().remove(&id);
    }

    pub fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// Non-blocking: a stalled client never blocks the watcher thread. Messages
    /// are queued per connection and drained by that connection's poll loop.
    pub fn broadcast(&self, text: &str) {
        let senders: Vec<Sender<String>> = self.0.lock().unwrap().values().cloned().collect();
        for tx in senders {
            let _ = tx.send(text.to_string());
        }
    }
}

/// Serve one websocket connection. Blocks until the peer goes away.
pub fn session(stream: TcpStream, hub: Hub, boot_id: String) {
    let _ = stream.set_nodelay(true);
    // The opening handshake must block normally; the 20 ms poll clock only
    // starts once the connection is upgraded.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(err) => {
            log("warn", &format!("hmr handshake failed: {err}"));
            return;
        }
    };
    let _ = ws.get_ref().set_read_timeout(Some(POLL));

    let (tx, rx) = mpsc::channel::<String>();
    let id = hub.register(tx);

    let hello = serde_json::json!({
        "type": "connected",
        "bootId": boot_id,
        "server": "rustex1",
        "clients": hub.count(),
    })
    .to_string();

    if push(&mut ws, &hello).is_err() {
        hub.remove(id);
        return;
    }
    log("hmr", &format!("client connected ({})", hub.count()));

    let mut last_beat = Instant::now();
    loop {
        // 1 · anything the watcher queued for this socket
        loop {
            match rx.try_recv() {
                Ok(text) => {
                    if push(&mut ws, &text).is_err() {
                        hub.remove(id);
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    hub.remove(id);
                    return;
                }
            }
        }

        // 2 · unidirectional heartbeat; browsers answer pings for us
        if last_beat.elapsed() >= HEARTBEAT {
            last_beat = Instant::now();
            let _ = ws.write(Message::Ping(Vec::new().into()));
        }
        let _ = ws.flush();

        // 3 · drive the protocol (close handshake, queued pongs, incoming frames)
        match ws.read() {
            Ok(Message::Close(_)) => break,
            Ok(Message::Text(_)) => {}
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(Error::WriteBufferFull(_)) => {}
            Err(_) => break,
        }
    }

    hub.remove(id);
    log("hmr", &format!("client left ({})", hub.count()));
}

fn push<W: std::io::Read + std::io::Write>(
    ws: &mut tungstenite::WebSocket<W>,
    text: &str,
) -> Result<(), Error> {
    ws.write(Message::text(text.to_string()))?;
    ws.flush()
}
