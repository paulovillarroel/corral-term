//! Security profiles: named sets of the harness's own settings, for all your work or
//! for one project folder. They add no new mechanism: a profile is `agents` (who must
//! approve a request) and `agent_output` (what comes back), and `sensitive` also marks
//! every session that starts in its project, as corral_sensitive() does.
//!
//!   open       agents on,  output full        public or synthetic data
//!   standard   agents ask, output full        the default
//!   sensitive  agents ask, output structure   personal or clinical data
//!
//! The global profile writes `agents` and `agent_output`. A project's profile is kept
//! in Corral's settings (`projects`, by folder), never in the repository, and applies to
//! sessions that started or run in that folder or below it. The stricter one wins: a
//! project can only tighten. Loosening from an agent's pane waits for the user's
//! approval, as `corral config` does.

use crate::env;
use crate::i18n::tr;
use crate::json::{obj, Value};
use std::path::{Path, PathBuf};

pub(crate) const PROFILES: [(&str, &str, &str); 3] = [
    ("open", "on", "full"),
    ("standard", "ask", "full"),
    ("sensitive", "ask", "structure"),
];

fn access_rank(a: &str) -> u8 {
    match a { "off" => 0, "on" => 2, _ => 1 }
}

/// The stricter of two output modes.
pub(crate) fn stricter_output(a: &'static str, b: &'static str) -> &'static str {
    if output_rank(b) < output_rank(a) { b } else { a }
}

fn output_rank(o: &str) -> u8 {
    match o { "structure" => 0, "review" => 1, _ => 2 }
}

fn profile_rank(name: &str) -> u8 {
    match name { "sensitive" => 0, "standard" => 1, _ => 2 }
}

/// A profile's name in the user's language.
pub(crate) fn shown(name: &str) -> &'static str {
    match name {
        "open" => tr("open", "abierto"),
        "standard" => tr("standard", "estándar"),
        "sensitive" => tr("sensitive", "sensible"),
        _ => tr("custom", "a medida"),
    }
}

/// The rules for one session, or for a folder where none has started yet.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Rules {
    pub access: &'static str,
    pub output: &'static str,
    pub sensitive: bool,
    /// The profile these rules match (`custom` for any other mix), and the project
    /// folder whose profile applies, if one does.
    pub profile: &'static str,
    pub project: Option<(String, String)>,
}

/// The profile a mix of settings is: one of PROFILES, or `custom`.
fn name_of(access: &str, output: &str, sensitive: bool) -> &'static str {
    if sensitive || (access == "ask" && output == "structure") {
        return "sensitive";
    }
    PROFILES.iter().find(|(_, a, o)| *a == access && *o == output).map(|(n, _, _)| *n).unwrap_or("custom")
}

/// Is `path` this folder or inside it? Whole components only: /data/x isn't in /data/xy.
fn within(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
}

/// The strictest project profile among the folders `paths` are in.
fn project_for(cfg: &Value, paths: &[PathBuf]) -> Option<(String, String)> {
    let Value::Obj(projects) = cfg.get("projects") else { return None };
    projects.iter()
        .filter_map(|(dir, v)| v.str().map(|name| (dir.clone(), name.to_string())))
        .filter(|(_, name)| PROFILES.iter().any(|(n, _, _)| n == name))
        .filter(|(dir, _)| paths.iter().any(|p| within(p, Path::new(dir))))
        .min_by_key(|(dir, name)| (profile_rank(name), std::cmp::Reverse(dir.len())))
}

/// The settings, the project profile and the session's own mark, the stricter winning.
fn combine(cfg: &Value, paths: &[PathBuf], marked: bool) -> Rules {
    let global_access = crate::eval::access_from(cfg.get("agents").str());
    let global_output = cfg.get("agent_output").str().unwrap_or("full");
    let project = project_for(cfg, paths);
    let (mut access, mut output, mut sensitive) = (global_access, global_output.to_string(), marked);
    if let Some((_, name)) = &project {
        let (_, a, o) = PROFILES.iter().find(|(n, _, _)| n == name).copied().unwrap_or(PROFILES[1]);
        if access_rank(a) < access_rank(access) {
            access = a;
        }
        if output_rank(o) < output_rank(&output) {
            output = o.to_string();
        }
        sensitive |= name == "sensitive";
    }
    let (access, output) = crate::eval::policy(access, Some(&output), sensitive);
    Rules { access, output, sensitive, profile: name_of(access, output, sensitive), project }
}

