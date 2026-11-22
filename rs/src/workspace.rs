//! The Corral layout and its panes: `corral layout` (ctrl+b a, ctrl+b shift+a),
//! `corral close` (ctrl+b shift+c), `corral script` (the editor and its draft)
//! and `corral console` (ctrl+b m, mark a pane as the console).

use crate::console;
use crate::editor;
use crate::env;
use crate::herdr;
use crate::json::{self, obj, Value};
use crate::layout::PARKING_TAB;
use crate::theme;
use crate::vars;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SCRIPT_SHARE: f64 = 0.57;
const CORRAL_LABELS: [&str; 5] = ["corral:script", "corral:r", "corral:python", "corral:vars", "corral:shell"];
const OLD_PARKING_TABS: [&str; 2] = ["consola (oculta)", "script (oculto)"];
const SCRATCH_MAX_AGE: u64 = 2 * 86400;

/// Options like `--name value`, flags like `--name`, and the rest in order.
struct Args {
    opts: Vec<(String, Option<String>)>,
    rest: Vec<String>,
}

impl Args {
    fn parse(args: &[String], with_value: &[&str], flags: &[&str], usage: &str) -> Result<Args, String> {
        let mut out = Args { opts: Vec::new(), rest: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
                _ => (a.clone(), None),
            };
            if name == "-h" || name == "--help" {
                println!("{usage}");
                std::process::exit(0);
            } else if with_value.contains(&name.as_str()) {
                let v = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("falta el valor de {name}\n{usage}"))?;
                out.opts.push((name, Some(v)));
            } else if flags.contains(&name.as_str()) {
                out.opts.push((name, None));
            } else if a.starts_with("--") {
                return Err(format!("opción desconocida {a}\n{usage}"));
            } else {
                out.rest.push(a.clone());
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
    }

    fn flag(&self, name: &str) -> bool {
        self.opts.iter().any(|(n, _)| n == name)
    }
}

fn labeled(panes: &[Value], label: &str) -> Option<String> {
    herdr::labeled(panes, label).map(|p| herdr::pane_id(p).to_string())
}

fn layout_state() -> (Value, String) {
    let ws = env::current_workspace();
    (env::read_json(&env::layout_file()), env::ws_key(ws.as_deref()))
}

// ----------------------------------------------------------------------- layout

const LAYOUT_USAGE: &str = "uso: corral layout [--foco agente|codigo] [--cwd CARPETA] [--label NOMBRE] [--agent COMANDO] \
[--console COMANDO] [--ratio 0.516] [--shell] [--editor COMANDO]";

