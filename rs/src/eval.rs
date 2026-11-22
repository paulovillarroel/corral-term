//! `corral eval`: an agent (Claude) runs R or Python code in the user's console
//! and gets back what it printed. The code goes to a file, the console runs it
//! with `.corral_eval()` / `_corral_eval()`, which writes the output and a
//! status file; new plots are saved as PNG. Starting or moving a console goes
//! to the Python version; the usual case is handled here, without its startup.

use crate::env::{self, python_fallback};
use crate::herdr;
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

fn parse(args: &[String], all: &[String]) -> Opts {
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
                eprintln!("corral eval: falta el valor de {name}");
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
            "--reemplazar" => o.replace = true,
            s if s.starts_with("--") => python_fallback(all), // --help and anything new
            _ => o.code.push(a.clone()),
        }
    }
    if o.lang != "r" && o.lang != "python" {
        python_fallback(all);
    }
    o
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The console pane for `lang` and the session in it.
fn console_session(lang: &str, ws: Option<&str>) -> Result<(String, Value), String> {
    let label = format!("corral:{lang}");
    let mut console = None;
    for scope in [ws, None] {
        if let Ok(panes) = herdr::panes(scope) {
            if let Some(p) = herdr::labeled(&panes, &label) {
                console = Some(herdr::pane_id(p).to_string());
                break;
            }
        }
        if scope.is_none() {
            break;
        }
    }
    let Some(console) = console else {
        return Err(if lang == "python" {
            "no hay consola de Python: se abre sola al ejecutar código de Python, o con ctrl+b y".into()
        } else {
            format!("no hay consola {label} en herdr; ábrela con ctrl+b a o márcala con ctrl+b m")
        });
    };
    match env::session_for(&console) {
        Some(s) => Ok((console, s)),
        None if lang == "python" => Err(format!("la consola {console} no tiene Corral activo (¿IPython sigue arrancando?)")),
        None => Err(format!("la consola {console} no tiene Corral activo: abre R con `CORRAL=1 R` o corre corral_activar() en ese R (ctrl+b m lo hace)")),
    }
}

fn last_line(pane: &str) -> String {
    let params = obj(vec![("pane_id", pane.into()), ("source", "visible".into()), ("format", "text".into())]);
    let text = herdr::call("pane.read", params).ok()
        .and_then(|r| r.get("read").get("text").str().map(String::from)).unwrap_or_default();
    text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).last().unwrap_or("").to_string()
}

/// Wait for an empty prompt (R's `>`, or `In [n]:` in IPython and Ark); refuse half-typed input.
/// Is an earlier request still running? Its output file is there, its outcome is not yet.
fn eval_running(edir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(edir) else { return false };
    let newest = entries.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "out"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    newest.is_some_and(|e| !e.path().with_extension("json").exists())
}