/// The folder a session's process is in right now, as the kernel has it (/proc/PID/cwd).
/// A setwd() or os.chdir() shows here at once, before R or Python note it in meta.json
/// after the command: a request that follows straight away, or the one that moved, is
/// judged by where the session really is. None where there is no /proc.
pub(crate) fn live_cwd(meta: &Value) -> Option<PathBuf> {
    let pid = meta.get("pid").num()? as i64;
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// The rules for a session (its folder and meta.json). A session that started or runs in
/// a `sensitive` project gets the same mark corral_sensitive() leaves, and keeps it: the
/// rules don't loosen when it moves to another folder, and an agent can't unmark it.
pub(crate) fn session_rules(dir: &Path, meta: &Value) -> Rules {
    session_rules_seen(dir, meta, None)
}

/// As session_rules, counting a folder the session was seen in during a request too
/// (`corral eval` looks while it waits, for a console that ends before it can be read).
pub(crate) fn session_rules_seen(dir: &Path, meta: &Value, seen: Option<&Path>) -> Rules {
    let mut paths: Vec<PathBuf> = [meta.get("started_in").str(), meta.get("cwd").str()].into_iter().flatten().map(PathBuf::from).collect();
    paths.extend(live_cwd(meta));
    paths.extend(seen.map(Path::to_path_buf));
    let marker = dir.join("sensitive");
    let rules = combine(&env::config(), &paths, marker.exists());
    if rules.sensitive && !marker.exists() && dir.is_dir() {
        let _ = std::fs::write(&marker, "1\n");
    }
    rules
}

/// The rules for a session that would start in `folder` now.
pub(crate) fn folder_rules(folder: &Path) -> Rules {
    combine(&env::config(), &[folder.to_path_buf()], false)
}

/// The folder `corral profile --project` means: the one given, else the folder the
/// console in view started in, else the current one.
/// The folder the console in view started in, as its session wrote it. None without a console.
fn console_folder() -> Option<PathBuf> {
    let s = crate::console::pick_session()?;
    Some(PathBuf::from(s.get("started_in").str().or(s.get("cwd").str())?))
}

/// The console the palette speaks of: the one in view, and only in this workspace (with
/// none here, pick_session falls back to another workspace's, which isn't this one).
fn console_here() -> Option<Value> {
    let s = crate::console::pick_session()?;
    match env::current_workspace().filter(|_| env::in_herdr()) {
        Some(ws) if s.get("workspace_id").str() != Some(ws.as_str()) => None,
        _ => Some(s),
    }
}

/// A mix of the two settings, and the profile it makes (`custom` for another).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Mix {
    pub access: &'static str,
    pub output: &'static str,
    pub profile: &'static str,
}

/// What is in effect, for the palette: the global settings, the rules the console in
/// view runs by (its project's profile, its own sensitive mark and the folders it is in
/// can tighten them), and its project, if there is a console here.
pub(crate) struct InEffect {
    pub global: Mix,
    pub console: Mix,
    pub project: Option<Project>,
}

/// The console's project: the folder it started in (what `--project` acts on), the
/// profile set for that very folder, the one that applies there (maybe from a folder
/// above), and the profile the folder would have without its own.
pub(crate) struct Project {
    pub dir: PathBuf,
    pub own: Option<String>,
    pub applies: Option<String>,
    pub without_own: Mix,
}

pub(crate) fn in_effect() -> InEffect {
    let cfg = env::config();
    let (access, output) = crate::eval::policy(crate::eval::access_from(cfg.get("agents").str()), cfg.get("agent_output").str(), false);
    let global = Mix { access, output, profile: name_of(access, output, false) };
    let session = console_here();
    let console = session.as_ref().map(|s| {
        let r = session_rules(Path::new(s.get("dir").str().unwrap_or("")), s);
        Mix { access: r.access, output: r.output, profile: r.profile }
    }).unwrap_or(global);
    let project = session.as_ref()
        .and_then(|s| s.get("started_in").str().or(s.get("cwd").str()).map(PathBuf::from))
        .and_then(|d| std::fs::canonicalize(d).ok()).filter(|d| d.is_dir())
        .map(|dir| {
            let key = dir.display().to_string();
            let own = cfg.get("projects").get(&key).str().map(String::from);
            let applies = project_for(&cfg, std::slice::from_ref(&dir)).map(|(_, name)| name);
            let mut rest = cfg.clone();
            if let Value::Obj(p) = cfg.get("projects") {
                rest.set("projects", Value::Obj(p.iter().filter(|(k, _)| *k != key).cloned().collect()));
            }
            let r = combine(&rest, std::slice::from_ref(&dir), false);
            Project { dir, own, applies, without_own: Mix { access: r.access, output: r.output, profile: r.profile } }
        });
    InEffect { global, console, project }
}

/// A mix of settings in words, for a custom profile: "ask, you review results".
pub(crate) fn mix_in_words(access: &str, output: &str) -> String {
    let a = match access {
        "on" => tr("run without asking", "ejecutan sin preguntar"),
        "off" => tr("can't run code", "no ejecutan código"),
        _ => tr("ask", "preguntan"),
    };
    let o = match output {
        "review" => tr("you review results", "revisas los resultados"),
        "structure" => tr("results without values", "resultados sin valores"),
        _ => tr("full results", "resultados completos"),
    };
    format!("{a}, {o}")
}

