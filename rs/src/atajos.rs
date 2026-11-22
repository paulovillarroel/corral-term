//! `corral atajos` (`ctrl+b space`): every Corral shortcut in one searchable
//! list, Spotlight style, and Enter runs the ones that are actions. The list is
//! read from the real configuration (herdr's keybindings, the plugin's
//! actions, the editor's keys), so a change made by hand or with
//! `corral atajos --cambiar` shows up here. The keys of Corral's own screens
//! (console, variables, tables, Mirador) come from the code.
//!
//! `--lista` and `--json` print the same list (for people and for agents);
//! `--cambiar ID TECLA` rebinds a herdr or editor shortcut.

use crate::env;
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
const G_LAYOUT: &str = "Layouts";
const G_HERDR: &str = "En cualquier panel (herdr)";
const G_EDITOR: &str = "Editor de scripts";
const G_CONSOLE: &str = "Consola de R";
const G_VARS: &str = "Panel de variables";
const G_TABLE: &str = "Tablas (View)";
const G_MIRADOR: &str = "Mirador (navegador)";

/// Keys herdr itself binds after its prefix (herdr 0.9.5 defaults).
const HERDR_DEFAULTS: &[&str] = &[
    "?", "s", "w", "g", "b", "o", "q", "shift+r", "shift+n", "shift+g", "shift+w", "shift+d", "c", "shift+t", "p",
    "n", "1", "2", "3", "4", "5", "6", "7", "8", "9", "shift+x", "shift+p", "e", "[", "h", "j", "k", "l", "shift+h",
    "shift+j", "shift+k", "shift+l", "tab", "shift+tab", "v", "minus", "x", "z", "r",
];

#[derive(Clone)]
pub struct Entry {
    group: &'static str,
    keys: Vec<String>,
    desc: String,
    id: String,
    /// Arguments for `corral` (a plugin action: Enter runs it).
    run: Option<(String, Vec<String>)>,
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
                binds.push(Binding { key: String::new(), kind: String::new(), command: String::new(), key_line: None });
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
                b.command = v;
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
            // command = ["../bin/corral-run", "layout", …]
            a.2 = rest.split('"').skip(1).step_by(2).skip(1).map(String::from).collect();
        }
    }
    out.retain(|a| !a.0.is_empty() && a.0 != "open-url");
    out
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
        "corral_save" => "Guardar (el borrador pregunta dónde)",
        "corral_run_line" => "Ejecutar la línea o la selección",
        "corral_run_file" => "Ejecutar el archivo completo",
        "corral_insert_assign" => "Insertar <-",
        "corral_insert_pipe" => "Insertar |>",
        "toggle_comment" => "Comentar o descomentar",
        "lsp_completion" => "Autocompletar",
        other => other,
    }
    .to_string()
}

fn fixed(group: &'static str, prefix: &str, items: &[(&str, &[&str])]) -> Vec<Entry> {
    items.iter().enumerate().map(|(i, (desc, keys))| Entry {
        group,
        keys: keys.iter().map(|k| k.to_string()).collect(),
        desc: desc.to_string(),
        id: format!("{prefix}:{}", i + 1),
        run: None,
    }).collect()
}

