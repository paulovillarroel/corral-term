//! `corral eval`: an agent (Claude) runs R or Python code in the user's console
//! and gets back what it printed. The code goes to a file, the console runs it
//! with `.corral_eval()` / `_corral_eval()`, which writes the output and a
//! status file; new plots are saved as PNG. A console that isn't there, isn't
//! in view or was quit is started or brought into view first.
//!
//! What agents may do is the user's call, and Corral enforces it here, whatever the
//! agent was told: `corral config agents off|ask|on` (off refuses, ask shows every
//! request in a popup first, see approve.rs, and is the default; on runs it without
//! asking) and `corral config agent_output
//! full|review|structure` (review shows you the result before the agent gets it;
//! structure returns no values, only whether it worked and the shape of the
//! objects). A session marked with corral_sensitive() is always ask, plus review or
//! structure. Every request goes to the local audit log (audit.rs).

use crate::audit;
use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{self, obj, Value};
use crate::mirador::{plots_json, plots_raw};
use std::fs::File;
use std::io::{IsTerminal, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;

struct Opts {
    code: Vec<String>,
    lang: String,
    file: Option<String>,
    timeout: f64,
    wait_idle: f64,
    max_chars: usize,
    check: bool,
    replace: bool,
}

fn usage() -> &'static str {
    tr("usage: corral eval [--lang r|python] [--file FILE] [--timeout S] [--wait-idle S] [--max-chars N] [--check] [--replace] [CODE…]",
       "uso: corral eval [--lang r|python] [--file ARCHIVO] [--timeout S] [--wait-idle S] [--max-chars N] [--check] [--replace] [CÓDIGO…]")
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts { code: Vec::new(), lang: "r".into(), file: None, timeout: 300.0, wait_idle: 20.0,
                       max_chars: 20000, check: false, replace: false };
    let mut it = args.iter();
    let mut rest = false;
    while let Some(a) = it.next() {
        if rest {
            o.code.push(a.clone());
            continue;
        }
        let mut value = |name: &str| -> String {
            it.next().cloned().unwrap_or_else(|| {
                eprintln!("corral eval: {}", trf!("{name} needs a value", "falta el valor de {name}"));
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--" => rest = true,
            "--lang" => o.lang = value("--lang"),
            "--file" => o.file = Some(value("--file")),
            "--timeout" => o.timeout = value("--timeout").parse().unwrap_or(300.0),
            "--wait-idle" => o.wait_idle = value("--wait-idle").parse().unwrap_or(20.0),
            "--max-chars" => o.max_chars = value("--max-chars").parse().unwrap_or(20000),
            "--check" => o.check = true,
            "--replace" => o.replace = true,
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0)
            }
            s if s.starts_with("--") => {
                let usage = usage();
                fail(2, &trf!("unknown option {s}\n{usage}", "opción desconocida {s}\n{usage}"))
            }
            _ => o.code.push(a.clone()),
        }
    }
    if o.lang != "r" && o.lang != "python" {
        fail(2, usage());
    }
    o
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn clip_output(text: &str, limit: usize, full: &Path) -> String {
    let n = text.chars().count();
    if n <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit / 2).collect();
    let tail: String = text.chars().skip(n - limit / 2).collect();
    let full = full.display();
    trf!("{head}\n\n[corral] … output cut ({n} characters); full output in {full} …\n\n{tail}",
         "{head}\n\n[corral] … salida recortada ({n} caracteres); completa en {full} …\n\n{tail}")
}

/// Whether the output holds anything besides the console echoing the code back
/// (`> ` and `+ ` in R, `>>> ` and `... ` in Python): an assignment prints nothing
/// worth reviewing.
fn prints_something(output: &str, code: &str) -> bool {
    let code_lines: Vec<&str> = code.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    output.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).any(|l| {
        let rest = ["> ", "+ ", ">>> ", "... "].iter().find_map(|p| l.strip_prefix(p));
        !matches!(rest, Some(r) if code_lines.contains(&r.trim()))
    })
}

