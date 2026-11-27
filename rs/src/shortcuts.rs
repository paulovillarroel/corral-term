//! `corral keys` (`ctrl+b space`): every Corral shortcut in one searchable
//! list, Spotlight style, and Enter runs the ones that are actions. The list is
//! read from the real configuration (herdr's keybindings, the plugin's
//! actions, the editor's keys), so a change made by hand or with
//! `corral keys --change` shows up here. The keys of Corral's own screens
//! (console, variables, tables, Mirador) come from the code.
//!
//! `--list` and `--json` print the same list (for people and for agents);
//! `--change ID KEY` rebinds a herdr or editor shortcut.

use crate::env;
use crate::i18n::tr;
use crate::json::{obj, Value};
use crate::term::{clip, out, width, Screen};
use crate::vars::Colors;
use std::path::PathBuf;
use std::os::unix::process::CommandExt;
use std::process::Command;

extern "C" {
    fn setsid() -> i32;
}

const RESET: &str = "\x1b[0m";

/// The palette's groups, in display order: the parts of the IDE, from the layout
/// to the plots, then what you set (security, the agent, appearance) and help. Each
/// entry starts with a category ("Results: …", "Editor: …"), so typing it filters;
/// so does the group's name.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Group {
    Layout,
    Editor,
    Console,
    Vars,
    Table,
    Plots,
    Security,
    Agent,
    Settings,
    Help,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Layout => tr("Layouts and panes", "Layouts y paneles"),
            Group::Editor => tr("Script editor", "Editor de scripts"),
            Group::Console => tr("Console (R and Python)", "Consola (R y Python)"),
            Group::Vars => tr("Variables pane", "Panel de variables"),
            Group::Table => tr("Tables (View)", "Tablas (View)"),
            Group::Plots => tr("Plots and Mirador", "Gráficos y Mirador"),
            Group::Security => tr("Security", "Seguridad"),
            Group::Agent => tr("Agent", "Agente"),
            Group::Settings => tr("Appearance and language", "Apariencia e idioma"),
            Group::Help => tr("Help", "Ayuda"),
        }
    }
}

/// Keys herdr itself binds after its prefix (herdr 0.9.5 defaults).
const HERDR_DEFAULTS: &[&str] = &[
    "?", "s", "w", "g", "b", "o", "q", "shift+r", "shift+n", "shift+g", "shift+w", "shift+d", "c", "shift+t", "p",
    "n", "1", "2", "3", "4", "5", "6", "7", "8", "9", "shift+x", "shift+p", "e", "[", "h", "j", "k", "l", "shift+h",
    "shift+j", "shift+k", "shift+l", "tab", "shift+tab", "v", "minus", "x", "z", "r",
];

#[derive(Clone)]
pub struct Entry {
    group: Group,
    keys: Vec<String>,
    desc: String,
    id: String,
    /// Arguments for `corral` (a plugin action: Enter runs it).
    run: Option<(String, Vec<String>)>,
    /// What is in effect is marked: "●", or "● active" on the profile.
    current: &'static str,
}

// --------------------------------------------------------------------------- sources

fn herdr_config_path() -> PathBuf {
    std::env::var_os("HERDR_CONFIG_PATH").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config/herdr/config.toml"))
}

fn fresh_config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config")).join("fresh/config.json")
}

/// `name = "value"` on a TOML line (no escapes: enough for these files).
fn toml_str(line: &str, name: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(name)?.trim_start().strip_prefix('=')?.trim().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

struct Binding {
    key: String,
    kind: String,
    command: String,
    /// Line of `key = …` in the file.
    key_line: Option<usize>,
    /// Line of `command = …` in the file.
    command_line: Option<usize>,
}

/// A plugin action as it is called now, also when an older Corral wrote its old name into herdr's config.
fn current_command(command: &str) -> String {
    match command {
        "corral.atajos" => "corral.keys",
        "corral.layout-codigo" => "corral.layout-code",
        "corral.tema-dark-2026" => "corral.theme-dark-2026",
        "corral.tema-herdr" => "corral.theme-herdr",
        other => other,
    }
    .to_string()
}

/// herdr's prefix and its `[[keys.command]]` bindings.
fn herdr_bindings(text: &str) -> (String, Vec<Binding>) {
    let mut prefix = "ctrl+b".to_string();
    let mut binds: Vec<Binding> = Vec::new();
    let mut section = String::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t.to_string();
            if t == "[[keys.command]]" {
                binds.push(Binding { key: String::new(), kind: String::new(), command: String::new(), key_line: None, command_line: None });
            }
            continue;
        }
        if section == "[keys]" {
            if let Some(p) = toml_str(t, "prefix") {
                prefix = p;
            }
        } else if section == "[[keys.command]]" {
            let Some(b) = binds.last_mut() else { continue };
            if let Some(v) = toml_str(t, "key") {
                b.key = v;
                b.key_line = Some(i);
            } else if let Some(v) = toml_str(t, "type") {
                b.kind = v;
            } else if let Some(v) = toml_str(t, "command") {
                b.command = current_command(&v);
                b.command_line = Some(i);
            }
        }
    }
    (prefix, binds)
}

/// The plugin's actions: id, title and the `corral` arguments it runs.
fn plugin_actions() -> Vec<(String, String, Vec<String>)> {
    let text = std::fs::read_to_string(env::repo().join("herdr/herdr-plugin.toml")).unwrap_or_default();
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut in_action = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_action = t == "[[actions]]";
            if in_action {
                out.push((String::new(), String::new(), Vec::new()));
            }
            continue;
        }
        let Some(a) = out.last_mut().filter(|_| in_action) else { continue };
        if let Some(v) = toml_str(t, "id") {
            a.0 = v;
        } else if let Some(v) = toml_str(t, "title") {
            a.1 = v;
        } else if let Some(rest) = t.strip_prefix("command") {
            // command = ["../rs/target/release/corral", "layout", …]
            a.2 = rest.split('"').skip(1).step_by(2).skip(1).map(String::from).collect();
        }
    }
    out.retain(|a| !a.0.is_empty() && a.0 != "open-url");
    out
}

