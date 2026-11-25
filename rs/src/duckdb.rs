//! DuckDB for Corral's tables: the viewer and Mirador. Starting the DuckDB CLI
//! costs more than most of the queries they make (8 ms against 1 to 5 ms), so each
//! Corral process that needs it keeps one DuckDB open and sends it every query
//! over stdin, one after another. It closes after 30 s without queries, and with
//! the process that owns it (its stdin closes), so nothing stays open at rest.
//!
//! Each query is followed by `SELECT '<mark>'`: everything DuckDB prints before
//! the mark's row is that query's answer. stderr goes to the same pipe, so an
//! error arrives the same way, as text that isn't JSON.

use crate::env;
use crate::i18n::tr;
use crate::json::{self, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long DuckDB stays open without queries, in milliseconds (30 s; tests shorten it).
static IDLE_MS: AtomicU64 = AtomicU64::new(30_000);

fn idle() -> Duration {
    Duration::from_millis(IDLE_MS.load(Ordering::SeqCst))
}

struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<std::io::PipeReader>,
    mark: String,
    n: u64,
    used: Instant,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

impl Session {
    fn start() -> Result<Session, String> {
        let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let errors = writer.try_clone().map_err(|e| e.to_string())?;
        // AI_AGENT is left out: DuckDB 2 changes how it answers when that is set
        // and stdout isn't a terminal, and Corral reads plain JSON whoever started it.
        let mut child = Command::new("duckdb").arg("-json").env_remove("AI_AGENT")
            .stdin(Stdio::piped()).stdout(writer).stderr(errors)
            .spawn().map_err(|_| tr("DuckDB isn't on the PATH", "DuckDB no está en el PATH").to_string())?;
        let input = child.stdin.take().ok_or(tr("DuckDB failed", "DuckDB falló"))?;
        Ok(Session { child, input, output: BufReader::new(reader), mark: format!("corral-end-{}", env::random_hex(6)), n: 0, used: Instant::now() })
    }

    /// What DuckDB printed for `sql`, up to the mark.
    fn ask(&mut self, sql: &str) -> Result<String, String> {
        self.n += 1;
        let mark = format!("{}-{}", self.mark, self.n);
        let ended = || tr("DuckDB ended", "DuckDB terminó").to_string();
        write!(self.input, "{sql};\nSELECT '{mark}' AS corral_end;\n").and_then(|_| self.input.flush()).map_err(|_| ended())?;
        let mut text = String::new();
        loop {
            let mut line = String::new();
            if self.output.read_line(&mut line).map_err(|_| ended())? == 0 {
                return Err(ended());
            }
            if line.contains(&mark) {
                break;
            }
            text.push_str(&line);
        }
        self.used = Instant::now();
        Ok(text)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The rows of one query, as JSON. One retry with a new DuckDB when the open one
/// ended (killed, crashed); a query that fails says why, as before.
pub fn query(sql: &str) -> Result<Value, String> {
    let text = {
        let mut slot = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let first = match slot.as_mut() {
            Some(s) => s.ask(sql),
            None => Err(String::new()),
        };
        match first {
            Ok(t) => t,
            Err(_) => {
                *slot = None;
                let mut s = Session::start()?;
                let t = s.ask(sql)?;
                *slot = Some(s);
                close_when_idle();
                t
            }
        }
    };
    let t = text.trim();
    if t.is_empty() {
        Ok(Value::Arr(Vec::new()))
    } else if t.starts_with('[') {
        json::parse(t)
    } else {
        Err(t.to_string())
    }
}

/// A thread that closes DuckDB once it has been idle() without queries, and then
/// ends too: nothing runs while no table is open. Called with SESSION locked.
fn close_when_idle() {
    static WATCHING: AtomicBool = AtomicBool::new(false);
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(idle() / 6);
        let mut slot = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_none_or(|s| s.used.elapsed() >= idle()) {
            *slot = None;
            WATCHING.store(false, Ordering::SeqCst);
            return;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the duckdb CLI; without it, there is nothing to check.
    #[test]
    fn one_duckdb_answers_one_query_after_another() {
        if env::which("duckdb").is_none() {
            return;
        }
        let first = |v: Value, k: &str| v.arr().first().map(|r| r.get(k).clone()).unwrap_or(Value::Null);
        // Idle, it closes by itself, and the thread watching it ends.
        IDLE_MS.store(300, Ordering::SeqCst);
        assert_eq!(first(query("SELECT 1 AS one").unwrap(), "one").num(), Some(1.0));
        std::thread::sleep(Duration::from_millis(900));
        assert!(SESSION.lock().unwrap().is_none(), "DuckDB still open after being idle");
        IDLE_MS.store(30_000, Ordering::SeqCst);
        assert_eq!(first(query("SELECT 42 AS x").unwrap(), "x").num(), Some(42.0));
        let pid = SESSION.lock().unwrap().as_ref().map(|s| s.child.id());
        assert!(query("SELECT 1 AS a WHERE false").unwrap().arr().is_empty());
        let err = query("SELECT nope FROM nowhere").unwrap_err();
        assert!(err.contains("nowhere"), "{err}");
        // A string with a semicolon and quotes, then the same DuckDB still answers.
        assert_eq!(first(query("SELECT 'a;''b' AS s").unwrap(), "s").str(), Some("a;'b"));
        assert_eq!(SESSION.lock().unwrap().as_ref().map(|s| s.child.id()), pid);
        // If it dies, the next query starts another one.
        if let Some(s) = SESSION.lock().unwrap().as_mut() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
        assert_eq!(first(query("SELECT 7 AS y").unwrap(), "y").num(), Some(7.0));
    }
}
