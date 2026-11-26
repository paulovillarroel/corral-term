//! Mirador's server (`corral _mirador`): the page and its JSON API on
//! 127.0.0.1, behind a token. Sessions and variables come from the files the
//! R and Python sessions write, plots from their httpgd-style servers, tables
//! from the DuckDB CLI.

use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{self, obj, Value};
use crate::table;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ACTIVE_WINDOW_S: f64 = 6.0;
const MAX_PAGE: i64 = 500;
/// How long a page's /api/state?wait= request may wait for a change.
const WAIT_MAX: Duration = Duration::from_secs(25);
/// SVGs larger than this (a scatter of many thousand points) go out as PNG:
/// the browser would have to build every point as an element.
const HEAVY_SVG: usize = 2 * 1024 * 1024;

struct App {
    token: String,
    port: u16,
    last_client: f64,
    tables: Vec<Value>,
    schema: HashMap<String, Value>,
    /// Each table's number of rows, without filters.
    all: HashMap<String, usize>,
    show_plot: Value,
    theme_asked: Option<Instant>,
    /// Plots (session:id) whose SVG was too heavy: shown as PNG from then on.
    heavy: std::collections::HashSet<String>,
}

type Shared = Arc<Mutex<App>>;

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn sessions_dir() -> PathBuf {
    env::state_dir().join("sessions")
}

fn tables_file() -> PathBuf {
    env::state_dir().join("mirador-tables.json")
}

fn session_dir(sid: &str) -> Result<PathBuf, String> {
    // A name, never a way out of the sessions folder ("..").
    if sid.is_empty() || sid.chars().all(|c| c == '.') || !sid.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err(tr("invalid session", "sesión inválida").into());
    }
    Ok(sessions_dir().join(sid))
}

fn mtime(path: &Path) -> Option<f64> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64())
}

// --------------------------------------------------------------------------- small HTTP client (httpgd)

pub(crate) fn http_get(host: &str, target: &str) -> Option<(Vec<u8>, String)> {
    let mut s = TcpStream::connect(host).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write!(s, "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let status = head.split_whitespace().nth(1)?.parse::<u16>().ok()?;
    if status != 200 {
        return None;
    }
    let header = |name: &str| head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    });
    if header("transfer-encoding").map(|v| v.eq_ignore_ascii_case("chunked")).unwrap_or(false) {
        body = unchunk(&body);
    }
    Some((body, header("content-type").unwrap_or_else(|| "application/octet-stream".into())))
}

fn unchunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = data.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..eol]).split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = eol + 2;
        let end = (start + size).min(data.len());
        out.extend_from_slice(&data[start..end]);
        data = &data[(end + 2).min(data.len())..];
    }
    out
}

/// host:port and token of a session's plot server (httpgd, or the Python one).
fn plots_base(sid: &str) -> Option<(String, String)> {
    let url = std::fs::read_to_string(session_dir(sid).ok()?.join("plots-url")).ok()?;
    let url = url.trim().strip_prefix("http://")?;
    let (host, rest) = url.split_once('/').unwrap_or((url, ""));
    let hostname = host.split(':').next().unwrap_or("");
    if hostname != "127.0.0.1" && hostname != "localhost" {
        return None;
    }
    let token = rest.split_once('?').map(|(_, q)| query(q)).and_then(|q| q.get("token").cloned()).unwrap_or_default();
    Some((host.to_string(), token))
}

pub(crate) fn plots_raw(sid: &str, path: &str, params: &[(&str, String)]) -> Option<(Vec<u8>, String)> {
    let (host, token) = plots_base(sid)?;
    let mut q: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    if !token.is_empty() {
        q.push(format!("token={}", encode(&token)));
    }
    http_get(&host, &format!("/{path}?{}", q.join("&")))
}

pub(crate) fn plots_json(sid: &str, path: &str) -> Value {
    plots_raw(sid, path, &[]).and_then(|(b, _)| json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or(Value::Null)
}

// --------------------------------------------------------------------------- kept plots

/// Where a session's checkpoint keeps the plots a crashed session left it: corral.R
/// and corral_py.py move them there when they restore, as k0001.svg/.png and so on
/// (keep_plots). They go with the checkpoint: when the console closes, or after a week.
fn kept_dir(sid: &str) -> Option<PathBuf> {
    let meta = env::read_json(&session_dir(sid).ok()?.join("meta.json"));
    let prefix = if meta.get("lang").str() == Some("python") { "py" } else { "r" };
    Some(env::state_dir().join("checkpoints").join(format!("{prefix}-{sid}")).join("kept"))
}

/// The plot server's state, counting the kept plots too: the page reloads its list when
/// `upid` changes, and shows the kept ones even before the session draws its first plot.
fn shown_state(sid: &str) -> Value {
    let live = plots_json(sid, "state");
    let kept = kept_stems(sid).len();
    if kept == 0 {
        return live;
    }
    let upid = match live.get("upid") {
        Value::Num(n) => format!("{}k{kept}", *n as i64),
        _ => format!("k{kept}"),
    };
    let hsize = live.get("hsize").num().unwrap_or(0.0) + kept as f64;
    obj(vec![("upid", upid.into()), ("hsize", hsize.into()), ("active", true.into())])
}

/// The kept plots' names (k0001, k0002…), oldest first.
fn kept_stems(sid: &str) -> Vec<String> {
    let Some(dir) = kept_dir(sid) else { return Vec::new() };
    let mut stems: Vec<String> = std::fs::read_dir(dir).map(|d| d.flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".png").or(n.strip_suffix(".svg"))).map(String::from))
        .filter(|s| kept_name(s)).collect()).unwrap_or_default();
    stems.sort();
    stems.dedup();
    stems
}

