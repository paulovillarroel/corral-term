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

    /// The process of the live R session (Ark), from its meta.json.
    fn kernel_pid(&self) -> i64 {
        let sessions = self.dir.join("state/corral/sessions");
        std::fs::read_dir(&sessions).unwrap().flatten()
            .filter_map(|d| std::fs::read_to_string(d.path().join("meta.json")).ok())
            .filter_map(|m| field(&m, "pid")?.parse::<i64>().ok())
            .find(|p| Path::new(&format!("/proc/{p}")).exists())
            .expect("no live session")
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
