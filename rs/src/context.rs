//! `corral context`: what an agent needs to know before it runs anything here, in one
//! call. The consoles (running or not, their session, version and folder, which one
//! shows, whether SQL works there) and the rules the user set for agents, as they apply
//! to each session. Built from the files Corral already writes and its settings, plus
//! herdr's list of panes: it starts no console and asks nothing over the network.
//! `--json` is for agents; without it, the same in words (the palette shows it).

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{obj, Value};
use std::path::Path;

fn usage() -> &'static str {
    tr("usage: corral context [--json]", "uso: corral context [--json]")
}

/// `corral context [--json]`; `corral _context` is the palette's popup.
pub fn run(args: &[String], popup: bool) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", usage());
        return Ok(());
    }
    if let Some(bad) = args.iter().find(|a| a.as_str() != "--json") {
        return Err(trf!("unknown option {bad}\n{}", "opción desconocida {bad}\n{}", usage()));
    }
    let json = args.iter().any(|a| a == "--json");
    // From the palette there is no terminal to print to: a popup has one.
    if !popup && !json && std::env::var_os("HERDR_PLUGIN_ID").is_some() {
        return crate::vars::open_popup("context", &[]);
    }
    let ctx = gather();
    if json {
        println!("{}", ctx.to_json());
    } else {
        print!("{}", words(&ctx));
    }
    if popup {
        println!("\n{}", tr("Press Enter to close.", "Presiona Enter para cerrar."));
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    Ok(())
}

/// The snapshot, as `--json` prints it.
fn gather() -> Value {
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref());
    let in_herdr = panes.is_ok();
    let showing = panes.ok().map(|p| env::active_lang(ws.as_deref(), &p));
    let cfg = env::config();
    let python_starts = cfg.get("python_console").str().is_some() || env::which("uv").is_some();
    let consoles = ["r", "python"].iter().map(|&lang| {
        let found = if in_herdr { console::session(lang, ws.as_deref()) } else { Err(String::new()) };
        let entry = match found {
            Ok((pane, s)) => {
                let rules = crate::profile::session_rules(Path::new(s.get("dir").str().unwrap_or("")), &s);
                let (sensitive, acc, out) = (rules.sensitive, rules.access, rules.output);
                let version = s.get(if lang == "python" { "python_version" } else { "r_version" }).clone();
                obj(vec![("running", true.into()), ("pane", pane.into()), ("session", s.get("id").clone()),
                         ("version", version), ("cwd", s.get("cwd").clone()),
                         ("showing", (showing.as_deref() == Some(lang)).into()),
                         // A session from before this field was written: unknown.
                         ("sql", s.get("sql").clone()),
                         ("sensitive", sensitive.into()), ("access", acc.into()), ("output", out.into()),
                         ("profile", rules.profile.into())])
            }
            Err(why) => {
                let mut v = vec![("running", false.into())];
                if lang == "python" {
                    // The first Python request starts it (uv, or the console you set).
                    v.push(("starts_on_request", python_starts.into()));
                }
                // Why not, when it adds something: what an agent should tell the user.
                if !why.is_empty() && !(lang == "python" && python_starts) {
                    v.push(("note", why.into()));
                }
                obj(v)
            }
        };
        (lang, entry)
    }).collect::<Vec<_>>();
    // The rules for a console that starts here now (the project's profile included).
    let here = crate::profile::folder_rules(&std::env::current_dir().unwrap_or_else(|_| env::home()));
    obj(vec![
        ("corral", env!("CARGO_PKG_VERSION").into()),
        ("herdr", in_herdr.into()),
        ("workspace", ws.map(Value::from).unwrap_or(Value::Null)),
        ("showing", showing.map(Value::from).unwrap_or(Value::Null)),
        ("consoles", obj(consoles)),
        ("agents", obj(vec![("access", here.access.into()), ("output", here.output.into()), ("profile", here.profile.into())])),
    ])
}

fn access_words(access: &str) -> &'static str {
    match access {
        "on" => tr("run code without asking", "ejecutan código sin preguntar"),
        "off" => tr("can't run code", "no pueden ejecutar código"),
        _ => tr("ask before each request", "preguntan antes de cada petición"),
    }
}