/// Where each of Corral's plugin actions goes in the palette, and its text there,
/// with its category first (English, Spanish), in display order. The manifest's own
/// titles ("Corral: …") are what herdr's palette shows. The language actions read
/// in the language they switch to.
const ACTIONS: &[(&str, Group, &str, &str)] = &[
    ("layout", Group::Layout, "Layout: agent first", "Layout: foco en el agente"),
    ("layout-code", Group::Layout, "Layout: code first", "Layout: foco en el código"),
    ("layout-notebook", Group::Layout, "Layout: notebook (script over the console, agent and variables on the right)", "Layout: notebook (script sobre la consola, agente y variables a la derecha)"),
    ("terminal-tab-on", Group::Layout, "Layout: open a terminal tab with it", "Layout: abrir una pestaña terminal junto a él"),
    ("terminal-tab-off", Group::Layout, "Layout: no terminal tab", "Layout: sin pestaña terminal"),
    ("layout-sizes", Group::Layout, "Layout: back to Corral's sizes (forgets the ones you left)", "Layout: volver a los tamaños de Corral (olvida los que dejaste)"),
    ("start-r", Group::Layout, "Layout: start with R", "Layout: partir con R"),
    ("start-python", Group::Layout, "Layout: start with Python", "Layout: partir con Python"),
    ("toggle", Group::Layout, "Layout: expand the script (or agent) or the console", "Layout: expandir el script (o el agente) o la consola"),
    ("script", Group::Layout, "Layout: add the script editor above the console", "Layout: agregar el editor de scripts sobre la consola"),
    ("close", Group::Layout, "Layout: close (the agent and the terminal tab too, when idle)", "Layout: cerrar (también el agente y la pestaña terminal, si están libres)"),
    ("switch", Group::Console, "Console: R or Python", "Consola: R o Python"),
    ("r-version", Group::Console, "Console: R version (rig)", "Consola: versión de R (rig)"),
    ("python-version", Group::Console, "Console: Python version (uv)", "Consola: versión de Python (uv)"),
    ("checkpoints-on", Group::Console, "Console: keep the objects and plots if R or Python crashes", "Consola: conservar los objetos y gráficos si R o Python se cae"),
    ("checkpoints-off", Group::Console, "Console: don't keep checkpoints of the objects and plots", "Consola: no guardar respaldos de los objetos y gráficos"),
    ("send", Group::Console, "Console: send the selection (R or Python)", "Consola: enviar la selección (R o Python)"),
    ("send-sql", Group::Console, "Console: run the selection as SQL (DuckDB, in the session)", "Consola: ejecutar la selección como SQL (DuckDB, en la sesión)"),
    ("console", Group::Console, "Console: use this pane as the console", "Consola: usar este panel como consola"),
    ("vars", Group::Vars, "Variables: open them over the terminal", "Variables: abrirlas sobre la terminal"),
    ("plots", Group::Plots, "Plots: show them", "Gráficos: verlos"),
    ("mirador", Group::Plots, "Mirador: open it in the browser (plots and tables)", "Mirador: abrirlo en el navegador (gráficos y tablas)"),
    ("plot-size-window", Group::Plots, "Plots: Mirador fits them to its window", "Gráficos: Mirador los ajusta a su ventana"),
    ("plot-size-800", Group::Plots, "Plots: Mirador draws them at 800 × 500", "Gráficos: Mirador los dibuja de 800 × 500"),
    ("plot-size-1000", Group::Plots, "Plots: Mirador draws them at 1000 × 650", "Gráficos: Mirador los dibuja de 1000 × 650"),
    ("plot-size-1200", Group::Plots, "Plots: Mirador draws them at 1200 × 800", "Gráficos: Mirador los dibuja de 1200 × 800"),
    // Security: the profiles first (each one a mix of the two settings below them), the
    // console's project, then each setting on its own. The palette marks what is in effect.
    ("profile-open", Group::Security, "Open profile: run without asking, full results", "Perfil abierto: ejecutan sin preguntar, resultados completos"),
    ("profile-standard", Group::Security, "Standard profile: ask before each request, full results", "Perfil estándar: preguntan antes de cada petición, resultados completos"),
    ("profile-sensitive", Group::Security, "Sensitive profile: ask, results without values, no connections of its own", "Perfil sensible: preguntan, resultados sin valores, sin conexiones propias"),
    ("profile-project-sensitive", Group::Security, "Project: sensitive in every session", "Proyecto: sensible en cada sesión"),
    ("profile-project-clear", Group::Security, "Project: remove its profile", "Proyecto: quitar su perfil"),
    ("agents-ask", Group::Security, "Requests: ask before each one", "Peticiones: preguntar antes de cada una"),
    ("agents-on", Group::Security, "Requests: run without asking", "Peticiones: ejecutar sin preguntar"),
    ("agents-off", Group::Security, "Requests: blocked, agents can't run code", "Peticiones: bloqueadas, los agentes no ejecutan código"),
    ("output-full", Group::Security, "Results: full", "Resultados: completos"),
    ("output-review", Group::Security, "Results: you review them first", "Resultados: los revisas tú primero"),
    ("output-structure", Group::Security, "Results: only the structure, without values", "Resultados: solo la estructura, sin valores"),
    ("audit-30", Group::Security, "Audit: keep the log 30 days", "Auditoría: guardar el registro 30 días"),
    ("audit-90", Group::Security, "Audit: keep the log 90 days (the default)", "Auditoría: guardar el registro 90 días (lo predeterminado)"),
    ("audit-365", Group::Security, "Audit: keep the log a year", "Auditoría: guardar el registro un año"),
    ("context", Group::Security, "What your agent sees (consoles, SQL, its rules)", "Lo que ve tu agente (consolas, SQL, sus reglas)"),
    ("agent-claude", Group::Agent, "Agent: Claude Code in its pane", "Agente: Claude Code en su panel"),
    ("agent-codex", Group::Agent, "Agent: Codex in its pane", "Agente: Codex en su panel"),
    ("agent-gemini", Group::Agent, "Agent: Gemini CLI in its pane", "Agente: Gemini CLI en su panel"),
    ("agent-opencode", Group::Agent, "Agent: opencode in its pane", "Agente: opencode en su panel"),
    ("agent-none", Group::Agent, "Agent: none, a shell in its pane", "Agente: ninguno, una shell en su panel"),
    ("theme-dark-2026", Group::Settings, "Theme: Dark 2026 (VS Code's and Positron's)", "Tema: Dark 2026 (el de VS Code y Positron)"),
    ("theme-herdr", Group::Settings, "Theme: back to herdr's", "Tema: volver al de herdr"),
    ("language-en", Group::Settings, "Language: English", "Language: English"),
    ("language-es", Group::Settings, "Idioma: español", "Idioma: español"),
    ("update", Group::Settings, "Update: install the latest version of Corral", "Actualizar: instalar la última versión de Corral"),
    ("update-check-on", Group::Settings, "Update: tell me when there is a new version", "Actualizar: avisarme cuando haya una versión nueva"),
    ("update-check-off", Group::Settings, "Update: don't check for new versions", "Actualizar: no revisar si hay versiones nuevas"),
    ("telemetry-on", Group::Settings, "Install count: count my installs anonymously", "Conteo de instalaciones: contar las mías de forma anónima"),
    ("telemetry-off", Group::Settings, "Install count: don't count my installs", "Conteo de instalaciones: no contar las mías"),
    ("keys", Group::Help, "Help: shortcuts and actions (this palette)", "Ayuda: atajos y acciones (esta paleta)"),
];

/// A plugin action's place and text in the palette, in the user's language; an
/// action this list doesn't know yet goes under Layouts with its own title.
fn action_label(id: &str, title: &str) -> (Group, String, usize) {
    match ACTIONS.iter().position(|a| a.0 == id) {
        // With a newer Corral out, the entry names it.
        Some(i) if id == "update" && crate::install::newer_version().is_some() => {
            let v = crate::install::newer_version().unwrap_or_default();
            (ACTIONS[i].1, trf!("Update: Corral {v} is out, install it", "Actualizar: salió Corral {v}, instalarla"), i)
        }
        Some(i) => (ACTIONS[i].1, tr(ACTIONS[i].2, ACTIONS[i].3).to_string(), i),
        None => (Group::Layout, tidy(title), ACTIONS.len()),
    }
}

