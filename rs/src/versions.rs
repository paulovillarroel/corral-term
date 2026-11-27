//! `corral versions` ("Console: R version" and "Console: Python version" in the
//! palette): which R and which Python the consoles run.
//!
//! R comes from rig. Corral runs the chosen version's own R (from `rig list`)
//! and leaves rig's default alone: in rig's admin mode, changing the default
//! needs sudo, and the rest of the system (RStudio, other terminals) keeps it.
//! Without a choice, Corral runs the R on the PATH, as before.
//!
//! Python comes from uv. In a uv project the choice goes to the project's
//! .python-version (`uv python pin`), so `uv run` in a terminal agrees with the
//! console; uv rebuilds the project's .venv with it. Outside a project, Corral
//! passes it to `uv run --python`.

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{self, obj, Value};
use crate::setup;
use crate::term::{clip, out, Screen};
use crate::theme;
use crate::vars::Colors;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const SIGTERM: i32 = 15;

/// An installed version, as the popup lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// What Corral saves: rig's name for an R ("4.5.2"), the minor version for Python ("3.13").
    pub id: String,
    /// What else to show: rig's aliases, or which Python builds there are.
    pub detail: String,
    /// rig's default R.
    pub default: bool,
    /// The R executable (R only).
    pub binary: Option<PathBuf>,
}

// ----------------------------------------------------------------------- R through rig

