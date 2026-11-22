//! Where Corral keeps things, what herdr tells a plugin action, and the small
//! helpers (time, quoting, processes, the clipboard) the rest of Corral shares.

use crate::herdr;
use crate::json::{self, Value};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, default: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(default),
    }
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("corral")
}

pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("corral")
}

pub fn config() -> Value {
    read_json(&xdg("XDG_CONFIG_HOME", ".config").join("corral/config.json"))
}

/// The repository: CORRAL_REPO, else the folder this binary sits in
/// (<repo>/rs/target/release/corral, also in a downloaded release), else where
/// it was built from.
pub fn repo() -> PathBuf {
    static REPO: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    REPO.get_or_init(|| {
        if let Some(p) = std::env::var_os("CORRAL_REPO") {
            return PathBuf::from(p);
        }
        let is_repo = |d: &Path| d.join("rs/Cargo.toml").is_file() && d.join("herdr/herdr-plugin.toml").is_file();
        if let Ok(exe) = std::env::current_exe().and_then(|e| e.canonicalize()) {
            if let Some(d) = exe.ancestors().skip(1).find(|d| is_repo(d)) {
                return d.to_path_buf();
            }
        }
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new(".")).to_path_buf()
    }).clone()
}

pub fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|t| json::parse(&t).ok()).unwrap_or_else(Value::obj)
}

pub fn write_json(path: &Path, v: &Value) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, v.to_json());
}

pub fn plugin_context() -> Value {
    std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::Null)
}

pub fn current_workspace() -> Option<String> {
    if let Ok(ws) = std::env::var("HERDR_WORKSPACE_ID") {
        if !ws.is_empty() {
            return Some(ws);
        }
    }
    if let Some(ws) = plugin_context().get("workspace_id").str() {
        return Some(ws.to_string());
    }
    let list = herdr::call("workspace.list", Value::obj()).ok()?;
    list.get("workspaces").arr().iter().find(|w| w.get("focused") == &Value::Bool(true))
        .and_then(|w| w.get("workspace_id").str().map(String::from))
}

pub fn current_pane() -> Option<String> {
    for var in ["HERDR_PANE_ID"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    if let Some(p) = plugin_context().get("focused_pane_id").str() {
        return Some(p.to_string());
    }
    std::env::var("HERDR_ACTIVE_PANE_ID").ok().filter(|p| !p.is_empty())
}

/// Live Corral sessions (R or Python), from their meta.json files.
pub fn sessions() -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(state_dir().join("sessions")) else { return out };
    for dir in dirs.flatten() {
        let mut meta = read_json(&dir.path().join("meta.json"));
        let Some(pid) = meta.get("pid").num() else { continue };
        if !Path::new(&format!("/proc/{}", pid as i64)).exists() {
            continue;
        }
        if meta.get("lang").is_null() {
            meta.set("lang", "r".into());
        }
        meta.set("dir", dir.path().to_string_lossy().to_string().into());
        out.push(meta);
    }
    out
}

pub fn session_for(pane: &str) -> Option<Value> {
    sessions().into_iter().find(|s| s.get("pane_id").str() == Some(pane))
}

/// The layout state of all workspaces (the active console, what ctrl+b t hid).
pub fn layout_file() -> PathBuf {
    state_dir().join("expandido.json")
}

pub fn ws_key(ws: Option<&str>) -> String {
    ws.unwrap_or("default").to_string()
}

/// The language of the console in the layout's console spot: R unless you switched.
pub fn active_lang(ws: Option<&str>, panes: &[Value]) -> String {
    let saved = read_json(&layout_file());
    let lang = saved.get(&ws_key(ws)).get("active").str().unwrap_or("r").to_string();
    if lang != "r" && herdr::labeled(panes, &format!("corral:{lang}")).is_none() {
        return "r".into();
    }
    lang
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| p.is_file())
}


pub fn notify(title: &str, body: &str) {
    if body.is_empty() {
        eprintln!("corral: {title}");
    } else {
        eprintln!("corral: {title}: {body}");
    }
    if std::env::var("HERDR_ENV").as_deref() == Ok("1") {
        let body = if body.is_empty() { " " } else { body };
        let _ = herdr::call("notification.show", json::obj(vec![("title", title.into()), ("body", body.into())]));
    }
}

