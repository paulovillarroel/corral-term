//! Plots, tables and Mirador from the command line: `corral plots` (ctrl+b f),
//! `corral view` (View() in R and Python), `corral open` (links clicked in
//! herdr), `corral mirador`, and what the sessions call: `_plot-ready` (first
//! plot of a session), `_show-object` (a plot object from the variables pane)
//! and `_plots` (the in-terminal viewer, for terminals that draw images).

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{self, obj, Value};
use crate::mirador::plots_json;
use crate::term::{self, clip, out, Screen};
use crate::vars::{message_screen, open_popup};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KITTY_TERMINALS: [&str; 3] = ["wezterm", "kitty", "ghostty"];

// ----------------------------------------------------------------------- where plots are shown

/// Which terminal each attached herdr window runs in (from the client processes' environment).
fn herdr_client_terminals() -> Vec<String> {
    let mut found = Vec::new();
    for pid in crate::os::pids() {
        if crate::os::name(pid).is_none_or(|n| n != "herdr") {
            continue;
        }
        let args = crate::os::args(pid).unwrap_or_default();
        if args.get(1).is_some_and(|a| !a.starts_with("--")) {
            continue; // the server or a CLI call, not a window
        }
        let Some(environ) = crate::os::environ(pid) else { continue };
        let vars: Vec<(String, String)> = environ.iter()
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
        let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        found.push(if get("WEZTERM_PANE").is_some() || get("WEZTERM_EXECUTABLE").is_some() {
            "WezTerm".into()
        } else if get("KITTY_WINDOW_ID").is_some() {
            "kitty".into()
        } else {
            get("TERM_PROGRAM").or_else(|| get("TERM")).unwrap_or_else(|| tr("unknown", "desconocida").into())
        });
    }
    found
}

/// ("kitty" | "symbols", why). CORRAL_IMAGES or the config win; "auto" looks at herdr's windows.
pub fn image_mode() -> (String, String) {
    let configured = std::env::var("CORRAL_IMAGES").ok().filter(|v| !v.is_empty())
        .or_else(|| env::config().get("images").str().map(String::from)).unwrap_or_else(|| "auto".into());
    if configured != "auto" {
        return (configured, tr("configured", "configurado").into());
    }
    let terms = herdr_client_terminals();
    let kitty = terms.iter().any(|t| KITTY_TERMINALS.iter().any(|k| t.to_lowercase().contains(k)));
    let shown = if terms.is_empty() { tr("none", "ninguna").to_string() } else { terms.join(", ") };
    ((if kitty { "kitty" } else { "symbols" }).into(), trf!("herdr windows: {shown}", "ventanas de herdr: {shown}"))
}

fn sid(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn httpgd_ids(dir: &Path) -> Vec<String> {
    plots_json(&sid(dir), "plots").get("plots").arr().iter().filter_map(|p| match p.get("id") {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(format!("{}", *n as i64)),
        _ => None,
    }).collect()
}

fn httpgd_state(dir: &Path) -> Value {
    plots_json(&sid(dir), "state")
}

// ----------------------------------------------------------------------- Mirador

pub fn mirador_info() -> Value {
    env::read_json(&env::state_dir().join("mirador.json"))
}

/// A request to Mirador (GET without a body, POST with one). Its JSON reply.
fn mirador_call(info: &Value, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, String> {
    let port = info.get("port").num().ok_or(tr("Mirador isn't running", "Mirador no está corriendo"))? as u16;
    let token = info.get("token").str().unwrap_or("");
    let host = format!("127.0.0.1:{port}");
    // net: local: Mirador, on 127.0.0.1
    let mut s = TcpStream::connect_timeout(&host.parse().map_err(|e| format!("{e}"))?, timeout)
        .map_err(|e| trf!("Mirador isn't responding ({e})", "Mirador no responde ({e})"))?;
    s.set_read_timeout(Some(timeout)).ok();
    let data = body.map(|b| b.to_json()).unwrap_or_default();
    let method = if body.is_some() { "POST" } else { "GET" };
    write!(s, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nX-Corral-Token: {token}\r\nContent-Type: application/json\r\n\
               Content-Length: {}\r\nConnection: close\r\n\r\n{data}", data.len()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| trf!("Mirador isn't responding ({e})", "Mirador no responde ({e})"))?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or(tr("incomplete reply from Mirador", "respuesta de Mirador incompleta"))?;
    let text = String::from_utf8_lossy(&raw[split + 4..]).into_owned();
    Ok(json::parse(text.trim()).unwrap_or_else(|_| Value::obj()))
}

/// Is the server up? With `need_window`, is a Mirador page also polling it?
pub fn mirador_alive(info: &Value, need_window: bool) -> bool {
    let Some(pid) = info.get("pid").num() else { return false };
    if info.get("port").is_null() || !env::pid_alive(pid as i64) {
        return false;
    }
    match mirador_call(info, "/api/ping", None, Duration::from_millis(1500)) {
        Ok(ping) => ping.get(if need_window { "active" } else { "ok" }) == &Value::Bool(true),
        Err(_) => false,
    }
}

pub fn ensure_mirador() -> Result<Value, String> {
    let info = mirador_info();
    if mirador_alive(&info, false) {
        return Ok(info);
    }
    let _ = std::fs::create_dir_all(env::state_dir());
    // One start at a time. Callers that come at once (the first plots of R and Python, the
    // palette) would each start a server, and mirador.json would name only one of them:
    // the others would run where `corral mirador --stop` can't reach them.
    let _start = file_lock("mirador.lock", Duration::from_secs(8));
    let info = mirador_info();
    if mirador_alive(&info, false) {
        return Ok(info); // another caller started it while this one waited
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = env::state_dir().join("mirador.log");
    env::trim_log(&log);
    env::spawn_detached(Command::new(me).arg("_mirador").current_dir(env::repo()), Some(&log))
        .map_err(|e| trf!("couldn't start Mirador: {e}", "no pude iniciar Mirador: {e}"))?;
    let old = info.get("pid").num();
    let end = Instant::now() + Duration::from_secs(6);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(150));
        let info = mirador_info();
        if info.get("pid").num().is_some() && info.get("pid").num() != old && mirador_alive(&info, false) {
            return Ok(info);
        }
    }
    Err(trf!("Mirador didn't start; see {}", "Mirador no arrancó; revisa {}", log.display()))
}

/// A lock in Corral's folder that keeps something one at a time across processes (Mirador's
/// start, opening its window), held until it is dropped (or this process ends). It waits
/// up to `wait`, a little longer than the step can take; past that, or without the file,
/// it goes ahead.
fn file_lock(name: &str, wait: Duration) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let lock = std::fs::File::create(env::state_dir().join(name)).ok()?;
    let end = Instant::now() + wait;
    // SAFETY: flock on a file this function returns open; released when it closes. Rust
    // opens it close-on-exec, so the server started meanwhile doesn't keep it.
    while unsafe { crate::eval::flock(lock.as_raw_fd(), crate::eval::LOCK_EX | crate::eval::LOCK_NB) } != 0 {
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(lock)
}

pub fn open_mirador_window(info: &Value) {
    open_app_window(&with_fit(info.get("url").str().unwrap_or("")));
    let _ = mirador_call(info, "/api/opened", Some(&Value::obj()), Duration::from_secs(3)); // no second window while it loads
    bring_to_front("Mirador", true);
}

/// Mirador in a new window unless a page already shows it, one caller at a time: R's first
/// plot and the variables pane can ask at the same moment, and each, seeing no page yet,
/// would open its own. Under the lock the second sees the first's (`/api/opened` counts
/// as a page for up to 20 s, while the browser loads it, so a long drawing doesn't get a
/// second window either). Whether it opened one; the caller brings it forward.
pub fn open_window_once(info: &Value, url: &str) -> bool {
    let _lock = file_lock("mirador-window.lock", Duration::from_secs(5));
    if mirador_alive(info, true) {
        return false;
    }
    open_app_window(&with_fit(url));
    let _ = mirador_call(info, "/api/opened", Some(&Value::obj()), Duration::from_secs(3)); // no second window while it loads
    true
}

/// `url` with where Corral is about to place its new window (`fit=0.5x0.85`, fractions of
/// the work area), so the page draws its plot at the size the window will have, once,
/// instead of at the size it opens with and again when it is placed. Unchanged when
/// windows aren't placed (corral config mirador_window free).
fn with_fit(url: &str) -> String {
    fit_url(url, window_place())
}

fn fit_url(url: &str, (place, w, h): (&str, f64, f64)) -> String {
    if place.is_empty() {
        return url.to_string();
    }
    let fit = format!("fit={w}x{h}");
    if url.contains('#') { format!("{url}:{fit}") } else { format!("{url}#{fit}") }
}

fn mirador_usage() -> &'static str {
    tr("usage: corral mirador [--open] [--url] [--stop] [--foreground]", "uso: corral mirador [--open] [--url] [--stop] [--foreground]")
}

/// `corral mirador [--open] [--url] [--stop] [--foreground]`.
pub fn cmd_mirador(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", mirador_usage());
        return Ok(());
    }
    let has = |f: &str| args.iter().any(|a| a == f);
    if let Some(bad) = args.iter().find(|a| !["--open", "--url", "--stop", "--foreground"].contains(&a.as_str())) {
        return Err(trf!("unknown option {bad}\n{}", "opción desconocida {bad}\n{}", mirador_usage()));
    }
    if has("--stop") {
        let info = mirador_info();
        if mirador_alive(&info, false) {
            env::kill(info.get("pid").num().unwrap_or(0.0) as i32, 15);
            println!("{}", tr("Mirador stopped", "Mirador detenido"));
        } else {
            println!("{}", tr("Mirador wasn't running", "Mirador no estaba corriendo"));
        }
        return Ok(());
    }
    if has("--foreground") {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        return Err(Command::new(me).arg("_mirador").exec().to_string());
    }
    let info = ensure_mirador()?;
    if has("--url") {
        println!("{}", info.get("url").str().unwrap_or(""));
        return Ok(());
    }
    if has("--open") {
        open_mirador_window(&info);
    } else if open_window_once(&info, info.get("url").str().unwrap_or("")) {
        bring_to_front("Mirador", true);
    }
    Ok(())
}