/// Every shortcut, in display order.
pub fn entries() -> Vec<Entry> {
    let (prefix, binds) = herdr_bindings(&std::fs::read_to_string(herdr_config_path()).unwrap_or_default());
    let mut list = Vec::new();

    let mut herdr: Vec<Entry> = plugin_actions().into_iter().map(|(id, title, args)| {
        let command = format!("corral.{id}");
        let keys = binds.iter().filter(|b| b.kind == "plugin_action" && b.command == command)
            .map(|b| herdr_key(&prefix, &b.key)).collect();
        // The workspace layouts (layout, layout-codigo, …) get a group of their own, first.
        let group = if id == "layout" || id.starts_with("layout-") { G_LAYOUT } else { G_HERDR };
        Entry { group, keys, desc: tidy(&title), id: format!("herdr:{id}"), run: Some((id, args)) }
    }).collect();
    herdr.sort_by_key(|e| (e.group != G_LAYOUT, e.keys.is_empty())); // layouts, then the ones with a key
    list.extend(herdr);
    list.push(Entry { group: G_HERDR, keys: vec![format!("{prefix} ?")], desc: "Todos los atajos de herdr (paneles, pestañas, espacios)".into(),
                      id: "herdr:ayuda".into(), run: None });

    let fresh = env::read_json(&fresh_config_path());
    let mut actions: Vec<(String, Vec<String>)> = Vec::new();
    for kb in fresh.get("keybindings").arr() {
        let Some(action) = kb.get("action").str() else { continue };
        match actions.iter_mut().find(|(a, _)| a == action) {
            Some((_, keys)) => keys.push(fresh_key(kb)),
            None => actions.push((action.to_string(), vec![fresh_key(kb)])),
        }
    }
    for (action, keys) in actions {
        list.push(Entry { group: G_EDITOR, keys, desc: editor_desc(&action), id: format!("editor:{action}"), run: None });
    }
    list.extend(fixed(G_EDITOR, "editor-fijo", &[
        ("Paleta de comandos del editor (todas sus acciones)", &["ctrl+p"]),
        ("Buscar en el archivo", &["ctrl+f"]),
        ("Deshacer", &["ctrl+z"]),
        ("Cerrar el editor (el borrador se descarta)", &["ctrl+q"]),
    ]));
    list.extend(fixed(G_CONSOLE, "consola", &[
        ("Ejecutar, si la expresión está completa", &["enter"]),
        ("Ejecutar aunque falte cerrar algo", &["alt+enter"]),
        ("Historial (con algo escrito, solo lo que empieza igual)", &["↑", "↓"]),
        ("Completar nombres, argumentos y columnas", &["tab"]),
        ("Interrumpir lo que corre, o borrar lo escrito", &["ctrl+c"]),
        ("Cerrar la consola (en una línea vacía)", &["ctrl+d"]),
        ("Limpiar la pantalla", &["ctrl+l"]),
        ("Ir al inicio o al final de la línea", &["ctrl+a", "ctrl+e"]),
        ("Borrar hasta el final, hasta el inicio o la palabra anterior", &["ctrl+k", "ctrl+u", "ctrl+w"]),
    ]));
    list.extend(fixed(G_VARS, "variables", &[
        ("Cambiar de pestaña (datos, gráficos, valores, funciones, otros)", &["←", "→", "1–5"]),
        ("Moverse", &["↑", "↓", "pgup", "pgdn"]),
        ("Abrir: una tabla en el visor, un gráfico en Mirador, un valor con str()", &["enter"]),
        ("Buscar (las flechas eligen entre los resultados)", &["/"]),
        ("Cerrar la ventana", &["q"]),
    ]));
    list.extend(fixed(G_TABLE, "tablas", &[
        ("Moverse por las filas", &["↑", "↓", "pgup", "pgdn", "g", "G"]),
        ("Moverse por las columnas (también la rueda lateral)", &["←", "→", "0", "$"]),
        ("Ordenar por la columna", &["s"]),
        ("Buscar un texto en todas las columnas", &["/"]),
        ("Filtrar con una condición, en R o en SQL", &["f"]),
        ("Dejar solo las filas con el valor de la celda", &["="]),
        ("Quitar los filtros y la búsqueda", &["x"]),
        ("Ver la fila completa", &["enter"]),
        ("Resumen de cada columna", &["i"]),
        ("Cerrar", &["q"]),
    ]));
    list.extend(fixed(G_MIRADOR, "mirador", &[
        ("Gráfico anterior o siguiente", &["←", "→"]),
        ("Copiar el gráfico como imagen PNG", &["c"]),
        ("Mostrar u ocultar la barra lateral", &["v"]),
        ("Buscar en la barra lateral", &["/"]),
        ("Ver estos atajos", &["?"]),
    ]));
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
    let hay = norm(&format!("{} {} {} {}", e.desc, e.keys.join(" "), e.group, e.id));
    words.iter().all(|w| hay.contains(w.as_str()))
}

fn filter<'a>(all: &'a [Entry], q: &str) -> Vec<&'a Entry> {
    let words: Vec<String> = norm(q).split_whitespace().map(String::from).collect();
    all.iter().filter(|e| matches(e, &words)).collect()
}

// --------------------------------------------------------------------------- palette