/// New tab: the agent on the left; on the right, the script over the console and
/// the variables below. With `--foco codigo` the script and the agent trade
/// places: the script on the left, the agent over the console. The two panes
/// above the variables share that area; ctrl+b t gives it to one or the other.
pub fn layout(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--cwd", "--label", "--agent", "--console", "--ratio", "--editor", "--foco"], &["--shell"], LAYOUT_USAGE)?;
    let code = match a.value("--foco").unwrap_or("agente") {
        "agente" => false,
        "codigo" => true,
        other => return Err(format!("--foco acepta agente o codigo, no {other}")),
    };
    let ws = env::current_workspace().ok_or("no encontré un workspace de herdr")?;
    let ctx = env::plugin_context();
    let cwd = a.value("--cwd").map(String::from)
        .or_else(|| ctx.get("focused_pane_cwd").str().map(String::from))
        .or_else(|| ctx.get("workspace_cwd").str().map(String::from))
        .or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".into());
    let agent = a.value("--agent").map(String::from).unwrap_or_else(|| std::env::var("CORRAL_AGENT").unwrap_or_else(|_| "claude".into()));
    let editor_cmd = a.value("--editor").map(String::from).unwrap_or_else(|| std::env::var("CORRAL_EDITOR").unwrap_or_else(|_| "fresh".into()));
    let ratio: f64 = a.value("--ratio").map(|r| r.parse().map_err(|_| format!("--ratio no es un número: {r}"))).transpose()?.unwrap_or(0.516);
    let label = a.value("--label").unwrap_or("corral");

    theme::apply(); // herdr's theme rules every Corral window
    let tab = herdr::cli(&["tab", "create", "--workspace", &ws, "--label", label, "--cwd", &cwd, "--focus"])?;
    let left = tab.get("root_pane").get("pane_id").str().ok_or("herdr no creó la pestaña")?.to_string();
    let right = herdr::split(&left, "right", Some(ratio), Some(&cwd))?;

    // Only one of each per workspace: take the labels away from older panes.
    for p in herdr::panes(Some(&ws))? {
        if ["corral:r", "corral:python", "corral:vars", "corral:script"].contains(&p.get("label").str().unwrap_or("")) {
            herdr::cli(&["pane", "rename", herdr::pane_id(&p), "--clear"])?;
        }
    }
    let (mut saved, key) = layout_state();
    let mut state = Value::obj();

    // Right column: [script (or the agent) over the console] over the variables.
    let vars_pane = vars::split_under(&right, 0.73)?;
    let editor_ok = !editor_cmd.trim().is_empty() && editor_cmd.split_whitespace().next().and_then(env::which).is_some();
    if code && !editor_ok {
        return Err("el layout con foco en el código necesita el editor (Fresh); corral doctor dice qué falta".into());
    }
    let (agent_pane, script_pane) = if code { (right.clone(), left.clone()) } else { (left.clone(), right.clone()) };
    let console_pane = if editor_ok {
        let c = herdr::split(&right, "down", Some(SCRIPT_SHARE), Some(&cwd))?;
        herdr::rename(&script_pane, "corral:script")?;
        let draft = scratch_script(Some(&ws), &cwd)?;
        herdr::run(&script_pane, &editor_command(&editor_cmd, &draft, true))?;
        c
    } else {
        right.clone()
    };
    if code {
        // ctrl+b t expands the agent or the console over the variables.
        state.set("foco", "codigo".into());
        state.set("agente", agent_pane.as_str().into());
    }
    saved.set(&key, state);
    env::write_json(&env::layout_file(), &saved);
    herdr::rename(&console_pane, "corral:r")?;
    let console_cmd = a.value("--console").map(String::from).or_else(|| std::env::var("CORRAL_R").ok()).unwrap_or_else(|| console::command("r"));
    herdr::run(&console_pane, &console_cmd)?;

    if a.flag("--shell") {
        let shell = herdr::split(&vars_pane, "right", Some(0.5), Some(&cwd))?;
        herdr::rename(&shell, "corral:shell")?;
    }
    if !agent.is_empty() {
        herdr::run(&agent_pane, &agent)?;
    }
    println!("{}", obj(vec![
        ("tab", tab.get("tab").get("tab_id").clone()), ("agent", agent_pane.as_str().into()),
        ("console", console_pane.as_str().into()),
        ("script", if editor_ok { script_pane.as_str().into() } else { Value::Null }),
        ("vars", vars_pane.as_str().into()), ("foco", if code { "codigo" } else { "agente" }.into()),
    ]).to_json());
    Ok(())
}

// ----------------------------------------------------------------------- drafts

fn scratch_index() -> PathBuf {
    env::state_dir().join("borradores.json")
}

fn temp_scratch_root() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| env::home().join(".cache")).join("corral/borradores")
}

/// Where drafts go: <project>/.corral/ (so Air, lintr and the language server see
/// the project), or a temp folder when there is no project to speak of.
fn scratch_root(project: &str) -> PathBuf {
    let root = std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
    let home = std::fs::canonicalize(env::home()).unwrap_or_else(|_| env::home());
    if root != home && root != Path::new("/") && env::writable(&root) {
        root.join(".corral")
    } else {
        temp_scratch_root()
    }
}

