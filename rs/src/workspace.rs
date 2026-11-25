//! The Corral layout and its panes: `corral layout` (ctrl+b a, ctrl+b shift+s),
//! `corral close` (ctrl+b shift+q), `corral script` (the editor and its draft)
//! and `corral console` (ctrl+b m, mark a pane as the console).

use crate::console;
use crate::editor;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{self, obj, Value};
use crate::layout;
use crate::theme;
use crate::vars;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

extern "C" {
    fn setsid() -> i32;
}

const CORRAL_LABELS: [&str; 5] = ["corral:script", "corral:r", "corral:python", "corral:vars", "corral:shell"];
const SCRATCH_MAX_AGE: u64 = 2 * 86400;
/// The draft's file name, in English and Spanish, for an R or a Python draft;
/// Corral and its editor plugin know all four.
const SCRATCH_NAMES: [&str; 4] = ["draft.R", "borrador.R", "draft.py", "borrador.py"];

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
                let v = inline.or_else(|| it.next().cloned()).ok_or_else(|| trf!("{name} needs a value\n{usage}", "falta el valor de {name}\n{usage}"))?;
                out.opts.push((name, Some(v)));
            } else if flags.contains(&name.as_str()) {
                out.opts.push((name, None));
            } else if a.starts_with("--") {
                return Err(trf!("unknown option {a}\n{usage}", "opción desconocida {a}\n{usage}"));
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

fn layout_usage() -> &'static str {
    tr("usage: corral layout [--focus agent|code|notebook] [--lang r|python] [--cwd FOLDER] [--label NAME] [--agent COMMAND] \
[--console COMMAND] [--ratio N] [--shell] [--editor COMMAND] [--default-sizes]",
       "uso: corral layout [--focus agent|code|notebook] [--lang r|python] [--cwd CARPETA] [--label NOMBRE] [--agent COMANDO] \
[--console COMANDO] [--ratio N] [--shell] [--editor COMANDO] [--default-sizes]")
}

/// New tab: the agent on the left; on the right, the script over the console and
/// the variables below. With `--focus code` the script and the agent trade
/// places: the script on the left, the agent over the console. With `--focus
/// notebook`, the script is over the console on the left, and the agent over the
/// variables on the right. The script and the console share their area; ctrl+b t
/// gives it to one or the other.
/// It starts with R, or with Python: `--lang`, else `corral config start`.
/// `corral` typed in a terminal outside herdr: herdr opens on this folder's
/// workspace, with the layout. A herdr server already running keeps all it has;
/// otherwise Corral starts one the way herdr does, in a session of its own, so it
/// outlives this terminal. The workspace is the one whose Corral layout runs in this
/// folder, or a new one; then this terminal becomes herdr's.
pub fn launch_herdr() -> Result<(), String> {
    let Some(herdr_bin) = env::which("herdr") else {
        return Err(tr("Corral runs inside herdr, and herdr isn't installed: curl -fsSL https://herdr.dev/install.sh | sh",
                      "Corral funciona dentro de herdr, y herdr no está instalado: curl -fsSL https://herdr.dev/install.sh | sh").into());
    };
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    if herdr::call("workspace.list", Value::obj()).is_err() {
        start_server(&herdr_bin)?;
    }
    let panes = herdr::call("pane.list", Value::obj())?;
    let ws = match layout_workspace(panes.get("panes").arr(), &cwd) {
        Some(ws) => ws,
        None => {
            let made = herdr::call("workspace.create", obj(vec![("cwd", cwd.as_str().into()), ("focus", true.into())]))?;
            // Its own shell tab is the terminal tab, instead of a second shell.
            if let Some(tab) = made.get("tab").get("tab_id").str() {
                if env::config().get("terminal_tab").str() != Some("off") {
                    let _ = herdr::cli(&["tab", "rename", tab, TERMINAL_TAB]);
                }
            }
            made.get("workspace").get("workspace_id").str().map(String::from)
                .ok_or(tr("herdr didn't create the workspace", "herdr no creó el workspace"))?
        }
    };
    // The layout, as ctrl+b a opens it, in that workspace.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = std::process::Command::new(exe).args(["layout", "--cwd", &cwd]).env("HERDR_WORKSPACE_ID", &ws)
        .stdout(std::process::Stdio::null()).status().map_err(|e| e.to_string())?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1)); // `corral layout` said why
    }
    let _ = herdr::call("workspace.focus", obj(vec![("workspace_id", ws.as_str().into())]));
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(&herdr_bin).exec(); // returns only if herdr didn't start
    Err(trf!("couldn't open herdr: {err}", "no pude abrir herdr: {err}"))
}

