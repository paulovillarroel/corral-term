//! The variables pane (`corral vars --pane`, and the `_vars` popup): the
//! objects of the R or Python session, in tabs, refreshed as they change.
//! The session writes vars.json after every command; this only reads it.

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::json::{obj, Value};
use crate::term::{clip, out, width, Screen};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// The tabs: kind, English name, Spanish name.
const TABS: [(&str, &str, &str); 5] = [
    ("data", "Data", "Datos"),
    ("plotobj", "Plots", "Gráficos"),
    ("value", "Values", "Valores"),
    ("function", "Functions", "Funciones"),
    ("other", "Other", "Otros"),
];

fn tab_name(i: usize) -> &'static str {
    tr(TABS[i].1, TABS[i].2)
}
const LIST_TOP: usize = 5;
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

/// Colours from herdr's theme (~/.config/corral/palette.json), with defaults.
pub struct Colors {
    pub accent: String,
    pub text: String,
    pub muted: String,
    pub dim: String,
    pub blue: String,
    pub violet: String,
    pub sel_bg: String,
    seen: f64,
}

fn mtime(path: &Path) -> f64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn palette_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| env::home().join(".config"));
    config.join("corral/palette.json")
}


fn hex(p: &Value, key: &str, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    let Some(h) = p.get(key).str().and_then(|s| s.strip_prefix('#')) else { return fallback };
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    if h.len() == 6 { (byte(0), byte(2), byte(4)) } else { fallback }
}

impl Colors {
    pub fn load() -> Colors {
        let p = env::read_json(&palette_file());
        let fg = |k: &str, d: (u8, u8, u8)| {
            let (r, g, b) = hex(&p, k, d);
            format!("\x1b[38;2;{r};{g};{b}m")
        };
        let (r, g, b) = hex(&p, "selection", (34, 34, 34));
        Colors {
            accent: fg("accent", (224, 123, 83)),
            text: fg("text", (233, 233, 233)),
            muted: fg("muted", (155, 155, 155)),
            dim: fg("dim", (100, 100, 100)),
            blue: fg("blue", (121, 168, 255)),
            violet: fg("violet", (178, 148, 255)),
            sel_bg: format!("\x1b[48;2;{r};{g};{b}m"),
            seen: mtime(&palette_file()),
        }
    }

    /// The theme changed since these were read (herdr's theme follows into palette.json).
    pub fn changed(&self) -> bool {
        mtime(&palette_file()) != self.seen
    }

    fn icon(&self, kind: &str) -> (&str, &str) {
        match kind {
            "data" => (&self.blue, "▦"),
            "plotobj" => (&self.accent, "◇"),
            "function" => (&self.violet, "ƒ"),
            "value" => (&self.dim, "•"),
            _ => (&self.dim, "◇"),
        }
    }
}

