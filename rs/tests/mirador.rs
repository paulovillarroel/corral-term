//! Mirador's local API answers only with its token and only to its own address.
//! A request without the token, or one carrying another Host (a page on another
//! site that rebinds its DNS name to 127.0.0.1), gets 403.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::field;

struct Mirador {
    child: Child,
    home: PathBuf,
    port: u16,
    token: String,
}

impl Drop for Mirador {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A Mirador of its own for each test: tests run at the same time.
fn start(name: &str) -> Mirador {
    let home = std::env::temp_dir().join(format!("corral-test-mirador-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_corral"))
        .arg("_mirador")
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("CORRAL_LANG", "en")
        .env("CORRAL_MIRADOR_PORT", "0") // any free port
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let info = home.join(".local/state/corral/mirador.json");
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((port, token)) = std::fs::read_to_string(&info).ok()
            .and_then(|t| Some((field(&t, "port")?.parse().ok()?, field(&t, "token")?)))
        {
            return Mirador { child, home, port, token };
        }
        assert!(Instant::now() < end, "Mirador didn't write {}", info.display());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send a raw request; the status code and the body that came back.
fn request(port: u16, method: &str, host: &str, path: &str, token: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(t) = token {
        req += &format!("X-Corral-Token: {t}\r\n");
    }
    if method == "POST" {
        req += "Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    } else {
        req += "\r\n";
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut reply = String::new();
    let _ = s.read_to_string(&mut reply);
    let code = reply.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = reply.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (code, body)
}

#[test]
fn the_api_needs_the_token_and_its_own_address() {
    let m = start("token");
    let (port, token) = (m.port, m.token.as_str());
    let here = format!("127.0.0.1:{port}");

    assert_eq!(request(port, "GET", &here, "/api/ping", Some(token)).0, 200);
    assert_eq!(request(port, "GET", &format!("localhost:{port}"), "/api/ping", Some(token)).0, 200);
    assert_eq!(request(port, "GET", &here, &format!("/api/ping?token={token}"), None).0, 200, "the token in the URL works too");

    // Without the token, or with a wrong one of the same length.
    assert_eq!(request(port, "GET", &here, "/api/ping", None).0, 403);
    assert_eq!(request(port, "GET", &here, "/", None).0, 403, "not even the page");
    let mut wrong: Vec<char> = token.chars().collect();
    wrong[0] = if wrong[0] == 'a' { 'b' } else { 'a' };
    let wrong: String = wrong.into_iter().collect();
    assert_eq!(request(port, "GET", &here, "/api/ping", Some(&wrong)).0, 403);
    assert_eq!(request(port, "GET", &here, "/api/ping", Some(&token[..token.len() - 1])).0, 403);
    assert_eq!(request(port, "POST", &here, "/api/plots/clear", None).0, 403);

    // The right token from another address: DNS rebinding, or another port.
    assert_eq!(request(port, "GET", &format!("evil.example:{port}"), "/api/ping", Some(token)).0, 403);
    assert_eq!(request(port, "GET", "127.0.0.1", "/api/ping", Some(token)).0, 403);
    assert_eq!(request(port, "GET", &format!("127.0.0.1:{}", port.wrapping_add(1)), "/api/ping", Some(token)).0, 403);
}

#[test]
fn a_session_id_cannot_leave_the_sessions_folder() {
    let m = start("session");
    let here = format!("127.0.0.1:{}", m.port);
    for sid in ["..", ".", "../..", "..%2F..", "a/b", "%2Fetc"] {
        let (code, body) = request(m.port, "GET", &here, &format!("/api/vars?session={sid}"), Some(&m.token));
        assert_eq!(code, 400, "session={sid} answered {code}: {body}");
    }
}

/// The Mirador servers running with `home` as their HOME.
#[cfg(target_os = "linux")]
fn servers(home: &Path) -> Vec<i64> {
    use std::os::unix::ffi::OsStrExt;
    let home_var = [b"HOME=", home.as_os_str().as_bytes()].concat();
    std::fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i64>().ok())
        .filter(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.split(|b| *b == 0).nth(1) == Some(b"_mirador")))
        .filter(|pid| std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|e| e.split(|b| *b == 0).any(|v| v == home_var)))
        .collect()
}