// ----------------------------------------------------------------------- the browser: the system's default

/// How a browser opens Mirador: Chromium-based ones as an app window (no tabs or address
/// bar), Firefox in a new window, any other as the system opens a link.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Chromium,
    Firefox,
    Other,
}

/// The kind of a browser by its program, app or desktop file name (msedge.exe,
/// com.google.Chrome, brave-browser.desktop, org.mozilla.firefox…).
fn kind_of(name: &str) -> Kind {
    let n = name.to_lowercase();
    if ["chrome", "msedge", "edgemac", "microsoft-edge", "brave", "chromium", "vivaldi"].iter().any(|k| n.contains(k)) {
        Kind::Chromium
    } else if n.contains("firefox") {
        Kind::Firefox
    } else {
        Kind::Other
    }
}

/// The browsers CORRAL_BROWSER can name, and where each lives on each system: on WSL its
/// Windows program, on macOS its app, on Linux its command.
const NAMED: &[(&str, &[&str], &[&str], &[&str])] = &[
    ("firefox", &["/mnt/c/Program Files/Mozilla Firefox/firefox.exe", "/mnt/c/Program Files (x86)/Mozilla Firefox/firefox.exe"],
     &["Firefox.app"], &["firefox"]),
    ("edge", &["/mnt/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "/mnt/c/Program Files/Microsoft/Edge/Application/msedge.exe"],
     &["Microsoft Edge.app"], &["microsoft-edge", "microsoft-edge-stable"]),
    ("chrome", &["/mnt/c/Program Files/Google/Chrome/Application/chrome.exe", "/mnt/c/Program Files (x86)/Google/Chrome/Application/chrome.exe"],
     &["Google Chrome.app"], &["google-chrome", "google-chrome-stable"]),
    ("brave", &["/mnt/c/Program Files/BraveSoftware/Brave-Browser/Application/brave.exe"],
     &["Brave Browser.app"], &["brave-browser", "brave"]),
    ("chromium", &[], &["Chromium.app"], &["chromium", "chromium-browser"]),
    ("safari", &[], &["Safari.app"], &[]),
];

/// The command that opens `url` in a browser of `kind` that runs as `program` (a Windows
/// .exe from WSL, a command on Linux) or, on macOS, as the app `app`.
fn open_with(kind: Kind, program: Option<&str>, app: Option<&str>, url: &str) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    match (app, program, kind) {
        // macOS: a new instance passes its arguments to the running one, which opens the window.
        (Some(app), _, Kind::Chromium) => s(&["open", "-na", app, "--args", &format!("--app={url}")]),
        (Some(app), _, _) => s(&["open", "-a", app, url]),
        (None, Some(p), Kind::Chromium) => s(&[p, &format!("--app={url}")]),
        (None, Some(p), Kind::Firefox) => s(&[p, "--new-window", url]),
        (None, Some(p), Kind::Other) => s(&[p, url]),
        (None, None, _) => s(&[if cfg!(target_os = "macos") { "open" } else { "xdg-open" }, url]),
    }
}

/// A macOS app by its name, in /Applications or ~/Applications.
fn mac_app(name: &str) -> Option<PathBuf> {
    [PathBuf::from("/Applications"), env::home().join("Applications")].into_iter().map(|d| d.join(name)).find(|p| p.is_dir())
}

fn wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
}

/// The browser CORRAL_BROWSER names, as installed on this system.
fn named_browser(name: &str, url: &str) -> Option<Vec<String>> {
    let (_, windows, apps, commands) = NAMED.iter().find(|(n, ..)| *n == name)?;
    let kind = kind_of(name);
    if wsl() {
        let exe = windows.iter().find(|p| Path::new(p).exists())?;
        return Some(open_with(kind, Some(exe), None, url));
    }
    if cfg!(target_os = "macos") {
        let app = apps.iter().find_map(|a| mac_app(a))?;
        return Some(open_with(kind, None, Some(&app.to_string_lossy()), url));
    }
    let cmd = commands.iter().find_map(|c| env::which(c))?;
    Some(open_with(kind, Some(&cmd.to_string_lossy()), None, url))
}