fn tidy(title: &str) -> String {
    let t = title.strip_prefix("Corral: ").unwrap_or(title);
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn herdr_key(prefix: &str, key: &str) -> String {
    match key.strip_prefix("prefix+") {
        Some(rest) => format!("{prefix} {}", rest.replace("minus", "-")),
        None => key.to_string(),
    }
}

fn fresh_key(kb: &Value) -> String {
    let mods: Vec<&str> = kb.get("modifiers").arr().iter().filter_map(|m| m.str()).collect();
    let key = kb.get("key").str().unwrap_or("");
    let key = match key {
        "7" if mods == ["ctrl"] => "/".to_string(), // terminals send Ctrl+/ as Ctrl+7
        "minus" => "-".to_string(),
        k if k.len() > 1 && !k.starts_with('F') => k.to_lowercase(),
        k => k.to_string(),
    };
    let mut parts: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
    parts.push(key);
    parts.join("+")
}

fn editor_desc(action: &str) -> String {
    match action {
        "corral_save" => tr("Save (a draft asks where)", "Guardar (el borrador pregunta dónde)"),
        "corral_run_line" => tr("Run the line or the selection", "Ejecutar la línea o la selección"),
        "corral_run_file" => tr("Run the whole file", "Ejecutar el archivo completo"),
        "corral_insert_assign" => tr("Insert <-", "Insertar <-"),
        "corral_insert_pipe" => tr("Insert |>", "Insertar |>"),
        "toggle_comment" => tr("Comment or uncomment", "Comentar o descomentar"),
        "lsp_completion" => tr("Autocomplete", "Autocompletar"),
        other => other,
    }
    .to_string()
}

/// "Run the line" under the category "Editor": "Editor: run the line".
fn categorized(category: &str, desc: &str) -> String {
    let mut c = desc.chars();
    let first = c.next().map(|f| f.to_lowercase().collect::<String>()).unwrap_or_default();
    format!("{category}: {first}{}", c.as_str())
}

/// Keys that come from the code: (English, Spanish, keys), under a category (English, Spanish).
fn fixed(group: Group, prefix: &str, category: (&str, &str), items: &[(&str, &str, &[&str])]) -> Vec<Entry> {
    items.iter().enumerate().map(|(i, (en, es_text, keys))| Entry {
        group,
        keys: keys.iter().map(|k| k.to_string()).collect(),
        desc: categorized(tr(category.0, category.1), tr(en, es_text)),
        id: format!("{prefix}:{}", i + 1),
        run: None,
        current: "",
    }).collect()
}

/// Mark what is in effect among the Security and Agent entries. The console in view
/// can run by stricter rules than the global settings (its project's profile, its own
/// sensitive mark): then what it runs by is "● active" and the global value "● global";
/// when they agree, the profile is "● active" and each setting "●". A mix that is no
/// profile shows as an entry of its own. The project entries name the console's folder,
/// and go without a console here: their action would have no folder to act on.
fn mark_in_effect(list: &mut Vec<Entry>) {
    // The agent the layout opens: CORRAL_AGENT (empty for a shell), the setting, else
    // the first one installed.
    let chosen = std::env::var("CORRAL_AGENT").ok().or_else(|| env::config().get("agent").str().map(String::from));
    let agent = match chosen {
        Some(a) if a.trim().is_empty() || a == "none" => "none".to_string(),
        Some(a) => a.split_whitespace().next().unwrap_or("").to_string(),
        None => env::first_agent(|cmd| env::which(cmd).is_some()).unwrap_or("none").to_string(),
    };
    mark(list, &crate::profile::in_effect(), crate::audit::keep_days(), &agent);
}

/// mark_in_effect, given what is in effect: the profile and project, the audit log's
/// days (None: forever) and the agent's command.
fn mark(list: &mut Vec<Entry>, now: &crate::profile::InEffect, audit: Option<i64>, agent: &str) {
    let (global, console) = (now.global, now.console);
    let same = global == console;
    let active = tr("● active", "● activo");
    let global_mark = tr("● global", "● global");
    // The mark for an entry whose value is `v` among the global one and the console's.
    let pick = |v: &str, g: &str, c: &str, agree: &'static str| -> &'static str {
        match (v == g, v == c) {
            (true, true) => agree,
            (false, true) => active,
            (true, false) => global_mark,
            _ => "",
        }
    };
    let custom = |mix: crate::profile::Mix, current: &'static str| Entry {
        group: Group::Security, keys: Vec::new(),
        desc: trf!("Profile: custom ({})", "Perfil: a medida ({})", crate::profile::mix_in_words(mix.access, mix.output)),
        id: "profile-custom".into(), run: None, current,
    };
    let mut keep = Vec::with_capacity(list.len() + 2);
    for mut e in list.drain(..) {
        let Some(id) = e.id.strip_prefix("herdr:").map(String::from) else {
            keep.push(e);
            continue;
        };
        if id == "profile-open" {
            if global.profile == "custom" {
                keep.push(custom(global, if same { active } else { global_mark }));
            }
            if console.profile == "custom" && !same {
                keep.push(custom(console, active));
            }
        }
        match id.as_str() {
            "profile-open" | "profile-standard" | "profile-sensitive" => {
                let name = &id["profile-".len()..];
                e.current = pick(name, global.profile, console.profile, active);
            }
            "profile-project-sensitive" | "profile-project-clear" => {
                let Some(project) = &now.project else { continue };
                // The folder's name: a whole path would push the rest out of a narrow palette.
                // Shown with its control characters written out, so a folder's name can't
                // move the cursor or clear the screen when the palette draws it.
                let dir = &project.dir;
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| crate::profile::short(dir));
                let folder = crate::term::visible(&name, false).0;
                if id.ends_with("sensitive") {
                    e.desc = trf!("Project {folder}: sensitive in every session", "Proyecto {folder}: sensible en cada sesión");
                    if project.applies.as_deref() == Some("sensitive") {
                        e.current = "●";
                    }
                } else {
                    // Only a profile set for this very folder can be removed from here; what
                    // then applies there is the global profile, or a folder's above it.
                    if project.own.is_none() {
                        continue;
                    }
                    let rest = project.without_own;
                    e.desc = match rest.profile {
                        "custom" => trf!("Project {folder}: remove its profile (then: {})", "Proyecto {folder}: quitar su perfil (queda: {})",
                                         crate::profile::mix_in_words(rest.access, rest.output)),
                        p => trf!("Project {folder}: remove its profile (back to {})", "Proyecto {folder}: quitar su perfil (vuelve al {})", crate::profile::shown(p)),
                    };
                }
            }
            _ => {
                e.current = if let Some(a) = id.strip_prefix("agents-") {
                    pick(a, global.access, console.access, "●")
                } else if let Some(o) = id.strip_prefix("output-") {
                    pick(o, global.output, console.output, "●")
                } else if id.strip_prefix("audit-").and_then(|d| d.parse::<i64>().ok()).is_some_and(|d| Some(d) == audit)
                    || id.strip_prefix("agent-") == Some(agent) {
                    "●"
                } else {
                    ""
                };
            }
        }
        keep.push(e);
    }
    *list = keep;
}

/// Every shortcut, in display order.
pub fn entries() -> Vec<Entry> {
    let (prefix, binds) = herdr_bindings(&std::fs::read_to_string(herdr_config_path()).unwrap_or_default());
    let mut list = Vec::new();

    let mut actions: Vec<(usize, Entry)> = plugin_actions().into_iter().map(|(id, title, args)| {
        let command = format!("corral.{id}");
        let keys = binds.iter().filter(|b| b.kind == "plugin_action" && b.command == command)
            .map(|b| herdr_key(&prefix, &b.key)).collect();
        let (group, desc, order) = action_label(&id, &title);
        (order, Entry { group, keys, desc, id: format!("herdr:{id}"), run: Some((id, args)), current: "" })
    }).collect();
    actions.sort_by_key(|(order, _)| *order);
    list.extend(actions.into_iter().map(|(_, e)| e));
    list.push(Entry { group: Group::Help, keys: vec![format!("{prefix} ?")],
                      desc: tr("Help: all of herdr's shortcuts (panes, tabs, workspaces)", "Ayuda: todos los atajos de herdr (paneles, pestañas, espacios)").into(),
                      id: "herdr:help".into(), run: None, current: "" });

    let fresh = env::read_json(&fresh_config_path());
    let mut editor_actions: Vec<(String, Vec<String>)> = Vec::new();
    for kb in fresh.get("keybindings").arr() {
        let Some(action) = kb.get("action").str() else { continue };
        match editor_actions.iter_mut().find(|(a, _)| a == action) {
            Some((_, keys)) => keys.push(fresh_key(kb)),
            None => editor_actions.push((action.to_string(), vec![fresh_key(kb)])),
        }
    }
    for (action, keys) in editor_actions {
        list.push(Entry { group: Group::Editor, keys, desc: categorized("Editor", &editor_desc(&action)), id: format!("editor:{action}"), run: None, current: "" });
    }
    list.extend(fixed(Group::Editor, "editor-fixed", ("Editor", "Editor"), &[
        ("Command palette (all its actions)", "Paleta de comandos (todas sus acciones)", &["ctrl+p"]),
        ("Find in the file", "Buscar en el archivo", &["ctrl+f"]),
        ("Undo", "Deshacer", &["ctrl+z"]),
        ("Close (the drafts are discarded)", "Cerrar (los borradores se descartan)", &["ctrl+q"]),
    ]));
    // The line editor of Corral's R console (Ark); IPython has its own.
    list.extend(fixed(Group::Console, "console", ("R console", "Consola de R"), &[
        ("Run, if the expression is complete", "Ejecutar, si la expresión está completa", &["enter"]),
        ("Run even if something is left open", "Ejecutar aunque falte cerrar algo", &["alt+enter"]),
        ("History (with something typed, only what starts the same)", "Historial (con algo escrito, solo lo que empieza igual)", &["↑", "↓"]),
        ("Complete names, arguments and columns", "Completar nombres, argumentos y columnas", &["tab"]),
        ("Interrupt what is running, or clear what you typed", "Interrumpir lo que corre, o borrar lo escrito", &["ctrl+c"]),
        ("Close (on an empty line)", "Cerrar (en una línea vacía)", &["ctrl+d"]),
        ("Clear the screen", "Limpiar la pantalla", &["ctrl+l"]),
        ("Go to the start or the end of the line", "Ir al inicio o al final de la línea", &["ctrl+a", "ctrl+e"]),
        ("Delete to the end, to the start or the previous word", "Borrar hasta el final, hasta el inicio o la palabra anterior", &["ctrl+k", "ctrl+u", "ctrl+w"]),
    ]));
    list.extend(fixed(Group::Vars, "vars", ("Variables", "Variables"), &[
        ("Switch tabs (data, plots, values, functions, other)", "Cambiar de pestaña (datos, gráficos, valores, funciones, otros)", &["←", "→", "1–5"]),
        ("Move", "Moverse", &["↑", "↓", "pgup", "pgdn"]),
        ("Open the selected one (a table in the viewer, a plot in Mirador, a value with str())", "Abrir el seleccionado (una tabla en el visor, un gráfico en Mirador, un valor con str())", &["enter"]),
        ("Open the table in Mirador, in the browser", "Abrir la tabla en Mirador, en el navegador", &["m"]),
        ("Search (the arrows pick among the results)", "Buscar (las flechas eligen entre los resultados)", &["/"]),
        ("Close the window", "Cerrar la ventana", &["q"]),
    ]));
    list.extend(fixed(Group::Table, "table", ("Table", "Tabla"), &[
        ("Move through the rows", "Moverse por las filas", &["↑", "↓", "pgup", "pgdn", "g", "G"]),
        ("Move through the columns (also the sideways wheel)", "Moverse por las columnas (también la rueda lateral)", &["←", "→", "0", "$"]),
        ("Sort by the column", "Ordenar por la columna", &["s"]),
        ("Search for a text in every column", "Buscar un texto en todas las columnas", &["/"]),
        ("Filter with a condition, in R or in SQL", "Filtrar con una condición, en R o en SQL", &["f"]),
        ("Keep only the rows with the cell's value", "Dejar solo las filas con el valor de la celda", &["="]),
        ("Clear the filters and the search", "Quitar los filtros y la búsqueda", &["x"]),
        ("Show the whole row", "Ver la fila completa", &["enter"]),
        ("Summary of each column", "Resumen de cada columna", &["i"]),
        ("Open it in Mirador, in the browser", "Abrirla en Mirador, en el navegador", &["m"]),
        ("Close", "Cerrar", &["q"]),
    ]));
    list.extend(fixed(Group::Plots, "mirador", ("Mirador", "Mirador"), &[
        ("Previous or next plot", "Gráfico anterior o siguiente", &["←", "→"]),
        ("Copy the plot as a PNG image", "Copiar el gráfico como imagen PNG", &["c"]),
        ("Show these shortcuts", "Ver estos atajos", &["?"]),
    ]));
    // A table tab in Mirador has the table viewer's keys.
    list.extend(fixed(Group::Plots, "mirador-table", ("Mirador table", "Tabla en Mirador"), &[
        ("Move through the rows", "Moverse por las filas", &["↑", "↓", "pgup", "pgdn", "g", "G"]),
        ("Move through the columns", "Moverse por las columnas", &["←", "→", "0", "$"]),
        ("Sort by the column", "Ordenar por la columna", &["s"]),
        ("Search for a text in every column", "Buscar un texto en todas las columnas", &["/"]),
        ("Filter with a condition, in R or in SQL", "Filtrar con una condición, en R o en SQL", &["f"]),
        ("Keep only the rows with the cell's value", "Dejar solo las filas con el valor de la celda", &["="]),
        ("Clear the filters and the search", "Quitar los filtros y la búsqueda", &["x"]),
        ("Show the whole row", "Ver la fila completa", &["enter"]),
        ("Summary of each column", "Resumen de cada columna", &["i"]),
    ]));
    mark_in_effect(&mut list);
    // In display order: by group, and within one as listed here.
    list.sort_by(|a, b| a.group.partial_cmp(&b.group).unwrap_or(std::cmp::Ordering::Equal));
    list
}

