//! `corral toggle` (ctrl+b t) and `corral switch` (ctrl+b y): which pane gets
//! the area above the variables (script or console; agent or console in the
//! layout focused on code). herdr won't shrink a split below 10 %, so a
//! pane that has to disappear moves to the workspace's hidden tab, where it
//! keeps running, and comes back to its place later.

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{obj, Value};
use std::path::PathBuf;

/// The hidden tab's name in each language; a new one gets the user's.
const PARKING_TABS: [&str; 2] = ["corral (hidden)", "corral (oculto)"];
/// Hidden tabs left by older versions: `corral close` closes their panes too.
const OLD_PARKING_TABS: [&str; 2] = ["consola (oculta)", "script (oculto)"];
const SCRIPT_SHARE: f64 = 0.57;

/// The hidden tab's name for a new one, in the user's language.
pub fn parking_tab() -> &'static str {
    tr(PARKING_TABS[0], PARKING_TABS[1])
}

/// Is this the hidden tab, whatever language it was created in?
pub fn is_parking_tab(label: &str) -> bool {
    PARKING_TABS.contains(&label)
}

/// The hidden tab, or one an older version left.
pub fn is_any_parking_tab(label: &str) -> bool {
    is_parking_tab(label) || OLD_PARKING_TABS.contains(&label)
}

/// The layout focused on code (`corral layout --focus code`)? Also reads the state older versions saved.
pub fn code_focus(state: &Value) -> bool {
    state.get("focus").str() == Some("code") || state.get("foco").str() == Some("codigo")
}

/// The agent's pane in the layout focused on code (also as older versions saved it).
pub fn agent_pane(state: &Value) -> Option<&str> {
    state.get("agent").str().or_else(|| state.get("agente").str())
}

pub fn hidden_tab(ws: Option<&str>) -> Result<(Option<String>, Option<String>), String> {
    let Some(ws) = ws else { return Ok((None, None)) };
    let tab = herdr::tabs(ws)?.into_iter().find(|t| t.get("label").str().is_some_and(is_parking_tab))
        .and_then(|t| t.get("tab_id").str().map(String::from));
    let pane = match &tab {
        Some(tab) => herdr::panes(Some(ws))?.into_iter().find(|p| herdr::tab_id(p) == tab)
            .map(|p| herdr::pane_id(&p).to_string()),
        None => None,
    };
    Ok((tab, pane))
}

/// Move a pane out of sight, to the hidden tab (created when needed).
pub fn park(pane: &str, ws: Option<&str>) -> Result<(), String> {
    let destination = match hidden_tab(ws)? {
        (Some(tab), Some(other)) => obj(vec![("type", "tab".into()), ("tab_id", tab.into()),
                                             ("split", "right".into()), ("target_pane_id", other.into())]),
        _ => {
            let mut d = obj(vec![("type", "new_tab".into()), ("label", parking_tab().into())]);
            if let Some(ws) = ws {
                d.set("workspace_id", ws.into());
            }
            d
        }
    };
    herdr::call("pane.move", obj(vec![("pane_id", pane.into()), ("focus", false.into()), ("destination", destination)]))?;
    Ok(())
}

pub fn move_next_to(pane: &str, tab: &str, target: &str, ratio: f64) -> Result<(), String> {
    let destination = obj(vec![("type", "tab".into()), ("tab_id", tab.into()), ("target_pane_id", target.into()),
                               ("split", "down".into()), ("ratio", ratio.into())]);
    herdr::call("pane.move", obj(vec![("pane_id", pane.into()), ("focus", false.into()), ("destination", destination)]))?;
    Ok(())
}

fn contains(node: &Value, pane: &str) -> bool {
    if node.get("type").str() == Some("pane") {
        return herdr::pane_id(node) == pane;
    }
    contains(node.get("first"), pane) || contains(node.get("second"), pane)
}