/// A kept plot's name: k and four digits, never a way out of its folder.
fn kept_name(stem: &str) -> bool {
    stem.len() == 5 && stem.starts_with('k') && stem[1..].chars().all(|c| c.is_ascii_digit())
}

/// The plots to show for a session, as the page and the terminal's viewer list them:
/// the kept ones first ("kept-k0001"…), then the session's own. `corral eval` keeps
/// using the live list alone, where positions match the plot server's.
pub(crate) fn shown_plot_ids(sid: &str) -> Vec<String> {
    let live = plots_json(sid, "plots").get("plots").arr().iter().filter_map(|p| match p.get("id") {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(format!("{}", *n as i64)),
        _ => None,
    }).collect::<Vec<_>>();
    kept_stems(sid).into_iter().map(|s| format!("kept-{s}")).chain(live).collect()
}

/// A plot to show: a kept one from its file (the PNG when `renderer` asks for one or
/// there is no SVG, else the SVG), any other from the session's plot server.
pub(crate) fn shown_plot(sid: &str, id: &str, params: &[(&str, String)]) -> Option<(Vec<u8>, String)> {
    let Some(stem) = id.strip_prefix("kept-") else {
        let mut p = params.to_vec();
        p.push(("id", id.to_string()));
        return plots_raw(sid, "plot", &p);
    };
    if !kept_name(stem) {
        return None;
    }
    let dir = kept_dir(sid)?;
    let png = params.iter().any(|(k, v)| *k == "renderer" && v == "png");
    let svg = dir.join(format!("{stem}.svg"));
    if !png && svg.is_file() {
        return std::fs::read(svg).ok().map(|b| (b, "image/svg+xml".to_string()));
    }
    std::fs::read(dir.join(format!("{stem}.png"))).ok().map(|b| (b, "image/png".to_string()))
}

/// Remove the plot at `index` in the shown list: a kept one's files, or the live one at
/// that position among the session's own, with its copy in the checkpoint, so it doesn't
/// come back after a crash. All of them for `clear`.
fn remove_shown(sid: &str, index: Option<i64>) -> bool {
    let kept = kept_stems(sid);
    let checkpoint = kept_dir(sid).and_then(|k| k.parent().map(Path::to_path_buf));
    let Some(i) = index else {
        if let Some(c) = &checkpoint {
            let _ = std::fs::remove_dir_all(c.join("kept"));
            if let Ok(d) = std::fs::read_dir(c.join("plots")) {
                for f in d.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "png" || e == "svg")) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        return plots_raw(sid, "clear", &[]).is_some() || !kept.is_empty();
    };
    let total = shown_plot_ids(sid).len() as i64;
    let i = if i < 0 { total + i } else { i };
    if i < 0 || i >= total {
        return false;
    }
    if (i as usize) < kept.len() {
        // Both files: the list counts a plot while either is there.
        let Some(dir) = checkpoint.map(|c| c.join("kept")) else { return false };
        let gone: Vec<bool> = ["png", "svg"].iter().map(|e| std::fs::remove_file(dir.join(format!("{}.{e}", kept[i as usize]))).is_ok()).collect();
        return gone.contains(&true);
    }
    let live_index = i - kept.len() as i64;
    let live_id = shown_plot_ids(sid).get(i as usize).cloned().filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric()));
    let ok = plots_raw(sid, "remove", &[("index", live_index.to_string())]).is_some();
    if let (true, Some(c), Some(id)) = (ok, checkpoint, live_id) {
        for e in ["png", "svg"] {
            let _ = std::fs::remove_file(c.join("plots").join(format!("{id}.{e}")));
        }
    }
    ok
}