// --------------------------------------------------------------------------- search

fn norm(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'ä' => 'a',
        'é' | 'è' | 'ë' => 'e',
        'í' | 'ì' | 'ï' => 'i',
        'ó' | 'ò' | 'ö' => 'o',
        'ú' | 'ù' | 'ü' => 'u',
        'ñ' => 'n',
        c => c,
    }).collect()
}

/// Every word of the query appears in the description, the keys or the group.
fn matches(e: &Entry, words: &[String]) -> bool {
    let hay = norm(&format!("{} {} {} {}", e.desc, e.keys.join(" "), e.group.name(), e.id));
    words.iter().all(|w| hay.contains(w.as_str()))
}

fn filter<'a>(all: &'a [Entry], q: &str) -> Vec<&'a Entry> {
    let words: Vec<String> = norm(q).split_whitespace().map(String::from).collect();
    all.iter().filter(|e| matches(e, &words)).collect()
}

// --------------------------------------------------------------------------- palette

enum Row {
    Header(Group),
    Item(usize),
    Gap,
}

fn rows_for(shown: &[&Entry]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut group = None;
    for (i, e) in shown.iter().enumerate() {
        if group != Some(e.group) {
            if !rows.is_empty() {
                rows.push(Row::Gap);
            }
            rows.push(Row::Header(e.group));
            group = Some(e.group);
        }
        rows.push(Row::Item(i));
    }
    rows
}

/// A shortcut you can change: one of Corral's herdr actions or editor actions.
fn changeable(id: &str) -> bool {
    id.starts_with("herdr:") && id != "herdr:help" || id.starts_with("editor:")
}

/// `footer`: lines for the bottom of the palette (a message, or changing a key)
/// instead of the usual hint.
#[allow(clippy::too_many_arguments)]
fn draw(c: &Colors, cols: usize, height: usize, q: &str, shown: &[&Entry], total: usize, sel: usize, top: &mut usize, footer: &[String]) {
    let mut f = String::from("\x1b[H\x1b[2J");
    let count = if q.is_empty() { trf!("{total} shortcuts", "{total} atajos") } else { trf!("{} of {total}", "{} de {total}", shown.len()) };
    let search = if q.is_empty() {
        format!("{}{}{RESET}", c.dim, tr("Search for a shortcut or an action…", "Busca un atajo o una acción…"))
    } else {
        format!("{}{q}{RESET}{}▏{RESET}", c.text, c.accent)
    };
    f += &format!("\x1b[1;2H{}⌕{RESET}  {search}\x1b[1;{}H{}{count}{RESET}", c.accent, cols.saturating_sub(width(&count)), c.dim);
    f += &format!("\x1b[2;1H{}{}{RESET}", c.dim, "─".repeat(cols));

    let rows = rows_for(shown);
    let list_h = height.saturating_sub(4 + footer.len().saturating_sub(1));
    let sel_row = rows.iter().position(|r| matches!(r, Row::Item(i) if *i == sel)).unwrap_or(0);
    // Keep the selection in view, with its group's title when it is the first item.
    let want_top = if sel_row > 0 && matches!(rows[sel_row - 1], Row::Header(_)) { sel_row - 1 } else { sel_row };
    if want_top < *top {
        *top = want_top;
    } else if sel_row >= *top + list_h {
        *top = sel_row + 1 - list_h;
    }
    if shown.is_empty() {
        f += &format!("\x1b[4;4H{}{}{RESET}", c.muted, trf!("Nothing matches “{q}”", "Nada coincide con «{q}»"));
    }
    for (n, row) in rows.iter().skip(*top).take(list_h).enumerate() {
        let y = 3 + n;
        match row {
            Row::Gap => {}
            Row::Header(g) => f += &format!("\x1b[{y};3H\x1b[1m{}{}{RESET}", c.muted, g.name().to_uppercase()),
            Row::Item(i) => {
                let e = shown[*i];
                let keys = e.keys.join("  ");
                let key_w = width(&keys).min(cols / 2);
                let desc_w = cols.saturating_sub(key_w + 8);
                let on = *i == sel;
                let (bg, mark) = if on { (c.sel_bg.as_str(), format!("{}▸{RESET}{}", c.accent, c.sel_bg)) } else { ("", " ".to_string()) };
                let runnable = if e.run.is_some() && on { format!(" {}↵", c.dim) } else { String::new() };
                // What is in effect, after the text and never cut by it.
                let current_w = if e.current.is_empty() { 0 } else { width(e.current) + 2 };
                let current = if e.current.is_empty() { String::new() } else { format!("  {}{}{}", c.accent, e.current, c.text) };
                f += &format!("\x1b[{y};1H{bg}\x1b[2K {mark} {}{}{current}{runnable}", c.text, clip(&e.desc, desc_w.saturating_sub(current_w)));
                f += &format!("\x1b[{y};{}H{bg}{}{}{RESET}", cols.saturating_sub(key_w + 1), c.accent, clip(&keys, key_w));
            }
        }
    }
    if !footer.is_empty() {
        let first = height + 1 - footer.len();
        for (n, line) in footer.iter().enumerate() {
            let color = if n == 0 { &c.text } else if n + 1 == footer.len() { &c.dim } else { &c.accent };
            f += &format!("\x1b[{};1H\x1b[2K\x1b[{};2H{}{}{RESET}", first + n, first + n, color, clip(line, cols.saturating_sub(2)));
        }
        out(&f);
        return;
    }
    let tab = shown.get(sel).is_some_and(|e| changeable(&e.id));
    let hint = match shown.get(sel) {
        Some(e) if e.run.is_some() && tab => tr("↑↓ select · Enter run · Tab change key · Esc close · type to search",
                                                "↑↓ elegir · Enter ejecutar · Tab cambiar tecla · Esc cerrar · escribe para buscar"),
        Some(_) if tab => tr("↑↓ select · Tab change key · Esc close · type to search", "↑↓ elegir · Tab cambiar tecla · Esc cerrar · escribe para buscar"),
        Some(e) if e.run.is_some() => tr("↑↓ select · Enter run · Esc close · type to search", "↑↓ elegir · Enter ejecutar · Esc cerrar · escribe para buscar"),
        Some(_) => tr("↑↓ select · Esc close · type to search", "↑↓ elegir · Esc cerrar · escribe para buscar"),
        None => tr("Esc clears the search", "Esc borra la búsqueda"),
    };
    f += &format!("\x1b[{};2H{}{}{RESET}", height, c.dim, clip(hint, cols.saturating_sub(2)));
    out(&f);
}

