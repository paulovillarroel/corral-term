//! `corral _lsp r` with Ark: the editor's R language server is Ark's own,
//! inside the console's R. Fresh talks LSP over stdin/stdout; Ark starts its
//! server when a client opens the `positron.lsp` comm and then listens on a
//! local port. This asks the kernel for that server and copies bytes both
//! ways. Without Ark (or for Python), the Python version starts the usual one.

use crate::console;
use crate::env::{self, python_fallback};
use crate::herdr;
use crate::json::{obj, Value};
use crate::zmtp::{self, Connection, Socket};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn run(args: &[String], all: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str).unwrap_or("r") != "r" {
        python_fallback(all);
    }
    let mut conn = ark_connection();
    if conn.is_none() && console::r_flavor(&env::config()) == "ark" {
        // The editor may start before the console: wait for it.
        let end = Instant::now() + Duration::from_secs(60);
        while conn.is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_secs(1));
            conn = ark_connection();
        }
    }
    let Some(conn) = conn else { python_fallback(all) }; // plain R: languageserver
    bridge(&conn)
}

/// Connection file of the R console's Ark kernel: this workspace's, or one in this folder.
fn ark_connection() -> Option<String> {
    let usable = |s: &Value| s.get("ark_connection").str().filter(|c| Path::new(c).exists()).map(String::from);
    let sessions = env::sessions();
    let ws = env::current_workspace();
    for scope in [ws.as_deref(), None] {
        let Ok(panes) = herdr::panes(scope) else { continue };
        if let Some(console) = herdr::labeled(&panes, "corral:r") {
            let id = herdr::pane_id(console);
            if let Some(c) = sessions.iter().find(|s| s.get("pane_id").str() == Some(id)).and_then(usable) {
                return Some(c);
            }
        }
        if scope.is_none() {
            break;
        }
    }
    let here = std::env::current_dir().ok()?;
    let mut candidates: Vec<&Value> = sessions.iter().filter(|s| usable(s).is_some()).collect();
    candidates.sort_by(|a, b| b.get("started").num().unwrap_or(0.0).total_cmp(&a.get("started").num().unwrap_or(0.0)));
    candidates.into_iter()
        .find(|s| s.get("cwd").str().map(|c| here.starts_with(c)).unwrap_or(false))
        .and_then(usable)
}

fn io_err(e: std::io::Error) -> String {
    format!("Ark: {e}")
}

fn bridge(connection_file: &str) -> Result<(), String> {
    let conn = Connection::read(connection_file)?;
    let comm_id = zmtp::uuid();
    let session = zmtp::uuid();

    // Listen to IOPub first, so the reply can't be missed.
    let mut sub = Socket::connect(&conn.addr(conn.iopub), "SUB").map_err(io_err)?;
    sub.subscribe_all().map_err(io_err)?;
    let (tx, rx) = mpsc::channel();
    let wanted = comm_id.clone();
    std::thread::spawn(move || {
        while let Ok(parts) = sub.recv() {
            let Some((header, content)) = zmtp::parse_message(&parts) else { continue };
            if header.get("msg_type").str() == Some("comm_msg") && content.get("comm_id").str() == Some(&wanted) {
                if let Some(port) = content.get("data").get("content").get("port").num() {
                    let _ = tx.send(port as u16);
                    return;
                }
            }
        }
    });
    std::thread::sleep(Duration::from_millis(100)); // let the subscription reach the kernel

    let mut shell = Socket::connect(&conn.addr(conn.shell), "DEALER").map_err(io_err)?;
    let open = obj(vec![
        ("comm_id", comm_id.as_str().into()),
        ("target_name", "positron.lsp".into()),
        ("data", obj(vec![("ip_address", "127.0.0.1".into())])),
    ]);
    zmtp::send_message(&mut shell, &conn.key, &session, "comm_open", &open).map_err(io_err)?;
    let port = rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "Ark no inició su servidor de lenguaje".to_string())?;

    // Copy bytes: editor -> Ark on a thread, Ark -> editor here.
    let lsp = TcpStream::connect(("127.0.0.1", port)).map_err(io_err)?;
    let mut to_ark = lsp.try_clone().map_err(io_err)?;
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 65536];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || to_ark.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = to_ark.shutdown(Shutdown::Write);
    });
    let mut from_ark = lsp;
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 65536];
    while let Ok(n) = from_ark.read(&mut buf) {
        if n == 0 || stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
    let close = obj(vec![("comm_id", comm_id.as_str().into()), ("data", Value::obj())]);
    let _ = zmtp::send_message(&mut shell, &conn.key, &session, "comm_close", &close);
    Ok(())
}
