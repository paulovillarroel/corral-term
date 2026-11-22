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
/// to the plots, then what you set (agents, appearance) and help. Each entry
/// starts with a category ("Agents: …", "Editor: …"), so typing it filters.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Group {
    Layout,
    Editor,
    Console,
    Vars,
    Table,
    Plots,
    Agents,
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
            Group::Agents => tr("Agents", "Agentes"),
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
    ("start-r", Group::Layout, "Layout: start with R", "Layout: partir con R"),
    ("start-python", Group::Layout, "Layout: start with Python", "Layout: partir con Python"),
    ("toggle", Group::Layout, "Layout: expand the script (or agent) or the console", "Layout: expandir el script (o el agente) o la consola"),
    ("script", Group::Layout, "Layout: add the script editor above the console", "Layout: agregar el editor de scripts sobre la consola"),
    ("close", Group::Layout, "Layout: close (keeps the agent and the rest of the tab)", "Layout: cerrar (deja al agente y el resto de la pestaña)"),
    ("switch", Group::Console, "Console: R or Python", "Consola: R o Python"),
    ("send", Group::Console, "Console: send the selection (R or Python)", "Consola: enviar la selección (R o Python)"),
    ("console", Group::Console, "Console: use this pane as the console", "Consola: usar este panel como consola"),
    ("vars", Group::Vars, "Variables: open them over the terminal", "Variables: abrirlas sobre la terminal"),
    ("plots", Group::Plots, "Plots: show them", "Gráficos: verlos"),
    ("mirador", Group::Plots, "Mirador: open it in the browser (plots and tables)", "Mirador: abrirlo en el navegador (gráficos y tablas)"),
    ("agents-ask", Group::Agents, "Agents: ask before each request", "Agentes: preguntan antes de cada petición"),
    ("agents-on", Group::Agents, "Agents: run code without asking", "Agentes: ejecutan código sin preguntar"),
    ("agents-off", Group::Agents, "Agents: can't run code", "Agentes: no pueden ejecutar código"),
    ("output-full", Group::Agents, "Agents: see full results", "Agentes: ven los resultados completos"),
    ("output-review", Group::Agents, "Agents: you review results before they see them", "Agentes: revisas los resultados antes de que los vean"),
    ("output-structure", Group::Agents, "Agents: see only the structure of results", "Agentes: ven solo la estructura de los resultados"),
    ("theme-dark-2026", Group::Settings, "Theme: Dark 2026 (VS Code's and Positron's)", "Tema: Dark 2026 (el de VS Code y Positron)"),
    ("theme-herdr", Group::Settings, "Theme: back to herdr's", "Tema: volver al de herdr"),
    ("language-en", Group::Settings, "Language: English", "Language: English"),
    ("language-es", Group::Settings, "Idioma: español", "Idioma: español"),
    ("keys", Group::Help, "Help: shortcuts and actions (this palette)", "Ayuda: atajos y acciones (esta paleta)"),
];

/// A plugin action's place and text in the palette, in the user's language; an
/// action this list doesn't know yet goes under Layouts with its own title.
fn action_label(id: &str, title: &str) -> (Group, String, usize) {
    match ACTIONS.iter().position(|a| a.0 == id) {
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
    }).collect()
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
        (order, Entry { group, keys, desc, id: format!("herdr:{id}"), run: Some((id, args)) })
    }).collect();
    actions.sort_by_key(|(order, _)| *order);
    list.extend(actions.into_iter().map(|(_, e)| e));
    list.push(Entry { group: Group::Help, keys: vec![format!("{prefix} ?")],
                      desc: tr("Help: all of herdr's shortcuts (panes, tabs, workspaces)", "Ayuda: todos los atajos de herdr (paneles, pestañas, espacios)").into(),
                      id: "herdr:help".into(), run: None });

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
        list.push(Entry { group: Group::Editor, keys, desc: categorized("Editor", &editor_desc(&action)), id: format!("editor:{action}"), run: None });
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

fn draw(c: &Colors, cols: usize, height: usize, q: &str, shown: &[&Entry], total: usize, sel: usize, top: &mut usize) {
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
                f += &format!("\x1b[{y};1H{bg}\x1b[2K {mark} {}{}{runnable}", c.text, clip(&e.desc, desc_w));
                f += &format!("\x1b[{y};{}H{bg}{}{}{RESET}", cols.saturating_sub(key_w + 1), c.accent, clip(&keys, key_w));
            }
        }
    }
    let hint = match shown.get(sel) {
        Some(e) if e.run.is_some() => tr("↑↓ select · Enter run · Esc close · type to search", "↑↓ elegir · Enter ejecutar · Esc cerrar · escribe para buscar"),
        Some(_) => tr("↑↓ select · Esc close · type to search", "↑↓ elegir · Esc cerrar · escribe para buscar"),
        None => tr("Esc clears the search", "Esc borra la búsqueda"),
    };
    f += &format!("\x1b[{};2H{}{}{RESET}", height, c.dim, clip(hint, cols.saturating_sub(2)));
    out(&f);
}

fn palette() -> Result<(), String> {
    let all = entries();
    let Some(scr) = Screen::enter(true) else { return Err(tr("not a terminal", "no hay una terminal").into()) };
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
    let mut group = None;
    for e in all {
        if group != Some(e.group) {
            println!("{}{}", if group.is_none() { "" } else { "\n" }, e.group.name());
            group = Some(e.group);
        }
        let keys = if e.keys.is_empty() { tr("(no shortcut)", "(sin atajo)").to_string() } else { e.keys.join(", ") };
        println!("  {:<26} {}  [{}]", keys, e.desc, e.id);
    }
}

fn to_json(all: &[Entry]) -> Value {
    Value::Arr(all.iter().map(|e| obj(vec![
        ("group", e.group.name().into()),
        ("keys", Value::Arr(e.keys.iter().map(|k| k.as_str().into()).collect())),
        ("description", e.desc.as_str().into()),
        ("id", e.id.as_str().into()),
        ("runnable", e.run.is_some().into()),
        ("changeable", (e.id.starts_with("herdr:") && e.id != "herdr:help" || e.id.starts_with("editor:")).into()),
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

fn change_editor(action: &str, key: &str, force: bool) -> Result<String, String> {
    let path = fresh_config_path();
    let mut cfg = env::read_json(&path);
    let parts: Vec<String> = key.trim().to_lowercase().split('+').map(String::from).collect();
    let (k, mods) = parts.split_last().ok_or(tr("the key is missing", "falta la tecla"))?;
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
    tr("usage: corral keys [--list | --json | --change ID KEY [--force]]",
       "uso: corral keys [--list | --json | --change ID TECLA [--force]]")
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
        let force = has("--force");
        let msg = match id.split_once(':') {
            Some(("herdr", a)) if a != "help" => change_herdr(a, key, force)?,
            Some(("editor", a)) => change_editor(a, key, force)?,
            _ => return Err(trf!("{id} can't be changed in the configuration: it is part of Corral or of herdr",
                                 "{id} no se cambia desde la configuración: es parte de Corral o de herdr")),
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
