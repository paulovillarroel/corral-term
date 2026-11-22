//! Mirador's server (`corral _mirador`): the page and its JSON API on
//! 127.0.0.1, behind a token. Sessions and variables come from the files the
//! R and Python sessions write, plots from their httpgd-style servers, tables
//! from the DuckDB CLI. Same API as mirador/server.py, without Python.

use crate::env;
use crate::herdr;
use crate::json::{self, obj, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = "0.1.0";
const ACTIVE_WINDOW_S: f64 = 6.0;
const MAX_PAGE: i64 = 500;

struct App {
    token: String,
    port: u16,
    last_client: f64,
    tables: Vec<Value>,
    schema: HashMap<String, Value>,
    show_plot: Value,
    theme_asked: Option<Instant>,
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
    if sid.is_empty() || !sid.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err("sesión inválida".into());
    }
    Ok(sessions_dir().join(sid))
}

fn mtime(path: &Path) -> Option<f64> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64())
}

// --------------------------------------------------------------------------- small HTTP client (httpgd)

fn http_get(host: &str, target: &str) -> Option<(Vec<u8>, String)> {
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

fn plots_raw(sid: &str, path: &str, params: &[(&str, String)]) -> Option<(Vec<u8>, String)> {
    let (host, token) = plots_base(sid)?;
    let mut q: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    if !token.is_empty() {
        q.push(format!("token={}", encode(&token)));
    }
    http_get(&host, &format!("/{path}?{}", q.join("&")))
}

fn plots_json(sid: &str, path: &str) -> Value {
    plots_raw(sid, path, &[]).and_then(|(b, _)| json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or(Value::Null)
}

// --------------------------------------------------------------------------- DuckDB

pub(crate) fn sql_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(crate) fn sql_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub(crate) fn duck(sql: &str) -> Result<Value, String> {
    let out = Command::new("duckdb").args(["-json", "-c", sql]).output()
        .map_err(|_| "DuckDB no está en el PATH".to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "DuckDB falló".into() } else { err });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() { Ok(Value::Arr(Vec::new())) } else { json::parse(text.trim()) }
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
        let t = self.tables.iter().find(|t| t.get("id").str() == Some(tid)).ok_or("tabla desconocida")?;
        if !Path::new(t.get("path").str().unwrap_or("")).is_file() {
            return Err("el archivo ya no existe (¿se cerró la sesión?)".into());
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

fn rows(app: &Shared, q: &HashMap<String, String>) -> Result<Value, String> {
    let tid = q.get("id").map(String::as_str).unwrap_or("");
    let table = app.lock().unwrap().table(tid)?;
    let cols = schema(app, tid)?;
    let src = table_source(table.get("path").str().unwrap_or(""));
    let search = q.get("q").map(|s| s.trim().to_string()).unwrap_or_default();
    let where_ = if search.is_empty() {
        String::new()
    } else {
        let pattern = format!("%{}%", search.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let parts: Vec<String> = cols.arr().iter().map(|c| format!("CAST({} AS VARCHAR) ILIKE {} ESCAPE '\\'",
            sql_ident(c.get("name").str().unwrap_or("")), sql_lit(&pattern))).collect();
        if parts.is_empty() { String::new() } else { format!(" WHERE {}", parts.join(" OR ")) }
    };
    let sort = q.get("sort").cloned().unwrap_or_default();
    let order = if !sort.is_empty() && cols.arr().iter().any(|c| c.get("name").str() == Some(&sort)) {
        format!(" ORDER BY {} {} NULLS LAST", sql_ident(&sort), if q.get("desc").map(String::as_str) == Some("1") { "DESC" } else { "ASC" })
    } else {
        String::new()
    };
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(200).clamp(1, MAX_PAGE);
    let offset = q.get("offset").and_then(|l| l.parse::<i64>().ok()).unwrap_or(0).max(0);
    let rows = duck(&format!("SELECT * FROM {src}{where_}{order} LIMIT {limit} OFFSET {offset}"))?;
    let total = duck(&format!("SELECT count(*) AS n FROM {src}{where_}"))?.arr().first().map(|r| r.get("n").clone()).unwrap_or(Value::Num(0.0));
    Ok(obj(vec![("columns", cols), ("rows", rows), ("total", total), ("offset", (offset as f64).into())]))
}

// --------------------------------------------------------------------------- page and palette

fn palette_file() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config")).join("corral/palette.json")
}

fn palette(app: &Shared) -> Value {
    let p = env::read_json(&palette_file());
    let herdr_cfg = std::env::var_os("HERDR_CONFIG_PATH").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config/herdr/config.toml"));
    if let Some(h) = mtime(&herdr_cfg) {
        let mut a = app.lock().unwrap();
        let stale = (h - p.get("source_mtime").num().unwrap_or(0.0)).abs() > 1e-3;
        if stale && a.theme_asked.map(|t| t.elapsed() > Duration::from_secs(10)).unwrap_or(true) {
            a.theme_asked = Some(Instant::now());
            let _ = Command::new("python3").arg(env::repo().join("bin/corral")).arg("theme")
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        }
    }
    if matches!(p, Value::Obj(ref items) if items.is_empty()) { Value::Null } else { p }
}

fn themed_page() -> Vec<u8> {
    let page = std::fs::read_to_string(env::repo().join("mirador/index.html")).unwrap_or_default();
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
        if !Path::new(&format!("/proc/{}", pid as i64)).exists() {
            let _ = std::fs::remove_dir_all(&d);
            continue;
        }
        meta.set("vars_rev", mtime(&d.join("vars.json")).map(Value::Num).unwrap_or(Value::Null));
        let id = meta.get("id").str().unwrap_or("").to_string();
        meta.set("plots", plots_json(&id, "state"));
        out.push(meta);
    }
    out
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
        ("available", true.into()),
        ("agents", Value::Arr(Vec::new())),
        ("workspaces", workspaces),
        ("sessions", Value::Arr(sessions())),
        ("tables", Value::Arr(tables.iter().map(|t| obj(vec![
            ("id", t.get("id").clone()), ("title", t.get("title").clone()),
            ("session", t.get("session").clone()), ("added", t.get("added").clone())])).collect())),
        ("show_plot", show_plot),
        ("palette", palette(app)),
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
            error(403, "token inválido; abre Mirador con `corral mirador`")
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
            json_reply(200, &state(app))
        }
        "/api/vars" => {
            let v = env::read_json(&session_dir(&arg("session", ""))?.join("vars.json"));
            let v = if v.get("vars").is_null() { obj(vec![("vars", Value::Arr(Vec::new()))]) } else { v };
            json_reply(200, &v)
        }
        "/api/plots" => {
            let p = plots_json(&arg("session", ""), "plots");
            let p = if p.is_null() { obj(vec![("plots", Value::Arr(Vec::new()))]) } else { p };
            json_reply(200, &p)
        }
        "/api/plot" => {
            let renderer = arg("renderer", "svg");
            let mut params = vec![("width", arg("width", "720")), ("height", arg("height", "576")),
                                  ("renderer", renderer.clone()), ("zoom", arg("zoom", "1"))];
            if q.get("id").map(|s| !s.is_empty()).unwrap_or(false) {
                params.push(("id", arg("id", "")));
            } else {
                params.push(("index", arg("index", "-1")));
            }
            let Some((mut body, ctype)) = plots_raw(&arg("session", ""), "plot", &params) else {
                return Ok(error(404, "gráfico no disponible"));
            };
            if ctype.contains("svg") {
                body = fix_svg_fonts(&body);
            }
            let mut extra = Vec::new();
            if q.get("download").map(|s| !s.is_empty()).unwrap_or(false) {
                let ext = if renderer == "png" { "png" } else { "svg" };
                extra.push(("Content-Disposition".into(), format!("attachment; filename=\"grafico.{ext}\"")));
            }
            Reply { code: 200, ctype, body, extra }
        }
        "/api/table/rows" => json_reply(200, &rows(app, q)?),
        "/api/table/summary" => {
            let t = app.lock().unwrap().table(&arg("id", ""))?;
            let s = duck(&format!("SUMMARIZE SELECT * FROM {}", table_source(t.get("path").str().unwrap_or(""))))?;
            json_reply(200, &obj(vec![("columns", s)]))
        }
        _ => error(404, "no encontrado"),
    })
}