/// The R versions rig has installed (`rig list --json`); None without rig.
pub fn rig_versions() -> Option<Vec<Installed>> {
    let out = Command::new("rig").args(["list", "--json"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| parse_rig(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_rig(text: &str) -> Vec<Installed> {
    let Ok(list) = json::parse(text) else { return Vec::new() };
    list.arr().iter().filter_map(|e| {
        let name = e.get("name").str()?.to_string();
        let mut detail: Vec<String> = Vec::new();
        if let Some(v) = e.get("version").str().filter(|v| *v != name) {
            detail.push(v.to_string());
        }
        detail.extend(e.get("aliases").arr().iter().filter_map(|a| a.str().map(String::from)));
        Some(Installed {
            id: name,
            detail: detail.join(", "),
            default: e.get("default") == &Value::Bool(true),
            binary: e.get("binary").str().map(PathBuf::from),
        })
    }).collect()
}

/// The R version chosen for Corral (`corral config r_version`), if any.
fn chosen_r() -> Option<String> {
    env::config().get("r_version").str().filter(|v| !v.is_empty()).map(String::from)
}

/// The R Corral runs: the chosen version's own R, or the R on the PATH.
pub fn r_binary() -> Result<PathBuf, String> {
    let Some(v) = chosen_r() else {
        return env::which("R").ok_or_else(|| tr("R is not on the PATH", "R no está en el PATH").into());
    };
    let list = rig_versions().ok_or_else(|| trf!(
        "R {v} was chosen with rig, but rig doesn't answer; corral versions r default goes back to the R on the PATH",
        "se eligió R {v} con rig, pero rig no responde; corral versions r default vuelve al R del PATH"))?;
    list.into_iter().find(|i| i.id == v).and_then(|i| i.binary).filter(|b| b.is_file())
        .ok_or_else(|| trf!("rig no longer has R {v}; choose another with corral versions r",
                            "rig ya no tiene R {v}; elige otra con corral versions r"))
}

/// The Rscript next to the R Corral runs.
pub fn rscript() -> Option<PathBuf> {
    r_binary().ok().map(|r| r.with_file_name("Rscript")).filter(|p| p.is_file())
}

// ----------------------------------------------------------------------- Python through uv

/// The Python versions uv finds installed (`uv python list --only-installed`),
/// one per minor version, newest first; None without uv.
pub fn uv_versions() -> Option<Vec<Installed>> {
    // net: local: only what is installed
    let out = Command::new("uv").args(["python", "list", "--only-installed", "--output-format", "json"])
        .stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| parse_uv(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_uv(text: &str) -> Vec<Installed> {
    let Ok(list) = json::parse(text) else { return Vec::new() };
    // (major, minor) -> builds as (patch, "3.13.12", where)
    let mut groups: Vec<((u64, u64), Vec<(u64, String, &'static str)>)> = Vec::new();
    for e in list.arr() {
        if e.get("implementation").str() != Some("cpython") || e.get("variant").str().is_some_and(|v| v != "default") {
            continue;
        }
        let parts = e.get("version_parts");
        let (Some(major), Some(minor)) = (parts.get("major").num(), parts.get("minor").num()) else { continue };
        let patch = parts.get("patch").num().unwrap_or(0.0) as u64;
        let Some(version) = e.get("version").str() else { continue };
        let managed = [e.get("path").str(), e.get("symlink").str()].into_iter().flatten().any(|p| p.contains("/uv/python/"));
        let place = if managed { "uv" } else { "system" };
        let key = (major as u64, minor as u64);
        let i = groups.iter().position(|g| g.0 == key).unwrap_or_else(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        if !groups[i].1.iter().any(|b| b.1 == version && b.2 == place) {
            groups[i].1.push((patch, version.to_string(), place));
        }
    }
    groups.sort_by(|a, b| b.0.cmp(&a.0));
    groups.into_iter().map(|((major, minor), mut builds)| {
        builds.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(b.2)));
        let detail = builds.iter().map(|b| format!("{} ({})", b.1, b.2)).collect::<Vec<_>>().join(", ");
        Installed { id: format!("{major}.{minor}"), detail, default: false, binary: None }
    }).collect()
}

/// The uv project `dir` is in: the nearest folder with a pyproject.toml.
pub fn uv_project(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.join("pyproject.toml").is_file()).map(Path::to_path_buf)
}

/// The Python version pinned for `dir`, from the nearest .python-version.
fn pinned(dir: &Path) -> Option<String> {
    let file = dir.ancestors().map(|d| d.join(".python-version")).find(|f| f.is_file())?;
    std::fs::read_to_string(file).ok()?.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).map(String::from)
}

/// The Python version chosen for consoles outside uv projects (`corral config python_version`).
fn chosen_python() -> Option<String> {
    env::config().get("python_version").str().filter(|v| !v.is_empty()).map(String::from)
}

/// `uv run`'s choice of Python in `dir`. Inside a uv project, nothing: its
/// .python-version and environment decide. Outside one, the version chosen with
/// `corral config python_version`, and, where uv would pick an interpreter itself
/// (uv_picks_interpreter), a Python uv manages: a system Python brings the packages
/// the distribution put there (Ubuntu's numpy 1, for one), which the console's
/// pyarrow, built for numpy 2, can't import. The first time, uv downloads that
/// Python (about 30 MB); later consoles start as before.
pub fn uv_python_args(dir: &Path) -> Vec<String> {
    if uv_project(dir).is_some() {
        return Vec::new();
    }
    let mut args = Vec::new();
    if let Some(v) = chosen_python() {
        args.extend(["--python".to_string(), v]);
    }
    if uv_picks_interpreter(dir) {
        args.extend(["--python-preference".to_string(), "only-managed".to_string()]);
    }
    args
}

/// Whether uv would choose the interpreter itself in `dir`: no virtual environment
/// active (VIRTUAL_ENV) or in it or above it (a .venv), whose Python is yours to keep,
/// and no UV_PYTHON_PREFERENCE of your own.
fn uv_picks_interpreter(dir: &Path) -> bool {
    std::env::var_os("VIRTUAL_ENV").is_none_or(|v| v.is_empty())
        && std::env::var_os("UV_PYTHON_PREFERENCE").is_none()
        && !venv_around(dir)
}

/// A virtual environment in `dir` or a folder above it, as uv finds one.
fn venv_around(dir: &Path) -> bool {
    dir.ancestors().any(|d| d.join(".venv/pyvenv.cfg").is_file())
}

// ----------------------------------------------------------------------- choosing

/// The folder the consoles work in: the Python console's, else the R console's, else here.
fn work_dir() -> PathBuf {
    let ws = env::current_workspace();
    if let Ok(panes) = herdr::panes(ws.as_deref()) {
        for lang in ["python", "r"] {
            if let Some(p) = herdr::labeled(&panes, &console::label(lang)) {
                if let Some(d) = p.get("cwd").str().or_else(|| p.get("foreground_cwd").str()) {
                    return PathBuf::from(d);
                }
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| env::home())
}

fn set_config(key: &str, value: Option<&str>) {
    let mut cfg = env::config();
    match value {
        Some(v) => cfg.set(key, v.into()),
        None => cfg.remove(key),
    }
    theme::write_config(&cfg);
}

/// Make `id` (None: back to the default) the version for `lang`. Says what changed.
fn choose(lang: &str, id: Option<&str>, dir: &Path) -> Result<String, String> {
    if lang == "r" {
        if let Some(v) = id {
            let list = rig_versions().ok_or_else(|| tr("rig is not installed (https://github.com/r-lib/rig)",
                                                       "rig no está instalado (https://github.com/r-lib/rig)"))?;
            if !list.iter().any(|i| i.id == v) {
                let names: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
                return Err(trf!("rig has no R {v}; it has {}", "rig no tiene R {v}; tiene {}", names.join(", ")));
            }
        }
        set_config("r_version", id);
        return Ok(match id {
            Some(v) => trf!("Corral uses R {v}", "Corral usa R {v}"),
            None => tr("Corral uses the R on the PATH (rig's default)", "Corral usa el R del PATH (el predeterminado de rig)").into(),
        });
    }
    if let (Some(v), Some(list)) = (id, uv_versions()) {
        if !list.iter().any(|i| i.id == v) {
            let names: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
            return Err(trf!("uv has no Python {v} installed (it has {}); uv python install {v} adds it",
                            "uv no tiene Python {v} instalado (tiene {}); uv python install {v} lo agrega", names.join(", ")));
        }
    }
    let Some(project) = uv_project(dir) else {
        set_config("python_version", id);
        return Ok(match id {
            Some(v) => trf!("Corral uses Python {v} outside uv projects", "Corral usa Python {v} fuera de proyectos uv"),
            None => tr("Corral uses uv's default Python outside uv projects", "Corral usa el Python predeterminado de uv fuera de proyectos uv").into(),
        });
    };
    // net: local: writes .python-version; the next console fetches that Python (python-console)
    let mut cmd = Command::new("uv");
    cmd.args(["python", "pin"]).current_dir(&project).stdin(Stdio::null());
    match id {
        Some(v) => cmd.arg(v),
        None => cmd.arg("--rm"),
    };
    let out = cmd.output().map_err(|e| trf!("couldn't run uv: {e}", "no pude ejecutar uv: {e}"))?;
    let said = String::from_utf8_lossy(if out.status.success() { &out.stdout } else { &out.stderr }).trim().to_string();
    if !out.status.success() {
        return Err(said.strip_prefix("error: ").unwrap_or(&said).to_string());
    }
    Ok(said)
}

/// The version Corral uses now for `lang`, and the default it would follow.
fn current(lang: &str, dir: &Path, list: &[Installed]) -> (Option<String>, Option<String>) {
    if lang == "r" {
        return (chosen_r(), list.iter().find(|i| i.default).map(|i| i.id.clone()));
    }
    match uv_project(dir) {
        Some(_) => (pinned(dir), None),
        None => (chosen_python(), pinned(dir)),
    }
}

// ----------------------------------------------------------------------- restarting

/// End the console of `lang` (the job in its pane) and start it again there,
/// so it runs the version just chosen. No console open: nothing to do.
pub fn restart(lang: &str) -> Result<bool, String> {
    let ws = env::current_workspace();
    let Some(pane) = console::find(lang, ws.as_deref()) else { return Ok(false) };
    let info = herdr::cli(&["pane", "process-info", "--pane", &pane])?;
    let p = info.get("process_info");
    let names: Vec<&str> = p.get("foreground_processes").arr().iter().filter_map(|x| x.get("name").str()).collect();
    let ours = names.iter().any(|n| ["corral", "R", "ark", "uv", "ipython", "jupyter-console"].contains(n) || n.starts_with("python"));
    if let (Some(job), Some(shell)) = (p.get("foreground_process_group_id").num(), p.get("shell_pid").num()) {
        if job != shell {
            if !ours {
                return Err(tr("the console's pane is running something else; restart it by hand",
                              "el panel de la consola está ejecutando otra cosa; reiníciala a mano").into());
            }
            env::kill_group(job as i32, SIGTERM);
        }
    }
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(200));
        if console::ensure_running(&pane, lang) {
            return Ok(true);
        }
    }
    Err(tr("the console didn't come back; start it with corral switch", "la consola no volvió; ábrela con corral switch").into())
}

// ----------------------------------------------------------------------- the popup

fn name(lang: &str) -> &'static str {
    if lang == "python" { "Python" } else { "R" }
}

struct Row {
    id: Option<String>,
    label: String,
    detail: String,
}

fn rows(lang: &str, list: &[Installed], dir: &Path, default: Option<&str>) -> Vec<Row> {
    let first = match (lang, uv_project(dir)) {
        ("r", _) => Row { id: None, label: tr("rig's default", "el predeterminado de rig").into(),
                         detail: default.map(|d| trf!("now {d}; the R on the PATH", "ahora {d}; el R del PATH")).unwrap_or_default() },
        (_, Some(_)) => Row { id: None, label: tr("the project's default", "el predeterminado del proyecto").into(),
                              detail: tr("removes .python-version; requires-python decides", "quita .python-version; decide requires-python").into() },
        _ => Row { id: None, label: tr("uv's default", "el predeterminado de uv").into(),
                   detail: default.map(|d| trf!(".python-version here says {d}", ".python-version aquí dice {d}")).unwrap_or_default() },
    };
    let mut out = vec![first];
    out.extend(list.iter().map(|i| Row {
        id: Some(i.id.clone()),
        label: format!("{} {}", name(lang), i.id),
        detail: match (i.default, i.detail.is_empty()) {
            (true, true) => tr("rig's default", "predeterminado de rig").into(),
            (true, false) => format!("{}, {}", i.detail, tr("rig's default", "predeterminado de rig")),
            _ => i.detail.clone(),
        },
    }));
    out
}

fn draw(scr: &Screen, c: &Colors, lang: &str, title_note: &str, rows: &[Row], current: Option<&str>, sel: usize, message: &str) {
    let (cols, height) = scr.size();
    let mut f = String::from("\x1b[H\x1b[2J");
    f += &format!("\x1b[1;3H\x1b[1m{}{}{RESET}  {}{}{RESET}", c.text,
                  trf!("{} version", "Versión de {}", name(lang)), c.dim, clip(title_note, cols.saturating_sub(24)));
    f += &format!("\x1b[2;1H{}{}{RESET}", c.dim, "─".repeat(cols));
    for (n, r) in rows.iter().enumerate().take(height.saturating_sub(5)) {
        let y = 3 + n;
        let on = n == sel;
        let is_current = r.id.as_deref() == current;
        let (bg, mark) = if on { (c.sel_bg.as_str(), format!("{}▸{RESET}{}", c.accent, c.sel_bg)) } else { ("", " ".into()) };
        let dot = if is_current { format!("{}●{RESET}{bg}", c.accent) } else { " ".into() };
        let label_w = 24.min(cols / 2);
        f += &format!("\x1b[{y};1H{bg}\x1b[2K {mark} {dot} {}{}", c.text, clip(&r.label, label_w));
        f += &format!("\x1b[{y};{}H{bg}{}{}{RESET}", label_w + 7, c.muted, clip(&r.detail, cols.saturating_sub(label_w + 8)));
    }
    if !message.is_empty() {
        f += &format!("\x1b[{};3H{}{}{RESET}", height.saturating_sub(2), c.accent, clip(message, cols.saturating_sub(4)));
    }
    let hint = tr("↑↓ select · Enter use it · Esc close", "↑↓ elegir · Enter usar · Esc cerrar");
    f += &format!("\x1b[{height};2H{}{}{RESET}", c.dim, clip(hint, cols.saturating_sub(2)));
    out(&f);
}

/// What will happen, under the list. None: cancelled; Some(true): go on and
/// restart the console now; Some(false): go on, the console restarts later.
fn confirm(scr: &Screen, c: &Colors, lines: &[String], restart: bool) -> Option<bool> {
    let (cols, height) = scr.size();
    let top = height.saturating_sub(lines.len() + 3);
    let mut f = String::new();
    for y in top..=height {
        f += &format!("\x1b[{y};1H\x1b[2K");
    }
    for (n, l) in lines.iter().enumerate() {
        f += &format!("\x1b[{};3H{}{}{RESET}", top + n, if n == 0 { &c.text } else { &c.muted }, clip(l, cols.saturating_sub(4)));
    }
    let hint = if restart {
        tr("Enter restart now · l later · Esc cancel", "Enter reiniciar ahora · l después · Esc cancelar")
    } else {
        tr("Enter go ahead · Esc cancel", "Enter continuar · Esc cancelar")
    };
    f += &format!("\x1b[{height};2H{}{}{RESET}", c.dim, clip(hint, cols.saturating_sub(2)));
    out(&f);
    loop {
        let Some(k) = scr.key(500) else { continue };
        match k.as_str() {
            "enter" => return Some(restart),
            "l" | "L" if restart => return Some(false),
            "esc" | "quit" | "q" => return None,
            _ => {}
        }
    }
}

/// Is the console of `lang` open (in this workspace or another)?
fn console_open(lang: &str) -> bool {
    console::find(lang, env::current_workspace().as_deref()).is_some()
}

/// What to say before switching, and whether R's packages for Corral are there.
fn consequences(lang: &str, id: Option<&str>, dir: &Path, list: &[Installed]) -> Vec<String> {
    let mut lines = Vec::new();
    let shown = id.map(|v| format!("{} {v}", name(lang))).unwrap_or_else(|| tr("the default", "el predeterminado").into());
    if lang == "python" {
        if let Some(project) = uv_project(dir) {
            lines.push(trf!("{shown} for {}: uv python pin writes it to .python-version, and uv rebuilds the project's .venv.",
                            "{shown} para {}: uv python pin lo escribe en .python-version, y uv reconstruye el .venv del proyecto.",
                            project.display()));
        }
    }
    if lang == "r" {
        let binary = match id {
            Some(v) => list.iter().find(|i| i.id == v).and_then(|i| i.binary.clone()),
            None => env::which("R"),
        };
        if let Some(rscript) = binary.map(|b| b.with_file_name("Rscript")).filter(|p| p.is_file()) {
            let has = setup::r_packages(&rscript, &["httpgd", "jsonlite"]);
            let missing: Vec<&str> = ["httpgd", "jsonlite"].into_iter().zip(has).filter(|(_, h)| !h).map(|(p, _)| p).collect();
            if !missing.is_empty() {
                // net: text
                lines.push(trf!("This R doesn't have {}: install.packages(c(\"httpgd\", \"jsonlite\")) in it.",
                                "Este R no tiene {}: install.packages(c(\"httpgd\", \"jsonlite\")) en él.", missing.join(" y ")));
            }
        }
    }
    if console_open(lang) {
        lines.insert(0, trf!("Restart the {} console with {shown}? Your objects in it will be lost.",
                             "¿Reiniciar la consola de {} con {shown}? Se pierden sus objetos.", name(lang)));
    }
    lines
}

/// The popup: the installed versions; Enter uses one.
pub fn popup(lang: &str) -> Result<(), String> {
    let dir = work_dir();
    let list = if lang == "r" { rig_versions() } else { uv_versions() };
    let Some(scr) = Screen::enter(false) else { return Err(tr("not a terminal", "no hay una terminal").into()) };
    let c = Colors::load();
    let Some(list) = list else {
        let (what, how) = if lang == "r" {
            (tr("rig manages several R versions side by side.", "rig maneja varias versiones de R lado a lado."),
             "https://github.com/r-lib/rig")
        } else {
            (tr("uv manages the Python versions.", "uv maneja las versiones de Python."), "https://docs.astral.sh/uv/")
        };
        out(&format!("\x1b[H\x1b[2J\x1b[2;3H{}{}{RESET}\x1b[3;3H{}{}{RESET}\x1b[5;3H{}{}{RESET}", c.text,
                     trf!("{} isn't installed.", "{} no está instalado.", if lang == "r" { "rig" } else { "uv" }), c.muted, what, c.accent, how));
        scr.key(-1);
        return Ok(());
    };
    let (mut now, default) = current(lang, &dir, &list);
    let note = match (lang, uv_project(&dir)) {
        ("r", _) => tr("with rig · only in Corral; rig's default stays", "con rig · solo en Corral; el predeterminado de rig se queda").to_string(),
        (_, Some(p)) => trf!("with uv · project {}", "con uv · proyecto {}", p.display()),
        _ => tr("with uv · outside uv projects", "con uv · fuera de proyectos uv").to_string(),
    };
    let rows = rows(lang, &list, &dir, default.as_deref());
    let mut sel = rows.iter().position(|r| r.id == now).unwrap_or(0);
    let mut message = String::new();
    let mut size = (0, 0);
    let mut dirty = true;
    loop {
        // Draw only when something changed: the list, or the window's size.
        if dirty || scr.size() != size {
            size = scr.size();
            draw(&scr, &c, lang, &note, &rows, now.as_deref(), sel, &message);
            dirty = false;
        }
        let Some(k) = scr.key(500) else { continue };
        dirty = true;
        match k.as_str() {
            "quit" | "esc" | "q" => return Ok(()),
            "up" => sel = sel.saturating_sub(1),
            "down" => sel = (sel + 1).min(rows.len() - 1),
            "home" => sel = 0,
            "end" => sel = rows.len() - 1,
            "enter" => {
                let id = rows[sel].id.clone();
                if id == now {
                    return Ok(());
                }
                let lines = consequences(lang, id.as_deref(), &dir, &list);
                let open = console_open(lang);
                let restart_now = if open || !lines.is_empty() {
                    match confirm(&scr, &c, &lines, open) {
                        Some(r) => r,
                        None => continue,
                    }
                } else {
                    false
                };
                match choose(lang, id.as_deref(), &dir) {
                    Ok(said) => {
                        drop(scr);
                        if restart_now {
                            let mut cmd = Command::new(console::program());
                            cmd.args(["versions", lang, "--restart"]);
                            let _ = env::spawn_detached(&mut cmd, None);
                        }
                        env::notify(&trf!("{} version", "Versión de {}", name(lang)), &said);
                        return Ok(());
                    }
                    Err(e) => {
                        message = e;
                        now = current(lang, &dir, &list).0;
                    }
                }
            }
            _ => {}
        }
    }
}

// ----------------------------------------------------------------------- the command

fn usage() -> &'static str {
    tr("usage: corral versions r|python [VERSION | default] [--list | --json] [--restart]",
       "uso: corral versions r|python [VERSIÓN | default] [--list | --json] [--restart]")
}

fn open_popup(lang: &str) -> bool {
    let entry = format!("versions-{lang}");
    for _ in 0..8 {
        let ok = Command::new("herdr")
            .args(["plugin", "pane", "open", "--plugin", "corral", "--entrypoint", &entry, "--placement", "popup", "--focus"])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200)); // another modal still closing
    }
    false
}

/// `corral versions LANG [VERSION | default] [--list | --json] [--restart]`;
/// `corral _versions LANG` is the popup itself.
pub fn run(args: &[String], popup_pane: bool) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", usage());
        return Ok(());
    }
    let lang = match args.first().map(String::as_str) {
        Some(l @ ("r" | "python")) => l,
        _ => return Err(usage().into()),
    };
    if popup_pane {
        return popup(lang);
    }
    let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let (flags, values): (Vec<&str>, Vec<&str>) = rest.iter().partition(|a| a.starts_with("--"));
    if let Some(bad) = flags.iter().find(|f| !["--list", "--json", "--restart"].contains(f)) {
        return Err(trf!("unknown option {bad}\n{}", "opción desconocida {bad}\n{}", usage()));
    }
    if values.len() > 1 {
        return Err(usage().into());
    }
    let dir = std::env::current_dir().unwrap_or_else(|_| env::home());
    if let Some(v) = values.first() {
        let id = if *v == "default" { None } else { Some(*v) };
        println!("{}", choose(lang, id, &dir)?);
    }
    if flags.contains(&"--restart") {
        match restart(lang) {
            Ok(true) => env::notify(&trf!("{} console", "Consola de {}", name(lang)), tr("restarted", "reiniciada")),
            Ok(false) => {}
            Err(e) => {
                env::notify(&trf!("{} console", "Consola de {}", name(lang)), &e);
                return Err(e);
            }
        }
        return Ok(());
    }
    let listing = flags.contains(&"--list") || flags.contains(&"--json");
    if values.is_empty() && !listing {
        if (std::env::var("HERDR_ENV").as_deref() == Ok("1") || std::env::var_os("HERDR_SOCKET_PATH").is_some()) && open_popup(lang) {
            return Ok(());
        }
        if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            return popup(lang);
        }
    }
    if values.is_empty() || listing {
        let list = if lang == "r" { rig_versions() } else { uv_versions() }.unwrap_or_default();
        let (now, default) = current(lang, &dir, &list);
        if flags.contains(&"--json") {
            let entries = list.iter().map(|i| obj(vec![
                ("id", i.id.as_str().into()), ("detail", i.detail.as_str().into()), ("default", i.default.into()),
                ("current", (Some(&i.id) == now.as_ref()).into()),
            ])).collect();
            println!("{}", obj(vec![
                ("lang", lang.into()),
                ("tool", if lang == "r" { "rig" } else { "uv" }.into()),
                ("current", now.as_deref().map(Value::from).unwrap_or(Value::Null)),
                ("default", default.as_deref().map(Value::from).unwrap_or(Value::Null)),
                ("project", uv_project(&dir).filter(|_| lang == "python").map(|p| Value::from(p.to_string_lossy().as_ref())).unwrap_or(Value::Null)),
                ("versions", Value::Arr(entries)),
            ]).to_json_pretty(1));
        } else {
            if list.is_empty() {
                println!("{}", if lang == "r" { tr("rig isn't installed, or has no R", "rig no está instalado, o no tiene R") }
                               else { tr("uv isn't installed, or has no Python", "uv no está instalado, o no tiene Python") });
            }
            for i in &list {
                let mark = if Some(&i.id) == now.as_ref() { "●" } else { " " };
                let def = if i.default { tr(" (rig's default)", " (predeterminado de rig)") } else { "" };
                let detail = if i.detail.is_empty() { String::new() } else { format!("  {}", i.detail) };
                println!("{mark} {} {}{def}{detail}", name(lang), i.id);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rig_list() {
        let text = r#"[{"name":"4.5.2","default":false,"version":"4.5.2","aliases":["release"],"path":"/opt/R/4.5.2","binary":"/opt/R/4.5.2/bin/R"},
                       {"name":"4.6.1","default":true,"version":"4.6.1","aliases":[],"path":"/opt/R/4.6.1","binary":"/opt/R/4.6.1/bin/R"}]"#;
        let list = parse_rig(text);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Installed { id: "4.5.2".into(), detail: "release".into(), default: false, binary: Some("/opt/R/4.5.2/bin/R".into()) });
        assert!(list[1].default);
        assert!(parse_rig("not json").is_empty());
    }

    #[test]
    fn groups_uv_pythons_by_minor_version() {
        let text = r#"[
          {"version":"3.14.3","version_parts":{"major":3,"minor":14,"patch":3},"path":"/h/.local/bin/python3.14","symlink":"/h/.local/share/uv/python/cpython-3.14/bin/python3.14","implementation":"cpython","variant":"default"},
          {"version":"3.14.3","version_parts":{"major":3,"minor":14,"patch":3},"path":"/h/.local/share/uv/python/cpython-3.14/bin/python3.14","symlink":null,"implementation":"cpython","variant":"default"},
          {"version":"3.12.12","version_parts":{"major":3,"minor":12,"patch":12},"path":"/h/.local/share/uv/python/cpython-3.12/bin/python3.12","symlink":null,"implementation":"cpython","variant":"default"},
          {"version":"3.12.3","version_parts":{"major":3,"minor":12,"patch":3},"path":"/usr/bin/python3.12","symlink":null,"implementation":"cpython","variant":"default"},
          {"version":"3.14.3","version_parts":{"major":3,"minor":14,"patch":3},"path":"/x/python3.14t","symlink":null,"implementation":"cpython","variant":"freethreaded"},
          {"version":"3.10.14","version_parts":{"major":3,"minor":10,"patch":14},"path":"/x/pypy","symlink":null,"implementation":"pypy","variant":"default"}]"#;
        let list = parse_uv(text);
        let ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["3.14", "3.12"]);
        assert_eq!(list[0].detail, "3.14.3 (uv)");
        assert_eq!(list[1].detail, "3.12.12 (uv), 3.12.3 (system)");
    }

    #[test]
    fn finds_the_project_and_its_pin() {
        let root = std::env::temp_dir().join(format!("corral-versions-{}", std::process::id()));
        let sub = root.join("proj/src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(uv_project(&sub), None);
        std::fs::write(root.join("proj/pyproject.toml"), "[project]\nname = \"p\"\n").unwrap();
        std::fs::write(root.join("proj/.python-version"), "# pinned\n3.13\n").unwrap();
        assert_eq!(uv_project(&sub), Some(root.join("proj")));
        assert_eq!(pinned(&sub).as_deref(), Some("3.13"));
        assert!(uv_python_args(&sub).is_empty(), "in a project, .python-version decides");
        // A .venv in a folder or above it is the user's: uv uses it, so Corral doesn't choose.
        let plain = root.join("plain/deep");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(!venv_around(&plain));
        std::fs::create_dir_all(root.join("plain/.venv")).unwrap();
        std::fs::write(root.join("plain/.venv/pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        assert!(venv_around(&plain));
        let _ = std::fs::remove_dir_all(&root);
    }
}
