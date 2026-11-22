//! `corral _approve`: the popups that ask you about an agent's request.
//!
//! - approve (before): the code exactly as it will run, when `corral config agents
//!   ask` is set or the session is marked sensitive (`corral_sensitive()`).
//!   Enter or y runs it; n, Esc or q refuses.
//! - review (after, `corral config agent_output review`): the result exactly as the
//!   agent would get it. Enter or y shares it; n, Esc or q keeps it, and the agent
//!   gets only its structure.
//!
//! The answer goes back to `corral eval` in a file next to the request. Closing the
//! popup counts as no, and it closes itself when the request is answered or expires.

use crate::env;
use crate::i18n::tr;
use crate::json::Value;
use crate::term::{clip, out, Screen};
use crate::vars::Colors;
use std::path::PathBuf;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

pub fn run(_args: &[String]) -> Result<(), String> {
    let req = PathBuf::from(std::env::var("CORRAL_REQUEST").map_err(|_| tr("CORRAL_REQUEST is missing", "falta CORRAL_REQUEST"))?);
    let r = env::read_json(&req);
    let decision = PathBuf::from(r.get("decision").str().ok_or(tr("the request has no decision file", "la petición no tiene archivo de respuesta"))?);
    let reviewing = r.get("kind").str() == Some("review");
    let lang = if r.get("lang").str() == Some("python") { "Python" } else { "R" };
    let agent = r.get("agent_pane").str().unwrap_or("?").to_string();
    // What goes on screen: the code to approve, or the result to review.
    let mut body = r.get(if reviewing { "text" } else { "code" }).str().unwrap_or("").to_string();
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
        let word = match (reviewing, yes) {
            (false, true) => "run",
            (false, false) => "refuse",
            (true, true) => "share",
            (true, false) => "keep",
        };
        let _ = std::fs::write(&decision, word);
    };
    let Some(scr) = Screen::enter(false) else {
        answer(false);
        return Err(tr("no terminal to ask in", "no hay una terminal donde preguntar").into());
    };
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
                (false, "R") => tr("An agent wants to run this in your R console", "Un agente quiere ejecutar esto en tu consola de R"),
                (false, _) => tr("An agent wants to run this in your Python console", "Un agente quiere ejecutar esto en tu consola de Python"),
                (true, _) => tr("Share this result with the agent?", "¿Compartir este resultado con el agente?"),
            };
            let sub = if reviewing {
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
                let text = clip(&l.replace('\t', "    "), w.saturating_sub(if reviewing { 0 } else { numw + 2 }));
                if reviewing {
                    f += &format!("\x1b[{};3H{}{text}{RESET}", 5 + i, c.text);
                } else {
                    f += &format!("\x1b[{};3H{}{:>numw$}{RESET}  {}{text}{RESET}", 5 + i, c.dim, top + i + 1, c.text);
                }
            }
            let more = if lines.len() > area { tr(" · ↑↓ scroll", " · ↑↓ desplazar") } else { "" };
            let (yes, no) = if reviewing {
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
