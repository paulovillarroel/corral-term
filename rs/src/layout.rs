//! `corral toggle` (ctrl+b t) and `corral switch` (ctrl+b y): which pane gets
//! the area above the variables (script or console; agent or console in the
//! layout focused on code). herdr won't shrink a split below 10 %, so a
//! pane that has to disappear moves to the workspace's hidden tab, where it
//! keeps running, and comes back to its place later.

use crate::console;
use crate::env;
use crate::herdr;
use crate::json::{obj, Value};

pub const PARKING_TAB: &str = "corral (oculto)";
const SCRIPT_SHARE: f64 = 0.57;

pub fn hidden_tab(ws: Option<&str>) -> Result<(Option<String>, Option<String>), String> {
    let Some(ws) = ws else { return Ok((None, None)) };
    let tab = herdr::tabs(ws)?.into_iter().find(|t| t.get("label").str() == Some(PARKING_TAB))
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
            let mut d = obj(vec![("type", "new_tab".into()), ("label", PARKING_TAB.into())]);
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

/// Tab, split path, whether the script comes first, and the script's share.
fn script_split(script: &str, console: &str) -> Option<(String, Vec<bool>, bool, f64)> {
    let layout = herdr::call("layout.export", obj(vec![("pane_id", console.into())])).ok()?;
    let layout = layout.get("layout");
    let (path, ratio, script_first) = split_between(layout.get("root"), script, console, &mut Vec::new())?;
    let share = if script_first { ratio } else { 1.0 - ratio };
    Some((layout.get("tab_id").str()?.to_string(), path, script_first, share))
}

fn set_ratio(tab: &str, path: &[bool], ratio: f64) -> Result<(), String> {
    let path = Value::Arr(path.iter().map(|b| Value::Bool(*b)).collect());
    herdr::call("layout.set_split_ratio", obj(vec![("tab_id", tab.into()), ("path", path), ("ratio", ratio.into())]))?;
    Ok(())
}

fn label_of(mode: &str, code: bool) -> &'static str {
    match mode {
        "script" => "Script",
        "agente" => "Agente",
        "consola" => "Consola",
        _ if code => "Agente y consola",
        _ => "Script y consola",
    }
}

pub fn toggle(args: &[String]) -> Result<(), String> {
    let wanted = match args.first().map(String::as_str) {
        None => None,
        Some(m @ ("ambos" | "script" | "agente" | "consola")) => Some(m.to_string()),
        Some(_) => return Err("uso: corral toggle [ambos|script|agente|consola]".into()),
    };
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref())?;
    let lang = env::active_lang(ws.as_deref(), &panes);

    let path = env::layout_file();
    let mut saved = env::read_json(&path);
    let key = env::ws_key(ws.as_deref());
    let mut state = saved.get(&key).clone();

    // The pane above the console: the script, or the agent in the layout focused on code
    // (`corral layout --foco codigo`). Below, `script` is that pane, whichever it is.
    let code = state.get("foco").str() == Some("codigo");
    let upper_mode = if code { "agente" } else { "script" };
    if let Some(w) = wanted.as_deref().filter(|w| matches!(*w, "script" | "agente") && *w != upper_mode) {
        env::notify("Corral", &format!("{}: ya está a la izquierda", label_of(w, code)));
        return Ok(());
    }
    let upper = if code {
        state.get("agente").str().and_then(|id| panes.iter().find(|p| herdr::pane_id(p) == id))
    } else {
        herdr::labeled(&panes, "corral:script")
    };
    let (Some(script), Some(console)) = (upper, herdr::labeled(&panes, &format!("corral:{lang}"))) else {
        if code {
            env::notify("Sin agente", "No encontré el panel del agente; abre el espacio de nuevo desde la paleta (ctrl+b space)");
        } else {
            env::notify("Sin script", "Este workspace no tiene panel de script; usa ctrl+b a o 'corral script'");
        }
        return Ok(());
    };
    let (script_tab, console_tab) = (herdr::tab_id(script).to_string(), herdr::tab_id(console).to_string());
    let (script, console) = (herdr::pane_id(script).to_string(), herdr::pane_id(console).to_string());

    let (current, parked) = if script_tab == console_tab {
        ("ambos", None)
    } else {
        let recorded = state.get("parked").str().filter(|p| *p == script || *p == console).map(String::from);
        let parked = recorded.unwrap_or_else(|| {
            // The variables pane stays with the visible one.
            let home = herdr::labeled(&panes, "corral:vars").map(|p| herdr::tab_id(p).to_string());
            if Some(&console_tab) == home.as_ref() { script.clone() } else { console.clone() }
        });
        (if parked == console { upper_mode } else { "consola" }, Some(parked))
    };
    let focused = env::current_pane();
    let mode = wanted.unwrap_or_else(|| {
        if current != "ambos" {
            "ambos".into()
        } else if focused.as_deref() == Some(console.as_str()) {
            "consola".into()
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

    if mode != "ambos" {
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
    let suffix = if mode != "ambos" { " (ctrl+b t para volver)" } else { "" };
    env::notify("Corral", &format!("{}{suffix}", label_of(&mode, code)));
    Ok(())
}

/// `corral switch [r|python]` (ctrl+b y): the console spot shows that language;
/// without one, the other. Starts it the first time.
pub fn switch(args: &[String]) -> Result<(), String> {
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref())?;
    let current = env::active_lang(ws.as_deref(), &panes);
    let lang = match args.first().map(String::as_str) {
        None => if current == "r" { "python" } else { "r" },
        Some(l @ ("r" | "python")) => l,
        Some(_) => return Err("uso: corral switch [r|python]".into()),
    };
    let (pane, started) = console::show(lang, true)?;
    let started = started || pane.as_deref().is_some_and(|p| console::ensure_running(p, lang));
    let note = if started { ": iniciando (la primera vez uv prepara el entorno)" } else { "" };
    env::notify("Consola", &format!("{}{note}", console::lang_name(lang)));
    Ok(())
}
