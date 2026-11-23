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
}

impl Live {
    /// A herdr server, a workspace and a Corral layout with an R console, ready.
    fn start(name: &str) -> Live {
        // Under /tmp itself: herdr's socket path must stay short.
        let dir = PathBuf::from(format!("/tmp/corral-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["bin", "config/corral", "state", "proj"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_corral"), dir.join("bin/corral")).unwrap();
        // Agents run without asking: there is nobody to answer a popup here.
        std::fs::write(dir.join("config/corral/config.json"), r#"{"agents":"on"}"#).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        std::fs::write(dir.join("Rprofile"), format!("if (interactive()) source(\"{}\")\n", repo.join("r/corral.R").display())).unwrap();

        let mut live = Live { server: spawn_placeholder(), dir, console: String::new(), workspace: String::new() };
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
            // Panes run a shell that reads no rc file, so they keep this PATH and
            // run the corral under test, not the one installed in ~/.local/bin.
            .env("SHELL", "/bin/sh")
            .env_remove("ENV")
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
    let out = live.corral(&["vars", "--json"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("v14"));
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