/// The lines under the palette while you change a shortcut.
fn changing(e: &Entry, prefix: &str, problem: &str) -> Vec<String> {
    let now = if e.keys.is_empty() { tr("no key", "sin tecla").to_string() } else { e.keys.join(", ") };
    let how = if e.id.starts_with("herdr:") {
        trf!("Press the key that goes after {prefix} (for {prefix} u, just u)", "Presiona la tecla que va después de {prefix} (para {prefix} u, solo u)")
    } else {
        tr("Press the new shortcut, with ctrl or alt, or an F key", "Presiona el atajo nuevo, con ctrl o alt, o una tecla F").to_string()
    };
    let mut lines = vec![trf!("Change “{}” (now {now})", "Cambiar «{}» (ahora {now})", e.desc), how];
    if !problem.is_empty() {
        lines.push(problem.to_string());
    }
    lines.push(tr("Backspace Corral's default · Esc cancel", "Backspace la de Corral · Esc cancelar").into());
    lines
}

/// A message from changing a key, said for the palette (there is no --force here).
fn for_palette(msg: String) -> String {
    msg.replace("; use another key or --force", "; press another key").replace("; usa otra tecla o --force", "; presiona otra tecla")
}

fn palette() -> Result<(), String> {
    let mut all = entries();
    let Some(scr) = Screen::enter(true) else { return Err(tr("not a terminal", "no hay una terminal").into()) };
    let c = Colors::load();
    let prefix = herdr_bindings(&std::fs::read_to_string(herdr_config_path()).unwrap_or_default()).0;
    let (mut q, mut sel, mut top, mut dirty) = (String::new(), 0usize, 0usize, true);
    let mut chosen: Option<(String, Vec<String>)> = None;
    // Changing a shortcut (Tab): its id, and what went wrong with the last key pressed.
    let mut change: Option<(String, String)> = None;
    let mut note: Vec<String> = Vec::new();
    loop {
        let shown = filter(&all, &q);
        sel = sel.min(shown.len().saturating_sub(1));
        if dirty && !scr.has_input() {
            let (cols, rows) = scr.size();
            let footer = match &change {
                Some((id, problem)) => all.iter().find(|e| &e.id == id).map(|e| changing(e, &prefix, problem)).unwrap_or_default(),
                None => note.clone(),
            };
            draw(&c, cols, rows, &q, &shown, all.len(), sel, &mut top, &footer);
            dirty = false;
        }
        let Some(k) = scr.key(300) else { continue };
        dirty = true;
        if let Some((id, _)) = change.clone() {
            let result = match k.as_str() {
                "esc" | "quit" => {
                    change = None;
                    continue;
                }
                "backspace" => reset_key(&id),
                key => match pressed(key, id.starts_with("herdr:")).and_then(|spec| set_key(&id, &spec, false)) {
                    Ok(msg) => Ok(msg),
                    Err(e) => {
                        change = Some((id, for_palette(e)));
                        continue;
                    }
                },
            };
            change = None;
            note = vec![for_palette(result.unwrap_or_else(|e| e)), tr("Any key goes on", "Cualquier tecla sigue").into()];
            all = entries();
            continue;
        }
        if !note.is_empty() {
            note.clear();
            continue;
        }
        let last = shown.len().saturating_sub(1);
        match k.as_str() {
            "\t" => match shown.get(sel).filter(|e| changeable(&e.id)) {
                Some(e) => change = Some((e.id.clone(), String::new())),
                None => dirty = false,
            },
            "quit" => break,
            "esc" if q.is_empty() => break,
            "esc" | "\u{15}" => (q, sel, top) = (String::new(), 0, 0),
            "up" => sel = sel.saturating_sub(1),
            "down" => sel = (sel + 1).min(last),
            "pgup" => sel = sel.saturating_sub(10),
            "pgdn" => sel = (sel + 10).min(last),
            "home" => sel = 0,
            "end" => sel = last,
            "backspace" => {
                q.pop();
                (sel, top) = (0, 0);
            }
            "enter" => {
                if let Some(run) = shown.get(sel).and_then(|e| e.run.clone()) {
                    chosen = Some(run);
                    break;
                }
            }
            m if m.starts_with("mouse:") => {
                let b: Vec<&str> = m.split(':').collect();
                match b.get(1).copied() {
                    Some("64") => sel = sel.saturating_sub(3),
                    Some("65") => sel = (sel + 3).min(last),
                    _ => {}
                }
            }
            ch if ch.chars().count() == 1 && !ch.chars().next().is_some_and(char::is_control) => {
                q.push_str(ch);
                (sel, top) = (0, 0);
            }
            _ => dirty = false,
        }
    }
    drop(scr);
    if let Some((id, args)) = chosen {
        run_action(&id, &args);
    }
    Ok(())
}

/// Run a plugin action once the popup has closed, so it acts on the pane you were in.
fn run_action(id: &str, args: &[String]) {
    if std::env::var("HERDR_ENV").as_deref() == Ok("1") || std::env::var_os("HERDR_SOCKET_PATH").is_some() {
        let script = format!(
            "sleep 0.25; for i in 1 2 3 4 5 6; do herdr plugin action invoke {id} --plugin corral >/dev/null 2>&1 && break; sleep 0.2; done");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &script])
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // Its own session before it runs: when the popup closes, the hangup that
        // goes to the popup's processes must not reach this one.
        // SAFETY: setsid() is async-signal-safe and touches nothing of the parent.
        unsafe {
            cmd.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
        let _ = cmd.spawn();
    } else {
        let _ = Command::new(std::env::current_exe().unwrap_or_else(|_| "corral".into())).args(args).status();
    }
}

// --------------------------------------------------------------------------- list and change

fn print_list(all: &[Entry]) {
    let mut group = None;
    for e in all {
        if group != Some(e.group) {
            println!("{}{}", if group.is_none() { "" } else { "\n" }, e.group.name());
            group = Some(e.group);
        }
        let keys = if e.keys.is_empty() { tr("(no shortcut)", "(sin atajo)").to_string() } else { e.keys.join(", ") };
        let current = if e.current.is_empty() { String::new() } else { format!("  {}", e.current) };
        println!("  {:<26} {}{current}  [{}]", keys, e.desc, e.id);
    }
}

fn to_json(all: &[Entry]) -> Value {
    Value::Arr(all.iter().map(|e| obj(vec![
        ("group", e.group.name().into()),
        ("keys", Value::Arr(e.keys.iter().map(|k| k.as_str().into()).collect())),
        ("description", e.desc.as_str().into()),
        ("id", e.id.as_str().into()),
        ("runnable", e.run.is_some().into()),
        // "active": the console in view runs by it; "global": the global value, while the
        // console runs by a stricter one.
        ("in_effect", match e.current {
            "" => false.into(),
            c if c == tr("● global", "● global") => "global".into(),
            _ => "active".into(),
        }),
        ("changeable", changeable(&e.id).into()),
    ])).collect())
}

/// `KEY` as herdr writes it: what comes after the prefix (`t`, `shift+k`) or the whole shortcut.
fn herdr_key_spec(prefix: &str, key: &str) -> String {
    let key = key.trim().to_lowercase();
    if let Some(rest) = key.strip_prefix(&format!("{prefix} ")).or_else(|| key.strip_prefix(&format!("{prefix}+"))) {
        return format!("prefix+{}", rest.replace('-', "minus"));
    }
    if key.starts_with("prefix+") {
        return key;
    }
    format!("prefix+{}", if key == "-" { "minus".to_string() } else { key })
}