/// The label of the shell tab that opens next to the layout.
const TERMINAL_TAB: &str = "terminal";

/// A tab named terminal next to a new layout, with a shell in the project folder,
/// since the layout has none of its own. It opens behind Corral's tab, once per
/// workspace (an existing terminal tab is kept), and never fails the layout.
/// `corral config terminal_tab off` leaves it out. The tab it opened, if any.
fn terminal_tab(ws: &str, cwd: &str) -> Option<String> {
    if env::config().get("terminal_tab").str() == Some("off") {
        return None;
    }
    let has_one = herdr::tabs(ws).map(|t| t.iter().any(|t| t.get("label").str() == Some(TERMINAL_TAB))).unwrap_or(true);
    if has_one {
        return None;
    }
    let made = herdr::cli(&["tab", "create", "--workspace", ws, "--label", TERMINAL_TAB, "--cwd", cwd, "--no-focus"]).ok()?;
    made.get("tab").get("tab_id").str().map(String::from)
}

/// True when the pane's shell waits at its prompt: nothing runs in it.
fn shell_idle(pane: &str) -> bool {
    let Ok(info) = herdr::cli(&["pane", "process-info", "--pane", pane]) else { return false };
    let p = info.get("process_info");
    matches!((p.get("foreground_process_group_id").num(), p.get("shell_pid").num()), (Some(job), Some(shell)) if job == shell)
}

/// The workspace whose Corral layout runs in `cwd`: one of its panes is Corral's
/// (labeled corral:…) and started there.
fn layout_workspace(panes: &[Value], cwd: &str) -> Option<String> {
    panes.iter().find(|p| p.get("label").str().is_some_and(|l| l.starts_with("corral:")) && p.get("cwd").str() == Some(cwd))
        .and_then(|p| p.get("workspace_id").str().map(String::from))
}

/// herdr's server, started as herdr starts it: a session of its own, no terminal.
fn start_server(herdr_bin: &Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(herdr_bin);
    cmd.arg("server").current_dir(env::home())
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // SAFETY: setsid only changes this child's session; it allocates nothing.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| trf!("couldn't start herdr: {e}", "no pude iniciar herdr: {e}"))?;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if herdr::call("workspace.list", Value::obj()).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(tr("herdr's server didn't start; run herdr to see why", "el servidor de herdr no arrancó; corre herdr para ver por qué").into())
}

