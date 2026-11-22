//! `corral send`: code from the editor (Ctrl+R, F5), a selection or the
//! clipboard to the console, which is brought into view (and started) if needed.

use crate::console;
use crate::env;
use crate::herdr;
use crate::json::Value;
use std::io::{IsTerminal, Read};

const USAGE: &str = "uso: corral send [--lang r|python] [--source] [--name ARCHIVO] [--clipboard] [CÓDIGO…]";

pub fn run(args: &[String], _all: &[String]) -> Result<(), String> {
    let mut parts: Vec<String> = Vec::new();
    let (mut lang, mut source, mut name, mut clipboard) = (None::<String>, false, None::<String>, false);
    let mut it = args.iter();
    let mut rest = false;
    while let Some(a) = it.next() {
        if rest {
            parts.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--" => rest = true,
            "--lang" => lang = it.next().cloned(),
            "--source" => source = true,
            "--name" => name = it.next().cloned(),
            "--clipboard" => clipboard = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with("--") => return Err(format!("opción desconocida {s}\n{USAGE}")),
            _ => parts.push(a.clone()),
        }
    }
    if lang.as_deref().is_some_and(|l| l != "r" && l != "python") {
        return Err(USAGE.into());
    }

    // Sources, in order: arguments, stdin, the herdr selection, the clipboard. herdr
    // copies a selection to the clipboard and clears it on mouse release, so from a
    // keybinding the clipboard is the usual source.
    let from_plugin = std::env::var_os("HERDR_PLUGIN_ID").is_some();
    let (mut text, mut origin) = (parts.join(" "), "argumentos");
    if text.trim().is_empty() && !from_plugin && !std::io::stdin().is_terminal() {
        text.clear();
        let _ = std::io::stdin().read_to_string(&mut text);
        origin = "stdin";
    }
    if text.trim().is_empty() {
        text = env::plugin_context().get("selected_text").str().unwrap_or("").to_string();
        origin = "selección";
    }
    if text.trim().is_empty() && (from_plugin || clipboard) {
        text = env::read_clipboard();
        origin = "portapapeles";
    }
    if text.trim().is_empty() {
        env::notify("Nada que enviar", "Selecciona código (se copia solo) y vuelve a intentar");
        return Ok(());
    }
    let text = text.trim_end().to_string();

    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref())?;
    let current = env::active_lang(ws.as_deref(), &panes);
    let lang = lang.unwrap_or_else(|| current.clone());
    // The usual case: the console is there, in view and running.
    let target = herdr::labeled(&panes, &console::label(&lang)).map(|t| herdr::pane_id(t).to_string());
    let main = main_tab(&panes);
    let in_view = |t: &str| lang == current || main.is_none()
        || panes.iter().any(|p| herdr::pane_id(p) == t && main.as_deref() == Some(herdr::tab_id(p)));
    let pane = match target {
        Some(t) if in_view(&t) && env::session_for(&t).is_some() => {
            if lang != current {
                let path = env::layout_file();
                let mut saved = env::read_json(&path);
                let key = env::ws_key(ws.as_deref());
                let mut state = saved.get(&key).clone();
                state.set("active", lang.as_str().into());
                saved.set(&key, state);
                env::write_json(&path, &saved);
            }
            t
        }
        // Otherwise bring it into view, and start it if it isn't there or was quit.
        _ => console::ready(&lang).map_err(|e| {
            env::notify("Sin consola", &e);
            e
        })?,
    };

    let jupyter = console::is_jupyter(&pane, &lang);
    if source {
        // A whole file: run a copy instead of pasting every line.
        let run_dir = env::state_dir().join("run");
        let _ = std::fs::create_dir_all(&run_dir);
        env::remove_old(&run_dir, "R", 86400);
        env::remove_old(&run_dir, "py", 86400);
        let label = name.as_deref().map(|n| n.rsplit('/').next().unwrap_or(n)).unwrap_or("script").to_string();
        let stamp = format!("{}-{}", now_secs(), env::random_hex(3));
        let command = if lang == "python" {
            let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
            let file = run_dir.join(format!("{stamp}-{}", safe.trim_end_matches(".py"))).with_extension("py");
            std::fs::write(&file, format!("{text}\n")).map_err(|e| e.to_string())?;
            format!("%run -i \"{}\"", file.display())
        } else {
            let file = run_dir.join(format!("{stamp}.R"));
            std::fs::write(&file, format!("{text}\n")).map_err(|e| e.to_string())?;
            format!("source(\"{}\", echo = TRUE, max.deparse.length = Inf, encoding = \"UTF-8\")  # {label}", file.display())
        };
        herdr::send_text(&pane, &format!("{command}\r"))?;
    } else if jupyter {
        // IPython or Ark's console: one pasted block, run with Enter (two
        // Enters when it ends inside an indented block).
        let last = text.lines().last().unwrap_or("");
        let enter = if last.starts_with([' ', '\t']) { "\r\r" } else { "\r" };
        herdr::send_text(&pane, &format!("\x1b[200~{text}\x1b[201~{enter}"))?;
    } else {
        herdr::send_text(&pane, &format!("{text}\r"))?;
    }
    let lines = if source { 1 } else { text.lines().count().max(1) };
    eprintln!("corral: enviado a {pane} desde {origin} ({lines} línea{})", if lines == 1 { "" } else { "s" });
    Ok(())
}

/// The tab of the Corral layout: where the variables pane (or the script) is.
pub fn main_tab(panes: &[Value]) -> Option<String> {
    ["corral:vars", "corral:script"].iter()
        .find_map(|l| herdr::labeled(panes, l))
        .map(|p| herdr::tab_id(p).to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