/// The Mirador servers running with `home` as their HOME: macOS has no /proc, and its
/// ps shows a process's environment after its command (-E), for this user's processes,
/// everything joined by spaces. The program's path may hold spaces too (a checkout in
/// "Work Projects"), so the command is cut at " _mirador", after a path ending in corral.
#[cfg(target_os = "macos")]
fn servers(home: &Path) -> Vec<i64> {
    let out = Command::new("ps").args(["-x", "-E", "-ww", "-o", "pid=,command="]).output().unwrap();
    let home_var = format!(" HOME={} ", home.display());
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|line| {
        let (pid, command) = line.trim_start().split_once(' ')?;
        let (program, rest) = command.split_once(" _mirador")?;
        (program.ends_with("/corral") && format!("{rest} ").contains(&home_var)).then(|| pid.parse().ok())?
    }).collect()
}

/// A HOME of its own, and no Mirador left running with it afterwards.
struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        for pid in servers(&self.0) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Mirador starts on demand (the first plot of R and of Python, the palette): callers
/// that come at the same moment share one server, the one mirador.json names, so
/// `--stop` reaches it. Before, each started its own; one exited when their shared
/// temporary file was moved, so mirador.json named a dead server and the other ran on.
#[test]
fn callers_at_the_same_time_share_one_mirador() {
    let home = Home(std::env::temp_dir().join(format!("corral-test-mirador-race-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&home.0);
    std::fs::create_dir_all(&home.0).unwrap();
    let corral = |args: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_corral"));
        c.args(args).env_clear().env("HOME", &home.0).env("PATH", "/usr/bin:/bin").env("CORRAL_LANG", "en")
            .env("CORRAL_MIRADOR_PORT", "0").env("CORRAL_NO_TELEMETRY", "1").stdin(Stdio::null());
        c
    };
    let callers: Vec<Child> = (0..4).map(|_| corral(&["mirador", "--url"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()).collect();
    let urls: Vec<String> = callers.into_iter().map(|c| {
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "a caller failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }).collect();
    assert!(urls.iter().all(|u| *u == urls[0]), "the callers got different servers: {urls:?}");
    let running = servers(&home.0);
    assert_eq!(running.len(), 1, "servers running: {running:?}");
    let info = std::fs::read_to_string(home.0.join(".local/state/corral/mirador.json")).unwrap();
    assert_eq!(field(&info, "pid").and_then(|p| p.parse::<i64>().ok()), Some(running[0]), "mirador.json names another server: {info}");
    assert!(corral(&["mirador", "--stop"]).output().unwrap().status.success());
    let end = Instant::now() + Duration::from_secs(5);
    while !servers(&home.0).is_empty() {
        assert!(Instant::now() < end, "`--stop` left {:?} running", servers(&home.0));
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// With no Mirador page open, callers that ask for its window at the same moment (R's first
/// plot and the variables pane) open one window: before, each saw no page yet and opened
/// its own. The address says where Corral will place it (fit=), so the page draws once.
#[test]
fn callers_at_the_same_time_open_one_window() {
    let home = Home(std::env::temp_dir().join(format!("corral-test-mirador-window-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&home.0);
    std::fs::create_dir_all(&home.0).unwrap();
    // The "browser" writes down each address it is asked to open. Asked for an app window
    // (--app), it gets a window of Mirador's own, the kind Corral places.
    let log = home.0.join("opened");
    let browser = home.0.join("browser.sh");
    std::fs::write(&browser, format!("#!/bin/sh\necho \"$1\" >> '{}'\n", log.display())).unwrap();
    let corral = |args: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_corral"));
        c.args(args).env_clear().env("HOME", &home.0).env("PATH", "/usr/bin:/bin").env("CORRAL_LANG", "en")
            .env("CORRAL_MIRADOR_PORT", "0").env("CORRAL_NO_TELEMETRY", "1")
            .env("CORRAL_BROWSER", format!("sh {} --app={{url}}", browser.display())).stdin(Stdio::null());
        c
    };
    // The server first, so the callers only meet at the window.
    assert!(corral(&["mirador", "--url"]).stdout(Stdio::null()).status().unwrap().success());
    let callers: Vec<Child> = (0..4).map(|_| corral(&["mirador"]).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap()).collect();
    for c in callers {
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "a caller failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    // The browser runs detached: give it a moment to write.
    std::thread::sleep(Duration::from_millis(800));
    let opened = std::fs::read_to_string(&log).unwrap_or_default();
    let lines: Vec<&str> = opened.lines().collect();
    assert_eq!(lines.len(), 1, "windows opened: {lines:?}");
    assert!(lines[0].contains("#fit=0.5x0.85"), "{}", lines[0]);
    assert!(corral(&["mirador", "--stop"]).output().unwrap().status.success());
}

#[test]
fn its_address_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let m = start("private");
    let mode = std::fs::metadata(m.home.join(".local/state/corral/mirador.json")).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "mirador.json holds the token: {mode:o}");
}
