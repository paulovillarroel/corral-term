//! The R console with Ark and without jupyter-console: this starts Ark
//! (Positron's R kernel) on its own connection file and talks to it over
//! ZeroMQ, with a small line editor in front: several lines, history, paste,
//! Tab completion, Ctrl+C to interrupt. The prompts look like jupyter-console's
//! (`In [n]:` and `...:`), which is what the rest of Corral looks for.

use crate::console;
use crate::env;
use crate::json::{self, obj, Value};
use crate::term::{self, Raw};
use crate::zmtp::{self, Connection, Socket};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::net::TcpListener;
use std::os::fd::FromRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
const SIGINT: i32 = 2;
const HISTORY_MAX: usize = 2000;

fn out(s: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}

fn log_file() -> PathBuf {
    env::state_dir().join("ark.log")
}

// --------------------------------------------------------------------------- kernel

enum Event {
    Bytes(Vec<u8>),
    /// msg_type, parent msg_id, content
    IoPub(String, String, Value),
    /// A shell reply: msg_type, parent msg_id, content
    Reply(String, String, Value),
    /// The kernel asks for input (readline, menu, browser): header, content
    Input(Value, Value),
    Gone,
}

struct Kernel {
    child: Child,
    key: Vec<u8>,
    session: String,
    shell: Socket,
    stdin: Socket,
    control: Socket,
}

impl Kernel {
    fn request(&mut self, kind: &str, content: Value) -> String {
        zmtp::send_message(&mut self.shell, &self.key, &self.session, kind, &content).unwrap_or_default()
    }

    fn interrupt(&self) {
        // SAFETY: a signal to our own child process.
        unsafe { kill(self.child.id() as i32, SIGINT) };
    }

