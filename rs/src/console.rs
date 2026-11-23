//! The consoles. `corral _console <lang>` is what corral types in a console
//! pane: it clears the screen and becomes the console (Ark, plain R or
//! IPython). The rest finds a console, brings it into the layout's console
//! spot, starts it when it isn't there, and waits for its prompt.

use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{obj, Value};
use crate::layout;
use crate::send::main_tab;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub fn label(lang: &str) -> String {
    format!("corral:{lang}")
}

pub fn lang_name(lang: &str) -> &'static str {
    if lang == "python" { "Python" } else { "R" }
}

/// The newest Ark under ~/.local/share/corral/ark (or corral config ark_bin).
pub fn ark_binary(cfg: &Value) -> Option<PathBuf> {
    if let Some(p) = cfg.get("ark_bin").str() {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let version = |p: &PathBuf| -> Vec<u64> {
        p.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()).unwrap_or("")
            .split('.').map(|x| x.parse().unwrap_or(0)).collect()
    };
    std::fs::read_dir(env::data_dir().join("ark")).ok()?
        .flatten().map(|e| e.path().join("ark")).filter(|p| p.is_file())
        .max_by_key(|p| version(p))
}

/// Ark's front end: Corral's own console, or jupyter-console (`ark_frontend: "jupyter"`).
pub fn jupyter_frontend(cfg: &Value) -> bool {
    cfg.get("ark_frontend").str() == Some("jupyter")
}

/// "ark" (the default when Ark is installed) or "r".
pub fn r_flavor(cfg: &Value) -> &'static str {
    let choice = cfg.get("r_console").str().unwrap_or("ark");
    let frontend = !jupyter_frontend(cfg) || env::which("jupyter-console").is_some();
    if choice == "ark" && frontend && ark_binary(cfg).is_some() && crate::versions::r_binary().is_ok() {
        "ark"
    } else {
        "r"
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let lang = args.first().map(String::as_str).unwrap_or("r");
    if lang != "r" && lang != "python" {
        return Err(tr("usage: corral _console r|python", "uso: corral _console r|python").into());
    }
    env::remove_dead_sessions(); // what consoles that ended left behind
    env::trim_log(&env::state_dir().join("ark.log"));
    print!("\x1b[H\x1b[2J\x1b[3J"); // the command corral typed goes away
    let _ = std::io::stdout().flush();
    let cfg = env::config();

    let mut cmd = if lang == "python" {
        match cfg.get("python_console").str() {
            Some(custom) => shell(custom),
            None => {
                let here = std::env::current_dir().unwrap_or_else(|_| env::home());
                let mut c = Command::new("uv");
                c.arg("run").args(crate::versions::uv_python_args(&here))
                    .args(["--with", "ipython", "ipython", "--no-banner", "-i"]).arg(env::repo().join("py/startup.py"));
                c
            }
        }
    } else {
        match cfg.get("r_console").str() {
            Some(custom) if custom != "ark" && custom != "r" => shell(custom),
            _ if r_flavor(&cfg) == "ark" && !jupyter_frontend(&cfg) => return crate::ark::run(),
            _ if r_flavor(&cfg) == "ark" => {
                let jupyter = kernelspec(&cfg)?;
                // The connection file goes where corral.R announces it, so
                // `corral _lsp r` can reach Ark's language server.
                let folder = env::state_dir().join("ark");
                let _ = std::fs::create_dir_all(&folder);
                env::remove_old(&folder, "json", 2 * 86400);
                let conn = folder.join(format!("kernel-{}.json", env::random_hex(4)));
                let mut c = Command::new("jupyter-console");
                c.args(["--kernel", "ark"])
                    .arg(format!("--ConnectionFileMixin.connection_file={}", conn.display()))
                    .arg("--ZMQTerminalInteractiveShell.banner=")
                    .env("CORRAL_ARK_CONNECTION", &conn)
                    .env("JUPYTER_PATH", &jupyter);
                c
            }
            _ => {
                let mut c = Command::new(crate::versions::r_binary()?);
                c.args(["--quiet", "--no-save"]);
                c
            }
        }
    };
    let err = cmd.env("CORRAL", "1").exec();
    Err(trf!("could not open the console: {err}", "no pude abrir la consola: {err}"))
}

fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", command]);
    c
}

/// Corral's own kernel spec for Ark, for jupyter-console: R_HOME and R's
/// library path as the R script sets them. Your Jupyter setup stays as it is.
pub fn kernelspec(cfg: &Value) -> Result<PathBuf, String> {
    let ark = ark_binary(cfg).ok_or(tr("Ark is not installed (corral ark --install)", "Ark no está instalado (corral ark --install)"))?;
    crate::ark::r_env(&ark)?; // writes the spec, for the R Corral runs
    Ok(env::data_dir().join("jupyter"))
}

/// How to call this program from a pane: `corral` when that is this binary
/// on the PATH, else its full path.
pub fn program() -> String {
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let on_path = env::which("corral").and_then(|p| p.canonicalize().ok());
    match &me {
        Some(m) if on_path.as_ref() != Some(m) => format!("'{}'", m.to_string_lossy().replace('\'', r"'\''")),
        _ => "corral".to_string(),
    }
}

/// How a pane starts its console: `corral _console <lang>`.
pub fn command(lang: &str) -> String {
    format!("{} _console {lang}", program())
}

/// The console pane for `lang`: this workspace first, then any.
pub fn find(lang: &str, ws: Option<&str>) -> Option<String> {
    let scopes: Vec<Option<&str>> = if ws.is_some() { vec![ws, None] } else { vec![None] };
    scopes.into_iter().find_map(|scope| {
        let panes = herdr::panes(scope).ok()?;
        herdr::labeled(&panes, &label(lang)).map(|p| herdr::pane_id(p).to_string())
    })
}

/// The console pane for `lang` and the Corral session in it.
pub fn session(lang: &str, ws: Option<&str>) -> Result<(String, Value), String> {
    let Some(console) = find(lang, ws) else {
        let label = label(lang);
        return Err(if lang == "python" {
            tr("no Python console: it opens on its own when Python code runs, or with ctrl+b y",
               "no hay consola de Python: se abre sola al ejecutar código de Python, o con ctrl+b y").into()
        } else {
            trf!("no {label} console in herdr; open one with ctrl+b a or mark one with ctrl+b m",
                 "no hay consola {label} en herdr; ábrela con ctrl+b a o márcala con ctrl+b m")
        });
    };
    match env::session_for(&console) {
        Some(s) => Ok((console, s)),
        None if lang == "python" => Err(trf!("the console {console} does not have Corral active (is IPython still starting?)",
                                             "la consola {console} no tiene Corral activo (¿IPython sigue arrancando?)")),
        None => Err(trf!("the console {console} does not have Corral active: open R with `CORRAL=1 R` or run corral_activate() in that R (ctrl+b m does it)",
                         "la consola {console} no tiene Corral activo: abre R con `CORRAL=1 R` o corre corral_activate() en ese R (ctrl+b m lo hace)")),
    }
}

/// The session for this place: CORRAL_SESSION, the active console, the workspace, the newest.
pub fn pick_session() -> Option<Value> {
    let mut all = env::sessions();
    if all.is_empty() {
        return None;
    }
    if let Ok(wanted) = std::env::var("CORRAL_SESSION") {
        if let Some(s) = all.iter().find(|s| s.get("id").str() == Some(wanted.as_str())) {
            return Some(s.clone());
        }
    }
    if env::in_herdr() {
        let ws = env::current_workspace();
        let lang = herdr::panes(ws.as_deref()).map(|p| env::active_lang(ws.as_deref(), &p)).unwrap_or_else(|_| "r".into());
        if let Some(console) = find(&lang, ws.as_deref()) {
            if let Some(s) = all.iter().find(|s| s.get("pane_id").str() == Some(console.as_str())) {
                return Some(s.clone());
            }
        }
        let same: Vec<Value> = all.iter().filter(|s| ws.is_some() && s.get("workspace_id").str() == ws.as_deref()).cloned().collect();
        if !same.is_empty() {
            all = same;
        }
    }
    all.into_iter().max_by(|a, b| a.get("started").num().unwrap_or(0.0).total_cmp(&b.get("started").num().unwrap_or(0.0)))
}