/// The split that separates `a` from `b`: its path, ratio, and whether `a` comes first.
fn split_between(node: &Value, a: &str, b: &str, path: &mut Vec<bool>) -> Option<(Vec<bool>, f64, bool)> {
    if node.get("type").str() != Some("split") {
        return None;
    }
    let (first, second) = (node.get("first"), node.get("second"));
    let ratio = node.get("ratio").num().unwrap_or(0.5);
    if contains(first, a) && contains(second, b) {
        return Some((path.clone(), ratio, true));
    }
    if contains(first, b) && contains(second, a) {
        return Some((path.clone(), ratio, false));
    }
    for (i, child) in [first, second].into_iter().enumerate() {
        if contains(child, a) && contains(child, b) {
            path.push(i == 1);
            return split_between(child, a, b, path);
        }
    }
    None
}

/// The split between panes `a` and `b`: tab, path, whether `a` comes first, and `a`'s share.
fn pane_split(a: &str, b: &str) -> Option<(String, Vec<bool>, bool, f64)> {
    let layout = herdr::call("layout.export", obj(vec![("pane_id", b.into())])).ok()?;
    let layout = layout.get("layout");
    let (path, ratio, a_first) = split_between(layout.get("root"), a, b, &mut Vec::new())?;
    let share = if a_first { ratio } else { 1.0 - ratio };
    Some((layout.get("tab_id").str()?.to_string(), path, a_first, share))
}

/// Tab, split path, whether the script comes first, and the script's share.
fn script_split(script: &str, console: &str) -> Option<(String, Vec<bool>, bool, f64)> {
    pane_split(script, console)
}

/// How wide the left column starts in each layout: about half for the agent,
/// more for the script, which needs the long lines.
pub fn default_width(kind: &str) -> f64 {
    if kind == "agent" { 0.516 } else { 0.62 }
}

/// The share of the panes over the variables.
const VARS_SHARE: f64 = 0.73;

/// What a layout's sizes measure: the left column's width, the share of the
/// panes over the variables, and the share of the pane over the console.
const SIZES: [&str; 3] = ["width", "over_vars", "over_console"];

/// The sizes you left in each layout ("agent", "code", "notebook"), for every
/// workspace, so a layout you resized by hand opens the same way next time.
fn sizes_file() -> PathBuf {
    env::state_dir().join("sizes.json")
}

fn usable(r: f64) -> bool {
    (0.15..=0.85).contains(&r)
}

/// Corral's sizes for a layout, in the order of `SIZES`.
fn corral_sizes(kind: &str) -> [f64; 3] {
    [default_width(kind), VARS_SHARE, SCRIPT_SHARE]
}

/// A layout's sizes, in the order of `SIZES`: the ones you left, else Corral's.
pub fn sizes(kind: &str) -> [f64; 3] {
    let saved = env::read_json(&sizes_file());
    let mut out = corral_sizes(kind);
    for (i, name) in SIZES.iter().enumerate() {
        if let Some(r) = saved.get(kind).get(name).num().filter(|r| usable(*r)) {
            out[i] = r;
        }
    }
    out
}

/// The panes of the layout open in a workspace (only those in its tab).
struct Open {
    tab: String,
    agent: Option<String>,
    script: Option<String>,
    console: Option<String>,
    vars: Option<String>,
    lang: String,
}

fn open_panes(ws: &str, panes: &[Value], state: &Value) -> Option<Open> {
    let tab = crate::send::main_tab(panes)?;
    let in_tab = |p: &&Value| herdr::tab_id(p) == tab;
    let id = |label: &str| herdr::labeled(panes, label).filter(in_tab).map(|p| herdr::pane_id(p).to_string());
    let lang = env::active_lang(Some(ws), panes);
    // The agent: as the layout saved it, else the one pane of the tab that isn't
    // Corral's (one herdr sees an agent in, if there are several).
    let agent = agent_pane(state).filter(|_| layout_kind(state) != "agent")
        .filter(|id| panes.iter().any(|p| herdr::pane_id(p) == *id)).map(String::from)
        .or_else(|| {
            let free: Vec<&Value> = panes.iter().filter(in_tab)
                .filter(|p| !p.get("label").str().is_some_and(|l| l.starts_with("corral:"))).collect();
            free.iter().find(|p| p.get("agent").str().is_some()).or(free.first()).map(|p| herdr::pane_id(p).to_string())
        });
    Some(Open { script: id("corral:script"), console: id(&console::label(&lang)), vars: id("corral:vars"), agent, lang, tab })
}