    fn answer(&mut self, parent: &Value, value: &str) {
        let content = obj(vec![("value", value.into())]);
        let _ = zmtp::send_reply(&mut self.stdin, &self.key, &self.session, "input_reply", &content, parent);
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn shutdown(&mut self) {
        if !self.alive() {
            return;
        }
        let content = obj(vec![("restart", false.into())]);
        let _ = zmtp::send_message(&mut self.control, &self.key, &self.session, "shutdown_request", &content);
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if !self.alive() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// R_HOME and R's library path for Ark, from Corral's kernel spec; the first
/// time they come from R itself (its startup script sets the library path) and
/// are kept in that spec, the one jupyter-console would use too.
fn r_env(ark: &Path) -> Result<Vec<(String, String)>, String> {
    let spec_path = env::data_dir().join("jupyter/kernels/ark/kernel.json");
    let spec = env::read_json(&spec_path);
    let e = spec.get("env");
    if let (Some(home), Some(lib)) = (e.get("R_HOME").str(), e.get("LD_LIBRARY_PATH").str()) {
        if Path::new(home).join("bin").is_dir() {
            return Ok(vec![("R_HOME".into(), home.into()), ("LD_LIBRARY_PATH".into(), lib.into())]);
        }
    }
    let probe = Command::new("R")
        .args(["--vanilla", "--no-echo", "-e", "cat(R.home(), Sys.getenv(\"LD_LIBRARY_PATH\"), sep = \"\\n\")"])
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("no pude ejecutar R: {e}"))?;
    let text = String::from_utf8_lossy(&probe.stdout).to_string();
    let mut lines = text.trim().splitn(2, '\n');
    let home = lines.next().unwrap_or("").trim().to_string();
    if home.is_empty() {
        return Err("R no dijo dónde está instalado".into());
    }
    let lib = lines.next().map(str::trim).filter(|l| !l.is_empty()).map(String::from).unwrap_or_else(|| format!("{home}/lib"));
    let argv: Vec<Value> = [ark.display().to_string(), "--connection_file".into(), "{connection_file}".into(),
                            "--session-mode".into(), "console".into(), "--default-repos".into(), "none".into(),
                            "--log".into(), log_file().display().to_string()]
        .into_iter().map(Value::from).collect();
    let spec = obj(vec![
        ("argv", Value::Arr(argv)),
        ("display_name", "R (Ark, Corral)".into()),
        ("language", "R".into()),
        ("env", obj(vec![("R_HOME", home.as_str().into()), ("LD_LIBRARY_PATH", lib.as_str().into())])),
    ]);
    if let Some(dir) = spec_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    env::write_json(&spec_path, &spec);
    Ok(vec![("R_HOME".into(), home), ("LD_LIBRARY_PATH".into(), lib)])
}

fn free_ports(n: usize) -> Result<Vec<u16>, String> {
    let listeners = (0..n).map(|_| TcpListener::bind("127.0.0.1:0")).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    listeners.iter().map(|l| l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())).collect()
}

fn connect(child: &mut Child, addr: &str, kind: &str, identity: Option<&[u8]>, end: Instant) -> Result<Socket, String> {
    loop {
        match Socket::connect_as(addr, kind, identity) {
            Ok(s) => return Ok(s),
            Err(e) => {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!("Ark terminó al iniciar ({status}); revisa {}", log_file().display()));
                }
                if Instant::now() > end {
                    return Err(format!("Ark no abrió sus puertos: {e}"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Start Ark on a new connection file and connect to its four channels.
fn start(ark: &Path, conn_path: &Path, tx: &Sender<Event>) -> Result<Kernel, String> {
    let ports = free_ports(5)?;
    let key = env::random_hex(16);
    let conn = obj(vec![
        ("shell_port", (ports[0] as f64).into()),
        ("iopub_port", (ports[1] as f64).into()),
        ("stdin_port", (ports[2] as f64).into()),
        ("control_port", (ports[3] as f64).into()),
        ("hb_port", (ports[4] as f64).into()),
        ("ip", "127.0.0.1".into()),
        ("key", key.as_str().into()),
        ("transport", "tcp".into()),
        ("signature_scheme", "hmac-sha256".into()),
        ("kernel_name", "ark".into()),
    ]);
    OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(conn_path)
        .and_then(|mut f| f.write_all(conn.to_json().as_bytes()))
        .map_err(|e| format!("{}: {e}", conn_path.display()))?;

    let mut child = Command::new(ark)
        .arg("--connection_file").arg(conn_path)
        .args(["--session-mode", "console", "--default-repos", "none", "--log"]).arg(log_file())
        .envs(r_env(ark)?)
        .env("CORRAL", "1")
        .env("CORRAL_ARK_CONNECTION", conn_path) // corral.R announces it, for the editor's language server
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("no pude iniciar Ark: {e}"))?;

    let c = Connection::read(&conn_path.to_string_lossy())?;
    let session = zmtp::uuid();
    let end = Instant::now() + Duration::from_secs(60);
    let mut iopub = connect(&mut child, &c.addr(c.iopub), "SUB", None, end)?;
    iopub.subscribe_all().map_err(|e| e.to_string())?;
    let identity = session.as_bytes().to_vec();
    let shell = connect(&mut child, &c.addr(c.shell), "DEALER", Some(&identity), end)?;
    let stdin = connect(&mut child, &c.addr(c.stdin), "DEALER", Some(&identity), end)?;
    let control = connect(&mut child, &c.addr(c.control), "DEALER", None, end)?;

    // Readers: each channel and the terminal feed one queue.
    let (t, s) = (tx.clone(), session.clone());
    std::thread::spawn(move || {
        while let Ok(parts) = iopub.recv() {
            let Some((h, p, content)) = zmtp::parse_full(&parts) else { continue };
            if p.get("session").str() != Some(s.as_str()) {
                continue; // other clients' messages (the editor's language server)
            }
            let kind = h.get("msg_type").str().unwrap_or("").to_string();
            let parent = p.get("msg_id").str().unwrap_or("").to_string();
            if t.send(Event::IoPub(kind, parent, content)).is_err() {
                break;
            }
        }
    });
    let mut replies = shell.try_clone().map_err(|e| e.to_string())?;
    let t = tx.clone();
    std::thread::spawn(move || {
        while let Ok(parts) = replies.recv() {
            let Some((h, p, content)) = zmtp::parse_full(&parts) else { continue };
            let kind = h.get("msg_type").str().unwrap_or("").to_string();
            let parent = p.get("msg_id").str().unwrap_or("").to_string();
            if t.send(Event::Reply(kind, parent, content)).is_err() {
                break;
            }
        }
    });
    let mut requests = stdin.try_clone().map_err(|e| e.to_string())?;
    let t = tx.clone();
    std::thread::spawn(move || {
        while let Ok(parts) = requests.recv() {
            let Some((h, _, content)) = zmtp::parse_full(&parts) else { continue };
            if h.get("msg_type").str() == Some("input_request") && t.send(Event::Input(h, content)).is_err() {
                break;
            }
        }
    });
    let t = tx.clone();
    std::thread::spawn(move || {
        // SAFETY: fd 0 stays open; ManuallyDrop keeps File from closing it.
        let mut tty = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
        let mut buf = [0u8; 8192];
        loop {
            match tty.read(&mut buf) {
                Ok(n) if n > 0 => {
                    if t.send(Event::Bytes(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = t.send(Event::Gone);
                    break;
                }
            }
        }
    });
    Ok(Kernel { child, key: key.into_bytes(), session, shell, stdin, control })
}

// --------------------------------------------------------------------------- keys

#[derive(Clone, Debug, PartialEq)]
enum Key {
    Char(char),
    Paste(String),
    Enter,
    AltEnter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordLeft,
    WordRight,
    Ctrl(u8),
    Other,
}

/// Bytes from the terminal to keys; a sequence cut between two reads waits for the rest.
#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
}

fn utf8_len(b: u8) -> usize {
    match b {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn csi(seq: &[u8]) -> Key {
    let s = std::str::from_utf8(seq).unwrap_or("");
    let (params, fin) = s.split_at(s.len().saturating_sub(1));
    let modified = params.contains(';'); // 1;5C is Ctrl+Right, 1;3C Alt+Right
    match fin {
        "A" => Key::Up,
        "B" => Key::Down,
        "C" if modified => Key::WordRight,
        "D" if modified => Key::WordLeft,
        "C" => Key::Right,
        "D" => Key::Left,
        "H" => Key::Home,
        "F" => Key::End,
        "~" => match params {
            "1" | "7" => Key::Home,
            "4" | "8" => Key::End,
            "3" => Key::Delete,
            _ => Key::Other,
        },
        _ => Key::Other,
    }
}

impl Decoder {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(bytes);
        let p = std::mem::take(&mut self.pending);
        let mut keys = Vec::new();
        let mut i = 0;
        while i < p.len() {
            if let Some(paste) = self.paste.as_mut() {
                match find(&p[i..], b"\x1b[201~") {
                    Some(at) => {
                        paste.extend_from_slice(&p[i..i + at]);
                        i += at + 6;
                        let text = String::from_utf8_lossy(&self.paste.take().unwrap_or_default()).into_owned();
                        keys.push(Key::Paste(text));
                    }
                    None => {
                        // Keep a possible half of the end marker for the next read.
                        let upto = p.len() - (p.len() - i).min(5);
                        paste.extend_from_slice(&p[i..upto]);
                        i = upto;
                        break;
                    }
                }
                continue;
            }
            let b = p[i];
            if b == 0x1b {
                let Some(&n) = p.get(i + 1) else { break };
                if n == b'[' {
                    let mut j = i + 2;
                    while j < p.len() && (0x20..=0x3f).contains(&p[j]) {
                        j += 1;
                    }
                    if j >= p.len() {
                        break;
                    }
                    let seq = &p[i + 2..=j];
                    i = j + 1;
                    if seq == b"200~" {
                        self.paste = Some(Vec::new());
                    } else {
                        keys.push(csi(seq));
                    }
                } else if n == b'O' {
                    let Some(&f) = p.get(i + 2) else { break };
                    i += 3;
                    keys.push(match f {
                        b'A' => Key::Up,
                        b'B' => Key::Down,
                        b'C' => Key::Right,
                        b'D' => Key::Left,
                        b'H' => Key::Home,
                        b'F' => Key::End,
                        _ => Key::Other,
                    });
                } else {
                    i += 2;
                    keys.push(match n {
                        b'\r' | b'\n' => Key::AltEnter,
                        b'b' => Key::WordLeft,
                        b'f' => Key::WordRight,
                        0x7f | 0x08 => Key::Ctrl(b'w'),
                        _ => Key::Other,
                    });
                }
                continue;
            }
            let len = utf8_len(b);
            if i + len > p.len() {
                break;
            }
            let key = match b {
                b'\r' => {
                    if p.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    Key::Enter
                }
                b'\n' => Key::Enter,
                b'\t' => Key::Tab,
                0x7f | 0x08 => Key::Backspace,
                1..=26 => Key::Ctrl(b + b'a' - 1),
                0..=31 => Key::Other,
                _ => std::str::from_utf8(&p[i..i + len]).ok().and_then(|s| s.chars().next()).map(Key::Char).unwrap_or(Key::Other),
            };
            i += len;
            keys.push(key);
        }
        self.pending = p[i..].to_vec();
        keys
    }

    /// Nothing more came: a lone Esc (or a broken sequence) is dropped.
    fn flush(&mut self) {
        if self.paste.is_none() {
            self.pending.clear();
        }
    }
}

// --------------------------------------------------------------------------- editor

#[derive(Clone)]
enum Prompt {
    In(u64),
    /// The kernel's own prompt (readline, menu, browser); hidden input for passwords.
    Ask(String, bool),
}

struct Editor {
    buf: Vec<char>,
    cur: usize,
    prompt: Prompt,
    /// Rows between the top of the input and the cursor, as last drawn.
    rows_above: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

impl Editor {
    fn new(prompt: Prompt) -> Editor {
        Editor { buf: Vec::new(), cur: 0, prompt, rows_above: 0 }
    }

    fn text(&self) -> String {
        self.buf.iter().collect()
    }

    fn set(&mut self, s: &str) {
        self.buf = s.chars().collect();
        self.cur = self.buf.len();
    }

    fn insert(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();
        self.buf.splice(self.cur..self.cur, chars);
        self.cur += n;
    }

    fn replace(&mut self, start: usize, end: usize, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();
        self.buf.splice(start..end, chars);
        self.cur = start + n;
    }

    fn line_start(&self, at: usize) -> usize {
        self.buf[..at].iter().rposition(|&c| c == '\n').map(|p| p + 1).unwrap_or(0)
    }

    fn line_end(&self, at: usize) -> usize {
        self.buf[at..].iter().position(|&c| c == '\n').map(|p| at + p).unwrap_or(self.buf.len())
    }

    fn line_index(&self) -> usize {
        self.buf[..self.cur].iter().filter(|&&c| c == '\n').count()
    }

    fn lines(&self) -> usize {
        self.buf.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// Move to the line above (-1) or below (+1), keeping the column.
    fn vertical(&mut self, dir: i32) {
        let start = self.line_start(self.cur);
        let col = self.cur - start;
        let target = if dir < 0 {
            if start == 0 {
                return;
            }
            self.line_start(start - 1)
        } else {
            let end = self.line_end(self.cur);
            if end >= self.buf.len() {
                return;
            }
            end + 1
        };
        self.cur = (target + col).min(self.line_end(target));
    }

    fn word_left(&self) -> usize {
        let mut i = self.cur;
        while i > 0 && !is_word(self.buf[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.buf[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let mut i = self.cur;
        while i < self.buf.len() && !is_word(self.buf[i]) {
            i += 1;
        }
        while i < self.buf.len() && is_word(self.buf[i]) {
            i += 1;
        }
        i
    }

    /// The prompt of line `i` (with color) and its width.
    fn prompt_for(&self, i: usize) -> (String, usize) {
        match &self.prompt {
            Prompt::In(n) => {
                let first = format!("In [{n}]: ");
                let w = first.chars().count();
                if i == 0 {
                    (format!("\x1b[32mIn [\x1b[1m{n}\x1b[22m]: \x1b[0m"), w)
                } else {
                    (format!("\x1b[32m{:>1$}: \x1b[0m", "...", w - 2), w)
                }
            }
            Prompt::Ask(p, _) => (if i == 0 { p.clone() } else { String::new() }, if i == 0 { p.chars().count() } else { 0 }),
        }
    }

    fn secret(&self) -> bool {
        matches!(self.prompt, Prompt::Ask(_, true))
    }

    /// Draw the input again, from its first row, and put the cursor in place.
    fn render(&mut self) {
        let width = term::size().0.max(10);
        let mut s = String::from("\r");
        if self.rows_above > 0 {
            s += &format!("\x1b[{}A", self.rows_above);
        }
        s += "\x1b[J";
        let text: String = if self.secret() { String::new() } else { self.text() };
        let cursor_line = self.line_index();
        let cursor_col = self.cur - self.line_start(self.cur);
        let (mut row, mut cur_row, mut cur_col) = (0, 0, 0);
        let lines: Vec<&str> = text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let (prompt, pw) = self.prompt_for(i);
            s += &prompt;
            s += line;
            let p = pw + line.chars().count();
            if i == cursor_line && !self.secret() {
                let pc = pw + cursor_col;
                cur_row = row + pc / width;
                cur_col = pc % width;
            } else if self.secret() {
                cur_row = row + pw / width;
                cur_col = pw % width;
            }
            if p > 0 && p % width == 0 {
                s += "\r\n"; // a full row: go to the next one, as the cursor would
            }
            row += p / width;
            if i + 1 < lines.len() {
                s += "\r\n";
                row += 1;
            }
        }
        if row > cur_row {
            s += &format!("\x1b[{}A", row - cur_row);
        }
        s += "\r";
        if cur_col > 0 {
            s += &format!("\x1b[{cur_col}C");
        }
        self.rows_above = cur_row;
        out(&s);
    }

    /// Clear the input from the screen, leaving the cursor where it started.
    fn erase(&mut self) {
        let up = if self.rows_above > 0 { format!("\x1b[{}A", self.rows_above) } else { String::new() };
        out(&format!("\r{up}\x1b[J"));
        self.rows_above = 0;
    }

    /// Leave the input on the screen and go to the line below it.
    fn finish(&mut self) {
        self.cur = self.buf.len();
        self.render();
        out("\r\n");
        self.rows_above = 0;
    }

    fn reset(&mut self, prompt: Prompt) {
        self.buf.clear();
        self.cur = 0;
        self.prompt = prompt;
        self.rows_above = 0;
    }

    /// Editing keys shared by the console and the kernel's questions; false when not one.
    fn edit(&mut self, key: &Key) -> bool {
        match key {
            Key::Char(c) => self.insert(&c.to_string()),
            Key::Paste(t) => self.insert(&t.replace("\r\n", "\n").replace('\r', "\n")),
            Key::Backspace => {
                if self.cur > 0 {
                    self.cur -= 1;
                    self.buf.remove(self.cur);
                }
            }
            Key::Delete => {
                if self.cur < self.buf.len() {
                    self.buf.remove(self.cur);
                }
            }
            Key::Left => self.cur = self.cur.saturating_sub(1),
            Key::Right => self.cur = (self.cur + 1).min(self.buf.len()),
            Key::Home | Key::Ctrl(b'a') => self.cur = self.line_start(self.cur),
            Key::End | Key::Ctrl(b'e') => self.cur = self.line_end(self.cur),
            Key::WordLeft => self.cur = self.word_left(),
            Key::WordRight => self.cur = self.word_right(),
            Key::Ctrl(b'b') => self.cur = self.cur.saturating_sub(1),
            Key::Ctrl(b'f') => self.cur = (self.cur + 1).min(self.buf.len()),
            Key::Ctrl(b'k') => {
                let end = self.line_end(self.cur);
                let end = if end == self.cur && end < self.buf.len() { end + 1 } else { end };
                self.buf.drain(self.cur..end);
            }
            Key::Ctrl(b'u') => {
                let start = self.line_start(self.cur);
                self.buf.drain(start..self.cur);
                self.cur = start;
            }
            Key::Ctrl(b'w') => {
                let start = self.word_left();
                self.buf.drain(start..self.cur);
                self.cur = start;
            }
            Key::Ctrl(b'l') => {
                out("\x1b[H\x1b[2J");
                self.rows_above = 0;
            }
            _ => return false,
        }
        true
    }
}

// --------------------------------------------------------------------------- history

struct History {
    items: Vec<String>,
    file: PathBuf,
    pos: Option<usize>,
    draft: String,
}

impl History {
    fn load(file: PathBuf) -> History {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let mut items: Vec<String> = text.lines().filter_map(|l| json::parse(l).ok()).filter_map(|v| v.str().map(String::from)).collect();
        if items.len() > HISTORY_MAX {
            items.drain(..items.len() - HISTORY_MAX);
            let body: String = items.iter().map(|i| format!("{}\n", Value::from(i.as_str()).to_json())).collect();
            let _ = std::fs::write(&file, body);
        }
        History { items, file, pos: None, draft: String::new() }
    }

    fn add(&mut self, code: &str) {
        self.pos = None;
        let code = code.trim_end();
        if code.trim().is_empty() || self.items.last().map(String::as_str) == Some(code) {
            return;
        }
        self.items.push(code.to_string());
        // Appended line by line, so several consoles can share the file.
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.file) {
            let _ = f.write_all(format!("{}\n", Value::from(code).to_json()).as_bytes());
        }
    }

    /// The previous entry starting like the draft (all entries when it is empty).
    fn prev(&mut self, current: &str) -> Option<String> {
        if self.pos.is_none() {
            self.draft = current.to_string();
        }
        let from = self.pos.unwrap_or(self.items.len());
        let prefix = if self.draft.contains('\n') { "" } else { self.draft.as_str() };
        let found = self.items[..from].iter().rposition(|i| i.starts_with(prefix) && i != current)?;
        self.pos = Some(found);
        Some(self.items[found].clone())
    }

    fn next(&mut self) -> Option<String> {
        let pos = self.pos?;
        let prefix = if self.draft.contains('\n') { "" } else { self.draft.as_str() };
        match self.items[pos + 1..].iter().position(|i| i.starts_with(prefix)) {
            Some(k) => {
                self.pos = Some(pos + 1 + k);
                Some(self.items[pos + 1 + k].clone())
            }
            None => {
                self.pos = None;
                Some(self.draft.clone())
            }
        }
    }
}

// --------------------------------------------------------------------------- console

enum Mode {
    Edit,
    Busy { id: String, replied: Option<Instant>, idle: bool },
    Ask { parent: Value, busy: String },
}

struct Console {
    k: Kernel,
    rx: Receiver<Event>,
    backlog: VecDeque<Event>,
    keys: VecDeque<Key>,
    dec: Decoder,
    ed: Editor,
    hist: History,
    mode: Mode,
    count: u64,
    at_line_start: bool,
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut n = first.chars().count();
    for other in &items[1..] {
        n = first.chars().zip(other.chars()).take(n).take_while(|(a, b)| a == b).count();
    }
    first.chars().take(n).collect()
}

impl Console {
    /// A reply to `id`; everything else that arrives meanwhile is kept for later, in order.
    fn wait_reply(&mut self, id: &str, timeout: Duration) -> Option<Value> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            match self.rx.recv_timeout(left) {
                Ok(Event::Reply(_, parent, content)) if parent == id => return Some(content),
                Ok(ev) => self.backlog.push_back(ev),
                Err(_) => return None,
            }
        }
    }

    /// Kernel output: below the input while editing, straight out while running.
    fn print(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let editing = !matches!(self.mode, Mode::Busy { .. });
        if editing {
            self.ed.erase();
        }
        out(s);
        self.at_line_start = s.ends_with('\n');
        if editing {
            if !self.at_line_start {
                out("\n");
                self.at_line_start = true;
            }
            self.ed.render();
        }
    }

    fn on_iopub(&mut self, kind: &str, parent: &str, content: &Value) {
        match kind {
            "stream" => self.print(content.get("text").str().unwrap_or("")),
            "execute_result" | "display_data" => {
                if let Some(t) = content.get("data").get("text/plain").str() {
                    let t = if t.ends_with('\n') { t.to_string() } else { format!("{t}\n") };
                    self.print(&t);
                }
            }
            "error" => {
                let tb: Vec<&str> = content.get("traceback").arr().iter().filter_map(|v| v.str()).collect();
                let text = if tb.is_empty() {
                    format!("{}\n", content.get("evalue").str().unwrap_or(""))
                } else {
                    format!("{}\n", tb.join("\n"))
                };
                self.print(&text);
            }
            "status" if content.get("execution_state").str() == Some("idle") => {
                if let Mode::Busy { id, idle, .. } = &mut self.mode {
                    if id == parent {
                        *idle = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn on_reply(&mut self, kind: &str, parent: &str, content: &Value) {
        if kind != "execute_reply" {
            return;
        }
        if let Mode::Busy { id, replied, .. } = &mut self.mode {
            if id == parent {
                *replied = Some(Instant::now());
                if let Some(n) = content.get("execution_count").num() {
                    self.count = n as u64 + 1;
                }
            }
        }
    }

    /// Done when the reply came and the kernel went idle (or a moment after the reply).
    fn maybe_done(&mut self) {
        let Mode::Busy { replied: Some(at), idle, .. } = &self.mode else { return };
        if !*idle && at.elapsed() < Duration::from_millis(500) {
            return;
        }
        if !self.at_line_start {
            out("\n");
        }
        out("\n");
        self.at_line_start = true;
        self.mode = Mode::Edit;
        self.ed.reset(Prompt::In(self.count));
        self.ed.render();
    }

    fn on_input(&mut self, header: Value, content: &Value) {
        let busy = match &self.mode {
            Mode::Busy { id, .. } => id.clone(),
            _ => String::new(),
        };
        let mut prompt = content.get("prompt").str().unwrap_or("").to_string();
        if let Some(cut) = prompt.rfind('\n') {
            out(&prompt[..=cut]);
            prompt = prompt[cut + 1..].to_string();
            self.at_line_start = true;
        }
        if !self.at_line_start {
            out("\n");
            self.at_line_start = true;
        }
        let password = matches!(content.get("password"), Value::Bool(true));
        self.mode = Mode::Ask { parent: header, busy };
        self.ed.reset(Prompt::Ask(prompt, password));
        self.ed.render();
    }

    fn submit(&mut self) {
        let code = self.ed.text();
        self.ed.finish();
        self.hist.add(&code);
        let content = obj(vec![
            ("code", code.as_str().into()),
            ("silent", false.into()),
            ("store_history", true.into()),
            ("user_expressions", Value::obj()),
            ("allow_stdin", true.into()),
            ("stop_on_error", true.into()),
        ]);
        let id = self.k.request("execute_request", content);
        self.at_line_start = true;
        self.mode = Mode::Busy { id, replied: None, idle: false };
    }

    /// Enter: run the code when R says it is complete, or go on to a new line.
    fn enter(&mut self) {
        if self.ed.cur < self.ed.buf.len() {
            self.ed.insert("\n");
            return;
        }
        let code = self.ed.text();
        if code.trim().is_empty() {
            self.ed.finish();
            self.ed.reset(Prompt::In(self.count));
            return;
        }
        let id = self.k.request("is_complete_request", obj(vec![("code", code.as_str().into())]));
        let reply = self.wait_reply(&id, Duration::from_secs(2));
        let status = reply.as_ref().and_then(|r| r.get("status").str()).unwrap_or("unknown").to_string();
        if status == "incomplete" {
            let given = reply.as_ref().and_then(|r| r.get("indent").str()).unwrap_or("").to_string();
            let line: String = self.ed.buf[self.ed.line_start(self.ed.cur)..self.ed.cur].iter().collect();
            let lead: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let opens = line.trim_end().ends_with(['{', '(', '[', ',']) || line.trim_end().ends_with("|>") || line.trim_end().ends_with('+');
            // Ark answers with R's continuation prompt ("+"), not an indent: only spaces count.
            let given = if given.chars().all(|c| c == ' ' || c == '\t') { given } else { String::new() };
            let indent = if !given.is_empty() { given } else if opens { format!("{lead}  ") } else { lead };
            self.ed.insert(&format!("\n{indent}"));
        } else {
            self.submit();
        }
    }

    /// Tab: R's own completion (Ark leaves completions to its language server). A
    /// silent run, which leaves `In [n]` as it is, writes the candidates to a file.
    fn complete(&mut self) {
        let cur = self.ed.cur;
        let start_of_line = self.ed.line_start(cur);
        if self.ed.buf[start_of_line..cur].iter().all(|c| c.is_whitespace()) {
            self.ed.insert("  ");
            return;
        }
        let line: String = self.ed.buf[start_of_line..self.ed.line_end(cur)].iter().collect();
        let file = env::state_dir().join("ark").join(format!("completar-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let lit = |s: &str| Value::from(s).to_json(); // a JSON string is also an R string
        let code = format!(
            "local(try({{ utils:::.assignLinebuffer({}); utils:::.assignEnd({}L); utils:::.guessTokenFromLine(); \
             utils:::.completeToken(); writeLines(c(utils:::.CompletionEnv[[\"token\"]], utils:::.retrieveCompletions()), {}) }}, silent = TRUE))",
            lit(&line), cur - start_of_line, lit(&file.to_string_lossy()));
        let content = obj(vec![
            ("code", code.as_str().into()),
            ("silent", true.into()),
            ("store_history", false.into()),
            ("user_expressions", Value::obj()),
            ("allow_stdin", false.into()),
            ("stop_on_error", false.into()),
        ]);
        let id = self.k.request("execute_request", content);
        if self.wait_reply(&id, Duration::from_secs(3)).is_none() {
            return;
        }
        let text = std::fs::read(&file).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
        let _ = std::fs::remove_file(&file);
        let mut lines = text.lines();
        let token = lines.next().unwrap_or("").to_string();
        let mut matches: Vec<String> = lines.filter(|m| !m.is_empty()).map(String::from).collect();
        matches.dedup();
        if matches.is_empty() {
            return;
        }
        let end = cur;
        let start = cur.saturating_sub(token.chars().count()).max(start_of_line);
        let typed: String = self.ed.buf[start..end].iter().collect();
        let common = common_prefix(&matches);
        if matches.len() == 1 {
            self.ed.replace(start, end, &matches[0]);
        } else if common.chars().count() > typed.chars().count() {
            self.ed.replace(start, end, &common);
        } else {
            // Show the choices under the input, in columns, and draw the input again.
            self.ed.finish();
            let width = term::size().0.max(10);
            let col = matches.iter().map(|m| m.chars().count()).max().unwrap_or(1) + 2;
            let per_row = (width / col).max(1);
            let shown = &matches[..matches.len().min(200)];
            let mut s = String::new();
            for (i, m) in shown.iter().enumerate() {
                s += &format!("{m:<col$}");
                if (i + 1) % per_row == 0 || i + 1 == shown.len() {
                    s = s.trim_end().to_string() + "\n";
                }
            }
            if matches.len() > shown.len() {
                s += &format!("… y {} más\n", matches.len() - shown.len());
            }
            out(&s);
        }
    }

    /// Returns false to leave the console.
    fn edit_key(&mut self, key: Key) -> bool {
        if !matches!(key, Key::Up | Key::Down) {
            self.hist.pos = None;
        }
        match key {
            Key::Enter => self.enter(),
            Key::AltEnter => {
                if self.ed.text().trim().is_empty() {
                    self.enter();
                } else {
                    self.submit();
                }
            }
            Key::Tab => self.complete(),
            Key::Up if self.ed.line_index() > 0 => self.ed.vertical(-1),
            Key::Down if self.ed.line_index() + 1 < self.ed.lines() => self.ed.vertical(1),
            Key::Up | Key::Ctrl(b'p') => {
                let current = self.ed.text();
                if let Some(t) = self.hist.prev(&current) {
                    self.ed.set(&t);
                }
            }
            Key::Down | Key::Ctrl(b'n') => {
                if let Some(t) = self.hist.next() {
                    self.ed.set(&t);
                }
            }
            Key::Ctrl(b'c') => {
                // Like IPython: drop what was typed and start over.
                self.ed.cur = self.ed.buf.len();
                self.ed.render();
                out("^C\r\n");
                self.ed.reset(Prompt::In(self.count));
            }
            Key::Ctrl(b'd') if self.ed.buf.is_empty() => {
                out("\r\n");
                return false;
            }
            Key::Ctrl(b'd') => {
                self.ed.edit(&Key::Delete);
            }
            other => {
                self.ed.edit(&other);
            }
        }
        if matches!(self.mode, Mode::Edit) {
            self.ed.render();
        }
        true
    }

    fn ask_key(&mut self, key: Key) {
        let Mode::Ask { parent, busy } = &self.mode else { return };
        let (parent, busy) = (parent.clone(), busy.clone());
        match key {
            Key::Enter | Key::AltEnter => {
                let value = self.ed.text();
                self.ed.finish();
                self.k.answer(&parent, &value);
                self.mode = Mode::Busy { id: busy, replied: None, idle: false };
                return;
            }
            Key::Ctrl(b'c') => {
                self.ed.finish();
                self.k.interrupt();
                self.mode = Mode::Busy { id: busy, replied: None, idle: false };
                return;
            }
            Key::Paste(t) => {
                // A pasted answer is one line; what follows a newline is the next answer.
                let t = t.replace("\r\n", "\n").replace('\r', "\n");
                let mut parts = t.splitn(2, '\n');
                self.ed.insert(parts.next().unwrap_or(""));
                if let Some(rest) = parts.next() {
                    self.keys.push_front(Key::Paste(rest.to_string()));
                    self.keys.push_front(Key::Enter);
                }
            }
            other => {
                self.ed.edit(&other);
            }
        }
        self.ed.render();
    }

    /// Keys act while editing or answering; while R runs only Ctrl+C does, the rest waits.
    fn process_keys(&mut self) -> bool {
        loop {
            match self.mode {
                Mode::Busy { .. } => {
                    if let Some(at) = self.keys.iter().position(|k| *k == Key::Ctrl(b'c')) {
                        self.keys.remove(at);
                        self.k.interrupt();
                    }
                    return true;
                }
                Mode::Edit => {
                    let Some(key) = self.keys.pop_front() else { return true };
                    if !self.edit_key(key) {
                        return false;
                    }
                }
                Mode::Ask { .. } => {
                    let Some(key) = self.keys.pop_front() else { return true };
                    self.ask_key(key);
                }
            }
        }
    }

    fn run(&mut self) -> i32 {
        let mut checked = Instant::now();
        loop {
            let ev = match self.backlog.pop_front() {
                Some(ev) => Some(ev),
                None => match self.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ev) => Some(ev),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return 1,
                },
            };
            match ev {
                Some(Event::Bytes(b)) => {
                    let keys = self.dec.feed(&b);
                    self.keys.extend(keys);
                }
                Some(Event::IoPub(kind, parent, content)) => self.on_iopub(&kind, &parent, &content),
                Some(Event::Reply(kind, parent, content)) => self.on_reply(&kind, &parent, &content),
                Some(Event::Input(header, content)) => self.on_input(header, &content),
                Some(Event::Gone) => return 0,
                None => self.dec.flush(),
            }
            self.maybe_done();
            if !self.process_keys() {
                return 0;
            }
            if checked.elapsed() > Duration::from_millis(200) {
                checked = Instant::now();
                if let Ok(Some(status)) = self.k.child.try_wait() {
                    // R quit (q()): the console ends with it.
                    if !self.at_line_start {
                        out("\n");
                    }
                    return status.code().unwrap_or(0);
                }
            }
        }
    }
}

pub fn run() -> Result<(), String> {
    let cfg = env::config();
    let ark = console::ark_binary(&cfg).ok_or("Ark no está instalado (corral ark --instalar)")?;
    let folder = env::state_dir().join("ark");
    let _ = std::fs::create_dir_all(&folder);
    env::remove_old(&folder, "json", 2 * 86400);
    let conn_path = folder.join(format!("kernel-{}.json", env::random_hex(4)));
    let (tx, rx) = mpsc::channel();
    let kernel = start(&ark, &conn_path, &tx)?;
    let raw = Raw::enter();
    out("\x1b[?2004h"); // pasted text comes marked, so it is not run line by line
    let mut c = Console {
        k: kernel,
        rx,
        backlog: VecDeque::new(),
        keys: VecDeque::new(),
        dec: Decoder::default(),
        ed: Editor::new(Prompt::In(1)),
        hist: History::load(env::state_dir().join("historial-r.jsonl")),
        mode: Mode::Edit,
        count: 1,
        at_line_start: true,
    };
    let id = c.k.request("kernel_info_request", Value::obj());
    if c.wait_reply(&id, Duration::from_secs(60)).is_none() {
        c.k.shutdown();
        drop(raw);
        return Err(format!("Ark no respondió; revisa {}", log_file().display()));
    }
    c.ed.render();
    let code = c.run();
    c.k.shutdown();
    out("\x1b[?2004l");
    drop(raw);
    let _ = std::fs::remove_file(&conn_path);
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_paste() {
        let mut d = Decoder::default();
        assert_eq!(d.feed(b"a\x1b[A\r"), vec![Key::Char('a'), Key::Up, Key::Enter]);
        assert_eq!(d.feed(b"\x1b[200~x <- 1\ny"), vec![]);
        assert_eq!(d.feed(b"\x1b[20"), vec![]);
        assert_eq!(d.feed(b"1~\r"), vec![Key::Paste("x <- 1\ny".into()), Key::Enter]);
        assert_eq!(d.feed("ñ".as_bytes()), vec![Key::Char('ñ')]);
        assert_eq!(d.feed(&"é".as_bytes()[..1]), vec![]);
        assert_eq!(d.feed(&"é".as_bytes()[1..]), vec![Key::Char('é')]);
        assert_eq!(d.feed(b"\x03\x1b[1;5D\x1b[3~"), vec![Key::Ctrl(b'c'), Key::WordLeft, Key::Delete]);
    }

    #[test]
    fn editing() {
        let mut e = Editor::new(Prompt::In(1));
        e.insert("f <- function(x) {\n  x + 1\n}");
        assert_eq!(e.lines(), 3);
        e.vertical(-1);
        assert_eq!(e.line_index(), 1);
        e.edit(&Key::Home);
        assert_eq!(e.cur, e.line_start(e.cur));
        e.edit(&Key::Ctrl(b'k'));
        assert_eq!(e.text(), "f <- function(x) {\n\n}");
        assert_eq!(common_prefix(&["mean".into(), "median".into()]), "me");
    }
}