pub fn layout(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--cwd", "--label", "--agent", "--console", "--ratio", "--editor", "--focus", "--lang"], &["--shell", "--default-sizes"], layout_usage())?;
    if a.flag("--default-sizes") {
        // Forget the sizes you left; the open layout goes back to Corral's.
        return layout::forget_sizes(env::current_workspace().as_deref());
    }
    // Whether there is a newer Corral: in the background, once a day at most.
    crate::install::check_for_update_later();
    let lang = match a.value("--lang") {
        Some(l @ ("r" | "python")) => l,
        Some(other) => return Err(trf!("--lang takes r or python, not {other}", "--lang acepta r o python, no {other}")),
        None => start_lang(),
    };
    let focus = match a.value("--focus").unwrap_or("agent") {
        f @ ("agent" | "code" | "notebook") => f,
        other => return Err(trf!("--focus takes agent, code or notebook, not {other}", "--focus acepta agent, code o notebook, no {other}")),
    };
    // A notebook starts as code first; then its console moves under the script.
    let code = focus != "agent";
    let ws = env::current_workspace().ok_or(tr("no herdr workspace found", "no encontré un workspace de herdr"))?;
    // A layout already open here is rearranged, never replaced: its console, draft
    // and agent keep running until Corral closes (ctrl+b shift+q).
    if let Some(kept) = layout::refocus(&ws, focus, true)? {
        let active = kept.get("lang").str().unwrap_or("r").to_string();
        if let Some(l) = a.value("--lang").filter(|l| *l != active) {
            layout::switch(&[l.to_string()])?;
        }
        println!("{}", kept.to_json());
        return Ok(());
    }
    let ctx = env::plugin_context();
    let cwd = a.value("--cwd").map(String::from)
        .or_else(|| ctx.get("focused_pane_cwd").str().map(String::from))
        .or_else(|| ctx.get("workspace_cwd").str().map(String::from))
        .or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".into());
    // The agent pane runs any agent: --agent, then CORRAL_AGENT, then `corral config
    // agent` (none for a shell), else the first agent installed, else a shell.
    let agent = a.value("--agent").map(String::from)
        .or_else(|| std::env::var("CORRAL_AGENT").ok())
        .or_else(|| env::config().get("agent").str().map(String::from))
        .map(|a| if a == "none" { String::new() } else { a })
        .or_else(|| env::first_agent(|cmd| env::which(cmd).is_some()).map(String::from))
        .unwrap_or_else(|| {
            env::notify(tr("No agent found", "No encontré un agente"),
                        tr("Corral looks for Claude Code, Codex, Gemini CLI and opencode; the agent pane is a shell. Choose one in the palette (Agents: use…) or with corral config agent.",
                           "Corral busca Claude Code, Codex, Gemini CLI y opencode; el panel del agente es una shell. Elige uno en la paleta (Agentes: usar…) o con corral config agent."));
            String::new()
        });
    let editor_cmd = a.value("--editor").map(String::from).unwrap_or_else(|| std::env::var("CORRAL_EDITOR").unwrap_or_else(|_| "fresh".into()));
    let ratio: Option<f64> = a.value("--ratio").map(|r| r.parse().map_err(|_| trf!("--ratio is not a number: {r}", "--ratio no es un número: {r}"))).transpose()?;
    // The sizes you left in this layout the last time, else Corral's.
    let [width, over_vars, over_console] = layout::sizes(focus);
    let label = a.value("--label").unwrap_or("corral");

    theme::apply(); // herdr's theme rules every Corral window
    // The tab you were in: closing Corral (ctrl+b shift+q) takes you back to it.
    let from_tab = herdr::tabs(&ws).ok()
        .and_then(|tabs| tabs.iter().find(|t| t.get("focused") == &Value::Bool(true)).and_then(|t| t.get("tab_id").str().map(String::from)));
    let tab = herdr::cli(&["tab", "create", "--workspace", &ws, "--label", label, "--cwd", &cwd, "--focus"])?;
    let left = tab.get("root_pane").get("pane_id").str().ok_or(tr("herdr did not create the tab", "herdr no creó la pestaña"))?.to_string();
    let right = herdr::split(&left, "right", Some(ratio.unwrap_or(width)), Some(&cwd))?;

    // Only one of each per workspace: take the labels away from older panes.
    for p in herdr::panes(Some(&ws))? {
        if ["corral:r", "corral:python", "corral:vars", "corral:script"].contains(&p.get("label").str().unwrap_or("")) {
            herdr::cli(&["pane", "rename", herdr::pane_id(&p), "--clear"])?;
        }
    }
    let (mut saved, key) = layout_state();
    let mut state = Value::obj();

    // Right column: [script (or the agent) over the console] over the variables.
    let vars_pane = vars::split_under(&right, over_vars)?;
    let editor_ok = !editor_cmd.trim().is_empty() && editor_cmd.split_whitespace().next().and_then(env::which).is_some();
    if code && !editor_ok {
        return Err(tr("the code-first and notebook layouts need the editor (Fresh); corral doctor tells what is missing",
                      "los layouts con foco en el código y notebook necesitan el editor (Fresh); corral doctor dice qué falta").into());
    }
    let (agent_pane, script_pane) = if code { (right.clone(), left.clone()) } else { (left.clone(), right.clone()) };
    let console_pane = if editor_ok {
        let c = herdr::split(&right, "down", Some(over_console), Some(&cwd))?;
        herdr::rename(&script_pane, "corral:script")?;
        let draft = scratch_script(Some(&ws), &cwd, lang)?;
        herdr::start(&script_pane, &editor_command(&editor_cmd, &draft, true))?;
        c
    } else {
        right.clone()
    };
    if code {
        // ctrl+b t expands the agent or the console over the variables.
        state.set("focus", "code".into());
        state.set("agent", agent_pane.as_str().into());
    }
    state.set("active", lang.into());
    // What closing (ctrl+b shift+q) takes back with it: the agent this layout started.
    state.set("agent_pane", agent_pane.as_str().into());
    if let Some(t) = &from_tab {
        state.set("from_tab", t.as_str().into());
    }
    saved.set(&key, state);
    env::write_json(&env::layout_file(), &saved);
    herdr::rename(&console_pane, &console::label(lang))?;
    let env_console = if lang == "r" { std::env::var("CORRAL_R").ok() } else { None };
    let console_cmd = a.value("--console").map(String::from).or(env_console).unwrap_or_else(|| console::command(lang));
    herdr::start(&console_pane, &console_cmd)?;
    if focus == "notebook" {
        layout::arrange_new(&ws, "notebook", ratio)?; // the console goes under the script
    }

    if a.flag("--shell") {
        let shell = herdr::split(&vars_pane, "right", Some(0.5), Some(&cwd))?;
        herdr::rename(&shell, "corral:shell")?;
    }
    if !agent.is_empty() {
        herdr::start(&agent_pane, &agent)?;
    }
    // The terminal tab this layout opened closes with it too, when nothing runs there.
    if let Some(t) = terminal_tab(&ws, &cwd) {
        let (mut saved, key) = layout_state();
        let mut opened = saved.get(&key).clone();
        opened.set("terminal_tab", t.as_str().into());
        saved.set(&key, opened);
        env::write_json(&env::layout_file(), &saved);
    }
    println!("{}", obj(vec![
        ("tab", tab.get("tab").get("tab_id").clone()), ("agent", agent_pane.as_str().into()),
        ("console", console_pane.as_str().into()),
        ("script", if editor_ok { script_pane.as_str().into() } else { Value::Null }),
        ("vars", vars_pane.as_str().into()), ("focus", focus.into()),
        ("lang", lang.into()),
    ]).to_json());
    Ok(())
}