/// `corral _keep-plots SESSION PID`: keeps an R session's plots in its checkpoint, so
/// they outlive a crash. corral.R starts it with the first plot and it stays while that
/// R (PID) lives, one per session (a second one finds the lock taken and leaves). Every
/// 250 ms it reads `plots-rev`, which R rewrites when the plots change, and then asks
/// httpgd for them over HTTP, as Mirador does. Not in R's forked copy, where drawing can
/// hang it for good (httpgd's server thread may hold a lock at the fork), nor started
/// from R for each plot: R's system2() forks R, 8 ms with 400-650 MB in use.
/// The newest 30 are kept. A plot whose SVG takes over 200 ms to draw is skipped from
/// then on (a million points takes 0.4 s and 90 MB); otherwise its SVG is kept up to
/// 2 MB, and a 720x576 PNG always, for the terminal's viewer. The newest is written
/// again each time, since lines() can draw over it; the others once.
pub(crate) fn cmd_keep_plots(rest: &[String]) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let (Some(sid), Some(pid)) = (rest.first(), rest.get(1).and_then(|p| p.parse::<i64>().ok())) else {
        return Err("usage: corral _keep-plots SESSION PID".into());
    };
    let sdir = session_dir(sid)?;
    let lock = std::fs::File::create(sdir.join("keep-plots.lock")).map_err(|e| e.to_string())?;
    // SAFETY: flock on a file this function keeps open; released when the process ends.
    if unsafe { crate::eval::flock(lock.as_raw_fd(), crate::eval::LOCK_EX | crate::eval::LOCK_NB) } != 0 {
        return Ok(());
    }
    let _ = std::fs::write(sdir.join("keep-plots.pid"), std::process::id().to_string());
    let checkpoint = env::state_dir().join("checkpoints").join(format!("r-{sid}"));
    let mut last = String::new();
    while env::pid_alive(pid) {
        let rev = std::fs::read_to_string(sdir.join("plots-rev")).unwrap_or_default();
        // R writes the checkpoint folder soon after it starts; until then, look again.
        if rev != last && checkpoint.is_dir() {
            last = rev;
            let dir = checkpoint.join("plots");
            if env::config().get("checkpoints").str() != Some("off") && std::fs::create_dir_all(&dir).is_ok() {
                write_plots(sid, &dir);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// One pass of `_keep-plots`: the files in `dir` follow the session's newest 30 plots.
fn write_plots(sid: &str, dir: &Path) {
    let list = plots_json(sid, "plots");
    // No answer: httpgd is gone or busy, and the files stay as they are.
    let Value::Arr(plots) = list.get("plots") else { return };
    let ids: Vec<String> = plots.iter().filter_map(|p| match p.get("id") {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(format!("{}", *n as i64)),
        _ => None,
    }).filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).collect();
    let ids = &ids[ids.len().saturating_sub(30)..];
    if let Ok(d) = std::fs::read_dir(dir) {
        for f in d.flatten() {
            let path = f.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if path.extension().is_some_and(|e| e == "png" || e == "svg") && !ids.iter().any(|id| id == stem) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let skipped_file = dir.join("skipped");
    let mut skipped: Vec<String> = std::fs::read_to_string(&skipped_file).unwrap_or_default().lines()
        .filter(|l| ids.iter().any(|id| id == l)).map(String::from).collect();
    let put = |path: PathBuf, bytes: &[u8]| {
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    };
    let size = |renderer: &str, id: &str| -> Vec<(&'static str, String)> {
        vec![("id", id.to_string()), ("renderer", renderer.to_string()), ("width", "720".into()), ("height", "576".into())]
    };
    for (i, id) in ids.iter().enumerate() {
        let png = dir.join(format!("{id}.png"));
        if skipped.contains(id) || (png.exists() && i + 1 < ids.len()) {
            continue;
        }
        let start = Instant::now();
        let Some((svg, _)) = plots_raw(sid, "plot", &size("svg", id)) else { continue };
        if start.elapsed() > Duration::from_millis(200) {
            skipped.push(id.clone());
            let _ = std::fs::remove_file(&png);
            let _ = std::fs::remove_file(dir.join(format!("{id}.svg")));
            continue;
        }
        if svg.len() <= HEAVY_SVG {
            put(dir.join(format!("{id}.svg")), &svg);
        } else {
            let _ = std::fs::remove_file(dir.join(format!("{id}.svg")));
        }
        if let Some((bytes, _)) = plots_raw(sid, "plot", &size("png", id)) {
            put(png, &bytes);
        }
    }
    let _ = std::fs::write(&skipped_file, skipped.join("\n"));
}

// --------------------------------------------------------------------------- DuckDB

pub(crate) fn sql_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(crate) fn sql_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The rows of one query, from the DuckDB this process keeps open (duckdb.rs).
pub(crate) fn duck(sql: &str) -> Result<Value, String> {
    crate::duckdb::query(sql)
}

pub(crate) fn table_source(path: &str) -> String {
    let low = path.to_lowercase();
    if low.ends_with(".parquet") {
        format!("read_parquet({})", sql_lit(path))
    } else if low.ends_with(".json") || low.ends_with(".ndjson") {
        format!("read_json_auto({})", sql_lit(path))
    } else {
        format!("read_csv_auto({})", sql_lit(path))
    }
}

impl App {
    fn save_tables(&self) {
        env::write_json(&tables_file(), &Value::Arr(self.tables.clone()));
    }

    fn table(&self, tid: &str) -> Result<Value, String> {
        let t = self.tables.iter().find(|t| t.get("id").str() == Some(tid)).ok_or(tr("unknown table", "tabla desconocida"))?;
        if !Path::new(t.get("path").str().unwrap_or("")).is_file() {
            return Err(tr("the file is gone (did the session end?)", "el archivo ya no existe (¿se cerró la sesión?)").into());
        }
        Ok(t.clone())
    }
}

fn schema(app: &Shared, tid: &str) -> Result<Value, String> {
    let (table, cached) = {
        let a = app.lock().unwrap();
        (a.table(tid)?, a.schema.get(tid).cloned())
    };
    if let Some(s) = cached {
        return Ok(s);
    }
    let cols = duck(&format!("DESCRIBE SELECT * FROM {}", table_source(table.get("path").str().unwrap_or(""))))?;
    let s = Value::Arr(cols.arr().iter().map(|c| obj(vec![
        ("name", c.get("column_name").clone()), ("type", c.get("column_type").clone())])).collect());
    app.lock().unwrap().schema.insert(tid.to_string(), s.clone());
    Ok(s)
}

/// The filters a page sends, `filters=[{"text": "mpg > 20"}, {"eq": "region", "value": "North"}]`,
/// made the way the terminal viewer makes them: (what it shows, its SQL).
fn conditions(q: &HashMap<String, String>, names: &[String]) -> Result<Vec<(String, String)>, String> {
    let Some(raw) = q.get("filters").filter(|s| !s.trim().is_empty()) else { return Ok(Vec::new()) };
    let list = json::parse(raw).map_err(|_| tr("unreadable filters", "filtros ilegibles").to_string())?;
    Ok(list.arr().iter().map(|f| match f.get("eq").str() {
        Some(name) => table::equal_condition(name, f.get("value")),
        None => {
            let text = f.get("text").str().unwrap_or("").trim().to_string();
            let sql = if text.is_empty() { "true".to_string() } else { table::translate(&text, names) };
            (text, sql)
        }
    }).collect())
}

/// A page of a table's rows, with the filters, the search and the sort of the
/// terminal viewer (the same SQL). `count=0` skips counting (more rows of the
/// same query). The error is the JSON reply: for a filter DuckDB rejects,
/// "invalid filter: …" and `bad`, which one.
fn rows(app: &Shared, q: &HashMap<String, String>) -> Result<Value, Value> {
    let fail = |e: String| obj(vec![("error", e.into())]);
    let tid = q.get("id").map(String::as_str).unwrap_or("");
    let table = app.lock().unwrap().table(tid).map_err(fail)?;
    let cols = schema(app, tid).map_err(fail)?;
    let names: Vec<String> = cols.arr().iter().map(|c| c.get("name").str().unwrap_or("").to_string()).collect();
    let src = table_source(table.get("path").str().unwrap_or(""));
    let conds = conditions(q, &names).map_err(fail)?;
    let where_ = table::where_sql(&conds, q.get("q").map(String::as_str).unwrap_or(""), &names);
    let sort = q.get("sort").cloned().unwrap_or_default();
    let order = if names.contains(&sort) { table::order_sql(&sort, q.get("desc").map(String::as_str) == Some("1")) } else { String::new() };
    let mut reply = vec![];
    if q.get("count").map(String::as_str) != Some("0") {
        match table::count(&format!("SELECT count(*) AS n FROM {src}{where_}")) {
            Ok(n) => reply.push(("total", (n as f64).into())),
            Err(e) => {
                // Which filter: DuckDB's reason for the one it rejects (the newest is the usual suspect).
                let bad = (0..conds.len()).rev().find_map(|i| {
                    let alone = table::where_sql(&conds[i..=i], "", &names);
                    table::count(&format!("SELECT count(*) AS n FROM {src}{alone}")).err().map(|e| (i, e))
                });
                return Err(match bad {
                    Some((i, e)) => obj(vec![("error", table::invalid_filter(&e).into()), ("bad", (i as f64).into())]),
                    None => fail(e),
                });
            }
        }
        // All the rows, without filters: counted once per table.
        let cached = app.lock().unwrap().all.get(tid).copied();
        let all = match cached {
            Some(n) => n,
            None => {
                let n = table::count(&format!("SELECT count(*) AS n FROM {src}")).map_err(fail)?;
                app.lock().unwrap().all.insert(tid.to_string(), n);
                n
            }
        };
        reply.push(("all", (all as f64).into()));
    }
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(200).clamp(1, MAX_PAGE);
    let offset = q.get("offset").and_then(|l| l.parse::<i64>().ok()).unwrap_or(0).max(0);
    let rows = duck(&format!("SELECT * FROM {src}{where_}{order} LIMIT {limit} OFFSET {offset}")).map_err(fail)?;
    // Each column with the name a filter starts with (`f`): as is or in `backticks`.
    let columns = Value::Arr(cols.arr().iter().map(|c| {
        let mut c = c.clone();
        c.set("ref", table::filter_name(c.get("name").str().unwrap_or("")).into());
        c
    }).collect());
    reply.extend([("columns", columns), ("rows", rows), ("offset", (offset as f64).into()),
                  ("conditions", Value::Arr(conds.into_iter().map(|(label, _)| label.into()).collect()))]);
    Ok(obj(reply))
}

// --------------------------------------------------------------------------- page and palette

fn palette_file() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config")).join("corral/palette.json")
}

fn palette(app: &Shared) -> Value {
    let mut p = env::read_json(&palette_file());
    // The palette follows herdr's config and Corral's own theme choice (corral config theme).
    let changed = crate::theme::sources_mtime();
    let stale = changed > 0.0 && (changed - p.get("source_mtime").num().unwrap_or(0.0)).abs() > 1e-3;
    if stale {
        let mut a = app.lock().unwrap();
        if a.theme_asked.map(|t| t.elapsed() > Duration::from_secs(10)).unwrap_or(true) {
            a.theme_asked = Some(Instant::now());
            drop(a);
            p = crate::theme::apply();
        }
    }
    if matches!(p, Value::Obj(ref items) if items.is_empty()) { Value::Null } else { p }
}

fn themed_page() -> Vec<u8> {
    let page = std::fs::read_to_string(env::repo().join("mirador/index.html")).unwrap_or_default();
    // The page is written in English and translates itself; it starts in this language.
    let page = page.replacen("<html lang=\"en\">", &format!("<html lang=\"{}\">", crate::i18n::lang()), 1);
    let p = env::read_json(&palette_file());
    if p.get("bg").is_null() {
        return page.into_bytes();
    }
    let map = [("--bg", "bg"), ("--panel", "surface"), ("--side", "surface"), ("--side-2", "raised"), ("--line", "border"),
               ("--line-2", "selection"), ("--text", "text"), ("--muted", "muted"), ("--dim", "dim"), ("--accent", "accent"),
               ("--green", "green"), ("--amber", "yellow"), ("--red", "red"), ("--blue", "blue"), ("--violet", "violet")];
    let css: Vec<String> = map.iter().map(|(k, v)| format!("{k}:{}", p.get(v).str().unwrap_or(""))).collect();
    page.replacen("</head>", &format!("<style>:root{{{}}}</style>\n</head>", css.join(";")), 1).into_bytes()
}

/// httpgd names fonts installed in WSL (Liberation *); give Windows browsers fallbacks.
fn fix_svg_fonts(svg: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(svg);
    let mut out = String::with_capacity(text.len() + 256);
    let mut rest: &str = &text;
    while let Some(i) = rest.find("font-family:") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 12..];
        let end = after.find([';', '"', '<', '>']).unwrap_or(after.len());
        let fam = after[..end].trim().trim_matches(|c| c == '\'' || c == '"').to_string();
        let low = fam.to_lowercase();
        let fallback = if low.contains("mono") || low.contains("courier") {
            "'Liberation Mono', 'Cascadia Mono', Consolas, monospace"
        } else if low.contains("serif") && !low.contains("sans") {
            "'Liberation Serif', 'Times New Roman', Georgia, serif"
        } else {
            "'Liberation Sans', Arial, 'Segoe UI', Helvetica, sans-serif"
        };
        out.push_str(&format!("font-family: '{fam}', {fallback}"));
        rest = &after[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

// --------------------------------------------------------------------------- state

fn sessions() -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(sessions_dir()) else { return out };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        let mut meta = env::read_json(&d.join("meta.json"));
        let Some(pid) = meta.get("pid").num() else { continue };
        if !env::pid_alive(pid as i64) {
            let _ = std::fs::remove_dir_all(&d);
            continue;
        }
        meta.set("vars_rev", mtime(&d.join("vars.json")).map(Value::Num).unwrap_or(Value::Null));
        let id = meta.get("id").str().unwrap_or("").to_string();
        meta.set("plots", shown_state(&id));
        out.push(meta);
    }
    out
}

/// A cheap summary of everything the page shows: it changes when a session
/// starts or ends, its variables or plots change (R and Python touch
/// `plots-rev` after a command that drew), a table opens, a plot is asked
/// for, or the palette changes. Only file dates, no question to httpgd.
fn fingerprint(app: &Shared) -> String {
    let mut out = String::new();
    if let Ok(entries) = std::fs::read_dir(sessions_dir()) {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            out.push_str(&d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            for f in ["meta.json", "vars.json", "plots-rev", "plots-url"] {
                out.push_str(&format!(":{}", mtime(&d.join(f)).unwrap_or(0.0)));
            }
            out.push('|');
        }
    }
    let a = app.lock().unwrap();
    for t in &a.tables {
        out.push_str(t.get("id").str().unwrap_or(""));
    }
    // The language too, so a page waiting for a change switches language at once.
    out.push_str(&format!("|{}|{}|{}|{}", a.show_plot.to_json(), mtime(&palette_file()).unwrap_or(0.0), crate::i18n::lang(), current_plot_size()));
    format!("{:016x}", fnv(out.as_bytes()))
}

/// The size Mirador draws R plots at (`corral config plot_size`): `window`, the
/// default, fits them to the page; WIDTHxHEIGHT in pixels keeps them that size
/// whatever the window. None when the text is neither.
pub fn plot_size(value: &str) -> Option<Option<(u32, u32)>> {
    let v = value.trim().to_lowercase();
    if v == "window" {
        return Some(None);
    }
    let (w, h) = v.split_once(['x', '×'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    ((200..=4000).contains(&w) && (150..=4000).contains(&h)).then_some(Some((w, h)))
}

/// The setting as Corral writes it: `window` or `1000x650`.
pub fn plot_size_text(size: Option<(u32, u32)>) -> String {
    size.map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| "window".into())
}

/// The plot size in effect: the setting when it is valid, otherwise the window's.
fn current_plot_size() -> String {
    let set = env::config().get("plot_size").str().and_then(plot_size).unwrap_or(None);
    plot_size_text(set)
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn state(app: &Shared) -> Value {
    let (tables, show_plot) = {
        let mut a = app.lock().unwrap();
        // Session temp files go away with the session: drop their tables.
        let before = a.tables.len();
        a.tables.retain(|t| Path::new(t.get("path").str().unwrap_or("")).is_file()
            && t.get("session").str().map(|s| sessions_dir().join(s).is_dir()).unwrap_or(true));
        if a.tables.len() != before {
            a.save_tables();
        }
        (a.tables.clone(), a.show_plot.clone())
    };
    let workspaces = herdr::call("workspace.list", Value::obj()).ok()
        .map(|w| Value::Arr(w.get("workspaces").arr().iter().map(|w| obj(vec![
            ("id", w.get("workspace_id").clone()), ("label", w.get("label").clone()),
            ("focused", w.get("focused").clone())])).collect()))
        .unwrap_or(Value::Arr(Vec::new()));
    obj(vec![
        ("version", VERSION.into()),
        ("now", now().into()),
        ("workspaces", workspaces),
        ("sessions", Value::Arr(sessions())),
        ("tables", Value::Arr(tables.iter().map(|t| obj(vec![
            ("id", t.get("id").clone()), ("title", t.get("title").clone()),
            ("session", t.get("session").clone()), ("added", t.get("added").clone())])).collect())),
        ("show_plot", show_plot),
        ("palette", palette(app)),
        ("lang", crate::i18n::lang().into()),
        ("plot_size", current_plot_size().into()),
    ])
}

// --------------------------------------------------------------------------- HTTP server

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn query(q: &str) -> HashMap<String, String> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    }).collect()
}

struct Reply {
    code: u16,
    ctype: String,
    body: Vec<u8>,
    extra: Vec<(String, String)>,
}

fn json_reply(code: u16, v: &Value) -> Reply {
    Reply { code, ctype: "application/json; charset=utf-8".into(), body: v.to_json().into_bytes(), extra: Vec::new() }
}

fn error(code: u16, msg: &str) -> Reply {
    json_reply(code, &obj(vec![("error", msg.into())]))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(mut stream: TcpStream, app: Shared) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    let head_end = loop {
        let Ok(n) = stream.read(&mut buf) else { return };
        if n == 0 {
            return;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if raw.len() > 65536 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.lines();
    let request = lines.next().unwrap_or("");
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let headers: HashMap<String, String> = lines.filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
    let length: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0).min(1_000_000);
    let mut body = raw[head_end + 4..].to_vec();
    while body.len() < length {
        let Ok(n) = stream.read(&mut buf) else { break };
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let (path, q) = target.split_once('?').map(|(p, q)| (p, query(q))).unwrap_or((target, HashMap::new()));

    let reply = {
        let (token, port) = {
            let a = app.lock().unwrap();
            (a.token.clone(), a.port)
        };
        let host = headers.get("host").map(|h| h.to_lowercase()).unwrap_or_default();
        let given = headers.get("x-corral-token").cloned().or_else(|| q.get("token").cloned()).unwrap_or_default();
        if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") || !same(&given, &token) {
            error(403, tr("invalid token; open Mirador with `corral mirador`", "token inválido; abre Mirador con `corral mirador`"))
        } else {
            let result = if method == "POST" {
                let body = json::parse(String::from_utf8_lossy(&body).trim()).unwrap_or_else(|_| Value::obj());
                post(&app, path, &body)
            } else {
                get(&app, path, &q)
            };
            result.unwrap_or_else(|e| error(400, &e))
        }
    };
    let reason = match reply.code { 200 => "OK", 400 => "Bad Request", 403 => "Forbidden", 404 => "Not Found", _ => "Error" };
    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
                            X-Content-Type-Options: nosniff\r\nConnection: close\r\n", reply.code, reply.ctype, reply.body.len());
    for (k, v) in &reply.extra {
        head += &format!("{k}: {v}\r\n");
    }
    head += "\r\n";
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
}

fn get(app: &Shared, path: &str, q: &HashMap<String, String>) -> Result<Reply, String> {
    let arg = |k: &str, d: &str| q.get(k).cloned().unwrap_or_else(|| d.to_string());
    Ok(match path {
        "/" | "/index.html" => Reply { code: 200, ctype: "text/html; charset=utf-8".into(), body: themed_page(), extra: Vec::new() },
        "/api/ping" => {
            let last = app.lock().unwrap().last_client;
            json_reply(200, &obj(vec![("ok", true.into()), ("version", VERSION.into()),
                                       ("active", (now() - last < ACTIVE_WINDOW_S).into()), ("pid", (std::process::id() as f64).into())]))
        }
        "/api/state" => {
            app.lock().unwrap().last_client = now();
            // With ?wait=<rev>, answer when something changed (or after a while),
            // instead of the page asking again and again.
            if let Some(seen) = q.get("wait") {
                let end = Instant::now() + WAIT_MAX;
                while fingerprint(app) == *seen && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(100));
                    app.lock().unwrap().last_client = now(); // the page is still open
                }
            }
            // The revision first: a change right after it only makes the next wait return at once.
            let rev = fingerprint(app);
            let mut st = state(app);
            st.set("rev", rev.into());
            json_reply(200, &st)
        }
        "/api/vars" => {
            let v = env::read_json(&session_dir(&arg("session", ""))?.join("vars.json"));
            let v = if v.get("vars").is_null() { obj(vec![("vars", Value::Arr(Vec::new()))]) } else { v };
            json_reply(200, &v)
        }
        "/api/plots" => {
            let ids = shown_plot_ids(&arg("session", ""));
            json_reply(200, &obj(vec![("plots", Value::Arr(ids.into_iter().map(|id| obj(vec![("id", id.into())])).collect()))]))
        }
        "/api/plot" => {
            // renderer=auto (what the page shows): SVG, or PNG for a plot whose SVG is too heavy.
            let asked = arg("renderer", "svg");
            let key = q.get("id").filter(|s| !s.is_empty()).map(|id| format!("{}:{id}", arg("session", "")));
            let known_heavy = key.as_ref().is_some_and(|k| app.lock().unwrap().heavy.contains(k));
            let renderer = if asked == "auto" { if known_heavy { "png" } else { "svg" }.to_string() } else { asked.clone() };
            let fetch = |renderer: &str| {
                // A PNG for the screen is drawn at the screen's pixel density.
                let zoom = if asked == "auto" && renderer == "png" { arg("dpr", "1") } else { arg("zoom", "1") };
                let z = zoom.parse::<f64>().unwrap_or(1.0).clamp(1.0, 2.0);
                let size = |k: &str, d: &str| -> String {
                    let v: f64 = arg(k, d).parse().unwrap_or(0.0);
                    if asked == "auto" && renderer == "png" { format!("{}", (v * z).round()) } else { arg(k, d) }
                };
                let mut params = vec![("width", size("width", "720")), ("height", size("height", "576")),
                                      ("renderer", renderer.to_string()), ("zoom", format!("{z}"))];
                if q.get("id").map(|s| !s.is_empty()).unwrap_or(false) {
                    return shown_plot(&arg("session", ""), &arg("id", ""), &params);
                }
                params.push(("index", arg("index", "-1")));
                plots_raw(&arg("session", ""), "plot", &params)
            };
            let Some((mut body, mut ctype)) = fetch(&renderer) else {
                return Ok(error(404, tr("plot not available", "gráfico no disponible")));
            };
            if asked == "auto" && renderer == "svg" && body.len() > HEAVY_SVG {
                if let Some(k) = &key {
                    app.lock().unwrap().heavy.insert(k.clone());
                }
                if let Some((png, png_type)) = fetch("png") {
                    body = png;
                    ctype = png_type;
                }
            }
            if ctype.contains("svg") {
                body = fix_svg_fonts(&body);
            }
            let mut extra = Vec::new();
            if q.get("download").map(|s| !s.is_empty()).unwrap_or(false) {
                let ext = if ctype.contains("png") { "png" } else { "svg" };
                let name = tr("plot", "grafico");
                extra.push(("Content-Disposition".into(), format!("attachment; filename=\"{name}.{ext}\"")));
            }
            Reply { code: 200, ctype, body, extra }
        }
        "/api/table/rows" => match rows(app, q) {
            Ok(v) => json_reply(200, &v),
            Err(e) => json_reply(400, &e),
        },
        "/api/table/summary" => {
            let t = app.lock().unwrap().table(&arg("id", ""))?;
            let s = duck(&format!("SUMMARIZE SELECT * FROM {}", table_source(t.get("path").str().unwrap_or(""))))?;
            json_reply(200, &obj(vec![("columns", s)]))
        }
        _ => error(404, tr("not found", "no encontrado")),
    })
}