/// Windows' default browser, seen from WSL: the program its https handler runs, from the
/// registry (reg.exe answers in about 40 ms), as a path WSL can run.
fn windows_default() -> Option<(String, Kind)> {
    let reg = |args: &[&str]| -> Option<String> {
        let mut cmd = Command::new("reg.exe");
        cmd.args(args).current_dir("/mnt/c").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        env::windows_env(&mut cmd);
        let out = cmd.output().ok().filter(|o| o.status.success())?;
        Some(String::from_utf8_lossy(&out.stdout).replace('\r', ""))
    };
    let choice = reg(&["query", r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "/v", "ProgId"])?;
    let prog_id = choice.lines().find(|l| l.contains("ProgId"))?.split_whitespace().last()?.to_string();
    let command = reg(&["query", &format!(r"HKCR\{prog_id}\shell\open\command"), "/ve"])?;
    windows_program(command.lines().find(|l| l.contains("REG_SZ"))?)
}

/// The program at the start of a registry open command (`"C:\…\firefox.exe" -osint -url
/// "%1"`), as a WSL path (/mnt/c/…), and its kind.
fn windows_program(line: &str) -> Option<(String, Kind)> {
    let command = line.split_once("REG_SZ").map(|(_, c)| c.trim()).unwrap_or(line.trim());
    let exe = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    let (drive, rest) = exe.split_once(":\\")?;
    let path = format!("/mnt/{}/{}", drive.to_lowercase(), rest.replace('\\', "/"));
    let name = path.rsplit('/').next().unwrap_or("").to_string();
    Some((path, kind_of(&name)))
}

/// macOS's default browser: the app LaunchServices hands https links to, by its bundle id
/// (Safari when none was chosen).
fn mac_default() -> String {
    let plist = env::home().join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let json = Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(&plist).output().ok()
        .and_then(|o| json::parse(String::from_utf8_lossy(&o.stdout).trim()).ok()).unwrap_or(Value::Null);
    mac_default_in(&json)
}

fn mac_default_in(plist: &Value) -> String {
    plist.get("LSHandlers").arr().iter().find(|h| h.get("LSHandlerURLScheme").str() == Some("https"))
        .and_then(|h| h.get("LSHandlerRoleAll").str()).unwrap_or("com.apple.safari").to_string()
}

/// Linux's default browser, as xdg-settings names it (firefox.desktop).
fn linux_default() -> Option<String> {
    let out = Command::new(env::which("xdg-settings")?).args(["get", "default-web-browser"]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// The command that opens `url`, or None when it should only be printed. Mirador opens in
/// the system's default browser, on WSL (Windows'), macOS and Linux alike: as an app
/// window when that browser is Chromium-based, in a new window with Firefox.
/// CORRAL_BROWSER: firefox | edge | chrome | brave | chromium | safari | print | a command
/// (`{url}` is replaced).
pub fn browser_argv(url: &str) -> Option<Vec<String>> {
    let choice = std::env::var("CORRAL_BROWSER").unwrap_or_default().trim().to_string();
    if choice == "print" {
        return None;
    }
    if NAMED.iter().any(|(n, ..)| *n == choice) {
        if let Some(argv) = named_browser(&choice, url) {
            return Some(argv);
        }
    } else if !choice.is_empty() {
        let mut argv: Vec<String> = env::sh_split(&choice).into_iter().map(|a| a.replace("{url}", url)).collect();
        if !choice.contains("{url}") {
            argv.push(url.to_string());
        }
        return Some(argv);
    }
    if wsl() {
        // The .exe directly, so cmd.exe never parses the '&' in httpgd's token URL.
        let (exe, kind) = windows_default().filter(|(p, _)| Path::new(p).exists())
            .or_else(|| ["firefox", "edge", "chrome"].iter().find_map(|n| {
                let (_, windows, ..) = NAMED.iter().find(|(m, ..)| m == n)?;
                windows.iter().find(|p| Path::new(p).exists()).map(|p| (p.to_string(), kind_of(n)))
            }))?;
        return Some(open_with(kind, Some(&exe), None, url));
    }
    if cfg!(target_os = "macos") {
        let id = mac_default();
        if kind_of(&id) == Kind::Chromium {
            return Some(vec!["open".into(), "-nb".into(), id, "--args".into(), format!("--app={url}")]);
        }
        return Some(open_with(Kind::Other, None, None, url));
    }
    if let Some(desktop) = linux_default().filter(|d| kind_of(d) == Kind::Chromium) {
        let name = NAMED.iter().find(|(n, ..)| desktop.to_lowercase().contains(&n.replace("edge", "microsoft-edge")));
        if let Some(cmd) = name.and_then(|(_, _, _, cmds)| cmds.iter().find_map(|c| env::which(c))) {
            return Some(open_with(Kind::Chromium, Some(&cmd.to_string_lossy()), None, url));
        }
    }
    env::which("xdg-open").map(|_| open_with(Kind::Other, None, None, url))
}

pub fn open_app_window(url: &str) {
    let Some(argv) = browser_argv(url) else {
        println!("{url}");
        return;
    };
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if argv[0].starts_with("/mnt/") {
        cmd.current_dir("/mnt/c");
    }
    env::windows_env(&mut cmd);
    let _ = env::spawn_detached(&mut cmd, None);
}

/// Where a new Mirador window goes: corral config mirador_window right|free, mirador_size 0.5x0.85.
pub fn window_place() -> (&'static str, f64, f64) {
    let cfg = env::config();
    // "derecha" is the value from before.
    let place = if matches!(cfg.get("mirador_window").str().unwrap_or("right"), "right" | "derecha") { "right" } else { "" };
    let size = cfg.get("mirador_size").str().unwrap_or("0.5x0.85").to_lowercase();
    let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?))).unwrap_or((0.5, 0.85));
    (place, w.clamp(0.2, 1.0), h.clamp(0.2, 1.0))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The PowerShell command that raises (and with `place`, places) a Windows window
/// whose title starts with `title` (lib/front.ps1). None outside WSL.
pub fn front_command(title: &str, wait: f64, extra: &str) -> Option<Command> {
    if std::env::var_os("WSL_DISTRO_NAME").is_none() || env::which("powershell.exe").is_none() {
        return None;
    }
    let script = std::fs::read_to_string(env::repo().join("lib/front.ps1")).ok()?;
    let command = format!("& {{ {script} }} -Prefix '{title}' -WaitSeconds {wait}{extra}");
    let utf16: Vec<u8> = command.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &base64(&utf16)]).current_dir("/mnt/c");
    env::windows_env(&mut cmd);
    Some(cmd)
}

