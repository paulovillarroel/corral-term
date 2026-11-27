//! Live tests: a real herdr (headless), R with Ark, and Corral's console, used
//! the way agents use them. They need herdr, R, Ark (`corral ark --install`),
//! R's jsonlite and rlang, and uv for Python, so they are ignored unless you ask
//! for them (the Live tests workflow, .github/workflows/live.yml, sets all that up):
//!
//!     cargo test --release --test live -- --ignored --test-threads 1
//!
//! Each test starts its own herdr server, with its settings and Corral's state
//! in a folder under /tmp; your herdr, sessions and settings are not touched.
//! R's packages and Ark come from your user, as in a normal session.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod common;
use common::field;

struct Live {
    dir: PathBuf,
    server: Child,
    console: String,
    /// The variables pane.
    vars: String,
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
        Live::start_full(name, extra, |_| {})
    }

    /// As start_with, with `setup` run in the project folder before the console opens.
    fn start_full(name: &str, extra: &[(&str, &str)], setup: impl FnOnce(&Path)) -> Live {
        // Under /tmp itself: herdr's socket path must stay short.
        let dir = PathBuf::from(format!("/tmp/corral-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["bin", "config/corral", "state", "proj"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        setup(&dir.join("proj"));
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_corral"), dir.join("bin/corral")).unwrap();
        // The panes' shell: bash with its line editor, like a real terminal, but
        // without rc files, which could put an installed corral first on the PATH.
        let shell = dir.join("bin/shell");
        std::fs::write(&shell, "#!/bin/sh\nexec bash --norc --noprofile \"$@\"\n").unwrap();
        std::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        // Agents run without asking: there is nobody to answer a popup here. No
        // check for new versions either: tests don't ask GitHub.
        std::fs::write(dir.join("config/corral/config.json"), r#"{"agents":"on","update_check":"off"}"#).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        std::fs::write(dir.join("Rprofile"), format!("if (interactive()) source(\"{}\")\n", repo.join("r/corral.R").display())).unwrap();

        let extra = extra.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut live = Live { server: spawn_placeholder(), dir, console: String::new(), vars: String::new(), workspace: String::new(), extra };
        live.server = live.command("herdr").arg("server")
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            .expect("herdr is not on the PATH");
        let socket = live.dir.join("config/herdr/herdr.sock");
        wait_for(Duration::from_secs(10), "herdr's socket", || socket.exists());

        let created = live.run("herdr", &["workspace", "create", "--cwd", live.dir.join("proj").to_str().unwrap()]);
        live.workspace = field(&created, "workspace_id").expect("no workspace");
        let layout = live.run("corral", &["layout", "--editor", ""]);
        live.console = field(&layout, "console").unwrap_or_else(|| panic!("no console in {layout}"));
        live.vars = field(&layout, "vars").unwrap_or_default();
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

    /// Mirador for this test (no browser): its port and token.
    fn mirador(&self) -> (u16, String) {
        self.run("corral", &["mirador", "--url"]);
        let info = std::fs::read_to_string(self.dir.join("state/corral/mirador.json")).unwrap_or_default();
        (field(&info, "port").and_then(|p| p.parse().ok()).expect("no Mirador port"), field(&info, "token").expect("no Mirador token"))
    }

    /// The id of the live session in `lang`, as Mirador and the checkpoints name it.
    fn session_id(&self, lang: &str) -> String {
        let (dir, _) = self.checkpoint(lang);
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        name.split_once('-').map(|(_, id)| id.to_string()).unwrap_or(name)
    }

    fn kernel_pid(&self) -> i64 {
        self.session("r").expect("no live R session").0
    }

    /// What the pane shows.
    fn screen(&self, pane: &str) -> String {
        self.run("herdr", &["pane", "read", pane])
    }

    /// Wait until the pane shows every one of `texts`, wherever it wrapped them: a
    /// console's notices can show up a moment after the request that follows them.
    fn wait_screen(&self, pane: &str, texts: &[&str]) {
        let squeeze = |s: &str| s.split_whitespace().collect::<String>();
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = self.screen(pane);
            if texts.iter().all(|t| squeeze(&screen).contains(&squeeze(t))) {
                return;
            }
            assert!(Instant::now() < end, "the pane doesn't show {texts:?}; it shows:\n{screen}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // A test that drew plots started Mirador for its session; it outlives herdr.
        let _ = self.corral(&["mirador", "--stop"]);
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

/// Kill a process as a crash would, and wait until it is gone: a request sent
/// right after the signal could still reach it.
/// GET `path` from Mirador on 127.0.0.1:`port`: the status line and the body.
fn mirador_get(port: u16, path: &str) -> (String, Vec<u8>) {
    use std::io::{Read, Write};
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).expect("Mirador isn't listening");
    write!(c, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
    let mut all = Vec::new();
    c.read_to_end(&mut all).unwrap();
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(all.len());
    let head = String::from_utf8_lossy(&all[..split]).to_string();
    (head.lines().next().unwrap_or("").to_string(), all[split..].to_vec())
}

/// POST a JSON `body` to Mirador on 127.0.0.1:`port`: the status line.
fn mirador_post(port: u16, path: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).expect("Mirador isn't listening");
    write!(c, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut all = String::new();
    c.read_to_string(&mut all).unwrap();
    all.lines().next().unwrap_or("").to_string()
}

/// How many files with this extension a folder holds.
fn count(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir).map(|d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == ext)).count()).unwrap_or(0)
}

fn crash(pid: i64) {
    Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
    wait_for(Duration::from_secs(10), "the killed process to end", || !alive(pid));
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
    live.wait_screen(&live.console, &["R ended unexpectedly"]);
    let started = Instant::now();
    let (code, text) = live.eval(&[r#"cat("back\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("back"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(5), "it took {:?}", started.elapsed());
}

/// When R starts its plot watcher again during `corral eval` (it had died), the eval
/// doesn't wait: the watcher keeps none of R's descriptors, httpgd's socket among them.
#[test]
#[ignore]
fn an_eval_that_starts_the_watcher_again_doesnt_wait() {
    let live = Live::start_with("rwatch2", &[("CORRAL_BROWSER", "print")]);
    assert_eq!(live.eval(&["plot(1:3)"]).0, 0);
    let sid = live.session_id("r");
    let pid_file = live.dir.join(format!("state/corral/sessions/{sid}/keep-plots.pid"));
    let read_pid = || std::fs::read_to_string(&pid_file).ok().and_then(|p| p.trim().parse::<i64>().ok());
    wait_for(Duration::from_secs(10), "the watcher", || read_pid().is_some_and(alive));
    let first = read_pid().unwrap();
    crash(first);
    let start = Instant::now();
    let (code, text) = live.eval(&["plot(3:1)"]);
    let took = start.elapsed();
    assert!(code == 0 && text.contains("saved to"), "{text}");
    assert!(took < Duration::from_secs(5), "the eval took {took:?}");
    wait_for(Duration::from_secs(10), "a new watcher", || read_pid().is_some_and(|p| p != first && alive(p)));
    // Its own requests to httpgd open sockets too; one that R also has, it got from R.
    let sockets = |pid: i64| std::fs::read_dir(format!("/proc/{pid}/fd")).map(|d| d.flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok()).map(|l| l.to_string_lossy().to_string())
        .filter(|l| l.starts_with("socket:")).collect::<Vec<_>>()).unwrap_or_default();
    let r = sockets(live.kernel_pid());
    let shared: Vec<String> = sockets(read_pid().unwrap()).into_iter().filter(|s| r.contains(s)).collect();
    assert!(shared.is_empty(), "the watcher holds R's sockets: {shared:?}");
}

/// A newer Corral, once the daily check has found one, shows at the foot of the
/// variables pane and names itself in the palette, and goes once it is installed:
/// herdr's toast, the other way Corral tells, is off by default.
#[test]
#[ignore]
fn a_newer_version_shows_in_the_variables_pane() {
    let live = Live::start("newer");
    // As the daily check leaves it; checks on (the harness turns them off), but the layout
    // is already open, so nothing asks GitHub.
    live.config(r#"{"agents":"on","update_check":"on"}"#);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    std::fs::write(live.dir.join("state/corral/update-check.json"), format!(r#"{{"checked":{now},"latest":"9.9.9"}}"#)).unwrap();
    live.wait_screen(&live.vars, &["Corral 9.9.9 is out: ctrl+b space → Update"]);
    let palette = live.run("corral", &["keys", "--json"]);
    assert!(palette.contains("Update: Corral 9.9.9 is out, install it"), "{palette}");
    // Installed (the install record says 9.9.9): the pane, still the old binary, drops it.
    std::fs::write(live.dir.join("state/corral/install.json"), r#"{"version":"9.9.9"}"#).unwrap();
    wait_for(Duration::from_secs(20), "the notice to go", || !live.screen(&live.vars).contains("9.9.9"));
    assert!(!live.run("corral", &["keys", "--json"]).contains("9.9.9"));
}

/// `corral context` tells an agent, in one call and without starting anything, which
/// consoles run, whether SQL works in each, and the rules as they apply to each session.
#[test]
#[ignore]
fn context_tells_an_agent_where_it_is_and_its_rules() {
    let live = Live::start("context");
    let context = || {
        let json = live.run("corral", &["context", "--json"]);
        let at = json.find(r#""python":"#).expect("no python in the context");
        let (r, python) = json.split_at(at);
        (json.clone(), r.to_string(), python.to_string())
    };
    let (all, r, python) = context();
    // R runs, in view, with its session; SQL as R itself sees it.
    let (_, sql) = live.eval(&[r#"cat("sql:", all(nzchar(c(system.file(package = "duckdb"), system.file(package = "DBI")))), "\n")"#]);
    let sql = if sql.contains("sql: TRUE") { "true" } else { "false" };
    for part in [r#""r":{"running":true"#, r#""showing":true"#, &format!(r#""sql":{sql}"#), r#""sensitive":false,"access":"on","output":"full","profile":"open""#] {
        assert!(r.contains(part), "{part} missing in {all}");
    }
    assert_eq!(field(&r, "session"), live.session_id("r").into(), "{all}");
    // Python isn't running, says it starts on request, and asking didn't start it.
    assert!(python.contains(r#""running":false,"starts_on_request":true"#), "{all}");
    assert!(live.session("python").is_none(), "corral context started Python");
    assert!(all.contains(r#""agents":{"access":"on","output":"full","profile":"open"}"#), "{all}");
    // A session marked sensitive: that console asks and gives only structure; new ones don't.
    assert_eq!(live.eval(&["corral_sensitive()"]).0, 0);
    let (all, r, _) = context();
    assert!(r.contains(r#""sensitive":true,"access":"ask","output":"structure","profile":"sensitive""#), "{all}");
    assert!(all.contains(r#""agents":{"access":"on","output":"full","profile":"open"}"#), "{all}");
    // Agents off: it says so, the way eval would refuse.
    live.config(r#"{"agents":"off","update_check":"off"}"#);
    let (all, r, _) = context();
    assert!(r.contains(r#""access":"off""#) && all.contains(r#""agents":{"access":"off""#), "{all}");
    // In words, for the palette's popup.
    let words = live.run("corral", &["context"]);
    assert!(words.contains("R 4.") && words.contains("Python") && words.contains("can't run code"), "{words}");
    // From the palette (Agents: what your agent sees): the action opens that popup.
    live.link_plugin();
    let running = || std::fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|p| std::fs::read(p.path().join("cmdline")).ok())
        .any(|c| String::from_utf8_lossy(&c).replace('\0', " ").ends_with("corral _context "));
    live.run("herdr", &["plugin", "action", "invoke", "context", "--plugin", "corral"]);
    wait_for(Duration::from_secs(20), "the context popup", running);
}

/// A crash while a command runs: the console says why, in words, and which command it
/// was (an agent's request, read from its file), then what came back.
#[test]
#[ignore]
fn a_crash_says_why_and_what_was_running() {
    let live = Live::start("why");
    assert_eq!(live.eval(&["kept <- 1:3"]).0, 0);
    live.wait_checkpoint("r", &["kept"]);
    std::thread::sleep(Duration::from_secs(5)); // older than 5 s: the console starts R again by itself
    let mut request = live.command("corral").args(["eval", "Sys.sleep(60); cat('never\\n')"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    live.wait_screen(&live.console, &["Sys.sleep(60)"]);
    let old = live.kernel_pid();
    crash(old);
    live.wait_screen(&live.console, &["R ended unexpectedly: it was stopped from outside (signal 9), while running: Sys.sleep(60); cat('never\\n')",
                                      "Back in", "with 1 object (kept)"]);
    let _ = request.wait();
}

/// Security profiles: a project marked sensitive makes every session in it ask and give
/// only structure, whatever the global profile; its sessions keep the mark; the pane
/// shows it; and loosening it from a shell without a terminal (an agent) is refused.
#[test]
#[ignore]
fn a_sensitive_project_applies_to_its_sessions() {
    let live = Live::start("profiles");
    let proj = live.dir.join("proj").canonicalize().unwrap();
    let proj = proj.to_str().unwrap();
    // The harness runs agents on: the global profile is open, and code runs at once.
    assert!(live.run("corral", &["profile", "--json"]).contains(r#""global":"open""#));
    assert_eq!(live.eval(&["v <- \"patient-4711\""]).0, 0);
    let vars = || String::from_utf8_lossy(&live.corral(&["vars", "--json"]).stdout).to_string();
    // Before: the value's preview reaches the agent.
    wait_for(Duration::from_secs(10), "v's preview", || vars().contains("patient-4711"));
    live.wait_screen(&live.vars, &["open"]);
    // Tightening never asks.
    live.run("corral", &["profile", "sensitive", "--project", proj]);
    let shown = live.run("corral", &["profile", "--json", "--project", proj]);
    assert!(shown.contains(r#""profile":"sensitive""#) && shown.contains(r#""global":"open""#), "{shown}");
    // Now each request needs the user's approval: nobody is here to give it, so nothing runs.
    let (code, text) = live.eval(&["cat(40 + 2, '\\n')"]);
    assert_eq!(code, 3, "it ran without approval: {text}");
    assert!(!text.contains("42"), "{text}");
    let ctx = live.run("corral", &["context", "--json"]);
    assert!(ctx.contains(r#""sensitive":true,"access":"ask","output":"structure","profile":"sensitive""#), "{ctx}");
    let sid = live.session_id("r");
    assert!(live.dir.join(format!("state/corral/sessions/{sid}/sensitive")).exists(), "the session wasn't marked");
    // The values' previews go, as in any sensitive session.
    let listed = vars();
    assert!(listed.contains("\"v\"") && !listed.contains("patient-4711"), "{listed}");
    // The variables pane says so, in its header.
    live.wait_screen(&live.vars, &["sensitive"]);
    // Loosening it from a shell without a terminal, as an agent's, needs the user: refused.
    let out = live.corral(&["profile", "--project", "--clear", proj]);
    assert!(!out.status.success(), "an agent removed the project's profile: {}", String::from_utf8_lossy(&out.stdout));
    assert!(live.run("corral", &["profile", "--json", "--project", proj]).contains(r#""profile":"sensitive""#));
}

/// A request that moves the session into a sensitive project gets that project's
/// rules for its own result, not only from the next request on; the next request, sent
/// at once, already asks. Through a symlink, and past its timeout, too.
#[test]
#[ignore]
fn moving_into_a_sensitive_project_applies_at_once() {
    let live = Live::start("cdinto");
    let proj = live.dir.join("proj").canonicalize().unwrap();
    for d in ["clinic", "registry"] {
        std::fs::create_dir_all(proj.join(d)).unwrap();
        std::fs::write(proj.join(d).join("datos.txt"), "paciente-4711\n").unwrap();
    }
    std::os::unix::fs::symlink(proj.join("registry"), proj.join("link")).unwrap();
    live.run("corral", &["profile", "sensitive", "--project", proj.join("clinic").to_str().unwrap()]);
    // Registered through the symlink: kept as the real folder.
    live.run("corral", &["profile", "sensitive", "--project", proj.join("link").to_str().unwrap()]);
    // R, in one request: into the project, read. The global profile is open, so it runs,
    // but what comes back is what a sensitive project allows.
    let (code, text) = live.eval(&[r#"setwd("clinic"); cat(readLines("datos.txt"), "\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("paciente-4711") && text.contains("profile is sensitive"), "{text}");
    // The next request, at once: it asks, and nobody is here to approve it.
    let (code, text) = live.eval(&[r#"cat(readLines("datos.txt"), "\n")"#]);
    assert_eq!(code, 3, "the next request ran without approval: {text}");
    // Python, through the symlink, past its timeout: the partial output stays out too.
    assert_eq!(live.eval(&["--lang", "python", "--timeout", "180", "print('ready')"]).0, 0);
    let (code, text) = live.eval(&["--lang", "python", "--timeout", "3",
        "import os, time\nos.chdir('link')\nprint(open('datos.txt').read(), flush=True)\ntime.sleep(8)"]);
    assert_eq!(code, 2, "{text}");
    assert!(!text.contains("paciente-4711") && text.contains("profile is sensitive"), "{text}");
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

/// Without --lang, `corral vars --json` lists the console in view, like the variables
/// pane: Python's objects while Python shows, R's again once R does.
#[test]
#[ignore]
fn corral_loads_in_a_project_with_renv() {
    // A project with renv: R reads its .Rprofile, not the user's, and its library
    // hides the user's, where Corral's packages are. Corral loads anyway (r/profile.R),
    // its variables pane works (jsonlite from the user's library), and the project's
    // .libPaths() stay renv's alone. No R_PROFILE_USER of the test's own here, so the
    // project's .Rprofile is the one R would read.
    let live = Live::start_full("renv", &[("R_PROFILE_USER", "")], |proj| {
        let out = Command::new("Rscript").current_dir(proj)
            .args(["--no-init-file", "-e", "renv::init(bare = TRUE, restart = FALSE)"]).output().unwrap();
        assert!(out.status.success(), "renv::init: {}", String::from_utf8_lossy(&out.stderr));
        assert!(proj.join(".Rprofile").exists(), "renv made no .Rprofile");
    });
    let (code, text) = live.eval(&["--lang", "r", "in_renv_project <- 42"]);
    assert_eq!(code, 0, "{text}");
    wait_for(Duration::from_secs(10), "the object in the variables pane", || {
        String::from_utf8_lossy(&live.corral(&["vars", "--json"]).stdout).contains("in_renv_project")
    });
    let probe = r#"cat(sep = "", "renv first: ", grepl("renv", .libPaths()[1]),
        "; jsonlite: ", "jsonlite" %in% loadedNamespaces(),
        "; user library added: ", any(normalizePath(strsplit(Sys.getenv("CORRAL_R_LIBS_USER"), ":")[[1]], mustWork = FALSE) %in%
                                      normalizePath(.libPaths(), mustWork = FALSE)), "\n")"#;
    let (code, text) = live.eval(&["--lang", "r", probe]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("renv first: TRUE; jsonlite: TRUE; user library added: FALSE"), "{text}");
}

#[test]
#[ignore]
fn vars_follow_the_console_in_view() {
    let live = Live::start("varsview");
    let (code, text) = live.eval(&["--lang", "r", "r_only <- 1"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "python", "py_only = 2"]);
    assert_eq!(code, 0, "{text}");
    let vars = || String::from_utf8_lossy(&live.corral(&["vars", "--json"]).stdout).to_string();
    wait_for(Duration::from_secs(10), "Python's objects, without --lang", || {
        let text = vars();
        text.contains("py_only") && !text.contains("r_only")
    });
    let (code, text) = live.eval(&["--lang", "r", "r_again <- 3"]);
    assert_eq!(code, 0, "{text}");
    wait_for(Duration::from_secs(10), "R's objects again, without --lang", || {
        let text = vars();
        text.contains("r_again") && !text.contains("py_only")
    });
    // --lang still picks a console, whichever is in view.
    let text = String::from_utf8_lossy(&live.corral(&["vars", "--json", "--lang", "python"]).stdout).to_string();
    assert!(text.contains("py_only") && !text.contains("r_only"), "{text}");
}

/// SQL runs in the console in view, R or Python, on DuckDB: the session's data
/// frames are tables by name, the result stays as a data frame (sql_result, or the
/// name a `-- name:` line gives it), and a mistake comes back as DuckDB's error.
#[test]
#[ignore]
fn sql_runs_in_the_session_in_r_and_python() {
    let live = Live::start("sql");
    // A home without ~/.duckdb, as on a new machine: R's duckdb 2 would ask there
    // whether to create it, and the console would wait for an answer.
    let (code, text) = live.eval(&["--lang", "r", "Sys.setenv(HOME = file.path(tempdir(), 'home')); dir.create(Sys.getenv('HOME'))"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "r", "sales <- data.frame(region = c('North', 'South', 'North'), amount = c(10, 20, 5))"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "sql", "SELECT region, sum(amount) AS total FROM sales GROUP BY region ORDER BY region"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("North") && text.contains("15") && text.contains("sql_result"), "{text}");
    // The audit log says it was SQL, and which console it ran in.
    let audit = String::from_utf8_lossy(&live.corral(&["audit", "--last", "1", "--json"]).stdout).to_string();
    assert!(audit.contains(r#""lang":"sql""#) && audit.contains(r#""console":"r""#), "{audit}");
    // A comment before the query: `-- name:` names the result, and sales is still found.
    let (code, text) = live.eval(&["--lang", "sql", "-- name: big\n-- the larger sales, and a longer comment to shift things\nSELECT * FROM sales WHERE amount > 8;\nCREATE TABLE kept AS SELECT 1 AS a"]);
    assert_eq!(code, 0, "{text}");
    let (_, text) = live.eval(&["--lang", "r", "cat(class(big), nrow(big), class(sql_result))"]);
    assert!(text.contains("data.frame 2 data.frame"), "{text}");
    let (_, text) = live.eval(&["--lang", "sql", "SELECT nope FROM nowhere"]);
    assert!(text.contains("nowhere"), "{text}");

    // Python, once its console is the one in view.
    let (code, text) = live.eval(&["--lang", "python", "import pyarrow as pa; trips = pa.table({'city': ['Lima', 'Quito', 'Lima'], 'km': [3, 4, 5]})"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "sql", "-- name: by_city\nSELECT city, sum(km) AS km FROM trips GROUP BY city ORDER BY city"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Lima") && text.contains("8") && text.contains("by_city"), "{text}");
    let (_, text) = live.eval(&["--lang", "python", "print(by_city.num_rows if hasattr(by_city, 'num_rows') else len(by_city))"]);
    assert!(text.contains('2'), "{text}");
}

/// One SQL file gives the same results in R and in Python. R splits statements
/// with its own expression and Python with DuckDB's parser, so the file holds what
/// could tell them apart: a semicolon in every kind of string, escaped quotes,
/// nested block comments, a CTE, DDL, a file written and read back, and several
/// `-- name:` results.
#[test]
#[ignore]
fn sql_gives_the_same_results_in_r_and_python() {
    let live = Live::start("sqlsame");
    let dir = live.dir.join("proj");
    let file = dir.join("battery.sql");
    std::fs::write(&file, format!(r#"-- A semicolon in every kind of string, and in a name.
-- name: strings
SELECT 'a;b' AS plain, 'it''s; here' AS doubled, $$c;d$$ AS dollar, $tag$e;f$tag$ AS tagged,
       E'g\'h;i' AS escaped, "col;name" FROM (SELECT 1 AS "col;name");
/* a block comment; with a semicolon
   over two lines /* and one inside; */ still the comment; */
-- name: cte
WITH x AS (SELECT range AS n FROM range(5))
SELECT sum(n)::BIGINT AS total, count(*)::BIGINT AS n FROM x;
CREATE TABLE t AS SELECT range::BIGINT AS id, 'r' || range AS label FROM range(3);
COPY t TO '{dir}/t.parquet' (FORMAT parquet);
-- name: from_file
SELECT * FROM '{dir}/t.parquet' ORDER BY id;
-- name: last
SELECT count(*)::BIGINT AS n FROM t;
"#, dir = dir.display())).unwrap();
    let names = ["strings", "cte", "from_file", "last"];
    let json_of = |text: &str| text.lines().find_map(|l| l.strip_prefix("JSON:")).map(str::trim).unwrap_or("").to_string();

    let (code, text) = live.eval(&["--lang", "sql", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    let list = names.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", ");
    let (_, text) = live.eval(&["--lang", "r", &format!("cat('JSON:', jsonlite::toJSON(mget(c({list})), dataframe = 'rows', digits = NA), '\\n')")]);
    let in_r = json_of(&text);

    let (code, text) = live.eval(&["--lang", "python", "1"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = live.eval(&["--lang", "sql", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    let rows = "(lambda v: v.to_pylist() if hasattr(v, 'to_pylist') else (v.to_dicts() if hasattr(v, 'to_dicts') else v.to_dict('records')))";
    let (_, text) = live.eval(&["--lang", "python", &format!("import json; print('JSON:', json.dumps({{n: {rows}(globals()[n]) for n in [{list}]}}, default=str))")]);
    let in_python = json_of(&text);

    assert!(!in_r.is_empty() && !in_python.is_empty(), "R: {in_r}\nPython: {in_python}");
    let same = Command::new("python3").args(["-I", "-c", "import json, sys; sys.exit(0 if json.loads(sys.argv[1]) == json.loads(sys.argv[2]) else 1)", &in_r, &in_python])
        .status().unwrap().success();
    assert!(same, "different results\nR:      {in_r}\nPython: {in_python}");
    assert!(in_r.contains(r#""escaped":"g'h;i""#) && in_r.contains(r#""total":10"#), "{in_r}");
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
    live.wait_screen(&live.console, &["R ended unexpectedly: it was stopped from outside (signal 9)", "Back in",
                                      "with 5 objects (big, check, f, m, v) and 1 package (tools)", "con (a connection)"]);
    // Said before the first command after the crash printed anything.
    live.wait_screen(&live.console, &["same: TRUE 42 5"]);
    let screen: String = live.screen(&live.console).split_whitespace().collect();
    let (notice, output) = (screen.find("Backin"), screen.find("same:TRUE"));
    assert!(matches!((notice, output), (Some(n), Some(o)) if n < o), "the notice came after the command:\n{screen}");
}

#[test]
#[ignore]
fn r_plots_survive_a_crash() {
    // A crash doesn't take R's plots: the checkpoint's copy keeps each one as a file,
    // and the next R in the project shows them first, in Mirador, before its own.
    let live = Live::start_with("rplots", &[("CORRAL_BROWSER", "print")]);
    let (code, text) = live.eval(&["plot(1:10); hist(c(1, 2, 2, 3, 3, 3)); invisible(0)"]);
    assert_eq!(code, 0, "{text}");
    let (dir, _) = live.checkpoint("r");
    wait_for(Duration::from_secs(30), "both plots in the checkpoint", || {
        count(&dir.join("plots"), "png") == 2 && count(&dir.join("plots"), "svg") == 2
    });
    std::thread::sleep(Duration::from_secs(6)); // older than 5 s: the console starts R again by itself
    let old = live.kernel_pid();
    crash(old);
    let end = Instant::now() + Duration::from_secs(60);
    while !live.session("r").is_some_and(|(p, _)| p != old) {
        assert!(Instant::now() < end, "no new R session after 60 s; the console shows:\n{}", live.screen(&live.console));
        std::thread::sleep(Duration::from_millis(200));
    }
    live.wait_screen(&live.console, &["Back in", "with 2 plots, first in Mirador"]);
    let (new_dir, _) = live.checkpoint("r");
    assert_eq!(count(&new_dir.join("kept"), "png"), 2, "{new_dir:?}");
    // Mirador lists them and serves them before the new R draws anything.
    let (port, token) = live.mirador();
    let sid = live.session_id("r");
    let (status, body) = mirador_get(port, &format!("/api/plots?session={sid}&token={token}"));
    let list = String::from_utf8_lossy(&body).to_string();
    assert!(status.contains("200") && list.contains("kept-k0001") && list.contains("kept-k0002"), "{status} {list}");
    let (status, png) = mirador_get(port, &format!("/api/plot?session={sid}&id=kept-k0001&renderer=png&token={token}"));
    assert!(status.contains("200") && png.starts_with(b"\x89PNG"), "{status}");
    let (status, svg) = mirador_get(port, &format!("/api/plot?session={sid}&id=kept-k0002&token={token}"));
    assert!(status.contains("200") && String::from_utf8_lossy(&svg).contains("<svg"), "{status}");
    // A new plot comes after them.
    assert_eq!(live.eval(&["plot(3:1)"]).0, 0);
    let list = String::from_utf8_lossy(&mirador_get(port, &format!("/api/plots?session={sid}&token={token}")).1).to_string();
    let kept2 = list.find("kept-k0002").unwrap_or(usize::MAX);
    assert!(list.matches("\"id\"").count() == 3 && kept2 < list.rfind("\"id\"").unwrap(), "{list}");
    // Removed in Mirador, a kept plot goes for good: both its files.
    let status = mirador_post(port, &format!("/api/plots/remove?token={token}"), &format!(r#"{{"session":"{sid}","index":0}}"#));
    assert!(status.contains("200"), "{status}");
    let list = String::from_utf8_lossy(&mirador_get(port, &format!("/api/plots?session={sid}&token={token}")).1).to_string();
    assert!(!list.contains("kept-k0001") && list.contains("kept-k0002"), "{list}");
    assert!(!new_dir.join("kept/k0001.png").exists() && !new_dir.join("kept/k0001.svg").exists(), "k0001 left files behind");
    let _ = live.corral(&["mirador", "--stop"]);
}

#[test]
#[ignore]
fn python_figures_survive_a_crash() {
    // Python's figures too: they are already drawn when shown, so the copy only writes
    // them, and the next Python in the project shows them first.
    let live = Live::start_with("pyplots", &[("CORRAL_BROWSER", "print")]);
    let startup = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("py/startup.py");
    live.config(&format!(r#"{{"agents":"on","update_check":"off","python_console":"uv run --with ipython --with cloudpickle --with pyarrow --with duckdb --with matplotlib ipython --no-banner -i {}"}}"#,
                         startup.display()));
    let code = "import matplotlib.pyplot as plt\nkeep = 41\nplt.figure(); plt.plot([1, 2, 3]); plt.show()\nplt.figure(); plt.bar(['a', 'b'], [2, 3]); plt.show()\nprint('drawn')";
    let (status, text) = live.eval(&["--lang", "python", "--timeout", "180", code]);
    assert_eq!(status, 0, "{text}");
    let (dir, _) = live.checkpoint("python");
    wait_for(Duration::from_secs(30), "both figures in the checkpoint", || count(&dir.join("plots"), "png") == 2);
    let (pid, _) = live.session("python").unwrap();
    crash(pid);
    let (status, text) = live.eval(&["--lang", "python", "--timeout", "180", "print('back', keep + 1)"]);
    assert!(status == 0 && text.contains("back 42"), "{text}");
    let (_, pane) = live.session("python").unwrap();
    live.wait_screen(&pane, &["Restored 1 objects", "And its 2 figures"]);
    let (port, token) = live.mirador();
    let sid = live.session_id("python");
    let list = String::from_utf8_lossy(&mirador_get(port, &format!("/api/plots?session={sid}&token={token}")).1).to_string();
    assert!(list.contains("kept-k0001") && list.contains("kept-k0002"), "{list}");
    let (status, png) = mirador_get(port, &format!("/api/plot?session={sid}&id=kept-k0002&renderer=png&token={token}"));
    assert!(status.contains("200") && png.starts_with(b"\x89PNG"), "{status}");
    let _ = live.corral(&["mirador", "--stop"]);
}

/// A figure an agent replaces (`corral eval --replace`) leaves the checkpoint too, so it
/// doesn't come back after a crash. In Python the checkpoint's copy writes the figures
/// at the end of the cell, before `corral eval` removes the old one.
#[test]
#[ignore]
fn replaced_figures_leave_the_checkpoint() {
    let live = Live::start_with("pyreplace", &[("CORRAL_BROWSER", "print")]);
    let startup = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("py/startup.py");
    live.config(&format!(r#"{{"agents":"on","update_check":"off","python_console":"uv run --with ipython --with cloudpickle --with pyarrow --with duckdb --with matplotlib ipython --no-banner -i {}"}}"#,
                         startup.display()));
    let draw = |n: i32, replace: bool| {
        let code = format!("import matplotlib.pyplot as plt\nplt.figure(); plt.plot([1, {n}]); plt.show()");
        let mut args = vec!["--lang", "python", "--timeout", "180"];
        if replace { args.push("--replace"); }
        args.push(&code);
        let (status, text) = live.eval(&args);
        assert_eq!(status, 0, "{text}");
        text
    };
    draw(2, false);
    let (dir, _) = live.checkpoint("python");
    let plots = dir.join("plots");
    wait_for(Duration::from_secs(30), "the first figure in the checkpoint", || count(&plots, "png") == 1);
    let first: Vec<_> = std::fs::read_dir(&plots).unwrap().flatten().map(|f| f.file_name()).filter(|n| n.to_string_lossy().ends_with(".png")).collect();
    let text = draw(3, true);
    assert!(text.contains("replaced 1 plot"), "{text}");
    std::thread::sleep(Duration::from_secs(2));
    let now: Vec<_> = std::fs::read_dir(&plots).unwrap().flatten().map(|f| f.file_name()).filter(|n| n.to_string_lossy().ends_with(".png")).collect();
    assert!(now.len() == 1 && !now.contains(&first[0]), "the replaced figure is still kept: before {first:?}, now {now:?}");
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
    crash(pid);
    let (status, text) = live.eval(&["--lang", "python", "print('same:', d['a'], f(21), p.n, js.dumps(1), 'lk' in dir())"]);
    assert_eq!(status, 0, "{text}");
    assert!(text.contains("same: [1, 2, 3] 42 7 1 False"), "{text}");
    live.wait_screen(&pane, &["Restored", "lk (lock)"]);
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

/// The 99th percentile and the maximum of times already sorted by stats(): the
/// pauses a median hides.
fn tail(v: &[f64]) -> (f64, f64) {
    (v[((v.len() - 1) as f64 * 0.99).round() as usize], v[v.len() - 1])
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
        let (p99, max) = tail(&waits);
        phases.push((on, rss0, rss_mb(pid), m, p90, zombies_of(pid), p99, max));
    }
    // Then 1,000 more with checkpoints on: memory every 200 commands should level off.
    live.config(r#"{"agents":"on"}"#);
    let mut samples = vec![rss_mb(pid)];
    let mut more = Vec::new();
    for i in 0..1000 {
        let code = match i % 5 { 0 => "d$x <- d$x + 1", 1 => "s <- summary(d$x)", _ => "invisible(nrow(d))" };
        more.push(timing.after(&live, &[code]));
        if i % 200 == 199 {
            samples.push(rss_mb(pid));
        }
    }
    let (dir, index) = live.checkpoint("r");
    let (_, text) = live.eval(&[r#"cat(round(sum(sapply(ls(), function(n) length(serialize(get(n), NULL)))) / 2^20))"#]);
    let serialized: f64 = text.lines().last().unwrap_or("0").trim().parse().unwrap_or(0.0);
    println!("\nLong session, 200 commands in each phase:");
    for (on, rss0, rss1, m, p90, z, p99, max) in &phases {
        println!("    checkpoints {:<3}  R {rss0:.0} -> {rss1:.0} MB  wait median {m:.2} ms, p90 {p90:.2} ms, p99 {p99:.2} ms, max {max:.2} ms  leftover processes {z}",
                 if *on { "on" } else { "off" });
    }
    let (m, _, p90) = stats(&mut more);
    let (p99, max) = tail(&more);
    println!("    1,000 more, the wait: median {m:.2} ms, p90 {p90:.2} ms, p99 {p99:.2} ms, max {max:.2} ms");
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


/// CPU time a process has used, in seconds.
fn cpu_s(pid: i64) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let f: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
    let tick = |k: usize| f.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (tick(11) + tick(12)) / 100.0
}

/// R starts its plot watcher again when it isn't running, and a process that only looks
/// like one (another session's watcher on a reused pid) doesn't count.
#[test]
#[ignore]
fn r_starts_its_plot_watcher_again() {
    let live = Live::start_with("rwatch", &[("CORRAL_BROWSER", "print")]);
    assert_eq!(live.eval(&["plot(1:3)"]).0, 0);
    let sid = live.session_id("r");
    let pid_file = live.dir.join(format!("state/corral/sessions/{sid}/keep-plots.pid"));
    let read_pid = || std::fs::read_to_string(&pid_file).ok().and_then(|p| p.trim().parse::<i64>().ok());
    wait_for(Duration::from_secs(10), "the watcher", || read_pid().is_some_and(alive));
    let first = read_pid().unwrap();
    // Another session's watcher on the pid R knows: R must not take it for its own.
    let mut other = Command::new("bash").args(["-c", "exec -a 'corral _keep-plots w0-p0-1 1' sleep 120"]).spawn().unwrap();
    wait_for(Duration::from_secs(5), "the look-alike", || std::fs::read(format!("/proc/{}/cmdline", other.id())).is_ok_and(|c| String::from_utf8_lossy(&c).contains("_keep-plots")));
    crash(first);
    std::fs::write(&pid_file, other.id().to_string()).unwrap();
    assert_eq!(live.eval(&["plot(3:1)"]).0, 0);
    wait_for(Duration::from_secs(10), "a new watcher for this session", || {
        read_pid().is_some_and(|p| p != other.id() as i64 && p != first && alive(p))
    });
    let _ = other.kill();
    let _ = other.wait();
}

/// What keeping R's plots costs (docs/performance.html#checkpoints-plots): the wait
/// after a command that draws, checkpoints off and on, the one watcher process
/// (`corral _keep-plots`) at rest, the time from a plot to its file, and the disk.
#[test]
#[ignore]
fn plots_checkpoint_cost_report() {
    let file = PathBuf::from(format!("/tmp/corral-live-plotcost-{}/timing", std::process::id()));
    let live = Live::start_with("plotcost", &[("CORRAL_TIMING", file.to_str().unwrap()), ("CORRAL_BROWSER", "print")]);
    let timing = Timing(file);
    let pid = live.kernel_pid();
    let sid = live.session_id("r");
    // As Ctrl+R sends it: `corral eval` would also ask httpgd for each new plot.
    let sent = |code: &str| -> f64 {
        let before = timing.lines().len();
        let out = live.corral(&["send", "--lang", "r", code]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        wait_for(Duration::from_secs(30), "the command's timing", || timing.lines().len() > before);
        *timing.lines().last().unwrap()
    };
    sent("plot(1:3)"); // starts httpgd, and with it the watcher
    std::thread::sleep(Duration::from_secs(2));
    let mut phases = Vec::new();
    for on in [false, true, false, true] {
        live.config(if on { r#"{"agents":"on","update_check":"off"}"# } else { r#"{"agents":"on","update_check":"off","checkpoints":"off"}"# });
        std::thread::sleep(Duration::from_secs(1));
        let (rss0, cpu0) = (rss_mb(pid), cpu_s(pid));
        let mut waits: Vec<f64> = (0..100).map(|i| sent(&format!("plot(rnorm(1000), main = '{i}')"))).collect();
        std::thread::sleep(Duration::from_secs(1));
        let (m, _, p90) = stats(&mut waits);
        let (p99, max) = tail(&waits);
        phases.push((on, rss0, rss_mb(pid), cpu_s(pid) - cpu0, m, p90, p99, max));
    }
    let (dir, _) = live.checkpoint("r");
    let watcher: i64 = std::fs::read_to_string(live.dir.join(format!("state/corral/sessions/{sid}/keep-plots.pid"))).unwrap().trim().parse().unwrap();
    let watchers = std::fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
        .filter(|c| String::from_utf8_lossy(c).contains("_keep-plots") && String::from_utf8_lossy(c).contains(&sid)).count();
    let pss = std::fs::read_to_string(format!("/proc/{watcher}/smaps_rollup")).unwrap_or_default().lines()
        .find(|l| l.starts_with("Pss:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()).unwrap_or(0.0) / 1024.0;
    let cpu0 = cpu_s(watcher);
    std::thread::sleep(Duration::from_secs(30));
    let rest = cpu_s(watcher) - cpu0;
    // From a command that draws to its PNG in the checkpoint.
    let mut reach = Vec::new();
    for i in 0..10 {
        let pngs = || std::fs::read_dir(dir.join("plots")).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == "png")).map(|f| f.file_name()).collect::<Vec<_>>();
        let before = pngs();
        let start = Instant::now();
        let _ = live.corral(&["send", "--lang", "r", &format!("plot(rnorm(1000), main = 'reach {i}')")]);
        wait_for(Duration::from_secs(10), "the new PNG", || pngs().iter().any(|f| !before.contains(f)));
        reach.push(start.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(Duration::from_millis(300));
    }
    let (reach_m, _, reach_p90) = stats(&mut reach);
    // A plot too heavy to keep (a 93 MB SVG) is left out, and remembered as such.
    sent("plot(runif(1e6), runif(1e6), pch = 16)");
    wait_for(Duration::from_secs(20), "the heavy plot left out", || {
        std::fs::read_to_string(dir.join("plots/skipped")).is_ok_and(|s| !s.trim().is_empty())
    });
    let count = |ext: &str| std::fs::read_dir(dir.join("plots")).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == ext)).count();
    let (svg, png) = (count("svg"), count("png"));
    let plots_mb = folder_mb(&dir.join("plots"));

    println!("\nR plots, 100 commands that draw in each phase:");
    for (on, rss0, rss1, cpu, m, p90, p99, max) in &phases {
        println!("    checkpoints {:<3}  R {rss0:.0} -> {rss1:.0} MB, CPU {cpu:.2} s  wait median {m:.2} ms, p90 {p90:.2} ms, p99 {p99:.2} ms, max {max:.2} ms",
                 if *on { "on" } else { "off" });
    }
    println!("    watchers for this session {watchers}; at rest {pss:.1} MB (PSS), CPU {rest:.3} s in 30 s");
    println!("    from the command to its PNG in the checkpoint: median {reach_m:.0} ms, p90 {reach_p90:.0} ms");
    println!("    in the checkpoint: {svg} SVG and {png} PNG, {plots_mb:.1} MB");

    let (off, on) = (phases[2], phases[3]);
    assert!(on.4 - off.4 < 1.0, "a command that draws waited {:.2} ms more", on.4 - off.4);
    assert_eq!(watchers, 1, "one watcher per session");
    assert!(rest < 0.1, "the watcher used {rest:.2} s of CPU at rest");
    assert_eq!(png, 29, "the newest 30 plots but the heavy one, left out");
    assert!((on.2 - on.1) <= (off.2 - off.1) + 50.0, "R's memory grew more with checkpoints on");
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

    live.wait_screen(&live.console, &["written out, it says:", "#   x <- 1 # ␛[1K␛[Gy <- 2"]);
    live.config(r#"{"agents":"on"}"#);
    let (_, text) = live.eval(&[r#"cat(exists("y"), x, length(z), "\n")"#]);
    assert!(text.contains("FALSE 1 61"), "{text}");

    // Python prints it written out too; plain code doesn't get the notice.
    let (status, text) = live.eval(&["--lang", "python", "a = 1 # \x1b[1K\x1bGb = 2"]);
    assert_eq!(status, 0, "{text}");
    let pane = live.session("python").expect("no Python session").1;
    live.wait_screen(&pane, &["#   a = 1 # ␛[1K␛Gb = 2"]);
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
    live.wait_screen(&live.console, &["Back in", "marked sensitive"]);

    assert_eq!(live.eval(&["--lang", "python", "s = 42"]).0, 0);
    live.wait_checkpoint("python", &["s"]);
    assert_eq!(live.eval(&["--lang", "python", "corral_sensitive()"]).0, 0);
    let (pid, pane) = live.session("python").unwrap();
    crash(pid);
    let (status, text) = live.eval(&["--lang", "python", "print(s)"]);
    assert_eq!(status, 3, "it ran without approval: {text}\nscreen:\n{}", live.screen(&pane));
    assert!(!text.contains("42"), "{text}");
    live.wait_screen(&pane, &["Restored", "marked sensitive"]);
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

/// Update from the palette runs in a popup: it says what it found (here, a Corral
/// with no install record, so nothing is replaced) and waits for Enter to close.
#[test]
#[ignore]
fn update_from_the_palette_runs_in_a_popup() {
    let live = Live::start("update");
    live.link_plugin();
    // The popup's program, in a pane of its own so its screen can be read.
    let split = live.run("herdr", &["pane", "split", &live.console, "--direction", "down"]);
    let pane = field(&split, "pane_id").unwrap_or_else(|| panic!("no pane in {split}"));
    live.run("herdr", &["pane", "run", &pane, "corral _update"]);
    live.wait_screen(&pane, &["Press Enter to close."]);
    let screen = live.screen(&pane);
    // A version commit runs these tests before its tag: then this corral is newer
    // than the latest published one, and saying so is the right answer too.
    assert!(screen.contains("is the latest version") || screen.contains("update it there")
            || screen.contains("newer than the latest"), "{screen}");
    live.run("herdr", &["pane", "send-text", &pane, "\r"]);
    wait_for(Duration::from_secs(10), "the popup to close on Enter", || !live.screen(&pane).trim_end().ends_with("Press Enter to close."));

    // From the palette: the action opens that popup.
    let running = || std::fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|p| std::fs::read(p.path().join("cmdline")).ok())
        .any(|c| String::from_utf8_lossy(&c).replace('\0', " ").ends_with("corral _update "));
    live.run("herdr", &["plugin", "action", "invoke", "update", "--plugin", "corral"]);
    wait_for(Duration::from_secs(20), "the update popup", running);
}

/// A shell that asks something as it starts (zsh's compinit does, about insecure
/// folders) takes the first key typed as its answer. Corral's commands still start
/// whole in every pane: the console, and the variables.
#[test]
#[ignore]
fn a_shell_that_asks_at_startup_still_starts_corral() {
    let ask = std::env::temp_dir().join(format!("corral-asking-shell-{}", std::process::id()));
    std::fs::write(&ask, "#!/bin/bash\nread -r -n 1 -p 'Ignore insecure directories and continue [y] or abort compinit [n]? ' answer\necho\nexec bash --norc --noprofile \"$@\"\n").unwrap();
    std::fs::set_permissions(&ask, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let live = Live::start_with("asks", &[("SHELL", ask.to_str().unwrap())]);
    let _ = std::fs::remove_file(&ask);
    let (code, text) = live.eval(&[r#"cat("works\n")"#]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("works"), "{text}");
    let panes = live.run("herdr", &["pane", "list"]);
    let vars = panes.split("\"pane_id\":\"").skip(1).find(|p| p.contains("corral:vars"))
        .and_then(|p| p.split('"').next()).unwrap_or_else(|| panic!("no variables pane in {panes}")).to_string();
    wait_for(Duration::from_secs(10), "the variables pane", || {
        live.run("herdr", &["pane", "process-info", "--pane", &vars]).contains("corral")
    });
    let screen = live.screen(&vars);
    assert!(!screen.contains("command not found"), "{screen}");
}

/// Without `corral config agent`, the agent pane runs the first agent installed:
/// here only opencode is (a stand-in for it), not Claude Code, and the layout
/// opens opencode.
#[test]
#[ignore]
fn the_agent_pane_runs_the_agent_that_is_installed() {
    let live = Live::start("agentpick");
    let fake = live.dir.join("bin/opencode");
    std::fs::write(&fake, "#!/bin/sh\necho 'opencode is running'\nexec sleep 600\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let herdr = String::from_utf8(Command::new("sh").args(["-c", "command -v herdr"]).output().unwrap().stdout).unwrap();
    let created = live.run("herdr", &["workspace", "create", "--cwd", live.dir.join("proj").to_str().unwrap()]);
    let ws = field(&created, "workspace_id").expect("no workspace");
    // A PATH with no agent but the stand-in: no Claude Code, Codex or Gemini CLI.
    let out = live.command("corral").env_remove("CORRAL_AGENT")
        .env("PATH", format!("{}:/usr/local/bin:/usr/bin:/bin", live.dir.join("bin").display()))
        .env("HERDR_BIN_PATH", herdr.trim()).env("HERDR_WORKSPACE_ID", &ws)
        .args(["layout", "--editor", ""]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    wait_for(Duration::from_secs(20), "opencode in the agent pane", || {
        let panes = live.run("herdr", &["pane", "list", "--workspace", &ws]);
        panes.split("\"pane_id\":\"").skip(1).filter_map(|p| p.split('"').next())
            .any(|p| live.screen(p).contains("opencode is running"))
    });
}