fn project_folder(given: Option<&String>) -> Result<PathBuf, String> {
    let dir = match given {
        Some(d) => PathBuf::from(d),
        // A console's folder that is gone is an error below, not a reason to act on another.
        None => match console_folder() {
            Some(d) => d,
            None => std::env::current_dir().map_err(|e| e.to_string())?,
        },
    };
    let dir = std::fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !dir.is_dir() {
        return Err(trf!("{} is not a folder", "{} no es una carpeta", dir.display()));
    }
    Ok(dir)
}

fn usage() -> &'static str {
    tr("usage: corral profile [open|standard|sensitive] [--project [FOLDER]] | corral profile --project --clear [FOLDER] | --json",
       "uso: corral profile [open|standard|sensitive] [--project [CARPETA]] | corral profile --project --clear [CARPETA] | --json")
}

/// What a profile means, for the notice and the approval popup.
fn meaning(name: &str) -> &'static str {
    match name {
        "open" => tr("Agents run code without asking and see full results.", "Los agentes ejecutan código sin preguntar y ven los resultados completos."),
        "sensitive" => tr("Each request waits for your approval, results come back without values, and every session is sensitive.",
                          "Cada petición espera tu aprobación, los resultados vuelven sin valores y cada sesión es sensible."),
        _ => tr("Each request waits for your approval; agents see full results.", "Cada petición espera tu aprobación; los agentes ven los resultados completos."),
    }
}

/// `corral profile …`
pub fn cmd_profile(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", usage());
        return Ok(());
    }
    if let Some(bad) = args.iter().find(|a| a.starts_with("--") && !["--project", "--clear", "--json"].contains(&a.as_str())) {
        return Err(trf!("unknown option {bad}\n{}", "opción desconocida {bad}\n{}", usage()));
    }
    let project = args.iter().any(|a| a == "--project");
    let clear = args.iter().any(|a| a == "--clear");
    let json = args.iter().any(|a| a == "--json");
    let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let name = rest.first().map(|s| s.as_str()).filter(|n| PROFILES.iter().any(|(p, _, _)| p == n));
    // With --project, what follows the profile (or stands alone) is the folder.
    let folder = if name.is_some() { rest.get(1).copied() } else { rest.first().copied() };
    if !project && rest.len() > usize::from(name.is_some()) {
        return Err(trf!("unknown profile {}\n{}", "perfil desconocido {}\n{}", rest[usize::from(name.is_some())], usage()));
    }
    match (name, project, clear) {
        (Some(name), false, false) => set_global(name),
        (Some(name), true, false) => set_project(&project_folder(folder)?, Some(name)),
        (None, true, true) => set_project(&project_folder(folder)?, None),
        (None, _, false) => show(json, folder),
        _ => Err(usage().into()),
    }
}

/// The global profile: `agents` and `agent_output`, with the check `corral config` makes,
/// so loosening either from an agent's pane waits for the user's approval; one notice.
fn set_global(name: &str) -> Result<(), String> {
    let (_, access, output) = PROFILES.iter().find(|(n, _, _)| *n == name).copied().unwrap_or(PROFILES[1]);
    let mut cfg = env::config();
    crate::theme::confirm_if_loosened(&cfg, "agents", access)?;
    crate::theme::confirm_if_loosened(&cfg, "agent_output", output)?;
    cfg.set("agents", access.into());
    cfg.set("agent_output", output.into());
    crate::theme::write_config(&cfg);
    env::notify(&trf!("Profile: {}", "Perfil: {}", shown(name)), meaning(name));
    Ok(())
}

/// A project's profile (None removes it). Tighter never asks; looser, or removing a
/// stricter one, from an agent's pane waits for the user's approval.
fn set_project(folder: &Path, name: Option<&str>) -> Result<(), String> {
    let key = folder.display().to_string();
    let mut cfg = env::config();
    let before = cfg.get("projects").get(&key).str().map(String::from);
    let looser = match (before.as_deref(), name) {
        (Some(b), Some(n)) => profile_rank(n) > profile_rank(b),
        (Some(b), None) => b != "open",
        (None, _) => false,
    };
    if looser && !crate::theme::by_the_user() {
        let from = before.as_deref().map(shown).unwrap_or("");
        let to = name.map(shown).unwrap_or(tr("none", "ninguno"));
        crate::eval::confirm_setting(&trf!("profile of {key}", "perfil de {key}"), from, to, name.map(meaning).unwrap_or(""))?;
    }
    let mut projects = match cfg.get("projects") {
        Value::Obj(p) => p.clone(),
        _ => Vec::new(),
    };
    projects.retain(|(k, _)| k != &key);
    if let Some(n) = name {
        projects.push((key.clone(), n.into()));
    }
    cfg.set("projects", Value::Obj(projects));
    crate::theme::write_config(&cfg);
    match name {
        Some(n) => env::notify(&trf!("Profile for {}: {}", "Perfil para {}: {}", short(folder), shown(n)), meaning(n)),
        None => env::notify(&trf!("{}: no profile of its own", "{}: sin perfil propio", short(folder)),
                            tr("The global profile applies there.", "Ahí se aplica el perfil global.")),
    }
    Ok(())
}