/// Windows opens windows from background processes behind the active one: wait
/// for Mirador's window and bring it forward (WSL only; best effort). With
/// `place` (new windows), also un-maximize it and put it on the right.
pub fn bring_to_front(title: &str, place: bool) {
    if std::env::var("CORRAL_BROWSER").as_deref() == Ok("print") {
        return; // no windows are being opened, so none to raise
    }
    let (where_, w, h) = window_place();
    let placing = place && !where_.is_empty();
    let extra = if placing { format!(" -Place {where_} -Width {w} -Height {h}") } else { String::new() };
    let place_arg = if placing { where_ } else { "" };
    let cmd = front_command(title, 8.0, &extra).or_else(|| {
        if cfg!(target_os = "macos") {
            let mut c = Command::new("osascript");
            c.args(["-l", "JavaScript", "-e", MAC_FRONT, title, "8", place_arg, &w.to_string(), &h.to_string()]);
            Some(c)
        } else if std::env::var_os("DISPLAY").is_some() && env::which("wmctrl").is_some() {
            let mut c = Command::new("sh");
            c.args(["-c", LINUX_FRONT, "sh", title, "8", place_arg, &w.to_string(), &h.to_string()]);
            Some(c)
        } else {
            None
        }
    });
    if let Some(mut cmd) = cmd {
        let _ = env::spawn_detached(&mut cmd, None);
    }
}

/// lib/front.ps1's job on macOS, run by osascript: wait up to WAIT seconds for a window
/// whose title starts with PREFIX, in any app (System Events), and with PLACE "right" put
/// it on the right of the screen's visible area (W x H, fractions of it), as on WSL; then
/// raise it and its app. It needs the terminal to be allowed to control the computer
/// (System Settings → Privacy & Security → Accessibility); without that it does nothing.
/// Arguments: PREFIX WAIT PLACE W H.
const MAC_FRONT: &str = r#"function run(argv) {
  const [prefix, wait, place, fw, fh] = argv;
  try {
    ObjC.import('AppKit');
    const se = Application('System Events');
    let win = null, proc = null;
    const end = Date.now() + parseFloat(wait) * 1000;
    while (!win && Date.now() < end) {
      for (const p of se.processes.whose({ backgroundOnly: false })()) {
        try {
          const ws = p.windows.whose({ name: { _beginsWith: prefix } })();
          if (ws.length) { win = ws[0]; proc = p; break; }
        } catch (e) {}
      }
      if (!win) delay(0.2);
    }
    if (!win) return 'not found';
    if (place === 'right') {
      const screen = $.NSScreen.mainScreen, full = screen.frame, vis = screen.visibleFrame;
      const aw = vis.size.width, ah = vis.size.height;
      const w = Math.round(aw * parseFloat(fw)), h = Math.round(ah * parseFloat(fh));
      const margin = Math.round(aw * 0.012);
      // System Events counts from the top left; AppKit, from the bottom left.
      const top = full.size.height - (vis.origin.y + vis.size.height);
      win.position = [Math.round(vis.origin.x + aw - w - margin), Math.round(top + (ah - h) / 2)];
      win.size = [w, h];
    }
    try { win.actions['AXRaise'].perform(); } catch (e) {}
    proc.frontmost = true;
    return 'ok';
  } catch (e) {
    return 'not allowed: ' + e;
  }
}"#;

/// lib/front.ps1's job on Linux with an X11 window manager, through wmctrl: wait up to
/// WAIT seconds for a window whose title starts with PREFIX, with PLACE "right" put it on
/// the right of the work area (W x H, fractions of it), then raise it. Wayland doesn't
/// let a program move another's windows, so there it does nothing.
/// Arguments: PREFIX WAIT PLACE W H.
const LINUX_FRONT: &str = r#"prefix=$1 wait=$2 place=$3 fw=$4 fh=$5
tries=$(awk -v w="$wait" 'BEGIN { print int(w * 5) }')
id=""
while [ "$tries" -gt 0 ]; do
  id=$(wmctrl -l 2>/dev/null | awk -v p="$prefix" '{ t = $0; for (k = 1; k <= 3; k++) sub(/^[^ ]+ +/, "", t); if (index(t, p) == 1) { print $1; exit } }')
  [ -n "$id" ] && break
  sleep 0.2
  tries=$((tries - 1))
done
[ -n "$id" ] || exit 0
if [ "$place" = right ]; then
  geometry=$(wmctrl -d 2>/dev/null | awk -v fw="$fw" -v fh="$fh" '$2 == "*" { for (k = 1; k <= NF; k++) if ($k == "WA:") { split($(k+1), o, ","); split($(k+2), s, "x");
    w = int(s[1] * fw); h = int(s[2] * fh); m = int(s[1] * 0.012); print "0," o[1] + s[1] - w - m "," o[2] + int((s[2] - h) / 2) "," w "," h; exit } }')
  if [ -n "$geometry" ]; then
    wmctrl -i -r "$id" -b remove,maximized_vert,maximized_horz
    wmctrl -i -r "$id" -e "$geometry"
  fi
fi
wmctrl -i -a "$id""#;

// ----------------------------------------------------------------------- showing a plot

/// Show one plot sharply: a herdr popup when the terminal draws images, else Mirador.
fn open_plot(dir: &Path, id: &str) -> Result<&'static str, String> {
    if image_mode().0 == "kitty" && env::in_herdr() {
        open_popup("plots", &[("CORRAL_SESSION_DIR", &dir.to_string_lossy()), ("CORRAL_PLOT_ID", id)])?;
        return Ok("herdr");
    }
    let info = ensure_mirador()?;
    mirador_call(&info, "/api/show-plot", Some(&obj(vec![("session", sid(dir).into()), ("id", id.into())])), Duration::from_secs(3))?;
    let opened = open_window_once(&info, &format!("{}#plot:{}:{id}", info.get("url").str().unwrap_or(""), sid(dir)));
    // Windows leaves windows opened from the background behind the active one.
    bring_to_front("Mirador", opened);
    Ok("Mirador")
}