/// For each size, the two panes whose split holds it: the first one's share.
fn size_pairs<'a>(kind: &str, o: &'a Open) -> [Option<(&'a str, &'a str)>; 3] {
    let (a, s, c, v) = (o.agent.as_deref(), o.script.as_deref(), o.console.as_deref(), o.vars.as_deref());
    let (left, right) = if kind == "agent" { (a, s.or(c)) } else { (s, a) };
    let over_vars = if kind == "agent" { s.or(c) } else { a };
    let over_console = if kind == "code" { a } else { s };
    [left.zip(right), over_vars.zip(v), over_console.zip(c)]
}

/// Keep the sizes the layout has now, which you may have changed by hand. Only
/// the ones that differ from Corral's are written down.
fn remember_sizes(kind: &str, o: &Open) {
    let path = sizes_file();
    let mut saved = env::read_json(&path);
    let mut mine = saved.get(kind).clone();
    for ((name, pair), corral) in SIZES.iter().zip(size_pairs(kind, o)).zip(corral_sizes(kind)) {
        match pair.and_then(|(x, y)| pane_split(x, y)).map(|s| s.3).filter(|r| usable(*r)) {
            Some(r) if (r - corral).abs() < 0.005 => mine.remove(name),
            Some(r) => mine.set(name, ((r * 1000.0).round() / 1000.0).into()),
            None => {}
        }
    }
    if mine != *saved.get(kind) {
        match &mine {
            Value::Obj(items) if items.is_empty() => saved.remove(kind),
            _ => saved.set(kind, mine),
        }
        env::write_json(&path, &saved);
    }
}

/// Give the layout its sizes; `width`, when given, instead of the saved one.
fn apply_sizes(kind: &str, o: &Open, width: Option<f64>) -> Result<(), String> {
    let mut want = sizes(kind);
    if let Some(w) = width {
        want[0] = w;
    }
    for (pair, w) in size_pairs(kind, o).into_iter().zip(want) {
        if let Some((tab, path, first, _)) = pair.and_then(|(x, y)| pane_split(x, y)) {
            set_ratio(&tab, &path, if first { w } else { 1.0 - w })?;
        }
    }
    Ok(())
}

fn read_open(ws: &str) -> Result<(Vec<Value>, Value), String> {
    let panes = herdr::panes(Some(ws))?;
    let state = env::read_json(&env::layout_file()).get(&env::ws_key(Some(ws))).clone();
    Ok((panes, state))
}

/// Keep the sizes of the layout open in `ws`, when all its panes are in view
/// (ctrl+b t hides one). Corral does it before it changes the layout or closes it.
pub fn remember_open(ws: Option<&str>) {
    let Some(ws) = ws else { return };
    let Ok((panes, state)) = read_open(ws) else { return };
    if state.get("parked").str().is_none() {
        if let Some(open) = open_panes(ws, &panes, &state) {
            remember_sizes(layout_kind(&state), &open);
        }
    }
}

/// Forget the sizes you left in every layout, and give the open one Corral's.
pub fn forget_sizes(ws: Option<&str>) -> Result<(), String> {
    let _ = std::fs::remove_file(sizes_file());
    if let Some(ws) = ws {
        let (panes, state) = read_open(ws)?;
        if let Some(open) = open_panes(ws, &panes, &state) {
            apply_sizes(layout_kind(&state), &open, None)?;
        }
    }
    env::notify("Corral", tr("Layout: Corral's sizes", "Layout: tamaños de Corral"));
    Ok(())
}