pub(crate) fn short(folder: &Path) -> String {
    folder.display().to_string().replace(&env::home().to_string_lossy().to_string(), "~")
}

/// `corral profile`: the global profile, this folder's, and the rules that result.
fn show(json: bool, folder: Option<&String>) -> Result<(), String> {
    let cfg = env::config();
    let here = project_folder(folder)?;
    let global = combine(&cfg, &[], false);
    let rules = folder_rules(&here);
    let projects: Vec<Value> = match cfg.get("projects") {
        Value::Obj(p) => p.iter().map(|(k, v)| obj(vec![("folder", k.as_str().into()), ("profile", v.clone())])).collect(),
        _ => Vec::new(),
    };
    if json {
        println!("{}", obj(vec![
            ("global", global.profile.into()),
            ("folder", here.display().to_string().into()),
            ("project", rules.project.as_ref().map(|(d, n)| obj(vec![("folder", d.as_str().into()), ("profile", n.as_str().into())])).unwrap_or(Value::Null)),
            ("profile", rules.profile.into()), ("access", rules.access.into()), ("output", rules.output.into()), ("sensitive", rules.sensitive.into()),
            ("projects", Value::Arr(projects)),
        ]).to_json());
        return Ok(());
    }
    println!("{}", trf!("Global profile: {}", "Perfil global: {}", shown(global.profile)));
    match &rules.project {
        Some((dir, name)) => println!("{}", trf!("This project ({}): {}", "Este proyecto ({}): {}", short(Path::new(dir)), shown(name))),
        None => println!("{}", trf!("This project ({}): no profile of its own", "Este proyecto ({}): sin perfil propio", short(&here))),
    }
    println!("{}", trf!("Here: {} · {}", "Aquí: {} · {}", shown(rules.profile), meaning(rules.profile)));
    if !projects.is_empty() {
        println!("\n{}", tr("Projects with a profile:", "Proyectos con perfil:"));
        for p in &projects {
            println!("  {}  {}", short(Path::new(p.get("folder").str().unwrap_or(""))), shown(p.get("profile").str().unwrap_or("")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: &str) -> Value {
        crate::json::parse(json).unwrap()
    }

    #[test]
    fn the_stricter_wins() {
        let c = cfg(r#"{"agents":"on","agent_output":"full","projects":{"/data/clinic":"sensitive","/data/open":"open"}}"#);
        let r = combine(&c, &[PathBuf::from("/data/clinic/study")], false);
        assert_eq!((r.access, r.output, r.sensitive, r.profile), ("ask", "structure", true, "sensitive"));
        assert_eq!(r.project.as_ref().map(|(d, _)| d.as_str()), Some("/data/clinic"));
        // A project can't loosen: open under a standard global is still standard.
        let s = cfg(r#"{"agents":"ask","projects":{"/data/open":"open"}}"#);
        assert_eq!(combine(&s, &[PathBuf::from("/data/open")], false).profile, "standard");
        // Elsewhere, the global settings.
        assert_eq!(combine(&c, &[PathBuf::from("/home/me")], false).profile, "open");
        // The session's own mark is sensitive whatever the settings say.
        assert_eq!(combine(&c, &[PathBuf::from("/home/me")], true).profile, "sensitive");
        // Review is kept, and is not one of the three.
        let r = combine(&cfg(r#"{"agents":"ask","agent_output":"review"}"#), &[], false);
        assert_eq!((r.output, r.profile), ("review", "custom"));
    }

    #[test]
    fn folders_match_whole_names_and_nested_ones_stay_strict() {
        let c = cfg(r#"{"projects":{"/data/x":"sensitive","/data":"open"}}"#);
        assert_eq!(project_for(&c, &[PathBuf::from("/data/xy")]).map(|(d, _)| d), Some("/data".to_string()));
        assert_eq!(project_for(&c, &[PathBuf::from("/data/x/y")]).map(|(_, n)| n), Some("sensitive".to_string()));
        // Where it started counts as much as where it runs now.
        assert_eq!(project_for(&c, &[PathBuf::from("/data/x"), PathBuf::from("/tmp")]).map(|(_, n)| n), Some("sensitive".to_string()));
    }
}