enum Row {
    Header(&'static str),
    Item(usize),
    Gap,
}

fn rows_for(shown: &[&Entry]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut group = "";
    for (i, e) in shown.iter().enumerate() {
        if e.group != group {
            if !rows.is_empty() {
                rows.push(Row::Gap);
            }
            rows.push(Row::Header(e.group));
            group = e.group;
        }
        rows.push(Row::Item(i));
    }
    rows
}

fn draw(c: &Colors, cols: usize, height: usize, q: &str, shown: &[&Entry], total: usize, sel: usize, top: &mut usize) {
    let mut f = String::from("\x1b[H\x1b[2J");
    let count = if q.is_empty() { format!("{total} atajos") } else { format!("{} de {total}", shown.len()) };
    let search = if q.is_empty() {
        format!("{}Busca un atajo o una acción…{RESET}", c.dim)
    } else {
        format!("{}{q}{RESET}{}▏{RESET}", c.text, c.accent)
    };
    f += &format!("\x1b[1;2H{}⌕{RESET}  {search}\x1b[1;{}H{}{count}{RESET}", c.accent, cols.saturating_sub(width(&count)), c.dim);
    f += &format!("\x1b[2;1H{}{}{RESET}", c.dim, "─".repeat(cols));

    let rows = rows_for(shown);
    let list_h = height.saturating_sub(4);
    let sel_row = rows.iter().position(|r| matches!(r, Row::Item(i) if *i == sel)).unwrap_or(0);
    // Keep the selection in view, with its group's title when it is the first item.
    let want_top = if sel_row > 0 && matches!(rows[sel_row - 1], Row::Header(_)) { sel_row - 1 } else { sel_row };
    if want_top < *top {
        *top = want_top;
    } else if sel_row >= *top + list_h {
        *top = sel_row + 1 - list_h;
    }
    if shown.is_empty() {
        f += &format!("\x1b[4;4H{}Nada coincide con «{q}»{RESET}", c.muted);
    }
    for (n, row) in rows.iter().skip(*top).take(list_h).enumerate() {
        let y = 3 + n;
        match row {
            Row::Gap => {}
            Row::Header(g) => f += &format!("\x1b[{y};3H\x1b[1m{}{}{RESET}", c.muted, g.to_uppercase()),
            Row::Item(i) => {
                let e = shown[*i];
                let keys = e.keys.join("  ");
                let key_w = width(&keys).min(cols / 2);
                let desc_w = cols.saturating_sub(key_w + 8);
                let on = *i == sel;
                let (bg, mark) = if on { (c.sel_bg.as_str(), format!("{}▸{RESET}{}", c.accent, c.sel_bg)) } else { ("", " ".to_string()) };
                let runnable = if e.run.is_some() && on { format!(" {}↵", c.dim) } else { String::new() };
                f += &format!("\x1b[{y};1H{bg}\x1b[2K {mark} {}{}{runnable}", c.text, clip(&e.desc, desc_w));
                f += &format!("\x1b[{y};{}H{bg}{}{}{RESET}", cols.saturating_sub(key_w + 1), c.accent, clip(&keys, key_w));
            }
        }
    }
    let hint = match shown.get(sel) {
        Some(e) if e.run.is_some() => "↑↓ elegir · Enter ejecutar · Esc cerrar · escribe para buscar",
        Some(_) => "↑↓ elegir · Esc cerrar · escribe para buscar",
        None => "Esc borra la búsqueda",
    };
    f += &format!("\x1b[{};2H{}{}{RESET}", height, c.dim, clip(hint, cols.saturating_sub(2)));
    out(&f);
}

fn palette() -> Result<(), String> {
    let all = entries();
    let Some(scr) = Screen::enter(true) else { return Err("no hay una terminal".into()) };
    let c = Colors::load();
    let (mut q, mut sel, mut top, mut dirty) = (String::new(), 0usize, 0usize, true);
    let mut chosen: Option<(String, Vec<String>)> = None;
    loop {
        let shown = filter(&all, &q);
        sel = sel.min(shown.len().saturating_sub(1));
        if dirty && !scr.has_input() {
            let (cols, rows) = scr.size();
            draw(&c, cols, rows, &q, &shown, all.len(), sel, &mut top);
            dirty = false;
        }
        let Some(k) = scr.key(300) else { continue };
        dirty = true;
        let last = shown.len().saturating_sub(1);
        match k.as_str() {
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
    let mut group = "";
    for e in all {
        if e.group != group {
            println!("{}{}", if group.is_empty() { "" } else { "\n" }, e.group);
            group = e.group;
        }
        let keys = if e.keys.is_empty() { "(sin atajo)".to_string() } else { e.keys.join(", ") };
        println!("  {:<26} {}  [{}]", keys, e.desc, e.id);
    }
}

fn to_json(all: &[Entry]) -> Value {
    Value::Arr(all.iter().map(|e| obj(vec![
        ("grupo", e.group.into()),
        ("teclas", Value::Arr(e.keys.iter().map(|k| k.as_str().into()).collect())),
        ("descripcion", e.desc.as_str().into()),
        ("id", e.id.as_str().into()),
        ("ejecutable", e.run.is_some().into()),
        ("se_puede_cambiar", (e.id.starts_with("herdr:") && e.id != "herdr:ayuda" || e.id.starts_with("editor:") && !e.id.starts_with("editor-fijo")).into()),
    ])).collect())
}

/// `TECLA` as herdr writes it: what comes after the prefix (`t`, `shift+k`) or the whole shortcut.
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
        return Err(format!("no hay una acción herdr:{action}; `corral atajos --lista` muestra las que hay"));
    };
    let path = herdr_config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (prefix, binds) = herdr_bindings(&text);
    let spec = herdr_key_spec(&prefix, key);
    let command = format!("corral.{action}");
    if !force {
        if let Some(rest) = spec.strip_prefix("prefix+") {
            if HERDR_DEFAULTS.contains(&rest) {
                return Err(format!("{} ya es un atajo propio de herdr (ver {prefix} ?); usa otra tecla o --forzar", herdr_key(&prefix, &spec)));
            }
        }
        if let Some(b) = binds.iter().find(|b| b.key == spec && b.command != command) {
            return Err(format!("{} ya está asignado a {}; usa otra tecla o --forzar", herdr_key(&prefix, &spec), b.command));
        }
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    match binds.iter().find(|b| b.kind == "plugin_action" && b.command == command).and_then(|b| b.key_line) {
        Some(i) => lines[i] = format!("key = \"{spec}\""),
        None => {
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
    Ok(format!("{} → {}{}", herdr_key(&prefix, &spec), tidy(title),
               if reloaded { " (herdr ya lo tomó)" } else { " (en herdr: ctrl+b shift+r para recargar)" }))
}

fn change_editor(action: &str, key: &str, force: bool) -> Result<String, String> {
    let path = fresh_config_path();
    let mut cfg = env::read_json(&path);
    let parts: Vec<String> = key.trim().to_lowercase().split('+').map(String::from).collect();
    let (k, mods) = parts.split_last().ok_or("falta la tecla")?;
    let mods: Vec<String> = mods.to_vec();
    let k = match k.as_str() {
        "/" if mods == ["ctrl"] => "7".to_string(),
        "-" => "minus".to_string(),
        "enter" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        f if f.starts_with('f') && f.len() > 1 && f[1..].chars().all(|c| c.is_ascii_digit()) => f.to_uppercase(),
        other => other.to_string(),
    };
    let new = {
        let mut pairs = vec![("key", Value::from(k.as_str()))];
        if !mods.is_empty() {
            pairs.push(("modifiers", Value::Arr(mods.iter().map(|m| m.as_str().into()).collect())));
        }
        pairs.push(("action", action.into()));
        obj(pairs)
    };
    let same_key = |kb: &Value| kb.get("key") == new.get("key") && kb.get("modifiers").arr() == new.get("modifiers").arr();
    let mut list: Vec<Value> = cfg.get("keybindings").arr().to_vec();
    if let Some(other) = list.iter().find(|kb| same_key(kb) && kb.get("action").str() != Some(action)) {
        if !force {
            return Err(format!("{} ya hace «{}» en el editor; usa otra tecla o --forzar",
                               fresh_key(&new), editor_desc(other.get("action").str().unwrap_or(""))));
        }
    }
    list.retain(|kb| kb.get("action").str() != Some(action) && !same_key(kb));
    list.push(new.clone());
    cfg.set("keybindings", Value::Arr(list));
    std::fs::write(&path, cfg.to_json_pretty(2)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("{} → {} (cierra el editor con Ctrl+Q y vuelve a abrirlo para usarlo)", fresh_key(&new), editor_desc(action)))
}

fn open_popup() -> bool {
    for _ in 0..8 {
        let ok = Command::new("herdr")
            .args(["plugin", "pane", "open", "--plugin", "corral", "--entrypoint", "atajos", "--placement", "popup", "--focus"])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200)); // another modal still closing
    }
    false
}

pub fn run(args: &[String], popup: bool) -> Result<(), String> {
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--json") {
        println!("{}", to_json(&entries()).to_json_pretty(1));
        return Ok(());
    }
    if has("--lista") {
        print_list(&entries());
        return Ok(());
    }
    if has("--cambiar") {
        let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
        let (Some(id), Some(key)) = (rest.first(), rest.get(1)) else {
            return Err("uso: corral atajos --cambiar ID TECLA [--forzar]   (los ID salen en corral atajos --lista)".into());
        };
        let force = has("--forzar");
        let msg = match id.split_once(':') {
            Some(("herdr", a)) if a != "ayuda" => change_herdr(a, key, force)?,
            Some(("editor", a)) => change_editor(a, key, force)?,
            _ => return Err(format!("{id} no se cambia desde la configuración: es parte de Corral o de herdr")),
        };
        println!("{msg}");
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
}
