//! The local audit log: every request an agent makes with `corral eval` (what it
//! ran, in which console, whether you approved it, what came back), one JSON line
//! each in ~/.local/state/corral/audit.jsonl. Nothing leaves your machine.
//! `corral audit` shows the latest ones; `--json` prints the lines as they are.

use crate::env;
use crate::i18n::tr;
use crate::json::{self, Value};
use std::io::Write;
use std::path::PathBuf;

pub fn file() -> PathBuf {
    env::state_dir().join("audit.jsonl")
}

/// One line in the log. `time` is added here; failures to write never stop the request.
pub fn record(mut entry: Value) {
    let (y, mo, d, h, mi, s) = env::local_now();
    entry.set("time", format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}").into());
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", entry.to_json());
    }
}

fn usage() -> &'static str {
    tr("usage: corral audit [--last N] [--json]", "uso: corral audit [--last N] [--json]")
}

/// `corral audit [--last N] [--json]`.
pub fn run(args: &[String]) -> Result<(), String> {
    let (mut last, mut raw) = (20usize, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--last" => last = it.next().and_then(|n| n.parse().ok()).ok_or(usage())?,
            "--json" => raw = true,
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => return Err(trf!("unknown option {other}\n{}", "opción desconocida {other}\n{}", usage())),
        }
    }
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let shown = &lines[lines.len().saturating_sub(last)..];
    if raw {
        for l in shown {
            println!("{l}");
        }
        return Ok(());
    }
    if shown.is_empty() {
        println!("{}", tr("No agent requests yet.", "Todavía no hay peticiones de agentes."));
        return Ok(());
    }
    for l in shown {
        let e = json::parse(l).unwrap_or(Value::Null);
        let outcome = match (e.get("decision").str(), e.get("status").str()) {
            (Some("ran"), Some(st)) => st.to_string(),
            (Some(d), _) => d.to_string(),
            _ => "?".into(),
        };
        let code = e.get("code").str().unwrap_or("").replace('\n', " ⏎ ");
        let code: String = if code.chars().count() > 70 { code.chars().take(69).collect::<String>() + "…" } else { code };
        println!("{}  {:<12} {:<6} {:<8} {}", e.get("time").str().unwrap_or(""), outcome,
                 e.get("lang").str().unwrap_or(""), e.get("agent_pane").str().unwrap_or("-"), code);
    }
    println!("\n{}", trf!("{} in all · {}", "{} en total · {}", lines.len(), file().display()));
    Ok(())
}