/// A new, empty .R draft for the script pane, thrown away unless you save it. It
/// must be a real file because the editor only offers completion on saved files.
/// Drafts no editor has open, and folders older than two days, go away here.
pub fn scratch_script(ws: Option<&str>, project: &str) -> Result<PathBuf, String> {
    let mut label = "corral".to_string();
    if let Ok(list) = herdr::call("workspace.list", Value::obj()) {
        if let Some(w) = list.get("workspaces").arr().iter().find(|w| w.get("workspace_id").str() == ws) {
            label = w.get("label").str().unwrap_or("corral").to_string();
        }
    }
    let mut clean: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    while clean.contains("--") {
        clean = clean.replace("--", "-");
    }
    let clean = clean.trim_matches('-');
    let label = if clean.is_empty() { "corral" } else { clean };
    let root = scratch_root(project);
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if root.file_name().is_some_and(|n| n == ".corral") {
        // Git ignores the whole folder without touching the project's .gitignore.
        let ignore = root.join(".gitignore");
        if !ignore.exists() {
            let _ = std::fs::write(&ignore, "# Borradores temporales de Corral\n*\n");
        }
    }
    let open = env::command_lines();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for old in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let shown = old.to_string_lossy().to_string();
            if !open.iter().any(|c| c.contains(&shown)) {
                editor::discard(&old.join("borrador.R"));
            }
            let age = std::fs::metadata(&old).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            if old.exists() && age.is_some_and(|a| a.as_secs() > SCRATCH_MAX_AGE) {
                let _ = std::fs::remove_dir_all(&old);
            }
        }
    }
    let folder = root.join(format!("{label}-{}", env::stamp()));
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = folder.join("borrador.R");
    std::fs::write(&path, "").map_err(|e| format!("{}: {e}", path.display()))?;
    let mut index = env::read_json(&scratch_index());
    index.set(ws.unwrap_or("default"), path.to_string_lossy().to_string().into());
    env::write_json(&scratch_index(), &index);
    Ok(path)
}

/// Command line for the script pane. Fresh starts clean (--no-restore), without
/// old tabs. A draft goes through `corral _editor`, which throws it away when
/// the editor ends.
pub fn editor_command(editor: &str, target: &Path, scratch: bool) -> String {
    let fresh = editor.split_whitespace().any(|t| Path::new(t).file_name().is_some_and(|n| n == "fresh"));
    let extra = if fresh { " --no-restore" } else { "" };
    // WSLg's Wayland has no clipboard protocol for Fresh: it warns and falls back
    // to X11 anyway, so start it on X11 directly.
    let prefix = if fresh && std::env::var_os("WSL_DISTRO_NAME").is_some() && std::env::var_os("DISPLAY").is_some() {
        "env -u WAYLAND_DISPLAY "
    } else {
        ""
    };
    let quoted = env::sh_quote(&target.to_string_lossy());
    let command = format!("{prefix}{editor}{extra} {quoted}");
    if scratch {
        format!("{} _editor {quoted} -- {command}", console::program())
    } else {
        command
    }
}