fn post(app: &Shared, path: &str, body: &Value) -> Result<Reply, String> {
    let field = |k: &str| body.get(k).str().unwrap_or("").to_string();
    Ok(match path {
        "/api/tables" => {
            let p = std::fs::canonicalize(field("path")).map_err(|_| trf!("{} doesn't exist", "no existe {}", field("path")))?;
            if !p.is_file() {
                return Err(trf!("{} doesn't exist", "no existe {}", p.display()));
            }
            let (title, session) = (field("title"), body.get("session").clone());
            let path_s = p.to_string_lossy().to_string();
            let mut a = app.lock().unwrap();
            // A new View() of the same object replaces its tab (new id, so the page opens it).
            let same_table = |t: &Value| t.get("path").str() == Some(&path_s)
                || (!session.is_null() && t.get("session") == &session && t.get("title").str() == Some(&title));
            let old: Vec<String> = a.tables.iter().filter(|t| same_table(t)).filter_map(|t| t.get("id").str().map(String::from)).collect();
            for id in old {
                a.schema.remove(&id);
                a.all.remove(&id);
            }
            a.tables.retain(|t| !same_table(t));
            let entry = obj(vec![
                ("id", env::random_hex(4).into()),
                ("path", path_s.clone().into()),
                ("title", if title.is_empty() { p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().into() } else { title.clone().into() }),
                ("session", session),
                ("added", now().into()),
            ]);
            a.tables.push(entry.clone());
            a.save_tables();
            json_reply(200, &entry)
        }
        "/api/show-plot" => {
            session_dir(&field("session"))?;
            let mut a = app.lock().unwrap();
            let seq = a.show_plot.get("seq").num().unwrap_or(0.0) + 1.0;
            let id = match body.get("id") { Value::Num(n) => format!("{}", *n as i64), v => v.str().unwrap_or("").to_string() };
            a.show_plot = obj(vec![("session", field("session").into()), ("id", id.into()), ("seq", seq.into())]);
            json_reply(200, &obj(vec![("ok", true.into()), ("seq", seq.into())]))
        }
        "/api/opened" => {
            app.lock().unwrap().last_client = now();
            json_reply(200, &obj(vec![("ok", true.into())]))
        }
        "/api/tables/close" => {
            let id = field("id");
            let mut a = app.lock().unwrap();
            a.tables.retain(|t| t.get("id").str() != Some(&id));
            a.schema.remove(&id);
            a.all.remove(&id);
            a.save_tables();
            json_reply(200, &obj(vec![("ok", true.into())]))
        }
        "/api/plot-size" => {
            // The page's size menu: the same setting as corral config plot_size.
            let Some(size) = plot_size(&field("size")) else {
                return Ok(error(400, tr("the size is window or WIDTHxHEIGHT in pixels, like 1000x650",
                                        "el tamaño es window o ANCHOxALTO en píxeles, como 1000x650")));
            };
            let mut cfg = env::config();
            cfg.set("plot_size", plot_size_text(size).into());
            crate::theme::write_config(&cfg);
            json_reply(200, &obj(vec![("ok", true.into()), ("plot_size", plot_size_text(size).into())]))
        }
        "/api/plots/remove" | "/api/plots/clear" => {
            let sid = field("session");
            let ok = if path.ends_with("clear") {
                remove_shown(&sid, None)
            } else {
                let index = match body.get("index") { Value::Num(n) => *n as i64, _ => -1 };
                remove_shown(&sid, Some(index))
            };
            if ok {
                // R isn't running a command, so mark the change here: open pages update at once.
                if let Ok(dir) = session_dir(&sid) {
                    let _ = std::fs::write(dir.join("plots-rev"), format!("removed {}", now()));
                }
            }
            json_reply(200, &obj(vec![("ok", ok.into())]))
        }
        _ => error(404, tr("not found", "no encontrado")),
    })
}