fn fmt_bytes(n: Option<f64>) -> String {
    let Some(mut n) = n else { return String::new() };
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || unit == "GB" {
            return if unit == "B" || n >= 10.0 { format!("{n:.0} {unit}") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    String::new()
}

/// The session of the console that's showing in this workspace (follows restarts and the R/Python switch).
fn resolve_session() -> Option<PathBuf> {
    let ws = env::current_workspace();
    let panes = herdr::panes(ws.as_deref()).ok()?;
    let lang = env::active_lang(ws.as_deref(), &panes);
    let label = format!("corral:{lang}");
    let console = herdr::labeled(&panes, &label).map(|p| herdr::pane_id(p).to_string()).or_else(|| {
        herdr::panes(None).ok().and_then(|all| herdr::labeled(&all, &label).map(|p| herdr::pane_id(p).to_string()))
    });
    let sessions = env::sessions();
    if let Some(console) = console {
        return sessions.iter().find(|s| s.get("pane_id").str() == Some(&console))
            .and_then(|s| s.get("dir").str().map(PathBuf::from));
    }
    let wanted = std::env::var("CORRAL_SESSION").ok();
    if let Some(s) = sessions.iter().find(|s| wanted.is_some() && s.get("id").str() == wanted.as_deref()) {
        return s.get("dir").str().map(PathBuf::from);
    }
    let same: Vec<&Value> = sessions.iter().filter(|s| ws.is_some() && s.get("workspace_id").str() == ws.as_deref()).collect();
    let pool: Vec<&Value> = if same.is_empty() { sessions.iter().collect() } else { same };
    pool.into_iter().max_by(|a, b| a.get("started").num().unwrap_or(0.0).total_cmp(&b.get("started").num().unwrap_or(0.0)))
        .and_then(|s| s.get("dir").str().map(PathBuf::from))
}

struct Viewer {
    session: Option<PathBuf>,
    follow: bool,
    keep_open: bool,
    items: Vec<Value>,
    meta: Value,
    rev: Option<f64>,
    tab: usize,
    tab_chosen: bool,
    sel: usize,
    top: usize,
    query: String,
    typing: bool,
    status: String,
    tab_spans: Vec<(usize, usize)>,
    row_items: Vec<(usize, usize)>,
    colors: Colors,
    theme_asked: Option<Instant>,
    /// A newer Corral, from the daily check: the hint row says so until you update.
    newer: Option<String>,
    newer_at: Instant,
    /// The security profile this session runs under (crate::profile), in the header.
    profile: &'static str,
}

impl Viewer {
    fn load(&mut self) -> bool {
        if self.follow {
            let found = resolve_session();
            if found != self.session {
                self.session = found;
                self.rev = None;
                self.items.clear();
                self.meta = Value::obj();
                self.sel = 0;
            }
        }
        let Some(dir) = &self.session else {
            let changed = !self.items.is_empty() || self.rev.is_some();
            self.rev = None;
            self.items.clear();
            return changed;
        };
        let file = dir.join("vars.json");
        if !file.is_file() {
            let changed = !self.items.is_empty() || self.rev.is_some();
            self.rev = None;
            self.items.clear();
            return changed;
        }
        let m = mtime(&file);
        if Some(m) == self.rev {
            return false;
        }
        self.rev = Some(m);
        self.items = env::read_json(&file).get("vars").arr().to_vec();
        self.meta = env::read_json(&dir.join("meta.json"));
        true
    }

    /// herdr's theme changed: have the palette rewritten, and pick it up when it is.
    fn palette_changed(&mut self) -> bool {
        let p = env::read_json(&palette_file());
        let source = p.get("source_mtime").num().unwrap_or(0.0);
        let changed = crate::theme::sources_mtime();
        if changed > 0.0 && (changed - source).abs() > 1e-3 && self.theme_asked.map(|t| t.elapsed() > Duration::from_secs(10)).unwrap_or(true) {
            self.theme_asked = Some(Instant::now());
            crate::theme::apply();
        }
        if mtime(&palette_file()) != self.colors.seen {
            self.colors = Colors::load();
            return true;
        }
        false
    }

    fn matches(&self, v: &Value) -> bool {
        let q = self.query.to_lowercase();
        q.is_empty() || v.get("name").str().unwrap_or("").to_lowercase().contains(&q)
            || v.get("class").str().unwrap_or("").to_lowercase().contains(&q)
    }

    fn in_tab(&self, i: usize) -> Vec<&Value> {
        let kind = TABS[i].0;
        let mut shown: Vec<&Value> = self.items.iter().filter(|v| v.get("kind").str() == Some(kind) && self.matches(v)).collect();
        shown.sort_by_key(|v| v.get("name").str().unwrap_or("").to_lowercase());
        shown
    }

    fn first_useful_tab(&self) -> usize {
        (0..TABS.len()).find(|&i| !self.in_tab(i).is_empty()).unwrap_or(0)
    }

    /// The profile of the session shown, as the agents' requests to it get it.
    fn current_profile(&self) -> &'static str {
        match (&self.session, self.meta.get("id").is_null()) {
            (Some(dir), false) => crate::profile::session_rules(dir, &self.meta).profile,
            _ => crate::profile::folder_rules(&std::env::current_dir().unwrap_or_else(|_| env::home())).profile,
        }
    }

    fn render(&mut self, scr: &Screen) {
        let (cols, rows) = scr.size();
        let c = &self.colors;
        let wide = cols >= 70;
        let vs: Vec<Value> = self.in_tab(self.tab).into_iter().cloned().collect();
        let mut f = String::from("\x1b[2J\x1b[H");

        let m = &self.meta;
        let lang = if m.get("lang").str() == Some("python") {
            format!("Python {}", m.get("python_version").str().unwrap_or(""))
        } else {
            format!("R {}", m.get("r_version").str().unwrap_or(""))
        };
        let home = env::home().to_string_lossy().to_string();
        let cwd = m.get("cwd").str().unwrap_or("").replace(&home, "~");
        let known = !m.get("id").is_null();
        let parts: Vec<&str> = [m.get("pane_id").str().unwrap_or(""), if known { lang.as_str() } else { "" }, cwd.as_str()]
            .into_iter().filter(|x| !x.is_empty()).collect();
        let where_ = parts.join(" · ");
        let n_vars = self.items.iter().filter(|v| self.matches(v)).count();
        let plural = if n_vars == 1 { "" } else { "s" };
        let count = trf!("{n_vars} object{plural}", "{n_vars} objeto{plural}");
        // The security profile first, so it is never the part clipped; in the accent
        // colour when it is sensitive.
        let profile = crate::profile::shown(self.profile);
        let pcolor = if self.profile == "sensitive" { &c.accent } else { &c.muted };
        f += &format!("\x1b[1;2H{}▸▸{RESET} {}{BOLD}Variables{RESET}  {pcolor}{profile}{RESET}{} · {}{RESET}", c.accent, c.text, c.dim,
                      clip(&where_, cols.saturating_sub(width(&count) + width(profile) + 22)));
        f += &format!("\x1b[1;{}H{}{count}{RESET}", cols.saturating_sub(width(&count)).max(1), c.muted);

        // Tab bar
        let mut x = 2;
        self.tab_spans.clear();
        let mut bar = String::new();
        for i in 0..TABS.len() {
            let n = self.in_tab(i).len();
            let text = format!(" {} {n} ", tab_name(i));
            let piece = if i == self.tab {
                format!("{}{}{BOLD}{text}{RESET}", c.sel_bg, c.accent)
            } else if n == 0 {
                format!("{}{text}{RESET}", c.dim)
            } else {
                format!("{}{text}{RESET}", c.muted)
            };
            self.tab_spans.push((x, x + width(&text) - 1));
            bar += &piece;
            bar.push(' ');
            x += width(&text) + 1;
        }
        f += &format!("\x1b[3;2H{bar}");
        let (a, b) = self.tab_spans[self.tab];
        f += &format!("\x1b[4;1H{}{}{RESET}", c.dim, "─".repeat(cols));
        f += &format!("\x1b[4;{a}H{}{}{RESET}", c.accent, "━".repeat(b - a + 1));

        if self.rev.is_none() {
            let msg = if self.follow {
                tr("Waiting for the console's session (R or Python)…", "Esperando la sesión de la consola (R o Python)…")
            } else {
                tr("This session doesn't publish variables.", "Esta sesión no publica variables.")
            };
            f += &format!("\x1b[6;3H{}{}{RESET}", c.muted, clip(msg, cols.saturating_sub(4)));
        }

        let area = rows.saturating_sub(LIST_TOP + 1);
        if self.sel < self.top {
            self.top = self.sel.saturating_sub(1);
        } else if self.sel >= self.top + area {
            self.top = self.sel + 1 - area;
        }
        self.row_items.clear();
        for (row, i) in (self.top..vs.len()).take(area).enumerate() {
            let v = &vs[i];
            let kind = v.get("kind").str().unwrap_or("other");
            let (color, icon) = c.icon(kind);
            let preview = v.get("preview").str().unwrap_or("").to_string();
            let size = fmt_bytes(v.get("size").num());
            let class = v.get("class").str().unwrap_or("");
            let meta_txt = if wide { if size.is_empty() { class.to_string() } else { format!("{class} · {size}") } } else { String::new() };
            let right = clip(&preview, (cols / 3).max(8));
            let name = clip(v.get("name").str().unwrap_or(""), cols.saturating_sub(width(&right) + width(&meta_txt) + 10).max(6));
            let pad = (cols as i64 - 6 - width(&name) as i64 - width(&right) as i64
                       - if meta_txt.is_empty() { 0 } else { width(&meta_txt) as i64 + 2 } - 1).max(1) as usize;
            let tail = if meta_txt.is_empty() { String::new() } else { format!("  {}{meta_txt}{RESET}", c.dim) };
            let body = format!("{color}{icon}{RESET} {}{BOLD}{name}{RESET}{}{}{right}{RESET}{tail}", c.text, " ".repeat(pad), c.muted);
            let y = row + LIST_TOP + 1;
            self.row_items.push((y, i));
            if i == self.sel {
                f += &format!("\x1b[{y};1H{sel}\x1b[2K {}▸{RESET}{sel} {body}{sel}\x1b[K{RESET}", c.accent, sel = c.sel_bg);
            } else {
                f += &format!("\x1b[{y};1H   {body}");
            }
        }
        if self.rev.is_some() && vs.is_empty() {
            let empty = if self.query.is_empty() {
                let name = tab_name(self.tab).to_lowercase();
                if self.tab == TABS.len() - 1 {
                    trf!("No other objects in the session.", "No hay {name} en la sesión.")
                } else {
                    trf!("No {name} in the session.", "No hay {name} en la sesión.")
                }
            } else {
                tr("No object matches.", "Ningún objeto coincide.").to_string()
            };
            f += &format!("\x1b[{};4H{}{empty}{RESET}", LIST_TOP + 2, c.muted);
        }

        let kind = vs.get(self.sel).and_then(|v| v.get("kind").str()).unwrap_or("");
        let action = match kind {
            "data" => tr("Enter table · m Mirador", "Enter tabla · m Mirador"),
            "plotobj" => tr("Enter draw", "Enter dibujar"),
            _ => tr("Enter show", "Enter ver"),
        };
        let hint = if self.typing {
            trf!("/{}▏  ↑↓ select · Enter open · Esc done", "/{}▏  ↑↓ elegir · Enter abrir · Esc terminar", self.query)
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            let quit = if self.keep_open { "" } else { tr(" · q quit", " · q salir") };
            trf!("←/→ tabs · ↑↓ move · {action} · / search{quit} · ctrl+b space shortcuts",
                 "←/→ pestañas · ↑↓ mover · {action} · / buscar{quit} · ctrl+b space atajos")
        };
        // A newer Corral goes first on that row, in the accent colour, until you update.
        let news = match (&self.newer, self.typing || !self.status.is_empty()) {
            (Some(v), false) => trf!("Corral {v} is out: ctrl+b space → Update", "Salió Corral {v}: ctrl+b space → Actualizar"),
            _ => String::new(),
        };
        let room = cols.saturating_sub(2);
        if news.is_empty() {
            f += &format!("\x1b[{rows};1H\x1b[2K {}{}{RESET}", c.dim, clip(&hint, room));
        } else {
            let news = clip(&news, room);
            let rest = room.saturating_sub(width(&news) + 3);
            let tail = if rest > 8 { format!("{} · {}{RESET}", c.dim, clip(&hint, rest)) } else { String::new() };
            f += &format!("\x1b[{rows};1H\x1b[2K {}{news}{RESET}{tail}", c.accent);
        }
        out(&f);
    }

    /// Enter on an object: a table opens, a plot is drawn, anything else is shown in the console.
    /// `m` (mirador) opens a table in Mirador instead of the terminal viewer.
    fn act(&self, v: &Value, mirador: bool) -> String {
        let name = v.get("name").str().unwrap_or("").to_string();
        let kind = v.get("kind").str().unwrap_or("");
        let Some(dir) = &self.session else { return String::new() };
        if kind == "plotobj" {
            return format!("{name}: {}", crate::plots::show_plot_object(dir, &name));
        }
        if mirador && kind != "data" {
            return tr("only tables open in Mirador", "solo las tablas se abren en Mirador").into();
        }
        let Some(pane) = self.meta.get("pane_id").str() else { return String::new() };
        let code = if self.meta.get("lang").str() == Some("python") {
            match kind {
                "data" if mirador => format!("View({name}, mirador=True)"),
                "data" => format!("View({name})"),
                "function" => format!("{name}?"),
                _ => name.clone(),
            }
        } else {
            let quoted = format!("`{}`", name.replace('`', "\\`"));
            match kind {
                "data" if mirador => format!("View({quoted}, mirador = TRUE)"),
                "data" => format!("View({quoted})"),
                _ => format!("str({quoted})"),
            }
        };
        match herdr::send_text(pane, &format!("{code}\r")) {
            Ok(_) => trf!("{code} sent to the console", "{code} enviado a la consola"),
            Err(_) => tr("couldn't send it to the console", "no pude enviar a la consola").into(),
        }
    }
}

pub fn message_screen(title: &str, body: &str) {
    let Some(scr) = Screen::enter(false) else { return };
    let c = Colors::load();
    let mut last = (0, 0);
    loop {
        let size = scr.size();
        if size != last {
            last = size;
            let lines: Vec<String> = std::iter::once(format!("{}{BOLD}{title}{RESET}", c.text)).chain(std::iter::once(String::new()))
                .chain(body.lines().map(|l| format!("{}{l}{RESET}", c.muted)))
                .chain([String::new(), format!("{}{}{RESET}", c.dim, tr("q to close", "q para cerrar"))]).collect();
            let top = size.1.saturating_sub(lines.len()) / 2;
            let mut f = String::from("\x1b[2J");
            for (i, l) in lines.iter().enumerate() {
                f += &format!("\x1b[{};3H{l}", top + i + 1);
            }
            out(&f);
        }
        if let Some(k) = scr.key(500) {
            if ["q", "esc", "enter", "quit"].contains(&k.as_str()) {
                return;
            }
        }
    }
}

fn usage() -> String {
    format!("{}: corral vars [--json [--lang r|python]] [--split] [--pane] [--inline]", tr("usage", "uso"))
}

/// `corral vars`: the variables as a popup over the terminal (or here with
/// --inline), as JSON for agents (--json), or as the layout's permanent pane
/// (--pane, which --split adds under the R console). `corral _vars` is the popup itself.
pub fn run(args: &[String], _all: &[String], popup: bool) -> Result<(), String> {
    let (mut json, mut split, mut pane_mode, mut inline, mut lang) = (false, false, false, false, None::<String>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--split" => split = true,
            "--pane" => pane_mode = true,
            "--inline" => inline = true,
            "--lang" => lang = it.next().cloned().filter(|l| l == "r" || l == "python"),
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => return Err(trf!("unknown option {other}\n{}", "opción desconocida {other}\n{}", usage())),
        }
    }
    if json {
        return print_json(lang.as_deref());
    }
    if split {
        return split_pane().map(|_| ());
    }
    if !popup && !pane_mode {
        // The popup over the terminal (in herdr), or the viewer right here.
        let dir = console::pick_session().and_then(|s| s.get("dir").str().map(String::from)).unwrap_or_default();
        if env::in_herdr() && !inline {
            return open_popup("vars", &[("CORRAL_SESSION_DIR", &dir)]);
        }
        std::env::set_var("CORRAL_SESSION_DIR", &dir);
        return run(&[], &[], true);
    }
    let session = if popup { std::env::var_os("CORRAL_SESSION_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) } else { None };
    if popup && session.is_none() {
        message_screen(tr("There is no Corral session", "No hay una sesión de Corral"),
                       tr("Open R or Python with ctrl+b a.", "Abre R o Python con ctrl+b a."));
        return Ok(());
    }
    let mut v = Viewer {
        session, follow: !popup, keep_open: !popup, items: Vec::new(), meta: Value::obj(), rev: None,
        tab: 0, tab_chosen: false, sel: 0, top: 0, query: String::new(), typing: false, status: String::new(),
        tab_spans: Vec::new(), row_items: Vec::new(), colors: Colors::load(), theme_asked: None,
        newer: crate::install::newer_version(), newer_at: Instant::now(), profile: "standard",
    };
    v.load();
    v.profile = v.current_profile();
    v.tab = v.first_useful_tab();
    let Some(scr) = Screen::enter(true) else { return Err(tr("not a terminal", "no hay una terminal").into()) };
    let mut size = scr.size();
    v.render(&scr);
    let mut last_check = Instant::now();
    let mut spanish = crate::i18n::es();
    let mut dirty = false;
    loop {
        // Draw once the input that already arrived is handled: a burst of wheel
        // events or key repeat must not queue a redraw (and a query) for each one.
        if dirty && !scr.has_input() {
            v.render(&scr);
            dirty = false;
        }
        let key = scr.key(300);
        if scr.size() != size {
            size = scr.size();
            v.render(&scr);
        }
        let Some(key) = key else {
            if last_check.elapsed() >= Duration::from_secs(1) {
                last_check = Instant::now();
                let mut changed = v.load();
                changed |= v.palette_changed();
                // The language switched (corral config language): draw in the new one.
                if crate::i18n::es() != spanish {
                    spanish = !spanish;
                    changed = true;
                }
                // A newer Corral (found once a day) or one just installed: three small
                // files, read every 10 s.
                if v.newer_at.elapsed() >= Duration::from_secs(10) {
                    v.newer_at = Instant::now();
                    let newer = crate::install::newer_version();
                    changed |= newer != v.newer;
                    v.newer = newer;
                    // The profile too: a setting or a project's profile may have changed.
                    let profile = v.current_profile();
                    changed |= profile != v.profile;
                    v.profile = profile;
                }
                if changed {
                    if !v.tab_chosen && v.in_tab(v.tab).is_empty() {
                        v.tab = v.first_useful_tab();
                    }
                    v.sel = v.sel.min(v.in_tab(v.tab).len().saturating_sub(1));
                    v.render(&scr);
                }
            }
            continue;
        };
        let n = v.in_tab(v.tab).len();
        v.status.clear();
        if v.typing && !key.starts_with("mouse:") {
            // Searching: the list filters as you type; arrows pick among the matches,
            // Enter opens the one selected, Esc ends the search (the filter stays).
            let mut changed = false;
            match key.as_str() {
                "esc" => v.typing = false,
                "enter" => {
                    v.typing = false;
                    if v.sel < n {
                        let item = v.in_tab(v.tab)[v.sel].clone();
                        v.status = "…".into();
                        v.render(&scr);
                        v.status = v.act(&item, false);
                        if !v.keep_open {
                            return Ok(());
                        }
                    }
                }
                "backspace" => {
                    v.query.pop();
                    changed = true;
                }
                "down" => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
                "up" => v.sel = v.sel.saturating_sub(1),
                "pgdn" => v.sel = (v.sel + 10).min(n.saturating_sub(1)),
                "pgup" => v.sel = v.sel.saturating_sub(10),
                "right" | "\t" => (v.tab, v.sel, v.top, v.tab_chosen) = ((v.tab + 1) % TABS.len(), 0, 0, true),
                "left" | "backtab" => (v.tab, v.sel, v.top, v.tab_chosen) = ((v.tab + TABS.len() - 1) % TABS.len(), 0, 0, true),
                k if k.chars().count() == 1 && !k.chars().next().unwrap().is_control() => {
                    v.query.push_str(k);
                    changed = true;
                }
                _ => {}
            }
            if changed {
                v.sel = 0;
                v.top = 0;
                // Nothing here matches, but another tab has matches: go there.
                if v.in_tab(v.tab).is_empty() {
                    if let Some(i) = (0..TABS.len()).find(|&i| !v.in_tab(i).is_empty()) {
                        v.tab = i;
                    }
                }
            }
            dirty = true;
            continue;
        }
        if let Some(rest) = key.strip_prefix("mouse:") {
            let nums: Vec<usize> = rest.split(':').filter_map(|x| x.parse().ok()).collect();
            if let [button, mx, my] = nums[..] {
                if button == 64 {
                    v.sel = v.sel.saturating_sub(3);
                } else if button == 65 {
                    v.sel = (v.sel + 3).min(n.saturating_sub(1));
                } else if button == 0 && my == 3 {
                    if let Some(i) = v.tab_spans.iter().position(|(a, b)| *a <= mx && mx <= *b) {
                        (v.tab, v.sel, v.top, v.tab_chosen) = (i, 0, 0, true);
                    }
                } else if button == 0 {
                    if let Some(&(_, clicked)) = v.row_items.iter().find(|(y, _)| *y == my) {
                        if clicked == v.sel && v.sel < n {
                            let item = v.in_tab(v.tab)[v.sel].clone();
                            v.status = "…".into();
                            v.render(&scr);
                            v.status = v.act(&item, false);
                            if !v.keep_open {
                                return Ok(());
                            }
                        }
                        v.sel = clicked;
                    }
                }
            }
            dirty = true;
            continue;
        }
        match key.as_str() {
            "q" | "quit" => return Ok(()),
            "esc" if !v.keep_open => return Ok(()),
            "esc" => v.query.clear(),
            "right" | "l" | "\t" => (v.tab, v.sel, v.top, v.tab_chosen) = ((v.tab + 1) % TABS.len(), 0, 0, true),
            "left" | "h" | "backtab" => (v.tab, v.sel, v.top, v.tab_chosen) = ((v.tab + TABS.len() - 1) % TABS.len(), 0, 0, true),
            k @ ("1" | "2" | "3" | "4" | "5") => (v.tab, v.sel, v.top, v.tab_chosen) = (k.parse::<usize>().unwrap() - 1, 0, 0, true),
            "down" | "j" => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            "up" | "k" => v.sel = v.sel.saturating_sub(1),
            "pgdn" => v.sel = (v.sel + 10).min(n.saturating_sub(1)),
            "pgup" => v.sel = v.sel.saturating_sub(10),
            "/" => (v.typing, v.query, v.sel) = (true, String::new(), 0),
            "enter" if v.sel < n => {
                let item = v.in_tab(v.tab)[v.sel].clone();
                v.status = "…".into();
                v.render(&scr);
                v.status = v.act(&item, false);
                if !v.keep_open {
                    return Ok(());
                }
            }
            "m" if v.sel < n => {
                let item = v.in_tab(v.tab)[v.sel].clone();
                v.status = v.act(&item, true);
            }
            _ => {}
        }
        dirty = true;
    }
}

/// `corral vars --json`: the variables of a console's session, for agents. With
/// `--lang`, that console's; without it, the console in view, like the variables pane
/// and `corral eval`, else the best match (pick_session).
fn print_json(lang: Option<&str>) -> Result<(), String> {
    let ws = env::current_workspace();
    let session = match lang {
        Some(l) => console::session(l, ws.as_deref()).ok().map(|(_, s)| s),
        None => {
            let in_view = env::active_lang(ws.as_deref(), &herdr::panes(ws.as_deref()).unwrap_or_default());
            console::session(&in_view, ws.as_deref()).ok().map(|(_, s)| s).or_else(console::pick_session)
        }
    };
    let Some(session) = session else {
        return Err(match lang {
            Some(l) => {
                let name = console::lang_name(l);
                trf!("there is no {name} session with Corral", "no hay una sesión de {name} con Corral")
            }
            None => tr("there is no R or Python session with Corral", "no hay una sesión de R ni de Python con Corral").to_string(),
        });
    };
    let dir = PathBuf::from(session.get("dir").str().unwrap_or(""));
    let data = std::fs::read_to_string(dir.join("vars.json")).ok().and_then(|t| crate::json::parse(&t).ok())
        .ok_or(tr("this session doesn't publish variables (is jsonlite missing?)", "esta sesión no publica variables (¿falta jsonlite?)"))?;
    // Restricted output (a sensitive session, or agent_output structure): no previews of
    // values, which can be the data itself; dimensions and signatures stay.
    // review counts too: these previews would reach the agent without the user seeing them.
    let rules = crate::profile::session_rules(&dir, &session);
    let restricted = rules.sensitive || matches!(rules.output, "structure" | "review");
    let mut vars = data.get("vars").clone();
    if restricted {
        if let Value::Arr(items) = &mut vars {
            for v in items.iter_mut() {
                let keep = matches!(v.get("kind").str(), Some("data") | Some("function"));
                if !keep {
                    v.set("preview", Value::Null);
                }
            }
        }
    }
    let out = obj(vec![("session", session.get("id").clone()),
                       ("lang", session.get("lang").str().unwrap_or("r").into()),
                       ("console", session.get("pane_id").clone()),
                       ("restricted", restricted.into()),
                       ("vars", vars)]);
    print!("{}", out.to_json_pretty(1));
    Ok(())
}

/// The permanent variables pane under the R console, like Positron's Variables view.
pub fn split_pane() -> Result<String, String> {
    let ws = env::current_workspace();
    let console = console::find("r", ws.as_deref())
        .ok_or(tr("there is no corral:r console in this workspace", "no hay consola corral:r en este workspace"))?;
    if let Some(p) = herdr::labeled(&herdr::panes(ws.as_deref())?, "corral:vars") {
        let id = herdr::pane_id(p).to_string();
        env::notify("Variables", &trf!("already open in {id}", "ya está abierto en {id}"));
        return Ok(id);
    }
    split_under(&console, 0.62)
}

/// A variables pane under `pane`, which keeps `ratio` of the height.
pub fn split_under(pane: &str, ratio: f64) -> Result<String, String> {
    let new = herdr::split(pane, "down", Some(ratio), None)?;
    herdr::rename(&new, "corral:vars")?;
    herdr::start(&new, &format!("{} vars --pane", console::program()))?;
    Ok(new)
}

/// Open one of the plugin's popups. Retries while another modal is still closing.
pub fn open_popup(entrypoint: &str, env_vars: &[(&str, &str)]) -> Result<(), String> {
    let pairs: Vec<String> = env_vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let mut args = vec!["plugin", "pane", "open", "--plugin", "corral", "--entrypoint", entrypoint, "--placement", "popup", "--focus"];
    for p in &pairs {
        args.extend(["--env", p.as_str()]);
    }
    let mut last = String::new();
    for _ in 0..8 {
        match herdr::cli(&args) {
            Ok(_) => return Ok(()),
            Err(e) if e.to_lowercase().contains("busy") => {
                last = e;
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}