/// Latest text of a draft: the editor plugin's live copy if newer, else the file.
fn scratch_text(scratch: &Path) -> String {
    let live = scratch.with_file_name(".actual.R");
    let newest = [live, scratch.to_path_buf()].into_iter()
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max_by_key(|(t, _)| *t);
    newest.and_then(|(_, p)| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Put text into the open draft (at the end, or instead of everything). The
/// editor doesn't let other programs change an unsaved buffer, so this leaves a
/// request next to the draft; Corral's editor plugin applies it, as one edit the
/// user can undo with Ctrl+Z, and answers.
fn scratch_write(scratch: &Path, text: &str, replace: bool) -> Result<(), String> {
    let folder = scratch.parent().ok_or("borrador sin carpeta")?;
    let rid = env::random_hex(4);
    let answer = folder.join(".respuesta.json");
    let tmp = folder.join(".pedido.tmp");
    let request = obj(vec![("id", rid.as_str().into()), ("modo", if replace { "reemplazar" } else { "agregar" }.into()),
                           ("texto", text.into())]);
    std::fs::write(&tmp, request.to_json()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, folder.join(".pedido.json")).map_err(|e| e.to_string())?; // in one step
    let end = Instant::now() + Duration::from_secs(5);
    let mut reply = Value::Null;
    while Instant::now() < end {
        reply = std::fs::read_to_string(&answer).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::Null);
        if reply.get("id").str() == Some(rid.as_str()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(folder.join(".pedido.json"));
    if reply.get("id").str() != Some(rid.as_str()) {
        return Err("el editor no respondió: ¿está abierto el borrador? Un editor abierto antes de actualizar Corral \
                    necesita cerrarse (Ctrl+Q) y abrirse otra vez".into());
    }
    if reply.get("ok") != &Value::Bool(true) {
        return Err(reply.get("msg").str().unwrap_or("el editor no aplicó el cambio").to_string());
    }
    let lines = text.matches('\n').count() + usize::from(!text.ends_with('\n') && !text.is_empty());
    let what = if replace { "reemplazó el borrador con" } else { "agregó al final del borrador" };
    println!("se {what} {lines} línea{}; el usuario lo deshace con Ctrl+Z", if lines == 1 { "" } else { "s" });
    Ok(())
}

const SCRIPT_USAGE: &str = "uso: corral script [ARCHIVO] [--editor COMANDO]\n       \
corral script --leer | --guardar RUTA [--forzar] | --escribir [--reemplazar] [ARCHIVO]";

/// `corral script`: add the editor over the console, or read, save or write the draft.
pub fn script(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--guardar", "--editor"], &["--split", "--leer", "--escribir", "--reemplazar", "--forzar"], SCRIPT_USAGE)?;
    let file = a.rest.first().cloned();
    let ws = env::current_workspace();
    if a.value("--guardar").is_some() || a.flag("--leer") || a.flag("--escribir") {
        let index = env::read_json(&scratch_index());
        let scratch = PathBuf::from(index.get(ws.as_deref().unwrap_or("default")).str().ok_or("este workspace no tiene un borrador de Corral")?);
        if a.flag("--escribir") {
            let text = match &file {
                Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?,
                None => {
                    let mut s = String::new();
                    if !std::io::stdin().is_terminal() {
                        let _ = std::io::stdin().read_to_string(&mut s);
                    }
                    s
                }
            };
            return scratch_write(&scratch, &text, a.flag("--reemplazar"));
        }
        let text = scratch_text(&scratch);
        if a.flag("--leer") {
            print!("{text}{}", if text.ends_with('\n') { "" } else { "\n" });
            return Ok(());
        }
        let raw = a.value("--guardar").unwrap_or_default();
        let expanded = raw.strip_prefix("~/").map(|r| env::home().join(r)).unwrap_or_else(|| PathBuf::from(raw));
        let target = if expanded.is_absolute() { expanded } else { std::env::current_dir().unwrap_or_default().join(expanded) };
        if target.exists() && !a.flag("--forzar") {
            return Err(format!("{} ya existe; usa --forzar para reemplazarlo", target.display()));
        }
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let body = if !text.is_empty() && !text.ends_with('\n') { format!("{text}\n") } else { text };
        std::fs::write(&target, body).map_err(|e| format!("{}: {e}", target.display()))?;
        println!("borrador guardado en {}", target.display());
        return Ok(());
    }

    let panes = herdr::panes(ws.as_deref())?;
    if labeled(&panes, "corral:script").is_some() {
        env::notify("Script", "ya hay un panel de script en este workspace");
        return Ok(());
    }
    let console = console::find("r", ws.as_deref()).ok_or("no hay consola corral:r en este workspace")?;
    let editor_cmd = a.value("--editor").map(String::from).unwrap_or_else(|| std::env::var("CORRAL_EDITOR").unwrap_or_else(|_| "fresh".into()));
    if editor_cmd.split_whitespace().next().and_then(env::which).is_none() {
        return Err(format!("no encuentro el editor '{editor_cmd}'"));
    }
    let new = herdr::split(&console, "down", Some(0.45), None)?;
    herdr::cli(&["pane", "swap", "--source-pane", &new, "--target-pane", &console])?;
    herdr::rename(&new, "corral:script")?;
    let target = match &file {
        Some(f) => PathBuf::from(f),
        None => {
            let project = env::plugin_context().get("focused_pane_cwd").str().map(String::from)
                .or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_else(|| ".".into());
            scratch_script(ws.as_deref(), &project)?
        }
    };
    herdr::run(&new, &editor_command(&editor_cmd, &target, file.is_none()))
}

// ----------------------------------------------------------------------- close

/// `corral close` (ctrl+b shift+c): Corral's panes (script, console, variables)
/// go; everything else, the agent included, stays.
pub fn close(_args: &[String]) -> Result<(), String> {
    let ws = env::current_workspace();
    let pane = env::current_pane();
    let mut tab = env::plugin_context().get("tab_id").str().map(String::from)
        .or_else(|| std::env::var("HERDR_TAB_ID").ok().filter(|t| !t.is_empty()));
    let all = herdr::panes(ws.as_deref())?;
    if tab.is_none() {
        if let Some(p) = &pane {
            tab = all.iter().find(|x| herdr::pane_id(x) == p).map(|x| herdr::tab_id(x).to_string());
        }
    }
    let is_corral = |p: &Value| CORRAL_LABELS.contains(&p.get("label").str().unwrap_or(""));
    let mut targets: Vec<String> = all.iter()
        .filter(|p| tab.as_deref().is_none_or(|t| herdr::tab_id(p) == t) && is_corral(p))
        .map(|p| herdr::pane_id(p).to_string()).collect();
    // Panes hidden by ctrl+b t or by the R/Python switch live in a tab of their own.
    if let Some(ws) = &ws {
        let hidden: Vec<String> = herdr::tabs(ws)?.iter()
            .filter(|t| { let l = t.get("label").str().unwrap_or(""); l == PARKING_TAB || OLD_PARKING_TABS.contains(&l) })
            .filter_map(|t| t.get("tab_id").str().map(String::from)).collect();
        for p in all.iter().filter(|p| hidden.iter().any(|h| h == herdr::tab_id(p)) && is_corral(p)) {
            let id = herdr::pane_id(p).to_string();
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
    }
    let (mut saved, key) = layout_state();
    let had_state = !saved.get(&key).is_null();
    if targets.is_empty() {
        env::notify("Corral", "no hay paneles de Corral en esta pestaña");
        return Ok(());
    }
    for p in &targets {
        let _ = herdr::cli(&["pane", "close", p]);
    }
    if had_state {
        saved.remove(&key);
        env::write_json(&env::layout_file(), &saved);
    }
    let note = format!("{} paneles cerrados; el borrador sin guardar se descartó", targets.len());
    env::notify("Corral cerrado", &note);
    println!("{note}");
    Ok(())
}

// ----------------------------------------------------------------------- console

/// `corral console` (ctrl+b m): mark a pane (this one by default) as the R or
/// Python console, and switch Corral on if R or IPython already runs there.
pub fn mark_console(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--pane", "--lang"], &[], "uso: corral console [--pane PANEL] [--lang r|python]")?;
    let pane = a.value("--pane").map(String::from).or_else(env::current_pane).ok_or("no sé qué panel marcar; usa --pane")?;
    let names: Vec<String> = herdr::cli(&["pane", "process-info", "--pane", &pane]).ok()
        .map(|i| i.get("process_info").get("foreground_processes").arr().iter()
            .map(|p| p.get("name").str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let session = env::session_for(&pane);
    let known = session.as_ref().map(|s| s.get("lang").str().unwrap_or("r").to_string());
    let is_ark = names.iter().any(|n| n.starts_with("jupyter-consol") || n == "ark");
    let is_python = !is_ark && names.iter().any(|n| matches!(n.as_str(), "ipython" | "python" | "python3") || n.starts_with("python3."));
    let lang = match a.value("--lang") {
        Some(l @ ("r" | "python")) => l.to_string(),
        Some(other) => return Err(format!("--lang acepta r o python, no {other}")),
        None => known.unwrap_or_else(|| if is_python { "python".into() } else { "r".into() }),
    };
    let label = console::label(&lang);
    let ws = env::current_workspace();
    for other in herdr::panes(ws.as_deref())? {
        if other.get("label").str() == Some(label.as_str()) && herdr::pane_id(&other) != pane {
            herdr::cli(&["pane", "rename", herdr::pane_id(&other), "--clear"])?;
        }
    }
    herdr::rename(&pane, &label)?;
    // If R or IPython is already running there without Corral, switch it on.
    let mut activated = false;
    if session.is_none() && lang == "r" && (is_ark || names.iter().any(|n| n == "R")) {
        herdr::run(&pane, "if (exists(\"corral_activar\")) corral_activar()")?;
        activated = true;
    } else if session.is_none() && lang == "python" && is_python {
        herdr::run(&pane, &format!("%run -i \"{}\"", env::repo().join("py/startup.py").display()))?;
        activated = true;
    }
    let (mut saved, key) = layout_state();
    let mut state = saved.get(&key).clone();
    state.set("active", lang.as_str().into());
    saved.set(&key, state);
    env::write_json(&env::layout_file(), &saved);
    env::notify("Consola marcada", &format!("{pane} es ahora {label}{}", if activated { " y Corral quedó activo" } else { "" }));
    Ok(())
}