pub fn serve() -> Result<(), String> {
    let state_dir = env::state_dir();
    let _ = std::fs::create_dir_all(&state_dir);
    let info_file = state_dir.join("mirador.json");
    let token = env::read_json(&info_file).get("token").str().map(String::from).unwrap_or_else(|| env::random_hex(12));
    let wanted: u16 = std::env::var("CORRAL_MIRADOR_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7317);
    let listener = TcpListener::bind(("127.0.0.1", wanted)).or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| trf!("Mirador couldn't open a port: {e}", "Mirador no pudo abrir un puerto: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let tables: Vec<Value> = env::read_json(&tables_file()).arr().iter()
        .filter(|t| Path::new(t.get("path").str().unwrap_or("")).is_file()).cloned().collect();
    let app: Shared = Arc::new(Mutex::new(App {
        token: token.clone(), port, last_client: 0.0, tables, schema: HashMap::new(), all: HashMap::new(), show_plot: Value::Null, theme_asked: None, heavy: Default::default(),
    }));

    let url = format!("http://127.0.0.1:{port}/?token={token}");
    let info = obj(vec![("url", url.clone().into()), ("port", (port as f64).into()), ("token", token.into()),
                        ("pid", (std::process::id() as f64).into()), ("started", now().into()), ("version", VERSION.into())]);
    let tmp = info_file.with_extension("tmp");
    std::fs::write(&tmp, info.to_json()).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    std::fs::rename(&tmp, &info_file).map_err(|e| e.to_string())?;
    println!("{}", trf!("Mirador at {url}", "Mirador en {url}"));

    for stream in listener.incoming().flatten() {
        let app = app.clone();
        std::thread::spawn(move || handle(stream, app));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plot_sizes() {
        assert_eq!(plot_size("window"), Some(None));
        assert_eq!(plot_size(" Window "), Some(None));
        assert_eq!(plot_size("1000x650"), Some(Some((1000, 650))));
        assert_eq!(plot_size("1000 × 650"), Some(Some((1000, 650))));
        assert_eq!(plot_size("800X500"), Some(Some((800, 500))));
        // Too small to read, too big to draw, or not a size.
        assert_eq!(plot_size("100x100"), None);
        assert_eq!(plot_size("9000x500"), None);
        assert_eq!(plot_size("big"), None);
        assert_eq!(plot_size("1000x"), None);
        assert_eq!(plot_size_text(Some((1000, 650))), "1000x650");
        assert_eq!(plot_size_text(None), "window");
    }
}