fn plot_ids(sid: &str) -> Vec<String> {
    plots_json(sid, "plots").get("plots").arr().iter().filter_map(|p| match p.get("id") {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(format!("{}", *n as i64)),
        _ => None,
    }).collect()
}

fn fail(code: i32, msg: &str) -> ! {
    env::notify("Error", msg);
    std::process::exit(code)
}

/// off, ask (the default) or on: `corral config agents`. Anything but an explicit
/// `on` asks, so a missing or mistyped setting never runs code unseen.
fn access_mode() -> &'static str {
    access_from(env::config().get("agents").str())
}

fn access_from(setting: Option<&str>) -> &'static str {
    match setting {
        Some("off") => "off",
        Some("on") => "on",
        _ => "ask",
    }
}

/// What every audit line of this request carries.
fn audit_entry(o: &Opts, code: &str, access: &str, output: &str, decision: &str) -> Value {
    obj(vec![
        ("decision", decision.into()),
        ("lang", o.lang.as_str().into()),
        ("agent_pane", std::env::var("HERDR_PANE_ID").ok().map(Value::from).unwrap_or(Value::Null)),
        ("workspace", env::current_workspace().map(Value::from).unwrap_or(Value::Null)),
        ("access", access.into()),
        ("output", output.into()),
        ("code", code.into()),
    ])
}

/// Why the user couldn't be asked, or didn't answer.
enum NoAnswer {
    Busy,
    Failed(String),
    Timeout,
}