pub fn last_line(pane: &str) -> String {
    let params = obj(vec![("pane_id", pane.into()), ("source", "visible".into()), ("format", "text".into())]);
    let text = herdr::call("pane.read", params).ok()
        .and_then(|r| r.get("read").get("text").str().map(String::from)).unwrap_or_default();
    text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).last().unwrap_or("").to_string()
}

/// Is an earlier `corral eval` request still running? Its output file is there, its outcome is not yet.
fn eval_running(edir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(edir) else { return false };
    let newest = entries.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "out"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    newest.is_some_and(|e| !e.path().with_extension("json").exists())
}

/// Wait for an empty prompt (R's `>`, or `In [n]:` in IPython and Ark); refuse half-typed input.
pub fn wait_for_prompt(pane: &str, timeout: f64, lang: &str, edir: Option<&Path>) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs_f64(timeout);
    loop {
        let line = last_line(pane);
        let (idle, half) = if let Some(rest) = line.strip_prefix("In [").and_then(|r| r.split_once("]:")).map(|(_, r)| r) {
            (rest.trim().is_empty(), !rest.trim().is_empty())
        } else if line.trim_start().starts_with("...:") || lang == "python" {
            (false, line.trim_start().starts_with("...:"))
        } else {
            (line.trim() == ">", line.starts_with('+') || (line.starts_with("> ") && !line[2..].trim().is_empty()))
        };
        if idle {
            return Ok(());
        }
        // An earlier request still running echoes its code like typed input: busy, not half-typed.
        let half = half && !edir.is_some_and(eval_running);
        if Instant::now() > end {
            let name = lang_name(lang);
            return Err(if half {
                tr("the console has a half-typed line; finish it or delete it and try again",
                   "la consola tiene una línea a medio escribir; termínala o bórrala y vuelve a intentar").into()
            } else {
                trf!("the {name} console is busy; wait for it to finish or use --wait-idle",
                     "la consola de {name} está ocupada; espera a que termine o usa --wait-idle")
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Is this console a Jupyter-style one (IPython, or R through Ark)?
pub fn is_jupyter(pane: &str, lang: &str) -> bool {
    if lang == "python" {
        return true;
    }
    match env::session_for(pane) {
        Some(s) => !s.get("ark_connection").is_null(),
        None => {
            let line = last_line(pane);
            line.starts_with("In [") || line.trim_start().starts_with("...:")
        }
    }
}

/// A new console pane: under `near`, or in the hidden tab.
fn start_pane(lang: &str, ws: Option<&str>, cwd: &str, near: Option<&str>) -> Result<String, String> {
    let pane = if let Some(near) = near {
        herdr::split(near, "down", Some(0.5), Some(cwd))?
    } else if let (_, Some(other)) = layout::hidden_tab(ws)? {
        herdr::split(&other, "right", None, Some(cwd))?
    } else {
        let mut args = vec!["tab", "create", "--label", layout::parking_tab(), "--cwd", cwd];
        if let Some(ws) = ws {
            args.extend(["--workspace", ws]);
        }
        herdr::cli(&args)?.get("root_pane").get("pane_id").str().map(String::from)
            .ok_or(tr("herdr did not create the tab", "herdr no creó la pestaña"))?
    };
    herdr::rename(&pane, &label(lang))?;
    herdr::run(&pane, &command(lang))?;
    Ok(pane)
}

/// Put `lang`'s console in the layout's console spot, starting it if needed.
/// The console it replaces goes to the hidden tab and keeps running. Returns
/// the pane (None only when `start` is false and there is none) and whether
/// it was just started.
pub fn show(lang: &str, start: bool) -> Result<(Option<String>, bool), String> {
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref())?;
    let path = env::layout_file();
    let mut saved = env::read_json(&path);
    let key = env::ws_key(ws.as_deref());
    let mut state = saved.get(&key).clone();
    let current = env::active_lang(ws.as_deref(), &panes);
    let target = herdr::labeled(&panes, &label(lang)).map(|p| herdr::pane_id(p).to_string());
    let other = if current != lang { herdr::labeled(&panes, &label(&current)).cloned() } else { None };
    let main = main_tab(&panes);
    let visible = other.as_ref().filter(|o| main.as_deref() == Some(herdr::tab_id(o))).map(|o| herdr::pane_id(o).to_string());
    let focused = env::current_pane();
    let mut started = false;

    let target = match target {
        Some(t) => {
            if let (Some(v), Some(m)) = (&visible, &main) {
                layout::move_next_to(&t, m, v, 0.5)?;
                layout::park(v, ws.as_deref())?;
            }
            t
        }
        None if !start => return Ok((None, false)),
        None => {
            let project = other.as_ref()
                .and_then(|o| o.get("foreground_cwd").str().or_else(|| o.get("cwd").str()).map(String::from))
                .or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()))
                .unwrap_or_else(|| ".".into());
            // Without a Corral layout, the new console just goes under the other one.
            let near = visible.clone().or_else(|| if main.is_none() { other.as_ref().map(|o| herdr::pane_id(o).to_string()) } else { None });
            let t = start_pane(lang, ws.as_deref(), &project, near.as_deref())?;
            started = true;
            if let Some(v) = &visible {
                layout::park(v, ws.as_deref())?;
            }
            t
        }
    };
    // With the script expanded (ctrl+b t), the console stays hidden; ctrl+b t brings back this one.
    if let Some(o) = &other {
        if state.get("parked").str() == Some(herdr::pane_id(o)) {
            state.set("parked", target.as_str().into());
        }
    }
    if visible.is_some() && focused == visible {
        herdr::focus(&target)?;
    }
    state.set("active", lang.into());
    saved.set(&key, state);
    env::write_json(&path, &saved);
    Ok((Some(target), started))
}

/// If the console was quit (q() or exit) and only its shell is left, start it again.
pub fn ensure_running(pane: &str, lang: &str) -> bool {
    if env::session_for(pane).is_some() {
        return false;
    }
    let Ok(info) = herdr::cli(&["pane", "process-info", "--pane", pane]) else { return false };
    let names: Vec<String> = info.get("process_info").get("foreground_processes").arr().iter()
        .map(|p| p.get("name").str().unwrap_or("").to_string()).collect();
    let shells = ["bash", "zsh", "fish", "sh", "dash"];
    // Only a shell waiting at an empty prompt: a command just typed there is still starting.
    let at_prompt = last_line(pane).ends_with(['$', '#', '%', '❯']);
    if !names.is_empty() && names.iter().all(|n| shells.contains(&n.as_str())) && at_prompt {
        return herdr::run(pane, &command(lang)).is_ok();
    }
    false
}

/// The console for `lang`, in view and running, ready for input (started if needed).
pub fn ready(lang: &str) -> Result<String, String> {
    let (pane, started) = show(lang, true)?;
    let label = label(lang);
    let pane = pane.ok_or_else(|| trf!("No {label} pane. Use ctrl+b a", "No hay panel {label}. Usa ctrl+b a"))?;
    let mut started = started || ensure_running(&pane, lang);
    if started || env::session_for(&pane).is_none() {
        // Just started, or still starting: wait for its Corral session, then an empty prompt.
        // The first Python console can take a while (uv prepares its environment).
        let mut end = Instant::now() + Duration::from_secs(if started { 180 } else { 30 });
        let mut tried = Instant::now();
        while env::session_for(&pane).is_none() {
            // A console that just ended (R crashed or quit) gives the pane back to
            // its shell a moment later: start it again then.
            if !started && tried.elapsed() > Duration::from_millis(500) {
                tried = Instant::now();
                if ensure_running(&pane, lang) {
                    started = true;
                    end = Instant::now() + Duration::from_secs(180);
                }
            }
            if Instant::now() > end {
                return Err(trf!("the {label} console ({pane}) does not have Corral active; open one with ctrl+b a or mark it with ctrl+b m",
                                "la consola {label} ({pane}) no tiene Corral activo; ábrela con ctrl+b a o márcala con ctrl+b m"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        wait_for_prompt(&pane, 60.0, lang, None)?;
    }
    Ok(pane)
}
