//! `corral _console <lang>`: what corral types in a console pane. It clears
//! the screen and becomes the console (Ark, plain R or IPython).

use crate::env::{self, python_fallback};
use crate::json::Value;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

/// The newest Ark under ~/.local/share/corral/ark (or corral config ark_bin).
pub fn ark_binary(cfg: &Value) -> Option<PathBuf> {
    if let Some(p) = cfg.get("ark_bin").str() {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let version = |p: &PathBuf| -> Vec<u64> {
        p.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()).unwrap_or("")
            .split('.').map(|x| x.parse().unwrap_or(0)).collect()
    };
    std::fs::read_dir(env::data_dir().join("ark")).ok()?
        .flatten().map(|e| e.path().join("ark")).filter(|p| p.is_file())
        .max_by_key(|p| version(p))
}

/// "ark" (the default when Ark and jupyter-console are installed) or "r".
pub fn r_flavor(cfg: &Value) -> &'static str {
    let choice = cfg.get("r_console").str().unwrap_or("ark");
    if choice == "ark" && ark_binary(cfg).is_some() && env::which("jupyter-console").is_some() && env::which("R").is_some() {
        "ark"
    } else {
        "r"
    }
}

pub fn run(args: &[String], all: &[String]) -> Result<(), String> {
    let lang = args.first().map(String::as_str).unwrap_or("r");
    if lang != "r" && lang != "python" {
        python_fallback(all);
    }
    print!("\x1b[H\x1b[2J\x1b[3J"); // the command corral typed goes away
    let _ = std::io::stdout().flush();
    let cfg = env::config();

    let mut cmd = if lang == "python" {
        match cfg.get("python_console").str() {
            Some(custom) => shell(custom),
            None => {
                let mut c = Command::new("uv");
                c.args(["run", "--with", "ipython", "ipython", "--no-banner", "-i"]).arg(env::repo().join("py/startup.py"));
                c
            }
        }
    } else {
        match cfg.get("r_console").str() {
            Some(custom) if custom != "ark" && custom != "r" => shell(custom),
            _ if r_flavor(&cfg) == "ark" => {
                let jupyter = env::data_dir().join("jupyter");
                if !jupyter.join("kernels/ark/kernel.json").is_file() {
                    python_fallback(all); // it writes the kernel spec first
                }
                // The connection file goes where corral.R announces it, so
                // `corral _lsp r` can reach Ark's language server.
                let folder = env::state_dir().join("ark");
                let _ = std::fs::create_dir_all(&folder);
                env::remove_old(&folder, "json", 2 * 86400);
                let conn = folder.join(format!("kernel-{}.json", env::random_hex(4)));
                let mut c = Command::new("jupyter-console");
                c.args(["--kernel", "ark"])
                    .arg(format!("--ConnectionFileMixin.connection_file={}", conn.display()))
                    .arg("--ZMQTerminalInteractiveShell.banner=")
                    .env("CORRAL_ARK_CONNECTION", &conn)
                    .env("JUPYTER_PATH", &jupyter);
                c
            }
            _ => {
                let mut c = Command::new("R");
                c.args(["--quiet", "--no-save"]);
                c
            }
        }
    };
    let err = cmd.env("CORRAL", "1").exec();
    Err(format!("no pude abrir la consola: {err}"))
}

fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", command]);
    c
}
