//! `corral _approve`: the popups that ask you about an agent's request.
//!
//! - approve (before): the code exactly as it will run, when `corral config agents
//!   ask` is set or the session is marked sensitive (`corral_sensitive()`).
//!   Enter or y runs it; n, Esc or q refuses.
//! - review (after, `corral config agent_output review`): the result exactly as the
//!   agent would get it. Enter or y shares it; n, Esc or q keeps it, and the agent
//!   gets only its structure.
//!
//! - setting: an agent's shell (no terminal) asks to loosen the harness, with
//!   `corral config agents on` for instance. Enter or y allows it.
//!
//! The answer goes back in a file next to the request. Closing the popup counts as
//! no, and it closes itself when the request is answered or expires. Keys pressed in
//! its first half second don't count: they were meant for the pane you were in.

use crate::env;
use crate::i18n::tr;
use crate::json::Value;
use crate::term::{clip, out, Screen};
use crate::vars::Colors;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a new popup ignores keys: one meant for another pane can't answer it.
const GUARD: Duration = Duration::from_millis(600);

/// Who asks: the agent herdr sees in that pane, and the pane; or nobody's pane.
fn who(pane: Option<&str>) -> String {
    let Some(pane) = pane else { return tr("outside herdr (no pane)", "fuera de herdr (sin panel)").into() };
    let agent = crate::herdr::panes(None).ok()
        .and_then(|ps| ps.into_iter().find(|p| crate::herdr::pane_id(p) == pane))
        .and_then(|p| p.get("agent").str().map(String::from));
    let name = match agent.as_deref() {
        Some("claude") => "Claude Code".to_string(),
        Some("codex") => "Codex".to_string(),
        Some("gemini") => "Gemini CLI".to_string(),
        Some(other) => other.to_string(),
        None => return trf!("pane {pane}", "panel {pane}"),
    };
    trf!("{name} · pane {pane}", "{name} · panel {pane}")
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

pub fn run(_args: &[String]) -> Result<(), String> {
    let req = PathBuf::from(std::env::var("CORRAL_REQUEST").map_err(|_| tr("CORRAL_REQUEST is missing", "falta CORRAL_REQUEST"))?);
    let r = env::read_json(&req);
    let decision = PathBuf::from(r.get("decision").str().ok_or(tr("the request has no decision file", "la petición no tiene archivo de respuesta"))?);
    let reviewing = r.get("kind").str() == Some("review");
    let setting = r.get("setting").clone();
    let changing = !setting.is_null();
    let lang = if r.get("lang").str() == Some("python") { "Python" } else { "R" };
    let agent = who(r.get("agent_pane").str());
    // What goes on screen: the code to approve, the result to review, or the setting.
    let mut body = if changing {
        format!("{}: {} → {}\n\n{}", setting.get("key").str().unwrap_or(""), setting.get("from").str().unwrap_or(""),
                setting.get("to").str().unwrap_or(""), setting.get("meaning").str().unwrap_or(""))
    } else {
        r.get(if reviewing { "text" } else { "code" }).str().unwrap_or("").to_string()
    };
    let plots = r.get("plots").num().unwrap_or(0.0) as usize;
    if reviewing && plots > 0 {
        body += &format!("\n\n{}", trf!("+ {plots} new plot(s), shared as PNG if you share this", "+ {plots} gráfico(s) nuevo(s), que se comparten como PNG si compartes esto"));
    }
    let output = match r.get("output").str() {
        Some("structure") => tr("it will only see the structure of the result", "solo verá la estructura del resultado"),
        Some("review") => tr("you'll see the result before it does", "verás el resultado antes que el agente"),
        _ if r.get("restricted") == &Value::Bool(true) => tr("it will only see the structure of the result", "solo verá la estructura del resultado"),
        _ => tr("it will see what the code prints, and new plots", "verá lo que imprima el código, y los gráficos nuevos"),
    };
    let lines: Vec<&str> = body.lines().collect();

    let answer = |yes: bool| {
        let word = match (reviewing, changing, yes) {
            (_, true, true) => "allow",
            (_, true, false) => "refuse",
            (false, _, true) => "run",
            (false, _, false) => "refuse",
            (true, _, true) => "share",
            (true, _, false) => "keep",
        };
        let _ = std::fs::write(&decision, word);
    };
    let Some(scr) = Screen::enter(false) else {
        answer(false);
        return Err(tr("no terminal to ask in", "no hay una terminal donde preguntar").into());
    };
    let opened = Instant::now();
    let mut top = 0usize;
    let mut drawn = (0, 0, usize::MAX);
    loop {
        let (cols, rows) = scr.size();
        let area = rows.saturating_sub(7).max(1);
        top = top.min(lines.len().saturating_sub(area));
        if drawn != (cols, rows, top) {
            drawn = (cols, rows, top);
            let c = Colors::load();
            let w = cols.saturating_sub(4);
            let title = match (reviewing, lang) {
                _ if changing => tr("An agent wants to change a Corral setting", "Un agente quiere cambiar un ajuste de Corral"),
                (false, "R") => tr("An agent wants to run this in your R console", "Un agente quiere ejecutar esto en tu consola de R"),
                (false, _) => tr("An agent wants to run this in your Python console", "Un agente quiere ejecutar esto en tu consola de Python"),
                (true, _) => tr("Share this result with the agent?", "¿Compartir este resultado con el agente?"),
            };
            let sub = if changing {
                trf!("from {agent} · it asked from a shell without a terminal, so you decide",
                     "desde {agent} · lo pidió desde una shell sin terminal, así que decides tú")
            } else if reviewing {
                trf!("from pane {agent} · exactly what it would get; keep it and it gets only the structure",
                     "desde el panel {agent} · lo que recibiría; si te lo guardas, recibe solo la estructura")
            } else {
                trf!("from pane {agent} · {output}", "desde el panel {agent} · {output}")
            };
            let mut f = String::from("\x1b[2J");
            f += &format!("\x1b[2;3H{}{BOLD}{}{RESET}", c.accent, clip(title, w));
            f += &format!("\x1b[3;3H{}{}{RESET}", c.muted, clip(&sub, w));
            let numw = lines.len().to_string().len();
            for (i, l) in lines.iter().skip(top).take(area).enumerate() {
                let text = clip(&l.replace('\t', "    "), w.saturating_sub(if reviewing || changing { 0 } else { numw + 2 }));
                if reviewing || changing {
                    f += &format!("\x1b[{};3H{}{text}{RESET}", 5 + i, c.text);
                } else {
                    f += &format!("\x1b[{};3H{}{:>numw$}{RESET}  {}{text}{RESET}", 5 + i, c.dim, top + i + 1, c.text);
                }
            }
            let more = if lines.len() > area { tr(" · ↑↓ scroll", " · ↑↓ desplazar") } else { "" };
            let (yes, no) = if changing {
                (tr("allow it", "permitirlo"), tr("refuse", "rechazarlo"))
            } else if reviewing {
                (tr("share it", "compartirlo"), tr("keep it to myself", "guardármelo"))
            } else {
                (tr("run it", "ejecutarlo"), tr("refuse", "rechazarlo"))
            };
            f += &format!("\x1b[{};3H{}{BOLD}Enter{RESET}{} {yes}   {}{BOLD}n{RESET}{} {no}{more}{RESET}", rows.saturating_sub(1),
                          c.text, c.muted, c.text, c.muted);
            out(&f);
        }
        let Some(key) = scr.key(500) else {
            if decision.exists() {
                return Ok(()); // answered or expired: nothing left to ask
            }
            continue;
        };
        let answering = matches!(key.as_str(), "enter" | "y" | "Y" | "n" | "N" | "esc" | "q" | "quit");
        if answering && opened.elapsed() < GUARD {
            continue; // typed for the pane you were in, before this popup showed up
        }
        match key.as_str() {
            "enter" | "y" | "Y" => {
                answer(true);
                return Ok(());
            }
            "n" | "N" | "esc" | "q" | "quit" => {
                answer(false);
                return Ok(());
            }
            "down" | "j" => top += 1,
            "up" | "k" => top = top.saturating_sub(1),
            "pgdn" | " " => top += area_of(&scr),
            "pgup" => top = top.saturating_sub(area_of(&scr)),
            _ => {}
        }
    }
}

fn area_of(scr: &Screen) -> usize {
    scr.size().1.saturating_sub(7).max(1)
}