/// Open one of Corral's question popups (approve.rs) and wait for the answer:
/// true for yes (run it, share it). herdr shows one popup at a time, so while the
/// user has another one open (the palette, a viewer), wait for it instead of failing.
fn ask_user(edir: &Path, rid: &str, entrypoint: &str, mut request: Value) -> Result<bool, NoAnswer> {
    let path = edir.join(format!("{rid}.{entrypoint}.json"));
    let decision = edir.join(format!("{rid}.{entrypoint}.decision"));
    request.set("kind", entrypoint.into());
    request.set("agent_pane", std::env::var("HERDR_PANE_ID").ok().map(Value::from).unwrap_or(Value::Null));
    request.set("decision", decision.to_string_lossy().to_string().into());
    env::write_json(&path, &request);
    let end = Instant::now() + Duration::from_secs(APPROVAL_SECS);
    loop {
        match crate::vars::open_popup(entrypoint, &[("CORRAL_REQUEST", &path.to_string_lossy())]) {
            Ok(()) => break,
            Err(e) if e.to_lowercase().contains("busy") && Instant::now() < end => std::thread::sleep(Duration::from_secs(1)),
            Err(e) if e.to_lowercase().contains("busy") => return Err(NoAnswer::Busy),
            Err(e) => return Err(NoAnswer::Failed(e)),
        }
    }
    while Instant::now() < end {
        if let Ok(answer) = std::fs::read_to_string(&decision) {
            return Ok(matches!(answer.trim(), "run" | "share"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::write(&decision, "expired"); // the popup sees it and closes
    Err(NoAnswer::Timeout)
}

/// Ask before the code runs; true when the user lets it run.
fn approve(edir: &Path, rid: &str, code: &str, lang: &str, output_mode: &str) -> Result<bool, String> {
    let request = obj(vec![("code", code.into()), ("lang", lang.into()), ("output", output_mode.into())]);
    ask_user(edir, rid, "approve", request).map_err(|why| match why {
        NoAnswer::Busy => tr("another Corral window stayed open in herdr, so the user couldn't be asked; nothing ran",
                             "otra ventana de Corral siguió abierta en herdr, así que no pude preguntarle al usuario; no se ejecutó nada").into(),
        NoAnswer::Failed(e) => trf!("couldn't ask the user ({e}); nothing ran", "no pude preguntarle al usuario ({e}); no se ejecutó nada"),
        NoAnswer::Timeout => trf!("the user didn't answer within {APPROVAL_SECS} s; nothing ran", "el usuario no respondió en {APPROVAL_SECS} s; no se ejecutó nada"),
    })
}

/// Review mode: show the user the result before the agent gets it; true to share it.
/// No answer (or no way to ask) keeps it: the agent gets only the structure.
fn review(edir: &Path, rid: &str, text: &str, lang: &str, plots: usize) -> bool {
    let request = obj(vec![("text", text.into()), ("lang", lang.into()), ("plots", (plots as f64).into())]);
    ask_user(edir, rid, "review", request).unwrap_or(false)
}

const APPROVAL_SECS: u64 = 300;

/// Restricted output: whether it worked, the error's first line, how many warnings,
/// and the shape of the session's objects. No printed output, no values, no plots.
fn restricted_report(sdir: &Path, since: SystemTime, output: &str, result: &Value) -> String {
    let mut out = Vec::new();
    let printed = output.lines().count();
    out.push(trf!("[corral] restricted output: the agent doesn't see values ({printed} lines printed in the user's console)",
                  "[corral] salida restringida: el agente no ve valores ({printed} líneas impresas en la consola del usuario)"));
    if result.get("status").str() == Some("error") {
        let first = result.get("error").str().unwrap_or("").lines().next().unwrap_or("").to_string();
        let first: String = first.chars().take(200).collect();
        out.push(trf!("[corral] error: {first}", "[corral] error: {first}"));
    }
    let warnings = result.get("warnings").arr().len();
    if warnings > 0 {
        out.push(trf!("[corral] {warnings} warning(s), not shown", "[corral] {warnings} advertencia(s), sin mostrar"));
    }
    // The session describes its objects right after the command; give it a moment.
    let vars_file = sdir.join("vars.json");
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        let fresh = std::fs::metadata(&vars_file).and_then(|m| m.modified()).is_ok_and(|t| t >= since);
        if fresh {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let vars = env::read_json(&vars_file);
    let shapes: Vec<String> = vars.get("vars").arr().iter().take(40).map(|v| {
        let (name, kind, class) = (v.get("name").str().unwrap_or("?"), v.get("kind").str().unwrap_or(""), v.get("class").str().unwrap_or(""));
        // Only dimensions and signatures: a value's preview can be the data itself.
        match kind {
            "data" | "function" => format!("{name} ({class}, {})", v.get("preview").str().unwrap_or("")),
            _ => format!("{name} ({class})"),
        }
    }).collect();
    if !shapes.is_empty() {
        out.push(trf!("[corral] objects: {}", "[corral] objetos: {}", shapes.join("; ")));
    }
    out.join("\n")
}

/// Requests and their results older than a day go away; the audit log keeps the
/// record. `last-plots.json` stays: `--replace` reads it.
fn remove_old_requests(edir: &Path) {
    for ext in ["R", "py", "out", "deadline", "decision"] {
        env::remove_old(edir, ext, 86400);
    }
    let keep = edir.join("last-plots.json");
    for p in std::fs::read_dir(edir).into_iter().flatten().flatten().map(|e| e.path()) {
        let old = p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 86400);
        if old && p != keep && p.extension().is_some_and(|x| x == "json") {
            let _ = std::fs::remove_file(&p);
        }
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let o = parse(args);
    let ws = env::current_workspace();
    if o.check {
        let (pane, s) = console::session(&o.lang, ws.as_deref()).unwrap_or_else(|e| fail(1, &e));
        let panes = herdr::panes(ws.as_deref()).unwrap_or_default();
        let version = s.get(if o.lang == "python" { "python_version" } else { "r_version" }).clone();
        println!("{}", obj(vec![("console", pane.into()), ("lang", o.lang.as_str().into()), ("session", s.get("id").clone()),
                               ("version", version), ("cwd", s.get("cwd").clone()),
                               ("active", (env::active_lang(ws.as_deref(), &panes) == o.lang).into())]).to_json());
        return Ok(());
    }

    let code = if !o.code.is_empty() {
        o.code.join(" ")
    } else if let Some(f) = &o.file {
        std::fs::read_to_string(f).unwrap_or_else(|e| fail(1, &format!("{f}: {e}")))
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s
    } else {
        fail(1, tr("pass the code as an argument, with --file or on stdin", "pasa el código como argumento, con --file o por stdin"))
    };
    if code.trim().is_empty() {
        fail(1, tr("no code to run", "no hay código que ejecutar"));
    }
    let access = access_mode();
    if access == "off" {
        audit::record(audit_entry(&o, &code, access, "-", "blocked"));
        println!("{}", tr("[corral] agents can't run code in the user's console (corral config agents off). Nothing ran; ask the user.",
                          "[corral] los agentes no pueden ejecutar código en la consola del usuario (corral config agents off). No se ejecutó nada; pregúntale al usuario."));
        std::process::exit(3);
    }

    // The console has to be there, running, and in view: the usual case needs nothing;
    // otherwise it is brought into view, or started, first.
    let panes = herdr::panes(ws.as_deref())?;
    let current = env::active_lang(ws.as_deref(), &panes);
    let found = herdr::labeled(&panes, &console::label(&o.lang)).map(|t| herdr::pane_id(t).to_string());
    let pane = match found {
        Some(p) if o.lang == current && env::session_for(&p).is_some() => p,
        _ => console::ready(&o.lang).unwrap_or_else(|e| fail(1, &e)),
    };
    let session = env::session_for(&pane).unwrap_or_else(|| {
        fail(1, &console::session(&o.lang, ws.as_deref()).err()
            .unwrap_or_else(|| tr("the console does not have Corral active", "la consola no tiene Corral activo").into()))
    });
    let sdir = PathBuf::from(session.get("dir").str().unwrap_or(""));
    let sid = session.get("id").str().unwrap_or("").to_string();
    let edir = sdir.join("eval");
    std::fs::create_dir_all(&edir).map_err(|e| e.to_string())?;
    remove_old_requests(&edir);

    // One request at a time per console: several agents queue instead of interleaving.
    let lock = File::create(edir.join(".lock")).map_err(|e| e.to_string())?;
    let give_up = Instant::now() + Duration::from_secs_f64(o.wait_idle + o.timeout);
    // SAFETY: flock on a file we keep open; released when the process ends.
    while unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        if Instant::now() > give_up {
            fail(2, tr("another request is still running in this console", "otra petición sigue corriendo en esta consola"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let sensitive = sdir.join("sensitive").exists();
    // full, review or structure; a sensitive session never gets full.
    let output_mode = match (env::config().get("agent_output").str(), sensitive) {
        (Some("review"), _) => "review",
        (Some("structure"), _) | (_, true) => "structure",
        _ => "full",
    };
    let access = if sensitive { "ask" } else { access };
    let secs = now() as u64;
    let rid = format!("{:02}{:02}{:02}-{}", secs / 3600 % 24, secs / 60 % 60, secs % 60, env::random_hex(3));
    if access == "ask" {
        match approve(&edir, &rid, &code, &o.lang, output_mode) {
            Ok(true) => {}
            Ok(false) => {
                audit::record(audit_entry(&o, &code, access, output_mode, "refused"));
                println!("{}", tr("[corral] the user refused this request: nothing ran. Don't run it again unless they ask for it.",
                                  "[corral] el usuario rechazó esta petición: no se ejecutó nada. No la repitas salvo que te lo pida."));
                std::process::exit(3);
            }
            Err(e) => {
                audit::record(audit_entry(&o, &code, access, output_mode, "not approved"));
                println!("[corral] {e}");
                std::process::exit(3);
            }
        }
    }

    if let Err(e) = console::wait_for_prompt(&pane, o.wait_idle, &o.lang, Some(&edir)) {
        fail(2, &e);
    }
    let ext = if o.lang == "python" { "py" } else { "R" };
    let body = if code.ends_with('\n') { code.clone() } else { format!("{code}\n") };
    std::fs::write(edir.join(format!("{rid}.{ext}")), body).map_err(|e| e.to_string())?;
    // If the console only gets to it after the agent gave up, it must not run.
    std::fs::write(edir.join(format!("{rid}.deadline")), format!("{}", now() + o.timeout)).map_err(|e| e.to_string())?;
    let before = plots_json(&sid, "state");
    let started = SystemTime::now();
    let call = if o.lang == "python" { format!("_corral_eval(\"{rid}\")") } else { format!(".corral_eval(\"{rid}\")") };
    herdr::send_text(&pane, &format!("{call}\r"))?;

    // Wait for the outcome: often a few milliseconds, so look often at first.
    let result_file = edir.join(format!("{rid}.json"));
    let out_file = edir.join(format!("{rid}.out"));
    let start = Instant::now();
    let end = start + Duration::from_secs_f64(o.timeout);
    while !result_file.exists() && Instant::now() < end {
        let waited = start.elapsed();
        std::thread::sleep(Duration::from_millis(if waited < Duration::from_secs(2) { 5 } else if waited < Duration::from_secs(15) { 25 } else { 100 }));
    }
    let output = std::fs::read(&out_file).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    if !result_file.exists() {
        if output_mode == "full" {
            println!("{}", clip_output(&output, o.max_chars, &out_file));
        }
        let mut e = audit_entry(&o, &code, access, output_mode, "ran");
        e.set("status", "timeout".into());
        audit::record(e);
        let (secs, file) = (o.timeout, out_file.display());
        println!("\n{}", trf!("[corral] still running after {secs:.0} s; the full output will be in {file}",
                                "[corral] sigue corriendo después de {secs:.0} s; la salida completa quedará en {file}"));
        std::process::exit(2);
    }
    let result = std::fs::read_to_string(&result_file).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::obj());
    let full_text = {
        let mut text = clip_output(output.trim_end(), o.max_chars, &out_file);
        for w in result.get("warnings").arr() {
            if let Some(w) = w.str() {
                text += &format!("\n{}", trf!("[corral] warning: {w}", "[corral] advertencia: {w}"));
            }
        }
        text
    };
    let after = plots_json(&sid, "state");
    let new_plots = if after.is_null() {
        0
    } else {
        let (old, new) = (before.get("hsize").num().unwrap_or(0.0) as usize, after.get("hsize").num().unwrap_or(0.0) as usize);
        let changed = before.is_null() || before.get("upid") != after.get("upid");
        if new > old { new - old } else if changed && new > 0 { 1 } else { 0 }
    };
    // Whether the agent gets the result itself: always (full), never (structure), or
    // when the user says so after seeing it (review; nothing to see needs no question).
    let share = match output_mode {
        "structure" => false,
        "review" if !prints_something(&output, &code) && result.get("warnings").arr().is_empty() && new_plots == 0 => true,
        "review" => review(&edir, &rid, &full_text, &o.lang, new_plots),
        _ => true,
    };
    let restricted = !share;
    let shown = if restricted {
        let mut text = restricted_report(&sdir, started, &output, &result);
        if output_mode == "review" {
            text += &format!("\n{}", tr("[corral] the user kept this result to themselves; you got only its structure. Ask them what they found, or for what you need.",
                                         "[corral] el usuario se guardó este resultado; recibiste solo su estructura. Pregúntale qué encontró, o pídele lo que necesitas."));
        }
        text
    } else {
        full_text
    };
    println!("{shown}");
    let status = result.get("status").str().unwrap_or("ok").to_string();
    let mut entry = audit_entry(&o, &code, access, output_mode, "ran");
    entry.set("status", status.as_str().into());
    entry.set("printed_chars", (output.chars().count() as f64).into());
    entry.set("returned_chars", (shown.chars().count() as f64).into());
    if output_mode == "review" {
        entry.set("shared", share.into());
    }
    if result.get("status").str() == Some("expired") {
        audit::record(entry);
        println!("{}", tr("[corral] the request expired before it ran", "[corral] la petición venció antes de ejecutarse"));
        std::process::exit(2);
    }

    // New plots: saved as PNG for the agent, numbered as the user sees them
    // (with restricted output, only counted: the user sees them, the agent doesn't).
    let mut plots_shared = 0usize;
    if restricted && !after.is_null() {
        if new_plots > 0 {
            let n = new_plots;
            println!("{}", trf!("[corral] {n} new plot(s) for the user (ctrl+b f); not shared with the agent (restricted output)",
                                "[corral] {n} gráfico(s) nuevo(s) para el usuario (ctrl+b f); no se comparten con el agente (salida restringida)"));
        }
    } else if !after.is_null() {
        let old = before.get("hsize").num().unwrap_or(0.0) as usize;
        let new = after.get("hsize").num().unwrap_or(0.0) as usize;
        let changed = before.is_null() || before.get("upid") != after.get("upid");
        let indexes: Vec<usize> = if new > old { (old..new).collect() } else if changed && new > 0 { vec![new - 1] } else { Vec::new() };
        let mut current = plot_ids(&sid);
        let drawn: Vec<String> = indexes.iter().filter_map(|&i| current.get(i).cloned()).collect();
        let last_file = edir.join("last-plots.json");
        let mut removed = 0;
        if o.replace && !drawn.is_empty() {
            let previous: Vec<String> = env::read_json(&last_file).get("ids").arr().iter().filter_map(|v| v.str().map(String::from)).collect();
            for id in previous.iter().filter(|id| !drawn.contains(id)) {
                if let Some(k) = plot_ids(&sid).iter().position(|x| x == id) {
                    if plots_raw(&sid, "remove", &[("index", k.to_string())]).is_some() {
                        removed += 1;
                    }
                }
            }
            current = plot_ids(&sid);
        }
        if !drawn.is_empty() {
            env::write_json(&last_file, &obj(vec![("rid", rid.as_str().into()),
                                                  ("ids", Value::Arr(drawn.iter().map(|d| d.as_str().into()).collect()))]));
        }
        for id in &drawn {
            let number = current.iter().position(|x| x == id).map(|p| p + 1).unwrap_or(0);
            let png = edir.join(format!("{rid}-plot-{number}.png"));
            let params = [("id", id.clone()), ("width", "900".into()), ("height", "600".into()), ("renderer", "png".into())];
            if let Some((bytes, _)) = plots_raw(&sid, "plot", &params) {
                if std::fs::write(&png, bytes).is_ok() {
                    plots_shared += 1;
                    let shown = png.display();
                    println!("{}", trf!("[corral] plot {number} saved to {shown} (ctrl+b f to see it in herdr)",
                                        "[corral] gráfico {number} guardado en {shown} (ctrl+b f para verlo en herdr)"));
                }
            }
        }
        if removed > 0 {
            println!("{}", trf!("[corral] replaced {removed} plot(s) from the previous request; the user sees only the new version",
                                "[corral] reemplazó {removed} gráfico(s) de la petición anterior; el usuario ve solo la versión nueva"));
        }
    }
    entry.set("plots_shared", (plots_shared as f64).into());
    audit::record(entry);
    std::process::exit(if result.get("status").str() == Some("error") { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::{access_from, prints_something};

    #[test]
    fn asks_unless_told_otherwise() {
        assert_eq!(access_from(None), "ask");
        assert_eq!(access_from(Some("ask")), "ask");
        assert_eq!(access_from(Some("On")), "ask");
        assert_eq!(access_from(Some("yes")), "ask");
        assert_eq!(access_from(Some("on")), "on");
        assert_eq!(access_from(Some("off")), "off");
    }

    #[test]
    fn echo_is_not_a_result() {
        assert!(!prints_something("> x <- 1\n", "x <- 1"));
        assert!(!prints_something("> f <- function(a) {\n+   a\n+ }\n", "f <- function(a) {\n  a\n}"));
        assert!(!prints_something(">>> y = 2\n", "y = 2"));
        assert!(prints_something("> mean(x)\n[1] 50.7\n", "mean(x)"));
        assert!(prints_something("> print(df)\n> other\n", "print(df)"));
    }
}