/// Open Mirador at the newest plot; wait a little for one to exist (R draws after opening the device).
fn show_latest_plot(dir: &Path, wait: f64) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs_f64(wait);
    loop {
        if httpgd_state(dir).get("hsize").num().unwrap_or(0.0) > 0.0 {
            if let Some(last) = httpgd_ids(dir).last() {
                open_plot(dir, last)?;
                return Ok(());
            }
        }
        if Instant::now() >= end {
            env::notify(tr("No plots", "Sin gráficos"), tr("No plots in this R session yet", "Todavía no hay gráficos en esta sesión de R"));
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Show a plot object (ggplot, lattice, ...): reuse its drawing if it has one, else draw it.
pub fn show_plot_object(dir: &Path, name: &str) -> String {
    let meta = env::read_json(&dir.join("meta.json"));
    let vars = env::read_json(&dir.join("vars.json"));
    let Some(pane) = meta.get("pane_id").str().map(String::from) else { return tr("the session isn't in a herdr pane", "la sesión no está en un panel de herdr").into() };
    let fingerprint = vars.get("vars").arr().iter().find(|v| v.get("name").str() == Some(name))
        .and_then(|v| v.get("hash").str().map(String::from));
    let map_file = dir.join("plotobj-map.json");
    let mut drawn = env::read_json(&map_file);
    if let Some(f) = &fingerprint {
        if let Some(id) = drawn.get(f).str().map(String::from) {
            if httpgd_ids(dir).contains(&id) {
                return match open_plot(dir, &id) {
                    Ok(_) => tr("already drawn; opened in Mirador", "ya estaba dibujado; abierto en Mirador").into(),
                    Err(e) => e,
                };
            }
        }
    }
    let before = httpgd_state(dir);
    let old = (before.get("hsize").num().unwrap_or(0.0), before.get("upid").clone());
    let code = if meta.get("lang").str() == Some("python") {
        format!("_corral_show({name})")
    } else {
        format!("print(`{}`)", name.replace('`', "\\`"))
    };
    if herdr::run(&pane, &code).is_err() {
        return tr("couldn't send it to the console", "no pude enviar a la consola").into();
    }
    // Mirador's window opens while R draws, not after it: a new window takes about as long
    // as a drawing. The page starts on this session's newest plot and goes to this one
    // when it is there.
    if !(image_mode().0 == "kitty" && env::in_herdr()) {
        if let Ok(info) = ensure_mirador() {
            if open_window_once(&info, &format!("{}#plot:{}:", info.get("url").str().unwrap_or(""), sid(dir))) {
                bring_to_front("Mirador", true);
            }
        }
    }
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(250));
        let state = httpgd_state(dir);
        let now = (state.get("hsize").num().unwrap_or(0.0), state.get("upid").clone());
        if now != old && now.0 > 0.0 {
            if let (Some(f), Some(last)) = (&fingerprint, httpgd_ids(dir).last()) {
                drawn.set(f, last.as_str().into());
                env::write_json(&map_file, &drawn);
            }
            return match show_latest_plot(dir, 0.0) {
                Ok(()) => tr("drawn and opened in Mirador", "dibujado y abierto en Mirador").into(),
                Err(e) => e,
            };
        }
    }
    tr("R didn't draw anything (still busy?)", "R no dibujó nada (¿sigue ocupado?)").into()
}

fn session_dir() -> Option<PathBuf> {
    console::pick_session().and_then(|s| s.get("dir").str().map(PathBuf::from))
}

/// `corral plots [--inline]` (ctrl+b f): Mirador at the latest plot, or the
/// in-terminal viewer on a terminal that draws images.
pub fn cmd_plots(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral plots [--inline]", "uso: corral plots [--inline]");
    if env::wants_help(args) {
        println!("{usage}");
        return Ok(());
    }
    let inline = args.iter().any(|a| a == "--inline");
    if let Some(bad) = args.iter().find(|a| *a != "--inline") {
        return Err(trf!("unknown option {bad}\n{usage}", "opción desconocida {bad}\n{usage}"));
    }
    if inline || image_mode().0 == "kitty" {
        let dir = session_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        if env::in_herdr() && !inline {
            return open_popup("plots", &[("CORRAL_SESSION_DIR", &dir)]);
        }
        std::env::set_var("CORRAL_SESSION_DIR", &dir);
        return cmd_plots_tui(&[]);
    }
    let dir = session_dir().ok_or(tr("no R session with Corral", "no hay una sesión de R con Corral"))?;
    show_latest_plot(&dir, 0.0)
}

/// `corral _plot-ready`: the first plot of a session (called by corral.R and Python).
pub fn cmd_plot_ready(_args: &[String]) -> Result<(), String> {
    if image_mode().0 == "kitty" {
        env::notify(tr("Plot ready", "Gráfico listo"), tr("ctrl+b f to see it", "ctrl+b f para verlo"));
        return Ok(());
    }
    match session_dir() {
        Some(dir) => show_latest_plot(&dir, 6.0),
        None => Ok(()),
    }
}

/// `corral _show-object SESSION NAME`.
pub fn cmd_show_object(args: &[String]) -> Result<(), String> {
    let (Some(dir), Some(name)) = (args.first(), args.get(1)) else { return Err(tr("usage: corral _show-object SESSION NAME", "uso: corral _show-object SESIÓN NOMBRE").into()) };
    println!("{}", show_plot_object(Path::new(dir), name));
    Ok(())
}

// ----------------------------------------------------------------------- tables and links

fn view_usage() -> &'static str {
    tr("usage: corral view FILE [--title TITLE] [--mirador] [--inline] [--session SESSION]",
       "uso: corral view ARCHIVO [--title TÍTULO] [--mirador] [--inline] [--session SESIÓN]")
}

// View() in R or Python saves each table to a temporary file, and a day's work makes
// many of them: a new View() of the same object replaces the previous file, and each
// session keeps its latest KEEP_VIEWS, up to KEEP_VIEW_BYTES in all (the newest one
// always stays). Only files `corral view` gets with --temp, which View() passes, are
// ever deleted; a file of yours never is. Mirador drops a table whose file is gone.
const KEEP_VIEWS: usize = 20;
const KEEP_VIEW_BYTES: u64 = 2 << 30;

/// A session's View() file: title, path and size.
type ViewFile = (String, String, u64);

/// Given a session's View() files (oldest first) and the new one: the files to
/// keep, the new one last, and the paths to delete.
fn prune_views(mut kept: Vec<ViewFile>, new: ViewFile) -> (Vec<ViewFile>, Vec<String>) {
    let mut gone = Vec::new();
    kept.retain(|(title, path, _)| {
        if *path == new.1 {
            false
        } else if *title == new.0 {
            gone.push(path.clone());
            false
        } else {
            true
        }
    });
    kept.push(new);
    while kept.len() > 1 && (kept.len() > KEEP_VIEWS || kept.iter().map(|v| v.2).sum::<u64>() > KEEP_VIEW_BYTES) {
        gone.push(kept.remove(0).1);
    }
    (kept, gone)
}

/// A file View() wrote: R's tempfile("corral-…"), or a file in Python's corral-py-… folder.
fn own_copy(p: &Path) -> bool {
    let name = |q: Option<&Path>| q.and_then(|q| q.file_name()).and_then(|n| n.to_str()).unwrap_or("").to_string();
    name(Some(p)).starts_with("corral-") || name(p.parent()).starts_with("corral-py-")
}

/// Record this View() file in its session's views.json and delete the files it makes stale.
fn tidy_views(session: &str, title: &str, path: &Path) {
    let plain = !session.is_empty() && !session.starts_with('.')
        && session.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let dir = env::state_dir().join("sessions").join(session);
    if !plain || !dir.is_dir() {
        return;
    }
    let list = dir.join("views.json");
    let kept: Vec<ViewFile> = env::read_json(&list).arr().iter().map(|v| (
        v.get("title").str().unwrap_or("").to_string(),
        v.get("path").str().unwrap_or("").to_string(),
        v.get("size").num().unwrap_or(0.0) as u64,
    )).collect();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let (kept, gone) = prune_views(kept, (title.to_string(), path.to_string_lossy().into_owned(), size));
    for p in gone {
        if own_copy(Path::new(&p)) && Path::new(&p).is_file() {
            let _ = std::fs::remove_file(&p);
        }
    }
    let rows = kept.into_iter().map(|(t, p, s)| obj(vec![("title", t.into()), ("path", p.into()), ("size", (s as f64).into())])).collect();
    env::write_json(&list, &Value::Arr(rows));
}