/// The console a new layout starts with: `corral config start`, R unless it says python.
pub fn start_lang() -> &'static str {
    if env::config().get("start").str() == Some("python") { "python" } else { "r" }
}

// ----------------------------------------------------------------------- drafts

fn scratch_index() -> PathBuf {
    env::state_file("drafts.json", "borradores.json")
}

fn temp_scratch_root() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| env::home().join(".cache")).join("corral/drafts")
}

/// Where drafts go: <project>/.corral/ (so Air, lintr and the language server see
/// the project), or a temp folder when there is no project to speak of.
fn scratch_root(project: &str) -> PathBuf {
    let root = std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
    let home = std::fs::canonicalize(env::home()).unwrap_or_else(|_| env::home());
    if root != home && root != Path::new("/") && env::writable(&root) {
        root.join(".corral")
    } else {
        // Drafts are your code: the folder that holds them is yours only.
        let root = temp_scratch_root();
        if let Some(corral) = root.parent() {
            env::make_private(corral);
        }
        root
    }
}

/// Is this file a draft's name (in either language)?
pub fn is_scratch_name(name: &str) -> bool {
    SCRATCH_NAMES.contains(&name)
}

/// A new, empty .R draft for the script pane, thrown away unless you save it. It
/// must be a real file because the editor only offers completion on saved files.
/// Drafts no editor has open, and folders older than two days, go away here.
pub fn scratch_script(ws: Option<&str>, project: &str, lang: &str) -> Result<PathBuf, String> {
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
            let _ = std::fs::write(&ignore, tr("# Corral's temporary drafts\n*\n", "# Borradores temporales de Corral\n*\n"));
        }
    }
    let open = env::command_lines();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for old in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let shown = old.to_string_lossy().to_string();
            if !open.iter().any(|c| c.contains(&shown)) {
                for name in SCRATCH_NAMES {
                    editor::discard(&old.join(name));
                }
            }
            let age = std::fs::metadata(&old).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            if old.exists() && age.is_some_and(|a| a.as_secs() > SCRATCH_MAX_AGE) {
                let _ = std::fs::remove_dir_all(&old);
            }
        }
    }
    let folder = root.join(format!("{label}-{}", env::stamp()));
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let (en, es) = if lang == "python" { (SCRATCH_NAMES[2], SCRATCH_NAMES[3]) } else { (SCRATCH_NAMES[0], SCRATCH_NAMES[1]) };
    let path = folder.join(tr(en, es));
    std::fs::write(&path, "").map_err(|e| format!("{}: {e}", path.display()))?;
    let mut index = env::read_json(&scratch_index());
    index.set(ws.unwrap_or("default"), path.to_string_lossy().to_string().into());
    env::write_json(&scratch_index(), &index);
    Ok(path)
}