/// A few random hex characters for file names.
pub fn random_hex(bytes: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        buf = t.as_nanos().to_le_bytes()[..bytes.min(16)].to_vec();
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Files in `dir` matching the extension, older than `seconds`, go away.
pub fn remove_old(dir: &Path, ext: &str, seconds: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some(ext) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok()
            .and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() > seconds).unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn in_herdr() -> bool {
    std::env::var("HERDR_ENV").as_deref() == Ok("1")
}

/// Windows programs started from WSL go through the interop socket in
/// WSL_INTEROP. The herdr server keeps the socket of the terminal that started
/// it, which goes away when that terminal closes: use one that is still alive.
pub fn windows_env(cmd: &mut Command) {
    if std::env::var_os("WSL_INTEROP").map(|p| Path::new(&p).exists()).unwrap_or(false) {
        return;
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir("/run/WSL").map(|d| {
        d.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with("_interop")).collect()
    }).unwrap_or_default();
    found.sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
    let first = Path::new("/run/WSL/1_interop");
    if let Some(p) = std::iter::once(first.to_path_buf()).chain(found).find(|p| p.exists()) {
        cmd.env("WSL_INTEROP", p);
    }
}

/// The text in the system clipboard (CORRAL_CLIPBOARD, Wayland, X11, or Windows
/// from WSL). Only text: an image there (a screenshot) counts as nothing.
pub fn read_clipboard() -> String {
    let mut candidates: Vec<Vec<String>> = Vec::new();
    if let Ok(custom) = std::env::var("CORRAL_CLIPBOARD") {
        candidates.push(vec!["sh".into(), "-c".into(), custom]);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && which("wl-paste").is_some() {
        candidates.push(vec!["wl-paste".into(), "--no-newline".into(), "--type".into(), "text".into()]);
    }
    if std::env::var_os("DISPLAY").is_some() && which("xclip").is_some() {
        candidates.push(vec!["xclip".into(), "-o".into(), "-selection".into(), "clipboard".into(), "-t".into(), "UTF8_STRING".into()]);
    }
    if std::env::var_os("WSL_DISTRO_NAME").is_some() && which("powershell.exe").is_some() {
        // Slow (about 2 s): the last resort.
        candidates.push(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                         "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw"]
                        .iter().map(|s| s.to_string()).collect());
    }
    for argv in candidates {
        let Some((prog, rest)) = argv.split_first() else { continue };
        let mut cmd = Command::new(prog);
        cmd.args(rest).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if prog.ends_with(".exe") {
            cmd.current_dir("/mnt/c");
            windows_env(&mut cmd);
        }
        if let Ok(out) = cmd.output() {
            let Ok(text) = String::from_utf8(out.stdout) else { continue };
            let text = text.replace("\r\n", "\n");
            if out.status.success() && !text.trim().is_empty() && !text.contains('\0') {
                return text;
            }
        }
    }
    String::new()
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const u8,
}

extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
}

/// Can this user write in `path` (like Python's os.access(path, os.W_OK))?
pub fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid NUL-terminated path; access only reads it.
    unsafe { access(c.as_ptr(), 2) == 0 }
}

/// Local date and time as (year, month, day, hour, minute, second).
pub fn local_now() -> (i32, i32, i32, i32, i32, i32) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: localtime_r only writes into the struct we pass.
    unsafe { localtime_r(&now, &mut tm) };
    (tm.year + 1900, tm.mon + 1, tm.mday, tm.hour, tm.min, tm.sec)
}

/// "20261122-013040": for file and folder names.
pub fn stamp() -> String {
    let (y, mo, d, h, mi, s) = local_now();
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Quote a word for a shell command line, like Python's shlex.quote.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// Command lines of this user's running processes (an editor shows the file it has open).
pub fn command_lines() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
        .map(|b| String::from_utf8_lossy(&b).replace('\0', " "))
        .collect()
}

extern "C" {
    fn setsid() -> i32;
}

/// Start a program on its own, in a new session: it outlives this process and
/// the pane or popup it came from. Output goes to `log`, or nowhere.
pub fn spawn_detached(cmd: &mut Command, log: Option<&Path>) -> std::io::Result<()> {
    use std::process::Stdio;
    cmd.stdin(Stdio::null());
    match log.and_then(|l| std::fs::OpenOptions::new().create(true).append(true).open(l).ok()) {
        Some(f) => {
            let f2 = f.try_clone()?;
            cmd.stdout(f).stderr(f2);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    // SAFETY: setsid() is async-signal-safe and touches nothing of the parent.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    cmd.spawn().map(|_| ())
}

extern "C" {
    #[link_name = "kill"]
    fn c_kill(pid: i32, sig: i32) -> i32;
}

/// Send a signal to a process. True if it was delivered.
pub fn kill(pid: i32, sig: i32) -> bool {
    pid > 0 && unsafe { c_kill(pid, sig) == 0 }
}

/// Split a command line into words like a shell (quotes and backslashes), like Python's shlex.split.
pub fn sh_split(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut has) = (Vec::new(), String::new(), false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has = true;
                for d in chars.by_ref() {
                    if d == '\'' { break; }
                    cur.push(d);
                }
            }
            '"' => {
                has = true;
                while let Some(d) = chars.next() {
                    match d {
                        '"' => break,
                        '\\' => if let Some(e) = chars.next() { cur.push(e) },
                        _ => cur.push(d),
                    }
                }
            }
            '\\' => if let Some(e) = chars.next() { cur.push(e); has = true },
            c if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        words.push(cur);
    }
    words
}