/// `corral view FILE`: a table (parquet, csv, …) in a herdr popup, in Mirador, or here.
/// The terminal viewer is the default; Mirador only when asked (`--mirador`, the
/// `m` key in the viewer or the variables pane, `View(x, mirador = TRUE)`).
/// `--temp` (from View()): the file is Corral's own copy, see tidy_views.
pub fn cmd_view(args: &[String]) -> Result<(), String> {
    let (mut file, mut title, mut inline, mut session, mut mirador) = (None::<String>, None::<String>, false, None::<String>, false);
    let mut temp = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--title" => title = it.next().cloned(),
            "--session" => session = it.next().cloned(),
            "--inline" => inline = true,
            "--mirador" => mirador = true,
            "--temp" => temp = true,
            "-h" | "--help" => {
                println!("{}", view_usage());
                return Ok(());
            }
            s if s.starts_with("--") => return Err(trf!("unknown option {s}\n{}", "opción desconocida {s}\n{}", view_usage())),
            _ => file = Some(a.clone()),
        }
    }
    let file = file.ok_or(view_usage())?;
    let path = std::fs::canonicalize(&file).map_err(|_| trf!("{file} doesn't exist", "no existe {file}"))?;
    if !path.is_file() {
        return Err(trf!("{} doesn't exist", "no existe {}", path.display()));
    }
    let title = title.unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let shown = path.to_string_lossy().into_owned();
    if let (true, Some(s)) = (temp, &session) {
        tidy_views(s, &title, &path);
    }

    // auto: inside herdr everything opens in herdr; elsewhere Mirador if it is running.
    let target = std::env::var("CORRAL_VIEW").unwrap_or_else(|_| "auto".into());
    let use_mirador = mirador || target == "mirador" || (target == "auto" && !env::in_herdr() && mirador_alive(&mirador_info(), false));
    if !inline && use_mirador {
        return table_to_mirador(&shown, &title, session.as_deref());
    }
    if env::in_herdr() && !inline {
        let mut vars = vec![("CORRAL_FILE", shown.as_str()), ("CORRAL_TITLE", title.as_str())];
        if let Some(s) = &session {
            vars.push(("CORRAL_SESSION", s.as_str()));
        }
        match open_popup("table", &vars) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("corral: {}", trf!("popup unavailable ({e}); opening a pane", "popup no disponible ({e}); abro un panel")),
        }
        // Plugin not linked or popup unavailable: a split pane.
        let here = env::current_pane();
        let mut split = vec!["pane".to_string(), "split".to_string()];
        split.push(here.clone().unwrap_or_else(|| "--current".into()));
        split.extend(["--direction", "right", "--ratio", "0.4", "--focus"].map(String::from));
        let args: Vec<&str> = split.iter().map(String::as_str).collect();
        let new = herdr::cli(&args)?.get("pane").get("pane_id").str().ok_or(tr("herdr didn't create the pane", "herdr no creó el panel"))?.to_string();
        herdr::rename(&new, &trf!("table: {title}", "tabla: {title}"))?;
        let viewer = format!("CORRAL_TITLE={} CORRAL_SESSION={} {} _table {}; exit", env::sh_quote(&title),
                             env::sh_quote(session.as_deref().unwrap_or("")), console::program(), env::sh_quote(&shown));
        return herdr::start(&new, &viewer);
    }
    std::env::set_var("CORRAL_TITLE", &title);
    if let Some(s) = &session {
        std::env::set_var("CORRAL_SESSION", s);
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    Err(Command::new(me).arg("_table").arg(&path).exec().to_string())
}

/// A table file in Mirador: it opens in its own tab, and the browser window
/// opens too when no Mirador page is showing. The same file the terminal viewer
/// reads, so nothing is copied.
pub fn table_to_mirador(path: &str, title: &str, session: Option<&str>) -> Result<(), String> {
    let info = ensure_mirador()?;
    let body = obj(vec![("path", path.into()), ("title", title.into()),
                        ("session", session.filter(|s| !s.is_empty()).map(Value::from).unwrap_or(Value::Null))]);
    mirador_call(&info, "/api/tables", Some(&body), Duration::from_secs(10))?;
    if open_window_once(&info, info.get("url").str().unwrap_or("")) {
        bring_to_front("Mirador", true);
    }
    Ok(())
}

/// `corral open [URL]`: a local URL (quarto preview, httpgd, a clicked link) in
/// the browser; without one, Mirador.
pub fn cmd_open(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", tr("usage: corral open [URL]", "uso: corral open [URL]"));
        return Ok(());
    }
    let url = args.first().cloned().or_else(|| std::env::var("HERDR_PLUGIN_CLICKED_URL").ok().filter(|u| !u.is_empty()));
    match url {
        Some(u) => {
            open_app_window(&u);
            Ok(())
        }
        None => {
            open_mirador_window(&ensure_mirador()?);
            Ok(())
        }
    }
}

// ----------------------------------------------------------------------- the in-terminal viewer