/// Command line for the script pane. Fresh starts clean (--no-restore), without
/// old tabs, and in Corral's language (--locale), unless the command already
/// names one. A draft goes through `corral _editor`, which throws it away when
/// the editor ends.
pub fn editor_command(editor: &str, target: &Path, scratch: bool) -> String {
    let fresh = editor.split_whitespace().any(|t| Path::new(t).file_name().is_some_and(|n| n == "fresh"));
    let extra = match (fresh, editor.contains("--locale")) {
        (true, false) => format!(" --no-restore --locale {}", crate::i18n::lang()),
        (true, true) => " --no-restore".to_string(),
        _ => String::new(),
    };
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

/// The editor plugin's live copy of a draft: .live.R or .live.py, next to it.
pub fn live_copy(scratch: &Path) -> PathBuf {
    let ext = scratch.extension().and_then(|e| e.to_str()).unwrap_or("R");
    scratch.with_file_name(format!(".live.{ext}"))
}

/// Latest text of a draft: the editor plugin's live copy if newer, else the file.
/// An editor started by Corral 0.1.7 or earlier writes its copy as .actual.R.
fn scratch_text(scratch: &Path) -> String {
    let live = live_copy(scratch);
    let ext = scratch.extension().and_then(|e| e.to_str()).unwrap_or("R");
    let older = scratch.with_file_name(format!(".actual.{ext}"));
    let newest = [live, older, scratch.to_path_buf()].into_iter()
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max_by_key(|(t, _)| *t);
    newest.and_then(|(_, p)| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Put text into the open draft (at the end, or instead of everything). The
/// editor doesn't let other programs change an unsaved buffer, so this leaves a
/// request next to the draft; Corral's editor plugin applies it, as one edit the
/// user can undo with Ctrl+Z, and answers.
/// Ask the editor plugin to do something with a draft (`mode`: append, replace
/// or open), and wait up to `wait` for its answer; None if it didn't answer.
fn scratch_request(scratch: &Path, mode: &str, text: &str, wait: Duration) -> Result<Option<Value>, String> {
    let folder = scratch.parent().ok_or(tr("the draft has no folder", "borrador sin carpeta"))?;
    let name = scratch.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let rid = env::random_hex(4);
    let answer = folder.join(".reply.json");
    let tmp = folder.join(".request.tmp");
    let request = obj(vec![("id", rid.as_str().into()), ("mode", mode.into()), ("file", name.into()), ("text", text.into())]);
    std::fs::write(&tmp, request.to_json()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, folder.join(".request.json")).map_err(|e| e.to_string())?; // in one step
    let end = Instant::now() + wait;
    let mut reply = None;
    while Instant::now() < end {
        let r = std::fs::read_to_string(&answer).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::Null);
        if r.get("id").str() == Some(rid.as_str()) {
            reply = Some(r);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(folder.join(".request.json"));
    Ok(reply)
}

/// The draft for `lang` next to `draft`, in the same folder and naming:
/// draft.R and draft.py, or borrador.R and borrador.py.
fn sibling_draft(draft: &Path, lang: &str) -> Option<PathBuf> {
    let name = draft.file_name()?.to_str()?;
    if !is_scratch_name(name) {
        return None;
    }
    let stem = if name.starts_with("borrador") { "borrador" } else { "draft" };
    Some(draft.with_file_name(format!("{stem}.{}", if lang == "python" { "py" } else { "R" })))
}

/// After the console switches to `lang`: the editor opens that language's draft
/// in another tab, next to the other one, and it becomes the workspace's draft
/// (what `corral script --read/--write/--save` use). Both stay temporary.
pub fn draft_follows(lang: &str) {
    let ws = env::current_workspace();
    let key = ws.as_deref().unwrap_or("default").to_string();
    let mut index = env::read_json(&scratch_index());
    let Some(current) = index.get(&key).str().map(PathBuf::from) else { return };
    let Some(target) = sibling_draft(&current, lang) else { return };
    let editor_open = herdr::panes(ws.as_deref()).ok().is_some_and(|p| labeled(&p, "corral:script").is_some());
    if !editor_open {
        return;
    }
    // Already there: the editor just shows its tab.
    if !target.exists() && std::fs::write(&target, "").is_err() {
        return;
    }
    // An editor started before this version doesn't know "open": then nothing opens.
    if let Ok(Some(reply)) = scratch_request(&target, "open", "", Duration::from_secs(2)) {
        if reply.get("ok") == &Value::Bool(true) {
            index.set(&key, target.to_string_lossy().to_string().into());
            env::write_json(&scratch_index(), &index);
        }
    }
}

fn scratch_write(scratch: &Path, text: &str, replace: bool) -> Result<(), String> {
    let reply = scratch_request(scratch, if replace { "replace" } else { "append" }, text, Duration::from_secs(5))?
        .unwrap_or(Value::Null);
    if reply.is_null() {
        return Err(tr("the editor did not answer: is the draft open? An editor opened before Corral was updated \
                       needs to be closed (Ctrl+Q) and opened again",
                      "el editor no respondió: ¿está abierto el borrador? Un editor abierto antes de actualizar Corral \
                       necesita cerrarse (Ctrl+Q) y abrirse otra vez").into());
    }
    if reply.get("ok") != &Value::Bool(true) {
        return Err(reply.get("msg").str().unwrap_or(tr("the editor did not apply the change", "el editor no aplicó el cambio")).to_string());
    }
    let lines = text.matches('\n').count() + usize::from(!text.ends_with('\n') && !text.is_empty());
    let s = if lines == 1 { "" } else { "s" };
    println!("{}", match replace {
        true => trf!("replaced the draft with {lines} line{s}; the user can undo it with Ctrl+Z",
                     "se reemplazó el borrador con {lines} línea{s}; el usuario lo deshace con Ctrl+Z"),
        false => trf!("added {lines} line{s} at the end of the draft; the user can undo it with Ctrl+Z",
                      "se agregó al final del borrador {lines} línea{s}; el usuario lo deshace con Ctrl+Z"),
    });
    Ok(())
}

fn script_usage() -> &'static str {
    tr("usage: corral script [FILE] [--editor COMMAND]\n       \
corral script --read | --save PATH [--force] | --write [--replace] [FILE]",
       "uso: corral script [ARCHIVO] [--editor COMANDO]\n     \
corral script --read | --save RUTA [--force] | --write [--replace] [ARCHIVO]")
}

/// `corral script`: add the editor over the console, or read, save or write the draft.
pub fn script(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--save", "--editor"], &["--split", "--read", "--write", "--replace", "--force"], script_usage())?;
    let file = a.rest.first().cloned();
    let ws = env::current_workspace();
    if a.value("--save").is_some() || a.flag("--read") || a.flag("--write") {
        let index = env::read_json(&scratch_index());
        let scratch = PathBuf::from(index.get(ws.as_deref().unwrap_or("default")).str()
            .ok_or(tr("this workspace has no Corral draft", "este workspace no tiene un borrador de Corral"))?);
        // In a session that goes back and forth between R and Python, the draft of the console in view.
        let lang = herdr::panes(ws.as_deref()).map(|p| env::active_lang(ws.as_deref(), &p)).unwrap_or_else(|_| "r".into());
        let scratch = sibling_draft(&scratch, &lang).filter(|d| d.exists()).unwrap_or(scratch);
        if a.flag("--write") {
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
            return scratch_write(&scratch, &text, a.flag("--replace"));
        }
        let text = scratch_text(&scratch);
        if a.flag("--read") {
            print!("{text}{}", if text.ends_with('\n') { "" } else { "\n" });
            return Ok(());
        }
        let raw = a.value("--save").unwrap_or_default();
        let expanded = raw.strip_prefix("~/").map(|r| env::home().join(r)).unwrap_or_else(|| PathBuf::from(raw));
        let target = if expanded.is_absolute() { expanded } else { std::env::current_dir().unwrap_or_default().join(expanded) };
        if target.exists() && !a.flag("--force") {
            let shown = target.display();
            return Err(trf!("{shown} already exists; use --force to replace it", "{shown} ya existe; usa --force para reemplazarlo"));
        }
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let body = if !text.is_empty() && !text.ends_with('\n') { format!("{text}\n") } else { text };
        std::fs::write(&target, body).map_err(|e| format!("{}: {e}", target.display()))?;
        let shown = target.display();
        println!("{}", trf!("draft saved to {shown}", "borrador guardado en {shown}"));
        return Ok(());
    }

    let panes = herdr::panes(ws.as_deref())?;
    if labeled(&panes, "corral:script").is_some() {
        env::notify("Script", tr("this workspace already has a script pane", "ya hay un panel de script en este workspace"));
        return Ok(());
    }
    // Over the console in view, R or Python; its draft is in that language.
    let lang = env::active_lang(ws.as_deref(), &panes);
    let console = console::find(&lang, ws.as_deref()).ok_or_else(|| {
        let label = console::label(&lang);
        trf!("no {label} console in this workspace", "no hay consola {label} en este workspace")
    })?;
    let editor_cmd = a.value("--editor").map(String::from).unwrap_or_else(|| std::env::var("CORRAL_EDITOR").unwrap_or_else(|_| "fresh".into()));
    if editor_cmd.split_whitespace().next().and_then(env::which).is_none() {
        return Err(trf!("editor '{editor_cmd}' not found", "no encuentro el editor '{editor_cmd}'"));
    }
    let new = herdr::split(&console, "down", Some(0.45), None)?;
    herdr::cli(&["pane", "swap", "--source-pane", &new, "--target-pane", &console])?;
    herdr::rename(&new, "corral:script")?;
    let target = match &file {
        Some(f) => PathBuf::from(f),
        None => {
            let project = env::plugin_context().get("focused_pane_cwd").str().map(String::from)
                .or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_else(|| ".".into());
            scratch_script(ws.as_deref(), &project, &lang)?
        }
    };
    herdr::start(&new, &editor_command(&editor_cmd, &target, file.is_none()))
}

// ----------------------------------------------------------------------- close

/// `corral close` (ctrl+b shift+q): Corral's panes (script, console, variables)
/// go; everything else, the agent included, stays.
pub fn close(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral close", "uso: corral close");
    // It takes nothing else: a typo like -help must not close the panes.
    if let Some(bad) = Args::parse(args, &[], &[], usage)?.rest.first() {
        return Err(trf!("unknown option {bad}\n{usage}", "opción desconocida {bad}\n{usage}"));
    }
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
            .filter(|t| t.get("label").str().is_some_and(layout::is_any_parking_tab))
            .filter_map(|t| t.get("tab_id").str().map(String::from)).collect();
        for p in all.iter().filter(|p| hidden.iter().any(|h| h == herdr::tab_id(p)) && is_corral(p)) {
            let id = herdr::pane_id(p).to_string();
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
    }
    layout::remember_open(ws.as_deref()); // the sizes you left, for the next layout
    let (mut saved, key) = layout_state();
    let had_state = !saved.get(&key).is_null();
    let opened = saved.get(&key).clone();
    let from_tab = opened.get("from_tab").str().map(String::from);
    if targets.is_empty() {
        env::notify("Corral", tr("no Corral panes in this tab", "no hay paneles de Corral en esta pestaña"));
        return Ok(());
    }
    // Closed on purpose: their checkpoints must not come back in the next console.
    let closed: Vec<Value> = env::sessions().into_iter()
        .filter(|s| s.get("pane_id").str().is_some_and(|p| targets.iter().any(|t| t == p))).collect();
    for p in &targets {
        let _ = herdr::cli(&["pane", "close", p]);
    }
    for s in &closed {
        env::close_checkpoint(s);
    }
    if had_state {
        saved.remove(&key);
        env::write_json(&env::layout_file(), &saved);
    }
    let kept = close_what_it_opened(&opened, &all, from_tab.as_deref(), tab.as_deref());
    // Back to the tab Corral was opened from, if it is still there.
    if let (Some(ws), Some(from)) = (&ws, &from_tab) {
        let open = herdr::tabs(ws).map(|t| t.iter().any(|t| t.get("tab_id").str() == Some(from.as_str()))).unwrap_or(false);
        if open && tab.as_deref() != Some(from.as_str()) {
            let _ = herdr::cli(&["tab", "focus", from]);
        }
    }
    let n = targets.len();
    let mut note = trf!("{n} panes closed; the unsaved draft was discarded", "{n} paneles cerrados; el borrador sin guardar se descartó");
    if !kept.is_empty() {
        note.push_str(&trf!("; still open: {}", "; siguen abiertos: {}", kept.join(", ")));
    }
    env::notify(tr("Corral closed", "Corral cerrado"), &note);
    println!("{note}");
    Ok(())
}

/// What the layout opened besides Corral's panes, closed when nothing runs there:
/// the agent it started, when herdr says it is idle (or it is a shell at its
/// prompt), and the terminal tab it opened, when its shell waits at the prompt.
/// A tab you opened Corral from is never closed. What stays, said in words.
fn close_what_it_opened(opened: &Value, all: &[Value], from_tab: Option<&str>, corral_tab: Option<&str>) -> Vec<String> {
    let mut kept = Vec::new();
    if let Some(agent) = opened.get("agent_pane").str() {
        if let Some(p) = all.iter().find(|p| herdr::pane_id(p) == agent) {
            let idle = match p.get("agent_status").str() {
                Some("idle") => true,
                Some("working") | Some("blocked") => false,
                _ => shell_idle(agent),
            };
            if idle {
                let _ = herdr::cli(&["pane", "close", agent]);
                // Nothing else of yours in that tab: it goes too.
                if let Some(t) = corral_tab.filter(|t| Some(*t) != from_tab) {
                    let rest = herdr::panes(None).map(|ps| ps.iter().any(|p| herdr::tab_id(p) == t)).unwrap_or(true);
                    if !rest {
                        let _ = herdr::cli(&["tab", "close", t]);
                    }
                }
            } else {
                kept.push(tr("the agent, which is busy", "el agente, que está trabajando").to_string());
            }
        }
    }
    if let Some(t) = opened.get("terminal_tab").str().filter(|t| Some(*t) != from_tab) {
        let panes: Vec<&Value> = all.iter().filter(|p| herdr::tab_id(p) == t).collect();
        if !panes.is_empty() {
            if panes.iter().all(|p| shell_idle(herdr::pane_id(p))) {
                let _ = herdr::cli(&["tab", "close", t]);
            } else {
                kept.push(tr("the terminal tab, where a command runs", "la pestaña terminal, donde corre un comando").to_string());
            }
        }
    }
    kept
}

// ----------------------------------------------------------------------- console

/// `corral console` (ctrl+b m): mark a pane (this one by default) as the R or
/// Python console, and switch Corral on if R or IPython already runs there.
pub fn mark_console(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral console [--pane PANE] [--lang r|python]", "uso: corral console [--pane PANEL] [--lang r|python]");
    let a = Args::parse(args, &["--pane", "--lang"], &[], usage)?;
    let pane = a.value("--pane").map(String::from).or_else(env::current_pane)
        .ok_or(tr("which pane to mark? Use --pane", "no sé qué panel marcar; usa --pane"))?;
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
        Some(other) => return Err(trf!("--lang takes r or python, not {other}", "--lang acepta r o python, no {other}")),
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
        herdr::run(&pane, "if (exists(\"corral_activate\")) corral_activate()")?;
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
    let note = if activated {
        trf!("{pane} is now {label} and Corral is active", "{pane} es ahora {label} y Corral quedó activo")
    } else {
        trf!("{pane} is now {label}", "{pane} es ahora {label}")
    };
    env::notify(tr("Console marked", "Consola marcada"), &note);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn each_language_has_its_draft_next_to_the_other() {
        let r = Path::new("/p/.corral/w-1/draft.R");
        assert_eq!(sibling_draft(r, "python"), Some(PathBuf::from("/p/.corral/w-1/draft.py")));
        assert_eq!(sibling_draft(r, "r"), Some(r.to_path_buf()));
        let py = Path::new("/p/.corral/w-1/borrador.py");
        assert_eq!(sibling_draft(py, "r"), Some(PathBuf::from("/p/.corral/w-1/borrador.R")));
        assert_eq!(sibling_draft(Path::new("/p/analysis.R"), "python"), None);
    }

    #[test]
    fn each_draft_has_its_own_live_copy() {
        assert_eq!(live_copy(Path::new("/d/draft.R")), PathBuf::from("/d/.live.R"));
        assert_eq!(live_copy(Path::new("/d/draft.py")), PathBuf::from("/d/.live.py"));
    }

    #[test]
    fn fresh_starts_in_corrals_language() {
        std::env::set_var("CORRAL_LANG", "en");
        let line = editor_command("fresh", Path::new("/tmp/a.R"), false);
        assert!(line.contains("fresh --no-restore --locale en"), "{line}");
        // A locale the user chose in CORRAL_EDITOR wins, and isn't doubled.
        let line = editor_command("fresh --locale ja", Path::new("/tmp/a.R"), false);
        assert_eq!(line.matches("--locale").count(), 1, "{line}");
        // Other editors get nothing added.
        assert!(!editor_command("nvim", Path::new("/tmp/a.R"), false).contains("--locale"));
    }
}