fn change_herdr(action: &str, key: &str, force: bool) -> Result<String, String> {
    let actions = plugin_actions();
    let Some((_, title, _)) = actions.iter().find(|a| a.0 == action) else {
        return Err(trf!("there is no action herdr:{action}; `corral keys --list` shows the ones there are",
                        "no hay una acción herdr:{action}; `corral keys --list` muestra las que hay"));
    };
    let path = herdr_config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (prefix, binds) = herdr_bindings(&text);
    let spec = herdr_key_spec(&prefix, key);
    let command = format!("corral.{action}");
    if !force {
        if let Some(rest) = spec.strip_prefix("prefix+") {
            if HERDR_DEFAULTS.contains(&rest) {
                return Err(trf!("{} is already one of herdr's own shortcuts (see {prefix} ?); use another key or --force",
                                "{} ya es un atajo propio de herdr (ver {prefix} ?); usa otra tecla o --force", herdr_key(&prefix, &spec)));
            }
        }
        if let Some(b) = binds.iter().find(|b| b.key == spec && b.command != command) {
            return Err(trf!("{} is already bound to {}; use another key or --force",
                            "{} ya está asignado a {}; usa otra tecla o --force", herdr_key(&prefix, &spec), b.command));
        }
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    match binds.iter().find(|b| b.kind == "plugin_action" && b.command == command) {
        Some(Binding { key_line: Some(i), command_line, .. }) => {
            lines[*i] = format!("key = \"{spec}\"");
            // An older Corral may have written the action's old name.
            if let Some(j) = command_line {
                lines[*j] = format!("command = \"{command}\"");
            }
        }
        _ => {
            lines.push(String::new());
            lines.push("[[keys.command]]".into());
            lines.push(format!("key = \"{spec}\""));
            lines.push("type = \"plugin_action\"".into());
            lines.push(format!("command = \"{command}\""));
            lines.push(format!("description = \"{title}\""));
        }
    }
    std::fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    let reloaded = Command::new("herdr").args(["server", "reload-config"]).output().map(|o| o.status.success()).unwrap_or(false);
    Ok(format!("{} → {}{}", herdr_key(&prefix, &spec), action_label(action, title).1,
               if reloaded { tr(" (herdr already picked it up)", " (herdr ya lo tomó)") }
               else { tr(" (in herdr: ctrl+b shift+r to reload)", " (en herdr: ctrl+b shift+r para recargar)") }))
}

/// An editor key ("ctrl+e", "f6", "ctrl+shift+enter") as a binding of Fresh's config.
fn editor_binding(action: &str, key: &str) -> Result<Value, String> {
    let parts: Vec<String> = key.trim().to_lowercase().split('+').map(String::from).collect();
    let (k, mods) = parts.split_last().filter(|(k, _)| !k.is_empty()).ok_or(tr("the key is missing", "falta la tecla"))?;
    let mods: Vec<String> = mods.to_vec();
    let k = match k.as_str() {
        "/" if mods == ["ctrl"] => "7".to_string(),
        "-" => "minus".to_string(),
        "enter" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        f if f.starts_with('f') && f.len() > 1 && f[1..].chars().all(|c| c.is_ascii_digit()) => f.to_uppercase(),
        other => other.to_string(),
    };
    let mut pairs = vec![("key", Value::from(k.as_str()))];
    if !mods.is_empty() {
        pairs.push(("modifiers", Value::Arr(mods.iter().map(|m| m.as_str().into()).collect())));
    }
    pairs.push(("action", action.into()));
    Ok(obj(pairs))
}

/// Two editor bindings on the same key.
fn same_combo(a: &Value, b: &Value) -> bool {
    a.get("key") == b.get("key") && a.get("modifiers").arr() == b.get("modifiers").arr()
}

fn change_editor(action: &str, key: &str, force: bool) -> Result<String, String> {
    let path = fresh_config_path();
    let mut cfg = env::read_json(&path);
    let new = editor_binding(action, key)?;
    let same_key = |kb: &Value| same_combo(kb, &new);
    let mut list: Vec<Value> = cfg.get("keybindings").arr().to_vec();
    if let Some(other) = list.iter().find(|kb| same_key(kb) && kb.get("action").str() != Some(action)) {
        if !force {
            return Err(trf!("{} already does “{}” in the editor; use another key or --force",
                            "{} ya hace «{}» en el editor; usa otra tecla o --force",
                            fresh_key(&new), editor_desc(other.get("action").str().unwrap_or(""))));
        }
    }
    list.retain(|kb| kb.get("action").str() != Some(action) && !same_key(kb));
    list.push(new.clone());
    cfg.set("keybindings", Value::Arr(list));
    std::fs::write(&path, cfg.to_json_pretty(2)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(trf!("{} → {} (close the editor with Ctrl+Q and open it again to use it)",
            "{} → {} (cierra el editor con Ctrl+Q y vuelve a abrirlo para usarlo)", fresh_key(&new), editor_desc(action)))
}

// --------------------------------------------------------------------------- your keys

/// The keys you changed (Tab in the palette, `corral keys --change`), by id:
/// "herdr:toggle" -> "prefix+u", "editor:corral_run_line" -> "ctrl+e". They live
/// in Corral's config, so the installer puts them back when Corral updates.
pub fn own_keys() -> Value {
    env::config().get("keys").clone()
}

fn remember(id: &str, key: Option<&str>) {
    let mut cfg = env::config();
    let mut keys = match cfg.get("keys") {
        v @ Value::Obj(_) => v.clone(),
        _ => Value::obj(),
    };
    match key {
        Some(k) => keys.set(id, k.into()),
        None => keys.remove(id),
    }
    if matches!(&keys, Value::Obj(items) if items.is_empty()) {
        cfg.remove("keys");
    } else {
        cfg.set("keys", keys);
    }
    crate::theme::write_config(&cfg);
}

/// herdr's shortcuts as Corral ships them (`[[keys.command]]` tables) with your
/// keys on top: yours replace the default, and an action that ships without a
/// key gets one. A default on a key you took for another action gives it up.
pub fn apply_herdr(wanted: Vec<Value>, own: &Value) -> Vec<Value> {
    let Value::Obj(items) = own else { return wanted };
    let mine: Vec<(String, String)> = items.iter()
        .filter_map(|(id, k)| Some((format!("corral.{}", id.strip_prefix("herdr:")?), k.str()?.to_string()))).collect();
    let mut out: Vec<Value> = wanted.into_iter()
        .filter(|k| {
            let command = k.get("command").str().unwrap_or("");
            mine.iter().any(|(c, _)| c == command) || !mine.iter().any(|(_, key)| Some(key.as_str()) == k.get("key").str())
        })
        .collect();
    let titles = plugin_actions();
    for (command, key) in &mine {
        match out.iter_mut().find(|k| k.get("command").str() == Some(command.as_str())) {
            Some(k) => k.set("key", key.as_str().into()),
            None => {
                let id = command.trim_start_matches("corral.");
                let title = titles.iter().find(|a| a.0 == id).map(|a| a.1.clone()).unwrap_or_else(|| format!("Corral: {id}"));
                out.push(obj(vec![("key", key.as_str().into()), ("type", "plugin_action".into()),
                                  ("command", command.as_str().into()), ("description", title.into())]));
            }
        }
    }
    out
}

/// The editor's shortcuts as Corral ships them, with your keys on top: an action
/// you changed keeps only your key, and any other binding on it goes.
pub fn apply_editor(list: Vec<Value>, own: &Value) -> Vec<Value> {
    let Value::Obj(items) = own else { return list };
    let mut out = list;
    for (id, key) in items {
        let (Some(action), Some(key)) = (id.strip_prefix("editor:"), key.str()) else { continue };
        let Ok(binding) = editor_binding(action, key) else { continue };
        out.retain(|kb| kb.get("action").str() != Some(action) && !same_combo(kb, &binding));
        out.push(binding);
    }
    out
}

/// Corral's default herdr key for an action (herdr/keys.example.toml), if it has one.
fn default_herdr_key(action: &str) -> Option<String> {
    let text = std::fs::read_to_string(env::repo().join("herdr/keys.example.toml")).ok()?;
    let command = format!("corral.{action}");
    crate::toml::parse(&text).ok()?.get("keys").get("command").arr().iter()
        .find(|k| k.get("command").str() == Some(command.as_str()))
        .and_then(|k| k.get("key").str().map(String::from))
}

/// Take a Corral action's whole `[[keys.command]]` entry out of herdr's config.
fn unbind_herdr(action: &str) -> Result<(), String> {
    let path = herdr_config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (_, binds) = herdr_bindings(&text);
    let command = format!("corral.{action}");
    let Some(at) = binds.iter().find(|b| b.command == command).and_then(|b| b.key_line) else { return Ok(()) };
    let mut lines: Vec<&str> = text.lines().collect();
    let Some(start) = (0..=at).rev().find(|&j| lines[j].trim() == "[[keys.command]]") else { return Ok(()) };
    let mut end = start + 1;
    while end < lines.len() && !lines[end].trim().is_empty() && !lines[end].trim_start().starts_with(['[', '#']) {
        end += 1;
    }
    if end < lines.len() && lines[end].trim().is_empty() {
        end += 1;
    }
    lines.drain(start..end);
    std::fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = Command::new("herdr").args(["server", "reload-config"]).output();
    Ok(())
}

/// Give `id` (herdr:ACTION or editor:ACTION) the key `key`, and remember it.
pub fn set_key(id: &str, key: &str, force: bool) -> Result<String, String> {
    let (msg, saved) = match id.split_once(':') {
        Some(("herdr", a)) if a != "help" => {
            let (prefix, _) = herdr_bindings(&std::fs::read_to_string(herdr_config_path()).unwrap_or_default());
            (change_herdr(a, key, force)?, herdr_key_spec(&prefix, key))
        }
        Some(("editor", a)) => (change_editor(a, key, force)?, fresh_key(&editor_binding(a, key)?)),
        _ => return Err(trf!("{id} can't be changed in the configuration: it is part of Corral or of herdr",
                             "{id} no se cambia desde la configuración: es parte de Corral o de herdr")),
    };
    remember(id, Some(&saved));
    Ok(msg)
}

/// Back to Corral's default key for `id`, and forget yours.
pub fn reset_key(id: &str) -> Result<String, String> {
    let msg = match id.split_once(':') {
        Some(("herdr", a)) if a != "help" => match default_herdr_key(a) {
            Some(k) => change_herdr(a, &k, false)?,
            None => {
                unbind_herdr(a)?;
                trf!("{} has no key again, as Corral ships it", "{} vuelve a no tener tecla, como viene en Corral",
                     action_label(a, a).1)
            }
        },
        Some(("editor", a)) => {
            let defaults: Vec<Value> = env::read_json(&env::repo().join("fresh/config.json")).get("keybindings").arr().iter()
                .filter(|kb| kb.get("action").str() == Some(a)).cloned().collect();
            let path = fresh_config_path();
            let mut cfg = env::read_json(&path);
            let mut list: Vec<Value> = cfg.get("keybindings").arr().iter().filter(|kb| kb.get("action").str() != Some(a)).cloned().collect();
            let back: Vec<Value> = defaults.into_iter().filter(|d| !list.iter().any(|kb| same_combo(kb, d))).collect();
            let shown = back.iter().map(fresh_key).collect::<Vec<_>>().join(", ");
            list.extend(back);
            cfg.set("keybindings", Value::Arr(list));
            std::fs::write(&path, cfg.to_json_pretty(2)).map_err(|e| format!("{}: {e}", path.display()))?;
            trf!("{shown} → {} (close the editor with Ctrl+Q and open it again to use it)",
                 "{shown} → {} (cierra el editor con Ctrl+Q y vuelve a abrirlo para usarlo)", editor_desc(a))
        }
        _ => return Err(trf!("{id} can't be changed in the configuration: it is part of Corral or of herdr",
                             "{id} no se cambia desde la configuración: es parte de Corral o de herdr")),
    };
    remember(id, None);
    Ok(msg)
}

/// The key you pressed in the palette, as herdr writes it after its prefix
/// (`herdr`) or as the editor writes a whole shortcut.
fn pressed(key: &str, herdr: bool) -> Result<String, String> {
    let ctrl = |c: char| (c as u32) >= 1 && (c as u32) <= 26 && !matches!(c, '\t' | '\n' | '\r' | '\x08');
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let spec = match (single, key) {
        (Some(c), _) if ctrl(c) => format!("ctrl+{}", (b'a' + (c as u8) - 1) as char),
        (_, k) if k.starts_with("alt+") || (k.starts_with('f') && k.len() > 1 && k[1..].chars().all(|d| d.is_ascii_digit())) => k.to_lowercase(),
        _ if !herdr => return Err(tr("in the editor, use a key with ctrl or alt, or an F key",
                                     "en el editor, usa una tecla con ctrl o alt, o una tecla F").into()),
        (Some('\t'), _) => "tab".into(),
        (Some(' '), _) => "space".into(),
        (Some('-'), _) => "minus".into(),
        (Some(c), _) if c.is_ascii_uppercase() => format!("shift+{}", c.to_ascii_lowercase()),
        (Some(c), _) if !c.is_control() => c.to_string(),
        (_, "backtab") => "shift+tab".into(),
        (_, k @ ("enter" | "up" | "down" | "left" | "right" | "home" | "end" | "pgup" | "pgdn")) => k.into(),
        _ => return Err(tr("that key can't be used here; try another", "esa tecla no se puede usar aquí; prueba otra").into()),
    };
    Ok(spec)
}

fn open_popup() -> bool {
    for _ in 0..8 {
        let ok = Command::new("herdr")
            .args(["plugin", "pane", "open", "--plugin", "corral", "--entrypoint", "keys", "--placement", "popup", "--focus"])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200)); // another modal still closing
    }
    false
}