fn set_ratio(tab: &str, path: &[bool], ratio: f64) -> Result<(), String> {
    let path = Value::Arr(path.iter().map(|b| Value::Bool(*b)).collect());
    herdr::call("layout.set_split_ratio", obj(vec![("tab_id", tab.into()), ("path", path), ("ratio", ratio.into())]))?;
    Ok(())
}

fn label_of(mode: &str, code: bool) -> &'static str {
    match mode {
        "script" => "Script",
        "agent" => tr("Agent", "Agente"),
        "console" => tr("Console", "Consola"),
        _ if code => tr("Agent and console", "Agente y consola"),
        _ => tr("Script and console", "Script y consola"),
    }
}

/// Which layout the saved state describes: "agent" (agent first, the default),
/// "code" (code first) or "notebook" (script over the console on the left).
pub fn layout_kind(state: &Value) -> &'static str {
    match state.get("focus").str() {
        Some("notebook") => "notebook",
        _ if code_focus(state) => "code",
        _ => "agent",
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "code" => tr("Layout: code first", "Layout: foco en el código"),
        "notebook" => tr("Layout: notebook", "Layout: notebook"),
        _ => tr("Layout: agent first", "Layout: foco en el agente"),
    }
}

/// Switch the layout already open in workspace `ws` to `to` ("agent", "code" or
/// "notebook") in place, so every pane keeps running: the console's session, the
/// draft and the agent. The three differ only in where panes sit:
///   agent first   agent | (script over console) over variables
///   code first    script | (agent over console) over variables
///   notebook      script over console | agent over variables
/// so the agent and the script swap places, and the console moves under the
/// script or under the agent. A pane hidden with ctrl+b t comes back first.
/// The layout you leave keeps its sizes for the next time, and the next one
/// gets the sizes you left in it, else Corral's. None when the workspace has no
/// layout yet. `announce` says it in a notice.
pub fn refocus(ws: &str, to: &str, announce: bool) -> Result<Option<Value>, String> {
    rearrange(ws, to, announce, true, None)
}

/// A new layout that starts as code first and becomes `to` (the notebook), with
/// `width` (`--ratio`) for its left column when given.
pub fn arrange_new(ws: &str, to: &str, width: Option<f64>) -> Result<(), String> {
    rearrange(ws, to, false, false, width).map(|_| ())
}

fn rearrange(ws: &str, to: &str, announce: bool, learn: bool, width: Option<f64>) -> Result<Option<Value>, String> {
    let (mut panes, mut state) = read_open(ws)?;
    if crate::send::main_tab(&panes).is_none() {
        return Ok(None);
    }
    if state.get("parked").str().is_some() {
        toggle(&["both".to_string()])?;
        (panes, state) = read_open(ws)?;
    }
    let from = layout_kind(&state);
    let Some(open) = open_panes(ws, &panes, &state) else { return Ok(None) };
    if learn {
        remember_sizes(from, &open);
    }
    let Open { tab, agent, script, console, lang, .. } = &open;
    if to != from {
        let (Some(s), Some(a)) = (script, agent) else {
            return Err(if script.is_none() {
                trf!("{} needs the script pane; corral script adds it", "{} necesita el panel de script; corral script lo agrega", kind_label(to))
            } else {
                tr("couldn't tell which pane is the agent's", "no pude saber cuál es el panel del agente").into()
            });
        };
        let swap = || herdr::call("pane.swap", obj(vec![("source_pane_id", a.as_str().into()), ("target_pane_id", s.as_str().into())])).map(|_| ());
        // herdr leaves a move within the same tab unchanged, so the console goes
        // through the hidden tab, as ctrl+b t does, and comes back under `target`.
        let console_under = |target: &str| match console {
            Some(c) => park(c, Some(ws)).and_then(|_| move_next_to(c, tab, target, SCRIPT_SHARE)),
            None => Ok(()), // no console in the tab: nothing to move
        };
        match (from, to) {
            ("agent", "code") | ("code", "agent") => swap()?,
            ("code", "notebook") => console_under(s)?,
            ("notebook", "code") => console_under(a)?,
            ("agent", "notebook") => {
                swap()?;
                console_under(s)?;
            }
            _ => {
                // notebook -> agent: back to code first, then swap.
                console_under(a)?;
                swap()?;
            }
        }
        apply_sizes(to, &open, width)?;
        let path = env::layout_file();
        let mut saved = env::read_json(&path);
        let key = env::ws_key(Some(ws));
        let mut st = saved.get(&key).clone();
        for k in ["focus", "foco", "agent", "agente", "widths"] {
            st.remove(k);
        }
        if to != "agent" {
            st.set("focus", to.into());
            st.set("agent", a.as_str().into());
        }
        saved.set(&key, st);
        env::write_json(&path, &saved);
    }
    // Where you work in each one: the script when it's on the left, else the agent.
    let keep = if to == "agent" { agent.as_ref().or(script.as_ref()) } else { script.as_ref() };
    if let Some(p) = keep {
        herdr::focus(p)?;
    }
    if announce {
        env::notify("Corral", &trf!("{}; same session", "{}; misma sesión", kind_label(to)));
    }
    let id_of = |l: &str| herdr::labeled(&panes, l).map(|p| Value::from(herdr::pane_id(p))).unwrap_or(Value::Null);
    let or_null = |p: &Option<String>| p.as_deref().map(Value::from).unwrap_or(Value::Null);
    Ok(Some(obj(vec![
        ("tab", tab.as_str().into()), ("agent", or_null(agent)),
        ("console", or_null(console)), ("script", or_null(script)),
        ("vars", id_of("corral:vars")), ("focus", to.into()),
        ("lang", lang.as_str().into()), ("kept", true.into()),
    ])))
}

