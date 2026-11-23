//! Every command prints its usage on -h and --help, exits 0, and stops there:
//! with an empty home and no herdr on the PATH, a command that went further
//! would fail or leave files behind.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const COMMANDS: &[&str] = &[
    "layout", "close", "toggle", "switch", "script", "console", "keys",
    "send", "eval", "audit", "vars", "view", "plots", "mirador", "open",
    "config", "theme", "ark", "doctor", "install", "uninstall",
];

fn empty_home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corral-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn corral(home: &Path, lang: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_corral"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", home.join("bin"))
        .env("CORRAL_LANG", lang)
        .output()
        .unwrap()
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

#[test]
fn every_command_prints_its_usage() {
    let home = empty_home("help");
    for cmd in COMMANDS {
        for flag in ["--help", "-h"] {
            for (lang, word) in [("en", "usage"), ("es", "uso")] {
                let out = corral(&home, lang, &[cmd, flag]);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let what = format!("corral {cmd} {flag} (CORRAL_LANG={lang})");
                assert!(out.status.success(), "{what} failed: {stderr}");
                assert!(stdout.starts_with(&format!("{word}: corral {cmd}")), "{what} printed: {stdout}");
                assert!(stderr.is_empty(), "{what} complained: {stderr}");
            }
        }
    }
    assert!(is_empty(&home), "corral COMMAND --help left files in {}", home.display());
    let _ = std::fs::remove_dir_all(&home);
}

/// Commands that change things and take no arguments refuse a typo like -help
/// instead of running: uninstall would remove Corral, close would close the panes.
#[test]
fn a_typo_does_not_run_them() {
    let home = empty_home("typo");
    for cmd in ["uninstall", "close", "doctor", "theme"] {
        let out = corral(&home, "en", &[cmd, "-help"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "corral {cmd} -help ran");
        assert!(out.stdout.is_empty(), "corral {cmd} -help started: {}", String::from_utf8_lossy(&out.stdout));
        assert!(stderr.contains("unknown option -help") && stderr.contains(&format!("usage: corral {cmd}")), "corral {cmd}: {stderr}");
    }
    assert!(is_empty(&home));
    let _ = std::fs::remove_dir_all(&home);
}
