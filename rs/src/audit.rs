//! The local audit log: every request an agent makes with `corral eval` (what it
//! ran, in which console, whether you approved it, what came back), one JSON line
//! each in ~/.local/state/corral/audit.jsonl. Nothing leaves your machine.
//! `corral audit` shows the latest ones; `--json` prints the lines as they are.
//! Entries older than `corral config audit_days` (90 by default) go away.

use crate::env;
use crate::i18n::tr;
use crate::json::{self, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub fn file() -> PathBuf {
    env::state_dir().join("audit.jsonl")
}

/// Days the log keeps when `corral config audit_days` doesn't say.
pub const KEEP_DAYS: i64 = 90;

/// How many days the log keeps (`corral config audit_days`): 90 unless you set
/// another number; None for "forever", which keeps everything.
pub fn keep_days() -> Option<i64> {
    let v = env::config().get("audit_days").clone();
    if v.str() == Some("forever") {
        return None;
    }
    let n = v.str().and_then(|s| s.trim().parse::<i64>().ok()).or_else(|| v.num().map(|n| n as i64));
    Some(n.filter(|d| *d >= 1).unwrap_or(KEEP_DAYS))
}

/// Days since 1970-01-01 of a date in the Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// The day of an entry, from its "time" ("YYYY-MM-DD hh:mm:ss", local).
fn entry_day(line: &str) -> Option<i64> {
    let entry = json::parse(line).ok()?;
    let date = entry.get("time").str()?.get(..10)?.to_string();
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    Some(days_from_civil(y, m, d))
}

/// Delete the entries older than the days the log keeps. Only the first line is
/// read when there is nothing to delete, which is almost always.
pub fn prune() {
    let Some(keep) = keep_days() else { return };
    let path = file();
    let Ok(f) = std::fs::File::open(&path) else { return };
    let mut first = String::new();
    if BufReader::new(f).read_line(&mut first).is_err() {
        return;
    }
    let (y, mo, d, ..) = env::local_now();
    let cutoff = days_from_civil(y as i64, mo as i64, d as i64) - keep;
    if entry_day(&first).is_none_or(|day| day >= cutoff) {
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let kept: String = text.lines().filter(|l| entry_day(l).is_none_or(|day| day >= cutoff)).map(|l| format!("{l}\n")).collect();
    let _ = env::write_private(&path, kept.as_bytes());
}

/// One line in the log. `time` is added here; failures to write never stop the request.
pub fn record(mut entry: Value) {
    prune();
    let (y, mo, d, h, mi, s) = env::local_now();
    entry.set("time", format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}").into());
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = env::append_private(&path) {
        let _ = writeln!(f, "{}", entry.to_json());
    }
}

fn usage() -> &'static str {
    tr("usage: corral audit [--last N] [--json]", "uso: corral audit [--last N] [--json]")
}

/// `corral audit [--last N] [--json]`.
pub fn run(args: &[String]) -> Result<(), String> {
    let (mut last, mut raw) = (20usize, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--last" => last = it.next().and_then(|n| n.parse().ok()).ok_or(usage())?,
            "--json" => raw = true,
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => return Err(trf!("unknown option {other}\n{}", "opción desconocida {other}\n{}", usage())),
        }
    }
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let shown = &lines[lines.len().saturating_sub(last)..];
    if raw {
        for l in shown {
            println!("{l}");
        }
        return Ok(());
    }
    if shown.is_empty() {
        println!("{}", tr("No agent requests yet.", "Todavía no hay peticiones de agentes."));
        return Ok(());
    }
    for l in shown {
        let e = json::parse(l).unwrap_or(Value::Null);
        let outcome = match (e.get("decision").str(), e.get("status").str()) {
            (Some("ran"), Some(st)) => st.to_string(),
            (Some(d), _) => d.to_string(),
            _ => "?".into(),
        };
        let code = e.get("code").str().unwrap_or("").replace('\n', " ⏎ ");
        let code: String = if code.chars().count() > 70 { code.chars().take(69).collect::<String>() + "…" } else { code };
        println!("{}  {:<12} {:<6} {:<8} {}", e.get("time").str().unwrap_or(""), outcome,
                 e.get("lang").str().unwrap_or(""), e.get("agent_pane").str().unwrap_or("-"), code);
    }
    println!("\n{}", trf!("{} in all · {}", "{} en total · {}", lines.len(), file().display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_count_from_1970() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28), 2); // leap year
        assert_eq!(days_from_civil(2026, 11, 23) - days_from_civil(2026, 8, 25), 90);
    }

    #[test]
    fn entry_day_reads_the_time() {
        let line = r#"{"code":"1+1","time":"2026-11-23 10:15:00"}"#;
        assert_eq!(entry_day(line), Some(days_from_civil(2026, 11, 23)));
        assert_eq!(entry_day(r#"{"code":"x"}"#), None);
        assert_eq!(entry_day("not json"), None);
    }
}
