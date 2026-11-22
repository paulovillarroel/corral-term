//! Where Corral keeps things, what herdr tells a plugin action, and the way
//! back to the Python version for everything this binary doesn't do itself.

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

/// The repository: CORRAL_REPO, or where this binary was built from (rs/..).
pub fn repo() -> PathBuf {
    if let Some(p) = std::env::var_os("CORRAL_REPO") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new(".")).to_path_buf()
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

/// Hand the whole command over to the Python version (it does everything).
pub fn python_fallback(args: &[String]) -> ! {
    let script = repo().join("bin/corral");
    let err = Command::new("python3").arg(script).args(args).exec();
    eprintln!("corral: no pude ejecutar la versión Python: {err}");
    std::process::exit(1)
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
