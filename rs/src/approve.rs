//! `corral _approve`: the popup that asks you before an agent's code runs in your
//! console. It opens when `corral config agents ask` is set, or when the session is
//! marked sensitive (`corral_sensitive()`). It shows the code exactly as it will
//! run; the answer goes back to `corral eval` in a file next to the request:
//! Enter or y runs it, n, Esc or q refuses. Closing the popup refuses too, and the
//! popup closes itself when the request is answered or expires elsewhere.

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
    let code = r.get("code").str().unwrap_or("").to_string();
    let lang = if r.get("lang").str() == Some("python") { "Python" } else { "R" };
    let agent = r.get("agent_pane").str().unwrap_or("?").to_string();
    let restricted = r.get("restricted") == &Value::Bool(true);
    let lines: Vec<&str> = code.lines().collect();

    let answer = |run: bool| {
        let _ = std::fs::write(&decision, if run { "run" } else { "refuse" });
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
            let title = if lang == "R" {
                tr("An agent wants to run this in your R console", "Un agente quiere ejecutar esto en tu consola de R")
            } else {
                tr("An agent wants to run this in your Python console", "Un agente quiere ejecutar esto en tu consola de Python")
            };
            let output = if restricted {
                tr("it will only see the structure of the result (restricted output)", "solo verá la estructura del resultado (salida restringida)")
            } else {
                tr("it will see what the code prints, and new plots", "verá lo que imprima el código, y los gráficos nuevos")
            };
            let mut f = String::from("\x1b[2J");
            f += &format!("\x1b[2;3H{}{BOLD}{}{RESET}", c.accent, clip(title, w));
            f += &format!("\x1b[3;3H{}{}{RESET}", c.muted, clip(&trf!("from pane {agent} · {output}", "desde el panel {agent} · {output}"), w));
            let numw = lines.len().to_string().len();
            for (i, l) in lines.iter().skip(top).take(area).enumerate() {
                f += &format!("\x1b[{};3H{}{:>numw$}{RESET}  {}{}{RESET}", 5 + i, c.dim, top + i + 1, c.text,
                              clip(&l.replace('\t', "    "), w.saturating_sub(numw + 2)));
            }
            let more = if lines.len() > area { tr(" · ↑↓ scroll", " · ↑↓ desplazar") } else { "" };
            f += &format!("\x1b[{};3H{}{BOLD}Enter{RESET}{} {}   {}{BOLD}n{RESET}{} {}{}{RESET}", rows.saturating_sub(1),
                          c.text, c.muted, tr("run it", "ejecutarlo"), c.text, c.muted, tr("refuse", "rechazarlo"), more);
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