fn output_words(output: &str) -> &'static str {
    match output {
        "review" => tr("you review results before they see them", "revisas los resultados antes de que los vean"),
        "structure" => tr("see only the structure of results", "ven solo la estructura de los resultados"),
        _ => tr("see full results", "ven los resultados completos"),
    }
}

/// The same snapshot in words, for people.
fn words(ctx: &Value) -> String {
    let mut out = String::new();
    out += tr("What your agent sees (corral context; --json gives it to agents)\n\n",
              "Lo que ve tu agente (corral context; --json se lo da a los agentes)\n\n");
    if ctx.get("herdr") != &Value::Bool(true) {
        out += tr("Not in herdr: there are no Corral consoles here.\n", "Fuera de herdr: aquí no hay consolas de Corral.\n");
    }
    let home = env::home().to_string_lossy().to_string();
    for (lang, name) in [("r", "R"), ("python", "Python")] {
        let c = ctx.get("consoles").get(lang);
        let line = if c.get("running") == &Value::Bool(true) {
            let mut parts = vec![
                tr("running", "corriendo").to_string(),
                c.get("pane").str().unwrap_or("").to_string(),
                format!("{name} {}", c.get("version").str().unwrap_or("")),
                c.get("cwd").str().unwrap_or("").replace(&home, "~"),
            ];
            if c.get("showing") == &Value::Bool(true) {
                parts.push(tr("in view", "a la vista").into());
            }
            parts.push(match c.get("sql") {
                Value::Bool(true) => tr("SQL: yes", "SQL: sí").into(),
                Value::Bool(false) => tr("SQL: no (needs duckdb)", "SQL: no (falta duckdb)").into(),
                _ => tr("SQL: unknown", "SQL: no se sabe").into(),
            });
            if c.get("sensitive") == &Value::Bool(true) {
                parts.push(trf!("sensitive: agents {}, {}", "sensible: los agentes {}, {}",
                                access_words(c.get("access").str().unwrap_or("ask")),
                                output_words(c.get("output").str().unwrap_or("structure"))));
            }
            parts.retain(|p| !p.is_empty());
            parts.join(" · ")
        } else if c.get("starts_on_request") == &Value::Bool(true) {
            tr("not running · starts with the first Python request", "no está corriendo · arranca con la primera petición de Python").into()
        } else {
            c.get("note").str().map(String::from).unwrap_or_else(|| tr("not running", "no está corriendo").into())
        };
        out += &format!("  {name:<8} {line}\n");
    }
    let a = ctx.get("agents");
    out += &format!("  {:<8} {} · {}\n", tr("Agents", "Agentes"),
                    access_words(a.get("access").str().unwrap_or("ask")), output_words(a.get("output").str().unwrap_or("full")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sensitive_session_asks_and_gives_only_structure() {
        use crate::eval::policy;
        assert_eq!(policy("on", None, false), ("on", "full"));
        assert_eq!(policy("on", None, true), ("ask", "structure"));
        assert_eq!(policy("off", None, true), ("off", "structure"));
        assert_eq!(policy("ask", Some("review"), true), ("ask", "review"));
        assert_eq!(policy("ask", Some("structure"), false), ("ask", "structure"));
    }

    #[test]
    fn the_words_say_what_the_json_says() {
        let ctx = obj(vec![
            ("herdr", true.into()),
            ("consoles", obj(vec![
                ("r", obj(vec![("running", true.into()), ("pane", "w1:p3".into()), ("version", "4.6.1".into()),
                               ("cwd", "/proj".into()), ("showing", true.into()), ("sql", false.into()),
                               ("sensitive", true.into()), ("access", "ask".into()), ("output", "structure".into())])),
                ("python", obj(vec![("running", false.into()), ("starts_on_request", true.into())])),
            ])),
            ("agents", obj(vec![("access", "on".into()), ("output", "full".into())])),
        ]);
        let text = words(&ctx);
        for part in ["w1:p3", "R 4.6.1", "/proj", "SQL", "duckdb", "Python"] {
            assert!(text.contains(part), "{part} missing in:\n{text}");
        }
        assert_eq!(text.lines().count(), 5, "{text}");
    }
}
