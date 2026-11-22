//! `corral eval`: an agent (Claude) runs R or Python code in the user's console
//! and gets back what it printed. The code goes to a file, the console runs it
//! with `.corral_eval()` / `_corral_eval()`, which writes the output and a
//! status file; new plots are saved as PNG. A console that isn't there, isn't
//! in view or was quit is started or brought into view first.

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

    if let Err(e) = console::wait_for_prompt(&pane, o.wait_idle, &o.lang, Some(&edir)) {
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
        let (secs, file) = (o.timeout, out_file.display());
        println!("\n{}", trf!("[corral] still running after {secs:.0} s; the full output will be in {file}",
                                "[corral] sigue corriendo después de {secs:.0} s; la salida completa quedará en {file}"));
        std::process::exit(2);
    }
    let result = std::fs::read_to_string(&result_file).ok().and_then(|t| json::parse(&t).ok()).unwrap_or(Value::obj());
    println!("{}", clip_output(output.trim_end(), o.max_chars, &out_file));
    for w in result.get("warnings").arr() {
        if let Some(w) = w.str() {
            println!("{}", trf!("[corral] warning: {w}", "[corral] advertencia: {w}"));
        }
    }
    if result.get("status").str() == Some("expired") {
        println!("{}", tr("[corral] the request expired before it ran", "[corral] la petición venció antes de ejecutarse"));
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
    std::process::exit(if result.get("status").str() == Some("error") { 1 } else { 0 })
}