fn wait_for_prompt(pane: &str, timeout: f64, lang: &str, edir: &Path) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs_f64(timeout);
    let name = if lang == "python" { "Python" } else { "R" };
    loop {
        let line = last_line(pane);
        let (idle, half) = if let Some(rest) = line.strip_prefix("In [").and_then(|r| r.split_once("]:")).map(|(_, r)| r) {
            (rest.trim().is_empty(), !rest.trim().is_empty())
        } else if line.trim_start().starts_with("...:") || lang == "python" {
            (false, line.trim_start().starts_with("...:"))
        } else {
            (line.trim() == ">", line.starts_with('+') || (line.starts_with("> ") && !line[2..].trim().is_empty()))
        };
        if idle {
            return Ok(());
        }
        // An earlier request still running echoes its code like typed input: busy, not half-typed.
        let half = half && !eval_running(edir);
        if Instant::now() > end {
            return Err(if half {
                "la consola tiene una línea a medio escribir; termínala o bórrala y vuelve a intentar".into()
            } else {
                format!("la consola de {name} está ocupada; espera a que termine o usa --wait-idle")
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn clip_output(text: &str, limit: usize, full: &Path) -> String {
    let n = text.chars().count();
    if n <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit / 2).collect();
    let tail: String = text.chars().skip(n - limit / 2).collect();
    format!("{head}\n\n[corral] … salida recortada ({n} caracteres); completa en {} …\n\n{tail}", full.display())
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

pub fn run(args: &[String], all: &[String]) -> Result<(), String> {
    let o = parse(args, all);
    let ws = env::current_workspace();
    if o.check {
        let (pane, s) = console_session(&o.lang, ws.as_deref()).unwrap_or_else(|e| fail(1, &e));
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
        fail(1, "pasa el código como argumento, con --file o por stdin")
    };
    if code.trim().is_empty() {
        fail(1, "no hay código que ejecutar");
    }

    // The console has to be there, running, and in view; otherwise Python starts or moves it.
    let panes = herdr::panes(ws.as_deref())?;
    let current = env::active_lang(ws.as_deref(), &panes);
    let Some(target) = herdr::labeled(&panes, &format!("corral:{}", o.lang)) else { python_fallback(all) };
    let pane = herdr::pane_id(target).to_string();
    // Switching languages moves panes and layout state: the Python version does that.
    if o.lang != current {
        python_fallback(all);
    }
    let Some(session) = env::session_for(&pane) else { python_fallback(all) };
    let sdir = PathBuf::from(session.get("dir").str().unwrap_or(""));
    let sid = session.get("id").str().unwrap_or("").to_string();
    let edir = sdir.join("eval");
    std::fs::create_dir_all(&edir).map_err(|e| e.to_string())?;

    // One request at a time per console: several agents queue instead of interleaving.
    let lock = File::create(edir.join(".lock")).map_err(|e| e.to_string())?;
    let give_up = Instant::now() + Duration::from_secs_f64(o.wait_idle + o.timeout);
    // SAFETY: flock on a file we keep open; released when the process ends.
    while unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        if Instant::now() > give_up {
            fail(2, "otra petición sigue corriendo en esta consola");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    if let Err(e) = wait_for_prompt(&pane, o.wait_idle, &o.lang, &edir) {
        fail(2, &e);
    }
    let secs = now() as u64;
    let rid = format!("{:02}{:02}{:02}-{}", secs / 3600 % 24, secs / 60 % 60, secs % 60, env::random_hex(3));
    let ext = if o.lang == "python" { "py" } else { "R" };
    let body = if code.ends_with('\n') { code.clone() } else { format!("{code}\n") };
    std::fs::write(edir.join(format!("{rid}.{ext}")), body).map_err(|e| e.to_string())?;
    // If the console only gets to it after the agent gave up, it must not run.
    std::fs::write(edir.join(format!("{rid}.deadline")), format!("{}", now() + o.timeout)).map_err(|e| e.to_string())?;
    let before = plots_json(&sid, "state");
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
        println!("{}", clip_output(&output, o.max_chars, &out_file));
        println!("\n[corral] sigue corriendo después de {:.0} s; la salida completa quedará en {}", o.timeout, out_file.display());
        std::process::exit(2);
    }
    let result = std::fs::read_to_string(&result_file).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::obj());
    println!("{}", clip_output(output.trim_end(), o.max_chars, &out_file));
    for w in result.get("warnings").arr() {
        if let Some(w) = w.str() {
            println!("[corral] advertencia: {w}");
        }
    }
    if result.get("status").str() == Some("expired") {
        println!("[corral] la petición venció antes de ejecutarse");
        std::process::exit(2);
    }

    // New plots: saved as PNG for the agent, numbered as the user sees them.
    let after = plots_json(&sid, "state");
    if !after.is_null() {
        let old = before.get("hsize").num().unwrap_or(0.0) as usize;
        let new = after.get("hsize").num().unwrap_or(0.0) as usize;
        let changed = before.is_null() || before.get("upid") != after.get("upid");
        let indexes: Vec<usize> = if new > old { (old..new).collect() } else if changed && new > 0 { vec![new - 1] } else { Vec::new() };
        let mut current = plot_ids(&sid);
        let drawn: Vec<String> = indexes.iter().filter_map(|&i| current.get(i).cloned()).collect();
        let last_file = edir.join("ultimos-graficos.json");
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
                    println!("[corral] gráfico {number} guardado en {} (ctrl+b f para verlo en herdr)", png.display());
                }
            }
        }
        if removed > 0 {
            println!("[corral] reemplazó {removed} gráfico(s) de la petición anterior; el usuario ve solo la versión nueva");
        }
    }
    std::process::exit(if result.get("status").str() == Some("error") { 1 } else { 0 })
}
