//! Live tests: a real herdr (headless), R with Ark, and Corral's console, used
//! the way agents use them. They need herdr, R, Ark (`corral ark --install`) and
//! R's jsonlite, so they are ignored unless you ask for them:
//!
//!     cargo test --release --test live -- --ignored --test-threads 1
//!
//! Each test starts its own herdr server, with its settings and Corral's state
//! in a folder under /tmp; your herdr, sessions and settings are not touched.
//! R's packages and Ark come from your user, as in a normal session.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::field;

struct Live {
    dir: PathBuf,
    server: Child,
    console: String,
    workspace: String,
    /// More environment for herdr, and so for the consoles it starts.
    extra: Vec<(String, String)>,
}

impl Live {
    /// A herdr server, a workspace and a Corral layout with an R console, ready.
    fn start(name: &str) -> Live {
        Live::start_with(name, &[])
    }

    fn start_with(name: &str, extra: &[(&str, &str)]) -> Live {
        // Under /tmp itself: herdr's socket path must stay short.
        let dir = PathBuf::from(format!("/tmp/corral-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["bin", "config/corral", "state", "proj"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_corral"), dir.join("bin/corral")).unwrap();
        // The panes' shell: bash with its line editor, like a real terminal, but
        // without rc files, which could put an installed corral first on the PATH.
        let shell = dir.join("bin/shell");
        std::fs::write(&shell, "#!/bin/sh\nexec bash --norc --noprofile \"$@\"\n").unwrap();
        std::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        // Agents run without asking: there is nobody to answer a popup here.
        std::fs::write(dir.join("config/corral/config.json"), r#"{"agents":"on"}"#).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        std::fs::write(dir.join("Rprofile"), format!("if (interactive()) source(\"{}\")\n", repo.join("r/corral.R").display())).unwrap();

        let extra = extra.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut live = Live { server: spawn_placeholder(), dir, console: String::new(), workspace: String::new(), extra };
        live.server = live.command("herdr").arg("server")
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            .expect("herdr is not on the PATH");
        let socket = live.dir.join("config/herdr/herdr.sock");
        wait_for(Duration::from_secs(10), "herdr's socket", || socket.exists());

        let created = live.run("herdr", &["workspace", "create", "--cwd", live.dir.join("proj").to_str().unwrap()]);
        live.workspace = field(&created, "workspace_id").expect("no workspace");
        let layout = live.run("corral", &["layout", "--editor", ""]);
        live.console = field(&layout, "console").unwrap_or_else(|| panic!("no console in {layout}"));
        wait_for(Duration::from_secs(60), "the R console", || live.corral(&["eval", "--check"]).status.success());
        // The console in the pane must be the corral under test, not one installed elsewhere.
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", live.kernel_pid())).unwrap();
        let parent = stat.rsplit_once(')').unwrap().1.split_whitespace().nth(1).unwrap().to_string();
        let console = std::fs::canonicalize(format!("/proc/{parent}/exe")).unwrap();
        assert_eq!(console, std::fs::canonicalize(env!("CARGO_BIN_EXE_corral")).unwrap(), "the pane runs another corral");
        live
    }

    /// A command with this test's environment: its herdr, its settings and state.
    fn command(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        c.current_dir(self.dir.join("proj"))
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("R_PROFILE_USER", self.dir.join("Rprofile"))
            .env("HERDR_SOCKET_PATH", self.dir.join("config/herdr/herdr.sock"))
            .env("HERDR_WORKSPACE_ID", &self.workspace)
            .env("CORRAL_LANG", "en")
            .env("CORRAL_NO_TELEMETRY", "1")
            .env("CORRAL_AGENT", "")
            // Panes run bash without rc files, so they keep this PATH and run the
            // corral under test, not the one installed in ~/.local/bin.
            .env("SHELL", self.dir.join("bin/shell"))
            .env_remove("ENV")
            .envs(self.extra.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_PLUGIN_CONTEXT_JSON")
            .env_remove("TMUX");
        c
    }

    fn corral(&self, args: &[&str]) -> Output {
        self.command("corral").args(args).output().unwrap()
    }

    /// Run and return what it printed; it must succeed.
    fn run(&self, program: &str, args: &[&str]) -> String {
        let out = self.command(program).args(args).output().unwrap();
        assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `corral eval ARGS`: exit code and output.
    fn eval(&self, args: &[&str]) -> (i32, String) {
        let mut all = vec!["eval"];
        all.extend_from_slice(args);
        let out = self.corral(&all);
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        (out.status.code().unwrap_or(-1), text)
    }

    /// The process and pane of the live session in `lang` (R's is Ark), from
    /// its meta.json; None while there is none.
    fn session(&self, lang: &str) -> Option<(i64, String)> {
        let sessions = self.dir.join("state/corral/sessions");
        std::fs::read_dir(&sessions).ok()?.flatten()
            .filter_map(|d| std::fs::read_to_string(d.path().join("meta.json")).ok())
            .filter(|m| field(m, "lang").unwrap_or_else(|| "r".into()) == lang)
            .filter_map(|m| Some((field(&m, "pid")?.parse::<i64>().ok()?, field(&m, "pane_id")?)))
            .find(|(p, _)| alive(*p))
    }

    /// Corral's settings for this test.
    fn config(&self, json: &str) {
        std::fs::write(self.dir.join("config/corral/config.json"), json).unwrap();
    }

    /// The live session's checkpoint folder, and its index once written.
    fn checkpoint(&self, lang: &str) -> (PathBuf, String) {
        let sessions = self.dir.join("state/corral/sessions");
        let id = std::fs::read_dir(&sessions).unwrap().flatten()
            .filter_map(|d| std::fs::read_to_string(d.path().join("meta.json")).ok())
            .filter(|m| field(m, "lang").unwrap_or_else(|| "r".into()) == lang)
            .filter(|m| field(m, "pid").and_then(|p| p.parse::<i64>().ok()).is_some_and(alive))
            .find_map(|m| field(&m, "id")).expect("no live session");
        let prefix = if lang == "python" { "py" } else { "r" };
        let dir = self.dir.join("state/corral/checkpoints").join(format!("{prefix}-{id}"));
        let index = std::fs::read_to_string(dir.join("index.json")).unwrap_or_default();
        (dir, index)
    }

    /// Wait until the checkpoint holds every one of `names`.
    fn wait_checkpoint(&self, lang: &str, names: &[&str]) {
        wait_for(Duration::from_secs(30), "the checkpoint", || {
            let index: String = self.checkpoint(lang).1.split_whitespace().collect();
            names.iter().all(|n| index.contains(&format!("\"{n}\":{{")))
        });
    }

    /// Link Corral's herdr plugin, for the popups that ask the user.
    fn link_plugin(&self) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        self.run("herdr", &["plugin", "link", repo.join("herdr").to_str().unwrap()]);
    }

    /// Wait for a popup's request (approve or review) newer than `seen`, and answer it
    /// as the popup would: the decision file next to it.
    fn answer_popup(&self, seen: &[PathBuf], word: &str) -> String {
        let found = self.wait_popup(seen);
        let request = std::fs::read_to_string(&found).unwrap();
        let decision = field(&request, "decision").expect("no decision file");
        std::fs::write(decision, word).unwrap();
        request
    }

    /// Wait for a popup's request not answered yet, newer than `seen`.
    fn wait_popup(&self, seen: &[PathBuf]) -> PathBuf {
        let dirs = [self.dir.join("state/corral/requests")];
        let mut found = None;
        wait_for(Duration::from_secs(20), "a popup's request", || {
            let mut all: Vec<PathBuf> = dirs.iter().chain(
                std::fs::read_dir(self.dir.join("state/corral/sessions")).into_iter().flatten().flatten().map(|d| d.path().join("eval")).collect::<Vec<_>>().iter())
                .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
                .filter(|p| [".approve.json", ".review.json", ".setting.json"].iter().any(|k| p.to_string_lossy().ends_with(k)))
                .filter(|p| !seen.contains(p))
                // Not one answered already: its decision file is there.
                .filter(|p| std::fs::read_to_string(p).ok().and_then(|r| field(&r, "decision")).is_some_and(|d| !Path::new(&d).exists()))
                .collect();
            all.sort();
            found = all.pop();
            found.is_some()
        });
        found.unwrap()
    }

    fn kernel_pid(&self) -> i64 {
        self.session("r").expect("no live R session").0
    }

    /// What the pane shows.
    fn screen(&self, pane: &str) -> String {
        self.run("herdr", &["pane", "read", pane])
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.command("herdr").args(["server", "stop"]).output();
        let _ = self.server.kill();
        let _ = self.server.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Running, and not a zombie.
fn alive(pid: i64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|s| s.rsplit_once(')').is_some_and(|(_, rest)| !rest.trim_start().starts_with(['Z', 'X'])))
}

/// A child that is already done, until the real server starts.
fn spawn_placeholder() -> Child {
    Command::new("true").spawn().unwrap()
}

fn wait_for(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !ready() {
        assert!(Instant::now() < end, "{what} wasn't ready after {limit:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Two agents asking at the same time take turns: each gets only its own
/// output, whole, and the second waits for the first.
#[test]
#[ignore]
fn two_agents_take_turns() {
    let live = Live::start("turns");
    let started = Instant::now();
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| live.eval(&[r#"cat("A1\n"); Sys.sleep(1.5); cat("A2\n")"#]));
        std::thread::sleep(Duration::from_millis(100));
        let b = s.spawn(|| live.eval(&[r#"cat("B1\n"); Sys.sleep(1.5); cat("B2\n")"#]));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(a.0, 0, "{}", a.1);
    assert_eq!(b.0, 0, "{}", b.1);
    assert!(a.1.contains("A1\nA2") && !a.1.contains("B1") && !a.1.contains("B2"), "A got: {}", a.1);
    assert!(b.1.contains("B1\nB2") && !b.1.contains("A1") && !b.1.contains("A2"), "B got: {}", b.1);
    assert!(started.elapsed() >= Duration::from_millis(2900), "they ran at once: {:?}", started.elapsed());
}

/// Past its timeout the agent stops waiting, but the code keeps running and its
/// output is kept; the next request waits for the console to be free.
#[test]
#[ignore]
fn a_timeout_leaves_the_code_running() {
    let live = Live::start("timeout");
    let (code, text) = live.eval(&["--timeout", "1", r#"Sys.sleep(3); cat("slow done\n")"#]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("still running"), "{text}");
    let file = text.rsplit("will be in ").next().unwrap().trim().to_string();
    let (code, text) = live.eval(&[r#"cat("next\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("next"), "{text}");
    let kept = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(kept.contains("slow done"), "{file} has: {kept}");
}

/// Ctrl+C in the console stops what an agent started, and the session's
/// objects stay.
#[test]
#[ignore]
fn ctrl_c_interrupts_and_keeps_the_session() {
    let live = Live::start("interrupt");
    assert_eq!(live.eval(&["y <- 7"]).0, 0);
    let (code, text) = live.eval(&["--timeout", "1", "Sys.sleep(60)"]);
    assert_eq!(code, 2, "{text}");
    let pressed = Instant::now();
    live.run("herdr", &["pane", "send-keys", &live.console, "ctrl+c"]);
    let (code, text) = live.eval(&["--wait-idle", "10", r#"cat("y =", y, "\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("y = 7"), "{text}");
    assert!(pressed.elapsed() < Duration::from_secs(10), "it took {:?}", pressed.elapsed());
}

/// When R dies in the middle of a request, the agent hears it at once instead of
/// waiting out its timeout, and the next request gets a new session.
#[test]
#[ignore]
fn a_crash_is_reported_at_once() {
    let live = Live::start("crash");
    let pid = live.kernel_pid();
    let started = Instant::now();
    let (code, text) = std::thread::scope(|s| {
        let request = s.spawn(|| live.eval(&["--timeout", "60", r#"cat("started\n"); Sys.sleep(30)"#]));
        std::thread::sleep(Duration::from_millis(1500));
        Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
        request.join().unwrap()
    });
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("console ended"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(10), "the agent waited {:?}", started.elapsed());
    let audit = std::fs::read_to_string(live.dir.join("state/corral/audit.jsonl")).unwrap();
    assert!(audit.lines().last().unwrap().contains(r#""status":"console ended""#), "{audit}");

    let (code, text) = live.eval(&[r#"cat("fresh\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("fresh"), "{text}");
    assert_ne!(live.kernel_pid(), pid);
}

/// Reading the variables while an agent keeps changing them never finds a
/// half-written file: R writes vars.json whole, then moves it into place.
#[test]
#[ignore]
fn variables_are_never_read_half_written() {
    let live = Live::start("vars");
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            for i in 0..15 {
                let code = format!("v{i} <- data.frame(a = seq_len({}), b = letters[1 + {i} %% 26])", 1000 * (i + 1));
                let (status, text) = live.eval(&[&code]);
                assert_eq!(status, 0, "{text}");
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let mut reads = 0;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let out = live.corral(&["vars", "--json"]);
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "corral vars: {}", String::from_utf8_lossy(&out.stderr));
            assert!(text.trim_start().starts_with('{') && text.trim_end().ends_with('}'), "read {reads}: {text}");
            reads += 1;
        }
        writer.join().unwrap();
        assert!(reads > 5, "only {reads} reads");
    });
    // The last request returns before the console describes its objects.
    wait_for(Duration::from_secs(5), "v14 in the variables", || {
        String::from_utf8_lossy(&live.corral(&["vars", "--json"]).stdout).contains("v14")
    });
}

/// When R crashes, the console starts a new session by itself and says so:
/// the pane doesn't fall back to the shell, and the next request runs at once.
#[test]
#[ignore]
fn r_comes_back_by_itself_after_a_crash() {
    let live = Live::start("restart");
    std::thread::sleep(Duration::from_secs(6)); // a kernel that crashes on starting isn't started again
    let old = live.kernel_pid();
    Command::new("kill").args(["-9", &old.to_string()]).status().unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while !live.session("r").is_some_and(|(p, _)| p != old) {
        assert!(Instant::now() < end, "no new R session after 30 s; the console shows:\n{}", live.screen(&live.console));
        std::thread::sleep(Duration::from_millis(200));
    }
    let screen = live.screen(&live.console);
    assert!(screen.contains("R ended unexpectedly"), "{screen}");
    let started = Instant::now();
    let (code, text) = live.eval(&[r#"cat("back\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("back"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(5), "it took {:?}", started.elapsed());
}

/// Python: the first request starts the console; objects stay between
/// requests, and two agents asking at once take turns.
#[test]
#[ignore]
fn python_keeps_its_objects_and_takes_turns() {
    let live = Live::start("python");
    let (code, text) = live.eval(&["--lang", "python", "x = 41"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "python", "print(x + 1)"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("42"), "{text}");
    let started = Instant::now();
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| live.eval(&["--lang", "python", "import time; print('A1'); time.sleep(1.5); print('A2')"]));
        std::thread::sleep(Duration::from_millis(100));
        let b = s.spawn(|| live.eval(&["--lang", "python", "import time; print('B1'); time.sleep(1.5); print('B2')"]));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!((a.0, b.0), (0, 0), "{} / {}", a.1, b.1);
    assert!(a.1.contains("A1\nA2") && !a.1.contains("B1"), "A got: {}", a.1);
    assert!(b.1.contains("B1\nB2") && !b.1.contains("A1"), "B got: {}", b.1);
    assert!(started.elapsed() >= Duration::from_millis(2900), "they ran at once: {:?}", started.elapsed());
}

/// Python: past the timeout the code keeps running; Ctrl+C stops it and the
/// objects stay.
#[test]
#[ignore]
fn python_timeout_and_ctrl_c() {
    let live = Live::start("pyint");
    assert_eq!(live.eval(&["--lang", "python", "y = 7"]).0, 0);
    let (code, text) = live.eval(&["--lang", "python", "--timeout", "1", "import time; time.sleep(3); print('slow done')"]);
    assert_eq!(code, 2, "{text}");
    let file = text.rsplit("will be in ").next().unwrap().trim().to_string();
    let (code, text) = live.eval(&["--lang", "python", "print('next')"]);
    assert_eq!(code, 0, "{text}");
    assert!(std::fs::read_to_string(&file).unwrap_or_default().contains("slow done"), "{file}");

    let (code, text) = live.eval(&["--lang", "python", "--timeout", "1", "import time; time.sleep(60)"]);
    assert_eq!(code, 2, "{text}");
    let (_, pane) = live.session("python").expect("no Python session");
    let pressed = Instant::now();
    live.run("herdr", &["pane", "send-keys", &pane, "ctrl+c"]);
    let (code, text) = live.eval(&["--lang", "python", "--wait-idle", "10", "print('y =', y)"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("y = 7"), "{text}");
    assert!(pressed.elapsed() < Duration::from_secs(10), "it took {:?}", pressed.elapsed());
}

/// Python: a crash in the middle of a request is reported at once, and the
/// next request starts a new console.
#[test]
#[ignore]
fn python_crash_is_reported_at_once() {
    let live = Live::start("pycrash");
    assert_eq!(live.eval(&["--lang", "python", "z = 1"]).0, 0);
    let (pid, _) = live.session("python").expect("no Python session");
    let started = Instant::now();
    let (code, text) = std::thread::scope(|s| {
        let request = s.spawn(|| live.eval(&["--lang", "python", "--timeout", "60", "import time; print('started'); time.sleep(30)"]));
        std::thread::sleep(Duration::from_millis(1500));
        Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
        request.join().unwrap()
    });
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("console ended"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(10), "the agent waited {:?}", started.elapsed());
    let (code, text) = live.eval(&["--lang", "python", "print('fresh')"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("fresh"), "{text}");
    assert_ne!(live.session("python").map(|s| s.0), Some(pid));
}

/// Python: the variables are never read half-written while they change.
#[test]
#[ignore]
fn python_variables_are_never_read_half_written() {
    let live = Live::start("pyvars");
    assert_eq!(live.eval(&["--lang", "python", "pass"]).0, 0);
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            for i in 0..15 {
                let code = format!("v{i} = list(range({}))", 1000 * (i + 1));
                let (status, text) = live.eval(&["--lang", "python", &code]);
                assert_eq!(status, 0, "{text}");
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let mut reads = 0;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let out = live.corral(&["vars", "--json", "--lang", "python"]);
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "corral vars: {}", String::from_utf8_lossy(&out.stderr));
            assert!(text.trim_start().starts_with('{') && text.trim_end().ends_with('}'), "read {reads}: {text}");
            reads += 1;
        }
        writer.join().unwrap();
        assert!(reads > 5, "only {reads} reads");
    });
}

/// The editor's R language server is Ark's, reached through the console's
/// kernel: the bridge checks the kernel's signed answer and then answers LSP.
#[test]
#[ignore]
fn the_editors_language_server_reaches_ark() {
    use std::io::{Read, Write};
    let live = Live::start("lsp");
    let mut lsp = live.command("corral").args(["_lsp", "r"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#;
    lsp.stdin.as_mut().unwrap().write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
    let mut stdout = lsp.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut seen = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            if seen.contains(r#""result":{"capabilities""#) {
                break;
            }
        }
        let _ = tx.send(seen);
    });
    let answer = rx.recv_timeout(Duration::from_secs(30)).unwrap_or_default();
    let _ = lsp.kill();
    let _ = lsp.wait();
    assert!(answer.starts_with("Content-Length:") && answer.contains(r#""result":{"capabilities""#), "the language server said: {answer}");
}


/// Resident memory of a process, in MB.
fn rss_mb(pid: i64) -> f64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status.lines().find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

/// Children of `pid` that ended and nobody collected.
fn zombies_of(pid: i64) -> usize {
    std::fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
        .filter(|stat| {
            let rest: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
            rest.first() == Some(&"Z") && rest.get(1).and_then(|p| p.parse::<i64>().ok()) == Some(pid)
        })
        .count()
}

fn folder_mb(dir: &Path) -> f64 {
    std::fs::read_dir(dir).map(|r| r.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>()).unwrap_or(0) as f64 / 1048576.0
}

/// After a crash, R's objects come back as they were: a big table, a function,
/// a model, the packages attached, and a change made in place. A connection
/// can't come back, and the console says so.
#[test]
#[ignore]
fn r_objects_survive_a_crash() {
    let live = Live::start_with("rkeep", &[("CORRAL_CHECKPOINT_EVERY", "2")]);
    let code = r#"library(tools); set.seed(1); big <- data.frame(id = seq_len(1e6), x = rnorm(1e6), g = sample(letters, 1e6, TRUE));
        f <- function(x) x * 2; m <- lm(x ~ g, big[1:1000, ]); v <- numeric(10); con <- file(tempfile(), "w");
        check <- c(sum(big$x), coef(m)[[1]]); cat("ok\n")"#;
    assert_eq!(live.eval(&[code]).0, 0);
    live.wait_checkpoint("r", &["big", "f", "m", "v", "check"]);
    // A change in place keeps the object's address: the minute pass (2 s here) catches it.
    assert_eq!(live.eval(&["v[1] <- 5"]).0, 0);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(live.eval(&["invisible(1)"]).0, 0);
    std::thread::sleep(Duration::from_millis(1500));
    std::thread::sleep(Duration::from_secs(3)); // older than 5 s: the console starts R again by itself
    let old = live.kernel_pid();
    Command::new("kill").args(["-9", &old.to_string()]).status().unwrap();
    wait_for(Duration::from_secs(60), "a new R session", || live.session("r").is_some_and(|(p, _)| p != old));
    let (code, text) = live.eval(&[r#"cat("same:", isTRUE(all.equal(check, c(sum(big$x), coef(m)[[1]]))), f(21), v[1],
        "package:tools" %in% search(), exists("con"), "\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("same: TRUE 42 5 TRUE FALSE"), "{text}");
    let screen = live.screen(&live.console);
    assert!(screen.contains("Restored") && screen.contains("con (a connection)"), "the console shows:\n{screen}");
}

/// Python too, after a crash: values, a function defined in the session, an
/// instance of a class defined there, and a module imported under another name.
#[test]
#[ignore]
fn python_objects_survive_a_crash() {
    let live = Live::start("pykeep");
    let code = "import json as js, threading\nd = {'a': [1, 2, 3], 'b': 'text'}\ndef f(x):\n    return x * 2\nclass P:\n    def __init__(self, n): self.n = n\np = P(7)\nlk = threading.Lock()\nprint('ok')";
    let (status, text) = live.eval(&["--lang", "python", code]);
    assert_eq!(status, 0, "{text}");
    live.wait_checkpoint("python", &["d", "f", "p"]);
    let (pid, pane) = live.session("python").unwrap();
    Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
    let (status, text) = live.eval(&["--lang", "python", "print('same:', d['a'], f(21), p.n, js.dumps(1), 'lk' in dir())"]);
    assert_eq!(status, 0, "{text}");
    assert!(text.contains("same: [1, 2, 3] 42 7 1 False"), "{text}");
    let screen = live.screen(&pane);
    assert!(screen.contains("Restored") && screen.contains("lk (lock)"), "the console shows:\n{screen}");
    // The crashed Python's temporary folder went when the new one started.
    let left: Vec<_> = std::fs::read_dir(std::env::temp_dir()).unwrap().flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("corral-py-{pid}-"))).collect();
    assert!(left.is_empty(), "the crashed Python's temporary folder is still there");
}

/// Closing on purpose starts clean: q() in R, and corral close.
#[test]
#[ignore]
fn closing_on_purpose_starts_clean() {
    let live = Live::start("clean");
    assert_eq!(live.eval(&["kept <- 1"]).0, 0);
    live.wait_checkpoint("r", &["kept"]);
    let (dir, _) = live.checkpoint("r");
    live.run("herdr", &["pane", "send-text", &live.console, "q('no')"]);
    live.run("herdr", &["pane", "send-keys", &live.console, "enter"]);
    wait_for(Duration::from_secs(10), "R to quit", || live.session("r").is_none());
    assert!(!dir.exists(), "q() left the checkpoint");
    let (code, text) = live.eval(&[r#"cat("kept exists:", exists("kept"), "\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("kept exists: FALSE"), "{text}");

    assert_eq!(live.eval(&["kept <- 2"]).0, 0);
    live.wait_checkpoint("r", &["kept"]);
    live.run("corral", &["close"]);
    live.run("corral", &["layout", "--editor", ""]);
    wait_for(Duration::from_secs(60), "the R console", || live.corral(&["eval", "--check"]).status.success());
    let (code, text) = live.eval(&[r#"cat("kept exists:", exists("kept"), "\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("kept exists: FALSE"), "after corral close: {text}");
}

/// The work Corral does after each command, as the console timed it
/// (CORRAL_TIMING), for the command a request just ran.
struct Timing(PathBuf);

impl Timing {
    fn lines(&self) -> Vec<f64> {
        std::fs::read_to_string(&self.0).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect()
    }

    /// Run `code` and return how long Corral's work after it kept the prompt waiting.
    fn after(&self, live: &Live, args: &[&str]) -> f64 {
        let before = self.lines().len();
        let (code, text) = live.eval(args);
        assert_eq!(code, 0, "{text}");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let lines = self.lines();
            if lines.len() > before {
                return *lines.last().unwrap();
            }
            assert!(Instant::now() < end, "no timing for {args:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn stats(v: &mut [f64]) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.1), at(0.9))
}

/// Requests alternating checkpoints off and on, for a command that changes
/// nothing and one that changes an object: the prompt's wait for each, in ms.
fn on_off(live: &Live, timing: &Timing, lang: &[&str], same: &str, change: &str, rounds: usize) -> [(f64, f64, f64); 4] {
    let mut t: [Vec<f64>; 4] = Default::default();
    for round in 0..rounds {
        for on in [round % 2 == 0, round % 2 == 1] {
            live.config(if on { r#"{"agents":"on"}"# } else { r#"{"agents":"on","checkpoints":"off"}"# });
            // The first command after switching carries what changed in the other mode.
            timing.after(live, &[lang, &[same]].concat());
            let k = if on { 1 } else { 0 };
            t[k].push(timing.after(live, &[lang, &[same]].concat()));
            t[2 + k].push(timing.after(live, &[lang, &[change]].concat()));
        }
    }
    live.config(r#"{"agents":"on"}"#);
    let names = ["changes nothing, off", "changes nothing, on ", "changes an object, off", "changes an object, on "];
    let mut out = [(0.0, 0.0, 0.0); 4];
    for (i, v) in t.iter_mut().enumerate() {
        out[i] = stats(v);
        println!("    {:<24} median {:6.2} ms  (p10 {:6.2}, p90 {:6.2})", names[i], out[i].0, out[i].1, out[i].2);
    }
    out
}

/// What checkpoints cost R, measured where the user waits: Corral's work after
/// each command, before the prompt comes back (timed inside R). Two session
/// sizes, checkpoints off and on alternating; then 200 commands with them off
/// and 200 with them on, for memory, leftover processes and disk.
#[test]
#[ignore]
fn checkpoints_cost_report() {
    let file = PathBuf::from(format!("/tmp/corral-live-cost-{}/timing", std::process::id()));
    let live = Live::start_with("cost", &[("CORRAL_TIMING", file.to_str().unwrap())]);
    let timing = Timing(file);
    let pid = live.kernel_pid();
    let mut fork_cost = Vec::new();
    for (size, load) in [
        ("350 MB", r#"set.seed(2); a <- data.frame(id = seq_len(1e7), x = rnorm(1e7), g = sample(letters, 1e7, TRUE)); invisible(gc())"#),
        ("1.5 GB", r#"b <- data.frame(id = seq_len(4e7), y = runif(4e7), z = rnorm(4e7)); invisible(gc())"#),
    ] {
        assert_eq!(live.eval(&[load]).0, 0);
        live.wait_checkpoint("r", &["a"]);
        std::thread::sleep(Duration::from_secs(2));
        println!("\nR with about {size} ({:.0} MB in use):", rss_mb(pid));
        let r = on_off(&live, &timing, &[], "invisible(1)", "v <- runif(10)", 20);
        fork_cost.push((r[1].0 - r[0].0, r[3].0 - r[2].0));
    }
    assert_eq!(live.eval(&["rm(b); invisible(gc())"]).0, 0);

    // A long session: the same 200 commands four times, checkpoints off, on,
    // off, on. One in five changes a 1-million-row table.
    assert_eq!(live.eval(&["d <- data.frame(x = rnorm(1e6))"]).0, 0);
    let mut phases = Vec::new();
    for on in [false, true, false, true] {
        live.config(if on { r#"{"agents":"on"}"# } else { r#"{"agents":"on","checkpoints":"off"}"# });
        std::thread::sleep(Duration::from_secs(1));
        let rss0 = rss_mb(pid);
        let mut waits = Vec::new();
        for i in 0..200 {
            let code = match i % 5 { 0 => "d$x <- d$x + 1", 1 => "s <- summary(d$x)", _ => "invisible(nrow(d))" };
            waits.push(timing.after(&live, &[code]));
        }
        std::thread::sleep(Duration::from_secs(3));
        timing.after(&live, &["invisible(1)"]); // collects the last copy
        let (m, _, p90) = stats(&mut waits);
        phases.push((on, rss0, rss_mb(pid), m, p90, zombies_of(pid)));
    }
    // Then 1,000 more with checkpoints on: memory every 200 commands should level off.
    live.config(r#"{"agents":"on"}"#);
    let mut samples = vec![rss_mb(pid)];
    for i in 0..1000 {
        let code = match i % 5 { 0 => "d$x <- d$x + 1", 1 => "s <- summary(d$x)", _ => "invisible(nrow(d))" };
        timing.after(&live, &[code]);
        if i % 200 == 199 {
            samples.push(rss_mb(pid));
        }
    }
    let (dir, index) = live.checkpoint("r");
    let (_, text) = live.eval(&[r#"cat(round(sum(sapply(ls(), function(n) length(serialize(get(n), NULL)))) / 2^20))"#]);
    let serialized: f64 = text.lines().last().unwrap_or("0").trim().parse().unwrap_or(0.0);
    println!("\nLong session, 200 commands in each phase:");
    for (on, rss0, rss1, m, p90, z) in &phases {
        println!("    checkpoints {:<3}  R {rss0:.0} -> {rss1:.0} MB  wait median {m:.2} ms, p90 {p90:.2} ms  leftover processes {z}",
                 if *on { "on" } else { "off" });
    }
    println!("    1,000 more with checkpoints on, R every 200: {}", samples.iter().map(|m| format!("{m:.0}")).collect::<Vec<_>>().join(" -> "));
    println!("    checkpoint on disk {:.0} MB; the objects, serialized: {serialized:.0} MB", folder_mb(&dir));
    // R hands memory back now and then, so it goes up and down: no new highs is what counts.
    let peak = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
    assert!(peak(&samples[3..]) <= peak(&samples[..3]) + 60.0, "memory kept growing: {samples:?}");

    for (nothing, change) in &fork_cost {
        assert!(*nothing < 1.0, "a command that changes nothing waited {nothing:.2} ms more");
        assert!(*change < 40.0, "a command that changes an object waited {change:.2} ms more");
    }
    let (off, on) = (phases[2], phases[3]);
    assert!(on.5 <= 1, "{} copies were never collected", on.5);
    assert!((on.2 - on.1) <= (off.2 - off.1) + 50.0, "memory grew more with checkpoints on");
    assert!(folder_mb(&dir) <= serialized * 1.1 + 1.0, "the checkpoint holds more than the objects");
    assert!(index.split_whitespace().collect::<String>().contains("\"d\":{"), "the checkpoint lost d");
}

/// The same for Python: a session with about 400 MB in use.
#[test]
#[ignore]
fn python_checkpoints_cost_report() {
    let file = PathBuf::from(format!("/tmp/corral-live-pycost-{}/timing", std::process::id()));
    let live = Live::start_with("pycost", &[("CORRAL_TIMING", file.to_str().unwrap())]);
    let timing = Timing(file);
    let py = ["--lang", "python"];
    assert_eq!(live.eval(&[&py[..], &["big = list(range(10**7)); text = 'x' * 10**8"]].concat()).0, 0);
    live.wait_checkpoint("python", &["big", "text"]);
    let (pid, _) = live.session("python").unwrap();
    println!("\nPython with {:.0} MB in use:", rss_mb(pid));
    let r = on_off(&live, &timing, &py, "None", "v = [1, 2, 3]", 20);
    std::thread::sleep(Duration::from_secs(3));
    timing.after(&live, &[&py[..], &["None"]].concat());
    let z = zombies_of(pid);
    println!("    leftover processes {z}");
    assert!(r[1].0 - r[0].0 < 1.0, "a cell that changes nothing waited more");
    assert!(r[3].0 - r[2].0 < 40.0, "a cell that changes an object waited much more");
    assert!(z <= 1, "{z} copies were never collected");
}


/// How long a crashed R takes to come back with its objects, for two sizes.
#[test]
#[ignore]
fn restore_time_report() {
    for (size, load) in [("250 MB", "a <- data.frame(id = seq_len(1e7), x = rnorm(1e7), g = sample(letters, 1e7, TRUE))"),
                         ("1 GB", "a <- data.frame(id = seq_len(4e7), x = rnorm(4e7), y = runif(4e7))")] {
        let live = Live::start("restore");
        assert_eq!(live.eval(&[&format!("{load}; n <- nrow(a)")]).0, 0);
        live.wait_checkpoint("r", &["a", "n"]);
        std::thread::sleep(Duration::from_secs(6));
        let old = live.kernel_pid();
        let killed = Instant::now();
        Command::new("kill").args(["-9", &old.to_string()]).status().unwrap();
        wait_for(Duration::from_secs(120), "R with its objects", || {
            live.session("r").is_some_and(|(p, _)| p != old) && live.eval(&[r#"cat(exists("a") && nrow(a) == n)"#]).1.contains("TRUE")
        });
        let (dir, _) = live.checkpoint("r");
        let _ = dir;
        println!("\n{size} of objects: back {:.1} s after the crash", killed.elapsed().as_secs_f64());
    }
}


/// An agent's shell (no terminal) can't loosen the harness on its own: the user
/// decides in a popup, and the audit log keeps the request. Tightening never asks.
#[test]
#[ignore]
fn an_agent_cannot_loosen_the_harness_by_itself() {
    let live = Live::start("loosen");
    live.link_plugin();
    let config = live.dir.join("config/corral/config.json");
    live.config(r#"{"agents":"ask","agent_output":"structure"}"#);
    let agent = |args: &[&str]| -> (i32, String) {
        let out = live.command("corral").args(args).stdin(Stdio::null()).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
    };

    let (code, text, request) = std::thread::scope(|s| {
        let asking = s.spawn(|| agent(&["config", "agents", "on"]));
        let request = live.answer_popup(&[], "refuse");
        let (code, text) = asking.join().unwrap();
        (code, text, request)
    });
    assert!(request.contains(r#""key":"agents""#) && request.contains(r#""to":"on""#), "{request}");
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("refused"), "{text}");
    let settings = || std::fs::read_to_string(&config).unwrap().split_whitespace().collect::<String>();
    assert!(settings().contains(r#""agents":"ask""#));

    let (code, text) = std::thread::scope(|s| {
        let asking = s.spawn(|| agent(&["config", "agent_output", "full"]));
        live.answer_popup(&[], "allow");
        asking.join().unwrap()
    });
    assert_eq!(code, 0, "{text}");
    assert!(settings().contains(r#""agent_output":"full""#));

    let started = Instant::now();
    let (code, text) = agent(&["config", "agents", "off"]);
    assert_eq!(code, 0, "{text}");
    assert!(started.elapsed() < Duration::from_secs(3), "tightening asked");

    let audit = std::fs::read_to_string(live.dir.join("state/corral/audit.jsonl")).unwrap();
    assert!(audit.contains(r#""decision":"refused""#) && audit.contains("corral config agents on"), "{audit}");
    assert!(audit.contains(r#""decision":"allowed""#) && audit.contains("corral config agent_output full"), "{audit}");
}

/// With restricted output, a request past its timeout doesn't tell the agent where
/// the full output is: that file holds the values it mustn't see.
#[test]
#[ignore]
fn restricted_output_never_points_to_the_values() {
    let live = Live::start("restricted");
    live.config(r#"{"agents":"on","agent_output":"structure"}"#);
    let (code, text) = live.eval(&["--timeout", "1", r#"Sys.sleep(3); cat("secret", 42, "\n")"#]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("still running"), "{text}");
    assert!(!text.contains(".out") && !text.contains("eval/"), "it pointed to the output: {text}");
    assert!(!text.contains("secret"), "{text}");
}

/// Code with control or invisible characters can't hide what it does: the approval
/// popup shows it written out (ESC as ␛) with a warning and wraps long lines, and
/// the R and Python consoles print it written out before echoing it.
#[test]
#[ignore]
fn hidden_characters_show_in_the_popup_and_the_console() {
    let live = Live::start("hidden");
    live.link_plugin();
    live.config(r#"{"agents":"ask"}"#);
    // It runs x <- 1, but a terminal draws y <- 2 over it; the second line is long.
    let long = format!("z <- c({}9999)", "1, ".repeat(60));
    let code = format!("x <- 1 # \x1b[1K\x1b[Gy <- 2\n{long}");
    let (status, text, popup) = std::thread::scope(|s| {
        let asking = s.spawn(|| live.eval(&[&code]));
        let request = live.wait_popup(&[]);
        // The popup as you'd see it, drawn in a pane of its own.
        let env = format!("CORRAL_REQUEST={}", request.display());
        let split = live.run("herdr", &["pane", "split", &live.console, "--direction", "down", "--env", &env]);
        let pane = field(&split, "pane_id").unwrap_or_else(|| panic!("no pane in {split}"));
        live.run("herdr", &["pane", "run", &pane, "corral _approve"]);
        let mut popup = String::new();
        wait_for(Duration::from_secs(15), "the popup", || {
            popup = live.screen(&pane);
            popup.contains("9999)") || popup.split_whitespace().collect::<String>().contains("9999)")
        });
        let decision = field(&std::fs::read_to_string(&request).unwrap(), "decision").unwrap();
        std::fs::write(decision, "run").unwrap();
        let (status, text) = asking.join().unwrap();
        (status, text, popup)
    });
    assert_eq!(status, 0, "{text}");
    assert!(popup.contains("control or invisible characters"), "no warning:\n{popup}");
    assert!(popup.contains("x <- 1 # ␛[1K␛[Gy <- 2"), "the line isn't written out:\n{popup}");
    let joined: String = popup.split_whitespace().collect();
    assert!(joined.contains(&long.split_whitespace().collect::<String>()), "the long line was cut:\n{popup}");

    let mut console = String::new();
    wait_for(Duration::from_secs(10), "the console's notice", || {
        console = live.screen(&live.console);
        console.contains("#   x <- 1 # ␛[1K␛[Gy <- 2")
    });
    assert!(console.contains("written out, it says:"), "{console}");
    live.config(r#"{"agents":"on"}"#);
    let (_, text) = live.eval(&[r#"cat(exists("y"), x, length(z), "\n")"#]);
    assert!(text.contains("FALSE 1 61"), "{text}");

    // Python prints it written out too; plain code doesn't get the notice.
    let (status, text) = live.eval(&["--lang", "python", "a = 1 # \x1b[1K\x1bGb = 2"]);
    assert_eq!(status, 0, "{text}");
    let pane = live.session("python").expect("no Python session").1;
    wait_for(Duration::from_secs(10), "Python's notice", || live.screen(&pane).contains("#   a = 1 # ␛[1K␛Gb = 2"));
    let (_, text) = live.eval(&["--lang", "python", "print('b' in globals(), a)"]);
    assert!(text.contains("False 1"), "{text}");
    assert!(!text.contains("written out"), "{text}");
}

/// A sensitive session stays sensitive after a crash: its objects come back into a
/// session that is marked too, so agents still need approval (here nobody can give
/// it, so nothing runs) and never get the values. R and Python.
#[test]
#[ignore]
fn sensitive_sessions_stay_sensitive_after_a_crash() {
    let live = Live::start("sensitive");
    assert_eq!(live.eval(&["secret <- 42"]).0, 0);
    live.wait_checkpoint("r", &["secret"]);
    // Marked after the checkpoint was written: the mark reaches it at once.
    assert_eq!(live.eval(&["corral_sensitive()"]).0, 0);
    std::thread::sleep(Duration::from_secs(5)); // older than 5 s: the console starts R again by itself
    let old = live.kernel_pid();
    Command::new("kill").args(["-9", &old.to_string()]).status().unwrap();
    wait_for(Duration::from_secs(60), "a new R session", || live.session("r").is_some_and(|(p, _)| p != old));
    let (status, text) = live.eval(&["cat(secret, \"\\n\")"]);
    assert_eq!(status, 3, "it ran without approval: {text}");
    assert!(!text.contains("42"), "{text}");
    // The console shows the notice a moment later, wrapped to the pane.
    let says = |pane: &str| { let s: String = live.screen(pane).split_whitespace().collect(); s.contains("Restored") && s.contains("markedsensitive") };
    wait_for(Duration::from_secs(10), "R's notice", || says(&live.console));

    assert_eq!(live.eval(&["--lang", "python", "s = 42"]).0, 0);
    live.wait_checkpoint("python", &["s"]);
    assert_eq!(live.eval(&["--lang", "python", "corral_sensitive()"]).0, 0);
    let (pid, pane) = live.session("python").unwrap();
    Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
    let (status, text) = live.eval(&["--lang", "python", "print(s)"]);
    assert_eq!(status, 3, "it ran without approval: {text}\nscreen:\n{}", live.screen(&pane));
    assert!(!text.contains("42"), "{text}");
    wait_for(Duration::from_secs(10), "Python's notice", || says(&pane));
}

/// An agent that runs its commands in a terminal of its own still can't loosen the
/// harness by itself: from a pane herdr sees an agent in, it asks. From your own
/// terminal it doesn't.
#[test]
#[ignore]
fn an_agent_in_a_terminal_still_asks() {
    let live = Live::start("agentpty");
    live.link_plugin();
    live.config(r#"{"agents":"ask"}"#);
    let settings = || std::fs::read_to_string(live.dir.join("config/corral/config.json")).unwrap().split_whitespace().collect::<String>();
    let pane = |name: &str| {
        let split = live.run("herdr", &["pane", "split", &live.console, "--direction", "down"]);
        let pane = field(&split, "pane_id").unwrap_or_else(|| panic!("no pane in {split}"));
        live.run("herdr", &["pane", "rename", &pane, name]);
        pane
    };

    // A shell with a terminal, in a pane herdr sees Claude Code in.
    let agent = pane("agent");
    live.run("herdr", &["pane", "report-agent", "--source", "corral-test", "--agent", "claude", "--state", "working", &agent]);
    let seen = live.run("herdr", &["pane", "get", &agent]);
    assert!(seen.contains("claude"), "herdr doesn't see the agent: {seen}");
    live.run("herdr", &["pane", "run", &agent, "corral config agents on"]);
    let request = live.answer_popup(&[], "refuse");
    assert!(request.contains(r#""key":"agents""#) && request.contains(&format!(r#""agent_pane":"{agent}""#)), "{request}");
    wait_for(Duration::from_secs(10), "the refusal", || live.screen(&agent).contains("refused"));
    assert!(settings().contains(r#""agents":"ask""#), "{}", settings());

    // The same in your own terminal: no popup.
    let mine = pane("mine");
    live.run("herdr", &["pane", "run", &mine, "corral config agents on"]);
    wait_for(Duration::from_secs(10), "the change", || settings().contains(r#""agents":"on""#));
}

/// A request that never gets to run is in the audit log too, with why: here a
/// second agent gives up waiting for its turn.
#[test]
#[ignore]
fn requests_that_never_run_are_audited() {
    let live = Live::start("audit");
    let (first, second) = std::thread::scope(|s| {
        let first = s.spawn(|| live.eval(&["Sys.sleep(3); cat('first\\n')"]));
        std::thread::sleep(Duration::from_millis(300));
        let second = live.eval(&["--timeout", "0.5", "--wait-idle", "0.5", "cat('second\\n')"]);
        (first.join().unwrap(), second)
    });
    assert_eq!(first.0, 0, "{}", first.1);
    assert_eq!(second.0, 2, "{}", second.1);
    assert!(second.1.contains("another request is still running"), "{}", second.1);
    let audit = std::fs::read_to_string(live.dir.join("state/corral/audit.jsonl")).unwrap();
    let line = audit.lines().find(|l| l.contains("second")).unwrap_or_else(|| panic!("not in the audit log:\n{audit}"));
    assert!(line.contains(r#""decision":"not run""#) && line.contains(r#""status":"queue timeout""#), "{line}");
}