fn post(app: &Shared, path: &str, body: &Value) -> Result<Reply, String> {
    let field = |k: &str| body.get(k).str().unwrap_or("").to_string();
    Ok(match path {
        "/api/tables" => {
            let p = std::fs::canonicalize(field("path")).map_err(|_| format!("no existe {}", field("path")))?;
            if !p.is_file() {
                return Err(format!("no existe {}", p.display()));
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
            a.save_tables();
            json_reply(200, &obj(vec![("ok", true.into())]))
        }
        "/api/agents/focus" => {
            let pane = field("pane_id");
            if pane.is_empty() || !pane.chars().all(|c| c.is_ascii_alphanumeric() || ":_-".contains(c)) {
                return Err("panel inválido".into());
            }
            let ok = Command::new("herdr").args(["agent", "focus", &pane]).output().map(|o| o.status.success()).unwrap_or(false);
            json_reply(200, &obj(vec![("ok", ok.into())]))
        }
        "/api/view-var" => {
            let (sid, name) = (field("session"), field("name"));
            let dir = session_dir(&sid)?;
            let known = env::read_json(&dir.join("vars.json")).get("vars").arr().iter().any(|v| v.get("name").str() == Some(&name));
            if !known {
                return Err("objeto desconocido".into());
            }
            let meta = env::read_json(&dir.join("meta.json"));
            let pane = meta.get("pane_id").str().ok_or("la sesión no está en un panel de herdr")?.to_string();
            let code = if meta.get("lang").str() == Some("python") { format!("View({name})") } else { format!("View(`{}`)", name.replace('`', "\\`")) };
            let ok = herdr::send_text(&pane, &format!("{code}\r")).is_ok();
            json_reply(200, &obj(vec![("ok", ok.into()), ("code", code.into())]))
        }
        "/api/plots/remove" | "/api/plots/clear" => {
            let sid = field("session");
            let ok = if path.ends_with("clear") {
                plots_raw(&sid, "clear", &[]).is_some()
            } else {
                let index = match body.get("index") { Value::Num(n) => format!("{}", *n as i64), _ => "-1".into() };
                plots_raw(&sid, "remove", &[("index", index)]).is_some()
            };
            json_reply(200, &obj(vec![("ok", ok.into())]))
        }
        _ => error(404, "no encontrado"),
    })
}

pub fn serve() -> Result<(), String> {
    let state_dir = env::state_dir();
    let _ = std::fs::create_dir_all(&state_dir);
    let info_file = state_dir.join("mirador.json");
    let token = env::read_json(&info_file).get("token").str().map(String::from).unwrap_or_else(|| env::random_hex(12));
    let wanted: u16 = std::env::var("CORRAL_MIRADOR_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7317);
    let listener = TcpListener::bind(("127.0.0.1", wanted)).or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("Mirador no pudo abrir un puerto: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let tables: Vec<Value> = env::read_json(&tables_file()).arr().iter()
        .filter(|t| Path::new(t.get("path").str().unwrap_or("")).is_file()).cloned().collect();
    let app: Shared = Arc::new(Mutex::new(App {
        token: token.clone(), port, last_client: 0.0, tables, schema: HashMap::new(), show_plot: Value::Null, theme_asked: None,
    }));

    let url = format!("http://127.0.0.1:{port}/?token={token}");
    let info = obj(vec![("url", url.clone().into()), ("port", (port as f64).into()), ("token", token.into()),
                        ("pid", (std::process::id() as f64).into()), ("started", now().into()), ("version", VERSION.into())]);
    let tmp = info_file.with_extension("tmp");
    std::fs::write(&tmp, info.to_json()).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    std::fs::rename(&tmp, &info_file).map_err(|e| e.to_string())?;
    println!("Mirador en {url}");

    for stream in listener.incoming().flatten() {
        let app = app.clone();
        std::thread::spawn(move || handle(stream, app));
    }
    Ok(())
}