fn toggle_usage() -> &'static str {
    tr("usage: corral toggle [both|script|agent|console]", "uso: corral toggle [both|script|agent|console]")
}

pub fn toggle(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", toggle_usage());
        return Ok(());
    }
    let wanted = match args.first().map(String::as_str) {
        None => None,
        Some(m @ ("both" | "script" | "agent" | "console")) => Some(m.to_string()),
        Some(_) => return Err(toggle_usage().into()),
    };
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref())?;
    let lang = env::active_lang(ws.as_deref(), &panes);

    let path = env::layout_file();
    let mut saved = env::read_json(&path);
    let key = env::ws_key(ws.as_deref());
    let mut state = saved.get(&key).clone();

    // The pane above the console: the script, or the agent in the layout focused on code
    // (`corral layout --focus code`). Below, `script` is that pane, whichever it is.
    let code = code_focus(&state);
    let upper_mode = if code { "agent" } else { "script" };
    if let Some(w) = wanted.as_deref().filter(|w| matches!(*w, "script" | "agent") && *w != upper_mode) {
        let label = label_of(w, code);
        env::notify("Corral", &trf!("{label}: already on the left", "{label}: ya está a la izquierda"));
        return Ok(());
    }
    let upper = if code {
        agent_pane(&state).and_then(|id| panes.iter().find(|p| herdr::pane_id(p) == id))
    } else {
        herdr::labeled(&panes, "corral:script")
    };
    let (Some(script), Some(console)) = (upper, herdr::labeled(&panes, &format!("corral:{lang}"))) else {
        if code {
            env::notify(tr("No agent", "Sin agente"), tr("The agent's pane was not found; open the workspace again from the palette (ctrl+b space)",
                                                       "No encontré el panel del agente; abre el espacio de nuevo desde la paleta (ctrl+b space)"));
        } else {
            env::notify(tr("No script", "Sin script"), tr("This workspace has no script pane; use ctrl+b a or 'corral script'",
                                                         "Este workspace no tiene panel de script; usa ctrl+b a o 'corral script'"));
        }
        return Ok(());
    };
    let (script_tab, console_tab) = (herdr::tab_id(script).to_string(), herdr::tab_id(console).to_string());
    let (script, console) = (herdr::pane_id(script).to_string(), herdr::pane_id(console).to_string());

    let (current, parked) = if script_tab == console_tab {
        ("both", None)
    } else {
        let recorded = state.get("parked").str().filter(|p| *p == script || *p == console).map(String::from);
        let parked = recorded.unwrap_or_else(|| {
            // The variables pane stays with the visible one.
            let home = herdr::labeled(&panes, "corral:vars").map(|p| herdr::tab_id(p).to_string());
            if Some(&console_tab) == home.as_ref() { script.clone() } else { console.clone() }
        });
        (if parked == console { upper_mode } else { "console" }, Some(parked))
    };
    if parked.is_none() {
        remember_open(ws.as_deref()); // every pane is in view: keep the sizes you left
    }
    let focused = env::current_pane();
    let mode = wanted.unwrap_or_else(|| {
        if current != "both" {
            "both".into()
        } else if focused.as_deref() == Some(console.as_str()) {
            "console".into()
        } else {
            upper_mode.into()
        }
    });
    if mode == current {
        env::notify("Corral", label_of(&mode, code));
        return Ok(());
    }

    if let Some(parked) = &parked {
        // Back to both, script above the console.
        let share = state.get("share").num().unwrap_or(SCRIPT_SHARE);
        let (visible_tab, back_to) = if *parked == console { (&script_tab, &script) } else { (&console_tab, &console) };
        if *parked == console {
            move_next_to(&console, visible_tab, back_to, share)?;
        } else {
            move_next_to(&script, visible_tab, back_to, 1.0 - share)?;
            herdr::call("pane.swap", obj(vec![("source_pane_id", script.as_str().into()), ("target_pane_id", console.as_str().into())]))?;
            if let Some((tab, path, script_first, _)) = script_split(&script, &console) {
                set_ratio(&tab, &path, if script_first { share } else { 1.0 - share })?;
            }
        }
        if let Some(f) = &focused {
            if *f == script || *f == console {
                herdr::focus(f)?; // the swap can move the focus; keep it where you were
            }
        }
        state.remove("parked");
        state.remove("share");
    }

    if mode != "both" {
        let (keep, hide) = if mode == upper_mode { (&script, &console) } else { (&console, &script) };
        let mut share = script_split(&script, &console).map(|s| s.3).unwrap_or(SCRIPT_SHARE);
        if !(0.15..=0.85).contains(&share) {
            share = SCRIPT_SHARE;
        }
        park(hide, ws.as_deref())?;
        herdr::focus(keep)?;
        state.set("parked", hide.as_str().into());
        state.set("share", share.into());
    }

    saved.set(&key, state);
    env::write_json(&path, &saved);
    let suffix = if mode != "both" { tr(" (ctrl+b t to go back)", " (ctrl+b t para volver)") } else { "" };
    env::notify("Corral", &format!("{}{suffix}", label_of(&mode, code)));
    Ok(())
}

fn switch_usage() -> &'static str {
    tr("usage: corral switch [r|python]", "uso: corral switch [r|python]")
}

/// `corral switch [r|python]` (ctrl+b y): the console spot shows that language;
/// without one, the other. Starts it the first time.
pub fn switch(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", switch_usage());
        return Ok(());
    }
    let lang = match args.first().map(String::as_str) {
        None => {
            let ws = env::current_workspace();
            let panes = herdr::panes(ws.as_deref())?;
            if env::active_lang(ws.as_deref(), &panes) == "r" { "python" } else { "r" }
        }
        Some(l @ ("r" | "python")) => l,
        Some(_) => return Err(switch_usage().into()),
    };
    let (pane, started) = console::show(lang, true)?;
    crate::workspace::draft_follows(lang);
    let started = started || pane.as_deref().is_some_and(|p| console::ensure_running(p, lang));
    let note = if started { tr(": starting (the first time, uv prepares the environment)", ": iniciando (la primera vez uv prepara el entorno)") } else { "" };
    env::notify(tr("Console", "Consola"), &format!("{}{note}", console::lang_name(lang)));
    Ok(())
}
