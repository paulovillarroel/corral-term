//! Every connection Corral makes beyond the machine is in the security page's list
//! (site/docs/security.html, its rows with id="net-…"), and every row there is still in
//! the code. The code marks each place that can open a connection with `net: ID` on its
//! line or the three before it: `net: local` for 127.0.0.1 and Unix sockets, `net: text`
//! where a message only names a command. A new one fails this test until it is marked
//! and listed, so the list stays the whole story.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// The files that make up Corral, with what in each can open a connection.
fn sources() -> Vec<(PathBuf, &'static [&'static str])> {
    const RUST: &[&str] = &["Command::new(\"curl\")", "which(\"curl\")", "Command::new(\"uv\")", "Command::new(\"uvx\")",
                            "Command::new(\"gh\")", "Command::new(\"wget\")", "TcpStream::connect", "UdpSocket", "TcpListener::bind",
                            "pak::pak", "install.packages(", "setup::download(", "github_digest("];
    const SHELL: &[&str] = &["curl -f", "wget ", "gh attestation", "gh auth status"];
    const R: &[&str] = &["download.file(", "install.packages(", "pak::", "url(\"http", "socketConnection(", "curl::", "httr"];
    const PYTHON: &[&str] = &["urllib.request", "urlopen(", "requests.", "http.client", "socket.socket(", "create_connection(", "pip install"];
    const PAGE: &[&str] = &["http://", "https://", "new WebSocket(", "navigator.sendBeacon("];
    const POWERSHELL: &[&str] = &["Invoke-WebRequest", "Invoke-RestMethod", "WebClient", "http://", "https://"];
    let mut out = Vec::new();
    let mut add = |dir: &str, ext: &str, kinds: &'static [&'static str]| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(repo().join(dir)).unwrap().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == ext)).collect();
        files.sort();
        out.extend(files.into_iter().map(|f| (f, kinds)));
    };
    add("rs/src", "rs", RUST);
    add("r", "R", R);
    add("py", "py", PYTHON);
    add("mirador", "html", PAGE);
    add("lib", "ps1", POWERSHELL);
    out.push((repo().join("install.sh"), SHELL));
    out
}

/// Each place that can open a connection: (file:line, the line, its mark).
fn marked_places() -> Vec<(String, String, Option<String>)> {
    let mut found = Vec::new();
    for (file, kinds) in sources() {
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // Rust's tests come last in each file; they don't run in Corral.
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if !kinds.iter().any(|k| line.contains(k)) {
                continue;
            }
            let mark = lines[i.saturating_sub(3)..=i].iter().rev().find_map(|l| {
                let at = l.find("net: ")?;
                Some(l[at + 5..].split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).next().unwrap_or("").to_string())
            });
            let shown = file.strip_prefix(repo()).unwrap().display().to_string();
            found.push((format!("{shown}:{}", i + 1), line.trim().to_string(), mark));
        }
    }
    found
}

/// The ids of the rows in the security page's list of connections.
fn listed() -> BTreeSet<String> {
    let page = std::fs::read_to_string(repo().join("site/docs/security.html")).unwrap();
    page.split("id=\"net-").skip(1).filter_map(|rest| rest.split('"').next().map(String::from)).collect()
}

#[test]
fn every_connection_is_marked_and_listed() {
    let places = marked_places();
    let unmarked: Vec<String> = places.iter().filter(|(_, _, m)| m.is_none()).map(|(at, l, _)| format!("{at}: {l}")).collect();
    assert!(unmarked.is_empty(), "not marked with `net: ID` (an id from the security page's list, local or text):\n{}", unmarked.join("\n"));
    let listed = listed();
    assert!(!listed.is_empty(), "the security page has no list of connections");
    let used: BTreeSet<String> = places.iter().filter_map(|(_, _, m)| m.clone()).filter(|m| m != "local" && m != "text").collect();
    let unlisted: Vec<&String> = used.difference(&listed).collect();
    assert!(unlisted.is_empty(), "marked in the code but not in the security page's list: {unlisted:?}");
    let gone: Vec<&String> = listed.difference(&used).collect();
    assert!(gone.is_empty(), "in the security page's list but nowhere in the code: {gone:?}");
}

/// The test sees what it looks for: a line that opens a connection, unmarked, is caught.
#[test]
fn an_unmarked_connection_is_caught() {
    let kinds: &[&str] = &["Command::new(\"curl\")"];
    let line = "    let out = Command::new(\"curl\").arg(url).output();";
    assert!(kinds.iter().any(|k| line.contains(k)));
    let text = "fn f() {\n    // fetch it\n    let out = Command::new(\"curl\").arg(url).output();\n}";
    let lines: Vec<&str> = text.lines().collect();
    let mark = lines[0..=2].iter().rev().find_map(|l| l.find("net: "));
    assert!(mark.is_none());
}

/// The sensitive profile keeps herdr from asking herdr.dev: `corral profile sensitive`
/// turns its two checks off in herdr's config, and another profile puts that config back
/// as it was, a setting of yours included.
#[test]
fn the_sensitive_profile_turns_herdrs_checks_off() {
    let home = std::env::temp_dir().join(format!("corral-test-sensitive-herdr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let herdr = home.join("config/herdr/config.toml");
    std::fs::create_dir_all(herdr.parent().unwrap()).unwrap();
    let mine = "[keys]\nprefix = \"ctrl+b\"\n\n[update]\nversion_check = true\n";
    std::fs::write(&herdr, mine).unwrap();
    let corral = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_corral")).args(args).env_clear()
            .env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config")).env("XDG_STATE_HOME", home.join("state"))
            .env("PATH", "/usr/bin:/bin").env("CORRAL_LANG", "en").env("CORRAL_NO_TELEMETRY", "1")
            // As the palette runs it: loosening the profile doesn't wait for an approval.
            .env("HERDR_PLUGIN_ID", "corral").output().unwrap();
        assert!(out.status.success(), "corral {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    corral(&["profile", "sensitive"]);
    let off = std::fs::read_to_string(&herdr).unwrap();
    assert!(off.contains("version_check = false") && off.contains("manifest_check = false") && off.contains("[keys]"), "{off}");
    corral(&["profile", "standard"]);
    assert_eq!(std::fs::read_to_string(&herdr).unwrap(), mine);
    let _ = std::fs::remove_dir_all(&home);
}

/// A first installation that stops for something missing still says what: with no
/// config.json yet, the install count's guard (which reads it) must not end the installer
/// first, as `set -eu` would on a failed read. The count goes to a closed local port.
#[test]
fn an_install_that_stops_says_why_without_a_config() {
    let home = std::env::temp_dir().join(format!("corral-test-install-stop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // Every program of the system but tar.
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for dir in ["/usr/bin", "/bin"] {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name();
            if name != "tar" && !bin.join(&name).exists() {
                let _ = std::os::unix::fs::symlink(e.path(), bin.join(&name));
            }
        }
    }
    let out = std::process::Command::new("/bin/sh").arg(repo().join("install.sh")).env_clear()
        .env("HOME", &home).env("PATH", &bin).env("CORRAL_LANG", "en")
        .env("CORRAL_TELEMETRY_URL", "http://127.0.0.1:9/").stdin(std::process::Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(said.contains("Some basic programs are missing") && said.contains("tar"), "it stopped without saying why: {said:?}");
    let _ = std::fs::remove_dir_all(&home);
}