fn fg(p: &Value, key: &str, fallback: &str) -> String {
    let [r, g, b] = crate::theme::rgb(p.get(key).str().unwrap_or(fallback));
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// `corral _plots`: the plots of CORRAL_SESSION_DIR, drawn with chafa (Kitty
/// graphics on terminals that support it, else block symbols); ←/→ history,
/// o opens Mirador, q quits.
pub fn cmd_plots_tui(_args: &[String]) -> Result<(), String> {
    let Some(dir) = std::env::var_os("CORRAL_SESSION_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) else {
        message_screen(tr("No R session with Corral", "No hay una sesión de R con Corral"),
                       tr("Open R in a herdr pane (with corral.R in your .Rprofile)\nor press ctrl+b a to set up the workspace.",
                          "Abre R en un panel de herdr (con corral.R en tu .Rprofile)\no usa ctrl+b a para armar el espacio de trabajo."));
        return Ok(());
    };
    if !dir.join("plots-url").is_file() {
        message_screen(tr("No plots yet", "Todavía no hay gráficos"),
                       tr("Draw something in the R console, for example plot(1:10),\nthen open this viewer again with ctrl+b f.",
                          "Dibuja algo en la consola de R, por ejemplo plot(1:10),\ny vuelve a abrir este visor con ctrl+b f."));
        return Ok(());
    }
    if env::which("chafa").is_none() {
        message_screen(tr("chafa is missing", "Falta chafa"), tr("Install it with: sudo apt install chafa", "Instálalo con: sudo apt install chafa"));
        return Ok(());
    }
    let kitty = image_mode().0 == "kitty";
    let start_id = std::env::var("CORRAL_PLOT_ID").ok().filter(|s| !s.is_empty());
    let session = sid(&dir);
    let tmp = dir.join("view.png");
    let p = env::read_json(&crate::theme::palette_file());
    let (accent, text, muted, dim, green) = (fg(&p, "accent", "#e07b53"), fg(&p, "text", "#e9e9e9"), fg(&p, "muted", "#9b9b9b"),
                                             fg(&p, "dim", "#646464"), fg(&p, "green", "#4fc27c"));
    const RESET: &str = "\x1b[0m";

    // The kept plots of a session that crashed come first, then this session's own.
    let mut ids = crate::mirador::shown_plot_ids(&session);
    if ids.is_empty() && httpgd_state(&dir).is_null() {
        message_screen(tr("httpgd isn't responding", "httpgd no responde"),
                       tr("Did you close R or the graphics device?\nDraw again and reopen the viewer.",
                          "¿Cerraste R o el dispositivo gráfico?\nDibuja de nuevo y vuelve a abrir el visor."));
        return Ok(());
    }
    let mut upid = httpgd_state(&dir).get("upid").clone();
    let mut idx: isize = ids.len() as isize - 1;
    let mut follow = true;
    if let Some(start) = &start_id {
        if let Some(i) = ids.iter().position(|x| x == start) {
            idx = i as isize;
            follow = i + 1 == ids.len();
        }
    }
    let status = String::new();
    let render = |ids: &[String], idx: isize, follow: bool, status: &str| {
        let (cols, rows) = term::size();
        let (xpx, ypx) = term::pixels();
        let area = rows.saturating_sub(2).max(4);
        out(&format!("{}\x1b[2J\x1b[H", if kitty { "\x1b_Ga=d,q=2\x1b\\" } else { "" }));
        if ids.is_empty() {
            out(&format!("\x1b[3;3H{muted}{}{RESET}", tr("No plots in this session. Draw something in R.", "No hay gráficos en esta sesión. Dibuja algo en R.")));
        } else {
            let (w, h, zoom) = if kitty {
                // Real pixels when the terminal reports them; otherwise ~9x19 px per cell.
                if xpx > 0 && ypx > 0 { (xpx, ypx * area / rows.max(1), 1.0) } else { (cols * 9, area * 19, 1.0) }
            } else {
                // About 10x20 px per cell; zoom makes labels readable at character resolution.
                (cols * 10, area * 20, 1.6)
            };
            let params = [("width", w.to_string()), ("height", h.to_string()),
                          ("renderer", "png".to_string()), ("zoom", zoom.to_string())];
            match crate::mirador::shown_plot(&session, &ids[idx as usize], &params) {
                Some((png, _)) if std::fs::write(&tmp, &png).is_ok() => {
                    let mut chafa = Command::new("chafa");
                    if kitty {
                        chafa.args(["-f", "kitty"]);
                    } else {
                        // Block elements only: every terminal font has them (sextants are often missing).
                        chafa.args(["-f", "symbols", "-c", "full", "--symbols", "block+border+space", "--color-space", "din99d", "-w", "9"]);
                    }
                    chafa.args(["--size", &format!("{cols}x{area}"), "--animate", "off"]).arg(&tmp);
                    match chafa.output() {
                        Ok(o) => {
                            let _ = std::io::stdout().write_all(&o.stdout);
                        }
                        Err(e) => out(&format!("\x1b[3;3H{accent}{}{RESET}", trf!("Couldn't draw the plot: {e}", "No pude dibujar el gráfico: {e}"))),
                    }
                }
                _ => out(&format!("\x1b[3;3H{accent}{}{RESET}", tr("Couldn't get the plot from httpgd", "No pude traer el gráfico de httpgd"))),
            }
        }
        let pos = if ids.is_empty() { "0/0".to_string() } else { format!("{}/{}", idx + 1, ids.len()) };
        let live = if follow { format!("{green}●{RESET} ") } else { String::new() };
        let hint = tr("←/→ history · o open in the browser · q quit", "←/→ historial · o abrir en el navegador · q salir");
        let plot = tr("Plot", "Gráfico");
        let mut footer = format!(" {accent}◔{RESET} {text}{plot} {pos}{RESET}  {live}{dim}{}{RESET}", clip(hint, cols.saturating_sub(20)));
        if !status.is_empty() {
            footer += &format!("  {muted}{status}{RESET}");
        }
        out(&format!("\x1b[{rows};1H\x1b[2K{footer}"));
    };

    let Some(scr) = Screen::enter(false) else { return Err(tr("no terminal", "no hay una terminal").into()) };
    let mut size = scr.size();
    render(&ids, idx, follow, &status);
    let mut last_poll = Instant::now();
    loop {
        if scr.size() != size {
            size = scr.size();
            render(&ids, idx, follow, &status);
        }
        let key = scr.key(300);
        match key.as_deref() {
            Some("q" | "esc" | "quit") => return Ok(()),
            Some("left" | "h") if idx > 0 => {
                idx -= 1;
                follow = false;
                render(&ids, idx, follow, &status);
            }
            Some("right" | "l") if idx + 1 < ids.len() as isize => {
                idx += 1;
                follow = idx + 1 == ids.len() as isize;
                render(&ids, idx, follow, &status);
            }
            Some("end") if !ids.is_empty() => {
                idx = ids.len() as isize - 1;
                follow = true;
                render(&ids, idx, follow, &status);
            }
            Some("o") => {
                render(&ids, idx, follow, tr("opening…", "abriendo…"));
                if let Ok(info) = ensure_mirador() {
                    open_mirador_window(&info);
                }
                render(&ids, idx, follow, "");
            }
            None if last_poll.elapsed() > Duration::from_secs(1) => {
                last_poll = Instant::now();
                let state = httpgd_state(&dir);
                if !state.is_null() && state.get("upid") != &upid {
                    upid = state.get("upid").clone();
                    ids = crate::mirador::shown_plot_ids(&session);
                    if follow || idx < 0 || idx >= ids.len() as isize {
                        idx = ids.len() as isize - 1;
                    }
                    render(&ids, idx, follow, &status);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_is_known_by_its_name() {
        for name in ["chrome.exe", "msedge.exe", "com.google.chrome", "com.microsoft.edgemac", "com.brave.Browser",
                     "google-chrome.desktop", "microsoft-edge.desktop", "chromium-browser.desktop", "brave", "chromium"] {
            assert_eq!(kind_of(name), Kind::Chromium, "{name}");
        }
        for name in ["firefox.exe", "org.mozilla.firefox", "firefox.desktop"] {
            assert_eq!(kind_of(name), Kind::Firefox, "{name}");
        }
        for name in ["com.apple.safari", "company.thebrowser.browser", "epiphany.desktop"] {
            assert_eq!(kind_of(name), Kind::Other, "{name}");
        }
    }

    /// Windows' default browser comes from its open command in the registry, as reg.exe prints it.
    #[test]
    fn windows_default_browser_from_the_registry() {
        let line = r#"    (Predeterminado)    REG_SZ    "C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url "%1""#;
        assert_eq!(windows_program(line), Some(("/mnt/c/Program Files/Mozilla Firefox/firefox.exe".into(), Kind::Firefox)));
        let edge = r#"    (Default)    REG_SZ    "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --single-argument %1"#;
        assert_eq!(windows_program(edge), Some(("/mnt/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe".into(), Kind::Chromium)));
        let bare = r#"    (Default)    REG_SZ    D:\Apps\Browser\browser.exe %1"#;
        assert_eq!(windows_program(bare), Some(("/mnt/d/Apps/Browser/browser.exe".into(), Kind::Other)));
        assert_eq!(windows_program("nothing here"), None);
    }

    /// macOS's default browser comes from LaunchServices' handler for https; Safari otherwise.
    #[test]
    fn macos_default_browser_from_launchservices() {
        let plist = json::parse(r#"{"LSHandlers":[{"LSHandlerURLScheme":"mailto","LSHandlerRoleAll":"com.apple.mail"},
            {"LSHandlerURLScheme":"https","LSHandlerRoleAll":"org.mozilla.firefox"}]}"#).unwrap();
        assert_eq!(mac_default_in(&plist), "org.mozilla.firefox");
        assert_eq!(mac_default_in(&Value::Null), "com.apple.safari");
    }

    /// Chromium-based browsers open Mirador as an app window, Firefox in a new window, and
    /// the rest as the system opens a link.
    #[test]
    fn each_kind_of_browser_opens_its_way() {
        let url = "http://127.0.0.1:9/?token=a&b=c";
        assert_eq!(open_with(Kind::Chromium, Some("/mnt/c/x/msedge.exe"), None, url), ["/mnt/c/x/msedge.exe", &format!("--app={url}")]);
        assert_eq!(open_with(Kind::Firefox, Some("/usr/bin/firefox"), None, url), ["/usr/bin/firefox", "--new-window", url]);
        assert_eq!(open_with(Kind::Chromium, None, Some("/Applications/Google Chrome.app"), url),
                   ["open", "-na", "/Applications/Google Chrome.app", "--args", &format!("--app={url}")]);
        assert_eq!(open_with(Kind::Other, None, Some("/Applications/Safari.app"), url), ["open", "-a", "/Applications/Safari.app", url]);
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        assert_eq!(open_with(Kind::Other, None, None, url), [opener, url]);
    }

    /// On Linux with X11, wmctrl finds the window by its title, places it on the right of
    /// the work area as on WSL, and raises it. Checked with a stand-in wmctrl.
    #[test]
    fn linux_places_and_raises_the_window_with_wmctrl() {
        let dir = std::env::temp_dir().join(format!("corral-test-wmctrl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        std::fs::write(dir.join("wmctrl"), format!(r#"#!/bin/sh
case "$1" in
  -l) printf '0x01  0 host Corral notes\n0x02  0 host Mirador — plots\n' ;;
  -d) printf '0  * DG: 1920x1080  VP: 0,0  WA: 0,27 1920x1053  Workspace 1\n' ;;
  *) echo "$*" >> {} ;;
esac
"#, log.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("wmctrl"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.display());
        let status = Command::new("sh").args(["-c", LINUX_FRONT, "sh", "Mirador", "1", "right", "0.5", "0.85"])
            .env("PATH", &path).status().unwrap();
        assert!(status.success());
        let calls = std::fs::read_to_string(&log).unwrap();
        // 960 x 895 at 1920 - 960 - 23 = 937, 27 + (1053 - 895) / 2 = 106.
        assert_eq!(calls, "-i -r 0x02 -b remove,maximized_vert,maximized_horz\n-i -r 0x02 -e 0,937,106,960,895\n-i -a 0x02\n");
        // Without placing, only raised.
        std::fs::remove_file(&log).unwrap();
        Command::new("sh").args(["-c", LINUX_FRONT, "sh", "Mirador", "1", "", "0.5", "0.85"]).env("PATH", &path).status().unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-i -a 0x02\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On macOS the script that places and raises the window runs (osascript compiles it)
    /// and gives up when no window matches, or when the terminal may not control others.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_front_script_runs() {
        let mut child = Command::new("osascript").args(["-l", "JavaScript", "-e", MAC_FRONT, "NoSuchCorralWindow", "0", "", "0.5", "0.85"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let end = Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > end {
                let _ = child.kill();
                panic!("osascript didn't finish in 60 s");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let out = child.wait_with_output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.starts_with("not found") || said.starts_with("not allowed"), "{said} {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A new window's address says where Corral will place it, after the plot it shows.
    #[test]
    fn a_new_window_says_where_it_goes() {
        let base = "http://127.0.0.1:7317/?token=t";
        assert_eq!(fit_url(&format!("{base}#plot:w1-p2-9:3"), ("right", 0.5, 0.85)), format!("{base}#plot:w1-p2-9:3:fit=0.5x0.85"));
        assert_eq!(fit_url(&format!("{base}#plot:w1-p2-9:"), ("right", 0.5, 0.85)), format!("{base}#plot:w1-p2-9::fit=0.5x0.85"));
        assert_eq!(fit_url(base, ("right", 0.42, 0.8)), format!("{base}#fit=0.42x0.8"));
        // Not placed (mirador_window free): the address as it was.
        assert_eq!(fit_url(base, ("", 0.5, 0.85)), base);
    }

    fn v(title: &str, path: &str, size: u64) -> ViewFile {
        (title.into(), path.into(), size)
    }

    #[test]
    fn a_new_view_of_the_same_object_replaces_its_file() {
        let (kept, gone) = prune_views(vec![v("sales", "/t/corral-1.parquet", 10), v("df", "/t/corral-2.parquet", 10)],
                                       v("sales", "/t/corral-3.parquet", 10));
        assert_eq!(gone, vec!["/t/corral-1.parquet"]);
        assert_eq!(kept, vec![v("df", "/t/corral-2.parquet", 10), v("sales", "/t/corral-3.parquet", 10)]);
    }

    #[test]
    fn only_views_own_copies_are_deleted() {
        assert!(own_copy(Path::new("/tmp/RtmpAb12/corral-1a2b.parquet")));
        assert!(own_copy(Path::new("/tmp/corral-py-x1y2/101500-abc123.parquet")));
        assert!(!own_copy(Path::new("/home/me/corral-term/data/sales.parquet")));
        assert!(!own_copy(Path::new("/home/me/data/sales.parquet")));
    }

    #[test]
    fn the_same_file_again_is_kept() {
        let (kept, gone) = prune_views(vec![v("sales", "/t/corral-1.parquet", 10)], v("sales", "/t/corral-1.parquet", 10));
        assert!(gone.is_empty());
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn keeps_the_latest_twenty() {
        let earlier: Vec<ViewFile> = (0..KEEP_VIEWS).map(|i| v(&format!("t{i}"), &format!("/t/corral-{i}"), 1)).collect();
        let (kept, gone) = prune_views(earlier, v("new", "/t/corral-new", 1));
        assert_eq!(gone, vec!["/t/corral-0"]);
        assert_eq!(kept.len(), KEEP_VIEWS);
        assert_eq!(kept.last().unwrap().0, "new");
    }

    #[test]
    fn keeps_under_the_size_limit_but_always_the_newest() {
        let gb = 1 << 30;
        let (kept, gone) = prune_views(vec![v("a", "/t/corral-a", gb), v("b", "/t/corral-b", gb)], v("c", "/t/corral-c", gb));
        assert_eq!(gone, vec!["/t/corral-a"]);
        assert_eq!(kept.iter().map(|k| k.0.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
        let (kept, gone) = prune_views(vec![v("a", "/t/corral-a", gb)], v("huge", "/t/corral-huge", 5 * gb));
        assert_eq!(gone, vec!["/t/corral-a"]);
        assert_eq!(kept, vec![v("huge", "/t/corral-huge", 5 * gb)]);
    }
}