fn usage() -> &'static str {
    tr("usage: corral keys [--list | --json | --change ID KEY [--force] | --reset ID]",
       "uso: corral keys [--list | --json | --change ID TECLA [--force] | --reset ID]")
}

pub fn run(args: &[String], popup: bool) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", usage());
        return Ok(());
    }
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--json") {
        println!("{}", to_json(&entries()).to_json_pretty(1));
        return Ok(());
    }
    if has("--list") {
        print_list(&entries());
        return Ok(());
    }
    if has("--change") {
        let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
        let (Some(id), Some(key)) = (rest.first(), rest.get(1)) else {
            return Err(tr("usage: corral keys --change ID KEY [--force]   (corral keys --list shows the IDs)",
                          "uso: corral keys --change ID TECLA [--force]   (los ID salen en corral keys --list)").into());
        };
        println!("{}", set_key(id, key, has("--force"))?);
        return Ok(());
    }
    if has("--reset") {
        let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
        let Some(id) = rest.first() else {
            return Err(tr("usage: corral keys --reset ID   (corral keys --list shows the IDs)",
                          "uso: corral keys --reset ID   (los ID salen en corral keys --list)").into());
        };
        println!("{}", reset_key(id)?);
        return Ok(());
    }
    // Inside herdr the palette opens as a popup over everything; here, or from the popup, it runs inline.
    if !popup && std::env::var("HERDR_ENV").as_deref() == Ok("1") && open_popup() {
        return Ok(());
    }
    palette()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InEffect, Mix, Project};

    fn mix(access: &'static str, output: &'static str, profile: &'static str) -> Mix {
        Mix { access, output, profile }
    }
    const OPEN: Mix = Mix { access: "on", output: "full", profile: "open" };
    const STANDARD: Mix = Mix { access: "ask", output: "full", profile: "standard" };
    const SENSITIVE: Mix = Mix { access: "ask", output: "structure", profile: "sensitive" };

    /// The Security and Agent entries as the palette lists them, marked for `now`:
    /// (id, mark, description).
    fn marked(now: &InEffect, audit: Option<i64>, agent: &str) -> Vec<(String, &'static str, String)> {
        let mut list: Vec<Entry> = ACTIONS.iter().filter(|a| matches!(a.1, Group::Security | Group::Agent)).map(|a| Entry {
            group: a.1, keys: Vec::new(), desc: a.2.into(), id: format!("herdr:{}", a.0), run: Some((a.0.into(), Vec::new())), current: "",
        }).collect();
        mark(&mut list, now, audit, agent);
        list.into_iter().map(|e| (e.id.trim_start_matches("herdr:").to_string(), e.current, e.desc)).collect()
    }

    /// The marked entries: (id, mark).
    fn on(list: &[(String, &'static str, String)]) -> Vec<(String, &'static str)> {
        list.iter().filter(|(_, c, _)| !c.is_empty()).map(|(id, c, _)| (id.clone(), *c)).collect()
    }

    fn pairs(items: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        items.iter().map(|(id, c)| (id.to_string(), *c)).collect()
    }

    fn project(own: Option<&str>, applies: Option<&str>, without_own: Mix) -> Option<Project> {
        Some(Project { dir: PathBuf::from("/data/clinic"), own: own.map(String::from), applies: applies.map(String::from), without_own })
    }

    #[test]
    fn the_palette_marks_what_is_in_effect() {
        let standard = InEffect { global: STANDARD, console: STANDARD, project: None };
        let list = marked(&standard, Some(90), "claude");
        assert_eq!(on(&list), pairs(&[("profile-standard", "● active"), ("agents-ask", "●"), ("output-full", "●"), ("audit-90", "●"), ("agent-claude", "●")]));
        // Without a console here, no project entries: their action would have no folder.
        assert!(!list.iter().any(|(id, _, _)| id.starts_with("profile-project")), "{list:?}");
        // Kept forever: no audit entry is in effect; a shell in the agent's pane.
        let list = marked(&standard, None, "none");
        assert!(!on(&list).iter().any(|(id, _)| id.starts_with("audit-")));
        assert!(on(&list).contains(&("agent-none".into(), "●")));
    }

    #[test]
    fn a_mix_that_is_no_profile_shows_as_custom() {
        let review = mix("ask", "review", "custom");
        let list = marked(&InEffect { global: review, console: review, project: None }, Some(30), "codex");
        assert_eq!(on(&list), pairs(&[("profile-custom", "● active"), ("agents-ask", "●"), ("output-review", "●"), ("audit-30", "●"), ("agent-codex", "●")]));
        assert!(list[0].2.contains("you review results"), "{:?}", list[0]);
        assert_eq!(list[1].0, "profile-open");
    }

    #[test]
    fn the_console_can_run_by_stricter_rules() {
        // Open globally, sensitive in the console (its project's profile, or its own mark):
        // what it runs by is active, and the global values say they are the global ones.
        let list = marked(&InEffect { global: OPEN, console: SENSITIVE, project: project(Some("sensitive"), Some("sensitive"), OPEN) }, Some(90), "claude");
        assert_eq!(on(&list), pairs(&[("profile-open", "● global"), ("profile-sensitive", "● active"), ("profile-project-sensitive", "●"),
                                      ("agents-ask", "● active"), ("agents-on", "● global"), ("output-full", "● global"),
                                      ("output-structure", "● active"), ("audit-90", "●"), ("agent-claude", "●")]));
        // A standard project under an open global profile: the standard profile is active.
        let list = marked(&InEffect { global: OPEN, console: STANDARD, project: project(Some("standard"), Some("standard"), OPEN) }, Some(90), "claude");
        let profiles: Vec<_> = on(&list).into_iter().filter(|(id, _)| id.starts_with("profile-")).collect();
        assert_eq!(profiles, pairs(&[("profile-open", "● global"), ("profile-standard", "● active")]));
        // Two mixes that are no profile: both shown, the console's active.
        let list = marked(&InEffect { global: mix("on", "review", "custom"), console: mix("ask", "review", "custom"), project: None }, Some(90), "claude");
        assert_eq!(on(&list)[..2], pairs(&[("profile-custom", "● global"), ("profile-custom", "● active")]));
    }

    #[test]
    fn the_project_is_named_by_its_folder() {
        let find = |list: &[(String, &'static str, String)], id: &str| list.iter().find(|e| e.0 == id).cloned();
        // Its own profile: removing it says what comes back, the global profile here.
        let own = InEffect { global: STANDARD, console: SENSITIVE, project: project(Some("sensitive"), Some("sensitive"), STANDARD) };
        let list = marked(&own, Some(90), "claude");
        assert_eq!(find(&list, "profile-project-sensitive").unwrap().2, "Project clinic: sensitive in every session");
        assert_eq!(find(&list, "profile-project-clear").unwrap().2, "Project clinic: remove its profile (back to standard)");
        // A folder above is sensitive: removing this one's own leaves that one, not the global.
        let above = InEffect { project: project(Some("standard"), Some("sensitive"), SENSITIVE), ..own };
        assert_eq!(find(&marked(&above, Some(90), "claude"), "profile-project-clear").unwrap().2, "Project clinic: remove its profile (back to sensitive)");
        // A folder's name can't send the terminal its own controls.
        let odd = InEffect { project: Some(Project { dir: PathBuf::from("/data/x\x1b[2Jy"), own: None, applies: None, without_own: STANDARD }), ..own };
        let shown = find(&marked(&odd, Some(90), "claude"), "profile-project-sensitive").unwrap().2;
        assert!(!shown.contains('\x1b') && shown.contains("x") && shown.contains("[2Jy"), "{shown:?}");
        // Sensitive only from a folder above: marked, and nothing to remove from here.
        let inherited = InEffect { global: STANDARD, console: SENSITIVE, project: project(None, Some("sensitive"), SENSITIVE) };
        let list = marked(&inherited, Some(90), "claude");
        assert_eq!(find(&list, "profile-project-sensitive").unwrap().1, "●");
        assert!(find(&list, "profile-project-clear").is_none(), "{list:?}");
    }

    #[test]
    fn the_key_you_press() {
        assert_eq!(pressed("u", true).unwrap(), "u");
        assert_eq!(pressed("U", true).unwrap(), "shift+u");
        assert_eq!(pressed(" ", true).unwrap(), "space");
        assert_eq!(pressed("-", true).unwrap(), "minus");
        assert_eq!(pressed("\t", true).unwrap(), "tab");
        assert_eq!(pressed("backtab", true).unwrap(), "shift+tab");
        assert_eq!(pressed("\u{5}", true).unwrap(), "ctrl+e");
        assert_eq!(pressed("f5", false).unwrap(), "f5");
        assert_eq!(pressed("alt+x", false).unwrap(), "alt+x");
        assert_eq!(pressed("\u{12}", false).unwrap(), "ctrl+r");
        assert!(pressed("u", false).is_err(), "a plain letter would stop you typing it in the editor");
    }

    fn herdr_entry(key: &str, command: &str) -> Value {
        obj(vec![("key", key.into()), ("type", "plugin_action".into()), ("command", command.into()), ("description", "d".into())])
    }

    #[test]
    fn your_herdr_keys_over_corrals() {
        let wanted = vec![herdr_entry("prefix+t", "corral.toggle"), herdr_entry("prefix+a", "corral.layout"), herdr_entry("prefix+y", "corral.switch")];
        let own = crate::json::parse(r#"{"herdr:toggle": "prefix+u", "herdr:switch": "prefix+a", "herdr:new-thing": "prefix+i"}"#).unwrap();
        let out = apply_herdr(wanted, &own);
        let key_of = |c: &str| out.iter().find(|k| k.get("command").str() == Some(c)).and_then(|k| k.get("key").str().map(String::from));
        assert_eq!(key_of("corral.toggle").as_deref(), Some("prefix+u"));
        assert_eq!(key_of("corral.switch").as_deref(), Some("prefix+a"));
        assert_eq!(key_of("corral.layout"), None, "layout gives up the key you took for switch");
        assert_eq!(key_of("corral.new-thing").as_deref(), Some("prefix+i"), "an action without a default key gets yours");
        assert_eq!(apply_herdr(vec![herdr_entry("prefix+t", "corral.toggle")], &Value::Null).len(), 1);
    }

    #[test]
    fn your_editor_keys_over_corrals() {
        let list = vec![editor_binding("corral_run_line", "ctrl+r").unwrap(), editor_binding("corral_run_line", "ctrl+enter").unwrap(),
                        editor_binding("corral_insert_pipe", "ctrl+e").unwrap(), editor_binding("corral_save", "ctrl+s").unwrap()];
        let own = crate::json::parse(r#"{"editor:corral_run_line": "ctrl+e", "herdr:toggle": "prefix+u"}"#).unwrap();
        let out = apply_editor(list, &own);
        let keys = |a: &str| out.iter().filter(|kb| kb.get("action").str() == Some(a)).map(fresh_key).collect::<Vec<_>>();
        assert_eq!(keys("corral_run_line"), ["ctrl+e"]);
        assert!(keys("corral_insert_pipe").is_empty(), "ctrl+e is yours now");
        assert_eq!(keys("corral_save"), ["ctrl+s"]);
    }

    #[test]
    fn herdr_keys() {
        let (prefix, binds) = herdr_bindings("[keys]\nprefix = \"ctrl+a\"\n\n[[keys.command]]\nkey = \"prefix+t\"\ntype = \"plugin_action\"\ncommand = \"corral.toggle\"\n");
        assert_eq!(prefix, "ctrl+a");
        assert_eq!(binds[0].command, "corral.toggle");
        assert_eq!(herdr_key(&prefix, &binds[0].key), "ctrl+a t");
        assert_eq!(herdr_key_spec("ctrl+b", "ctrl+b shift+k"), "prefix+shift+k");
        assert_eq!(herdr_key_spec("ctrl+b", "space"), "prefix+space");
        assert_eq!(norm("Gráfico"), "grafico");
    }

    #[test]
    fn old_action_names() {
        // What an older Corral wrote into herdr's config reads as today's actions.
        let (_, binds) = herdr_bindings("[[keys.command]]\nkey = \"prefix+space\"\ntype = \"plugin_action\"\ncommand = \"corral.atajos\"\n");
        assert_eq!(binds[0].command, "corral.keys");
        assert_eq!(binds[0].command_line, Some(3));
        assert_eq!(current_command("corral.layout-codigo"), "corral.layout-code");
        assert_eq!(current_command("corral.toggle"), "corral.toggle");
    }
}
