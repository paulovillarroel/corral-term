//! herdr's socket API: one newline-terminated JSON request, one JSON reply.
//! Talking to the socket directly avoids starting the herdr CLI for every call.

use crate::json::{self, obj, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HERDR_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    crate::env::home().join(".config/herdr/herdr.sock")
}

pub fn call(method: &str, params: Value) -> Result<Value, String> {
    let mut sock = UnixStream::connect(socket_path()).map_err(|e| format!("no pude hablar con herdr: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
    sock.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let request = obj(vec![("id", "corral".into()), ("method", method.into()), ("params", params)]);
    sock.write_all(format!("{}\n", request.to_json()).as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&sock).read_line(&mut line).map_err(|e| e.to_string())?;
    let reply = json::parse(line.trim())?;
    if !reply.get("error").is_null() {
        let msg = reply.get("error").get("message").str().unwrap_or("herdr falló");
        return Err(msg.to_string());
    }
    Ok(reply.get("result").clone())
}

pub fn panes(workspace: Option<&str>) -> Result<Vec<Value>, String> {
    let params = obj(vec![("workspace_id", workspace.map(Value::from).unwrap_or(Value::Null))]);
    Ok(call("pane.list", params)?.get("panes").arr().to_vec())
}

pub fn tabs(workspace: &str) -> Result<Vec<Value>, String> {
    Ok(call("tab.list", obj(vec![("workspace_id", workspace.into())]))?.get("tabs").arr().to_vec())
}

pub fn send_text(pane: &str, text: &str) -> Result<(), String> {
    call("pane.send_text", obj(vec![("pane_id", pane.into()), ("text", text.into())])).map(|_| ())
}

pub fn focus(pane: &str) -> Result<(), String> {
    call("pane.focus", obj(vec![("pane_id", pane.into())])).map(|_| ())
}

/// Find a pane by label among `panes`.
pub fn labeled<'a>(panes: &'a [Value], label: &str) -> Option<&'a Value> {
    panes.iter().find(|p| p.get("label").str() == Some(label))
}

pub fn pane_id(p: &Value) -> &str {
    p.get("pane_id").str().unwrap_or("")
}

pub fn tab_id(p: &Value) -> &str {
    p.get("tab_id").str().unwrap_or("")
}

/// herdr's command line, for what has no socket call here (split, rename, run,
/// tab create, process info, plugin panes). Returns its JSON `result`.
pub fn cli(args: &[&str]) -> Result<Value, String> {
    let bin = std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from)
        .or_else(|| crate::env::which("herdr")).ok_or("herdr no está en el PATH")?;
    let out = Command::new(bin).args(args).output().map_err(|e| format!("no pude ejecutar herdr: {e}"))?;
    let data = json::parse(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or_else(|_| Value::obj());
    if !out.status.success() || !data.get("error").is_null() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = data.get("error").get("message").str().map(String::from)
            .unwrap_or(if stderr.is_empty() { format!("herdr {} falló", args.join(" ")) } else { stderr });
        return Err(msg);
    }
    Ok(if data.get("result").is_null() { data } else { data.get("result").clone() })
}

/// Type a command in a pane and press Enter.
pub fn run(pane: &str, command: &str) -> Result<(), String> {
    cli(&["pane", "run", pane, command]).map(|_| ())
}

pub fn rename(pane: &str, label: &str) -> Result<(), String> {
    cli(&["pane", "rename", pane, label]).map(|_| ())
}

/// Split `pane`; the new pane's id.
pub fn split(pane: &str, direction: &str, ratio: Option<f64>, cwd: Option<&str>) -> Result<String, String> {
    let ratio = ratio.map(|r| r.to_string());
    let mut args = vec!["pane", "split", pane, "--direction", direction, "--no-focus"];
    if let Some(r) = &ratio {
        args.extend(["--ratio", r.as_str()]);
    }
    if let Some(c) = cwd {
        args.extend(["--cwd", c]);
    }
    let res = cli(&args)?;
    res.get("pane").get("pane_id").str().map(String::from).ok_or_else(|| "herdr no dijo qué panel creó".into())
}
