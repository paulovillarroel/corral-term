//! `corral _table FILE` (the table popup for View()): a table viewer that
//! asks DuckDB only for what is on screen. A page of rows, the count, a sort
//! or a search are single queries on the file, so a table of millions of rows
//! opens at once and costs no memory while you look at it.

use crate::env;
use crate::i18n::{thousands, tr};
use crate::json::Value;
use crate::mirador::{duck, sql_ident, sql_lit, table_source};
use crate::term::{clip, out, width, Screen};
use crate::vars::{message_screen, Colors};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

const PAGE: usize = 200;
const TOP: usize = 5; // first row of data on screen
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

struct Col {
    name: String,
    ty: String,
    numeric: bool,
}

impl AsRef<str> for Col {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

fn numeric(ty: &str) -> bool {
    let t = ty.to_uppercase();
    ["INT", "DOUBLE", "FLOAT", "DECIMAL", "REAL", "NUMERIC"].iter().any(|k| t.contains(k)) && !t.contains("INTERVAL")
}

/// A cell as text: NA for missing values, numbers without noise.
fn fmt(v: &Value) -> String {
    match v {
        Value::Null => "NA".into(),
        Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
        Value::Num(n) if n.abs() >= 1e15 || (n.abs() < 1e-4 && *n != 0.0) => format!("{n:.4e}"),
        Value::Num(n) => {
            let s = format!("{n:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Value::Str(s) => s.replace(['\n', '\r', '\t'], " "),
        other => other.to_json(),
    }
}

enum Mode {
    Data,
    Row(usize),
    Summary(Vec<Col>, Vec<Vec<Value>>, usize),
}

struct Viewer {
    src: String,
    path: String,
    title: String,
    cols: Vec<Col>,
    all: usize,
    total: usize,
    filter: String,
    /// Column filters: (what you typed, its SQL), all applied together.
    conditions: Vec<(String, String)>,
    /// A prompt being typed: (is it a filter?, text).
    typing: Option<(bool, String)>,
    sort: Option<(usize, bool)>,
    cache_start: usize,
    cache: Vec<Vec<Value>>,
    widths: Vec<usize>,
    row: usize,
    col: usize,
    top: usize,
    first_col: usize,
    mode: Mode,
    status: String,
    colors: Colors,
    header_spans: Vec<(usize, usize, usize)>, // x from, x to, column
}

impl Viewer {
    fn open(path: &str, title: &str) -> Result<Viewer, String> {
        let src = table_source(path);
        let cols: Vec<Col> = duck(&format!("DESCRIBE SELECT * FROM {src}"))?.arr().iter().map(|c| {
            let ty = c.get("column_type").str().unwrap_or("").to_string();
            Col { name: c.get("column_name").str().unwrap_or("").to_string(), numeric: numeric(&ty), ty }
        }).collect();
        let all = count(&format!("SELECT count(*) AS n FROM {src}"))?;
        let mut v = Viewer {
            src, path: path.into(), title: title.into(), cols, all, total: all, filter: String::new(), conditions: Vec::new(), typing: None, sort: None,
            cache_start: 0, cache: Vec::new(), widths: Vec::new(), row: 0, col: 0, top: 0, first_col: 0,
            mode: Mode::Data, status: String::new(), colors: Colors::load(), header_spans: Vec::new(),
        };
        v.fetch(0)?;
        Ok(v)
    }

    /// A query's error, or what happened when the file itself is gone.
    fn why(&self, error: String) -> String {
        if Path::new(&self.path).exists() {
            return error;
        }
        tr("This table's file is gone: a newer View() replaced it, or its session ended. q closes the viewer.",
           "El archivo de esta tabla ya no está: lo reemplazó un View() más nuevo, o terminó su sesión. q cierra el visor.").into()
    }

    fn where_clause(&self) -> String {
        where_sql(&self.conditions, &self.filter, &self.cols)
    }

    fn filtered(&self) -> bool {
        !self.filter.is_empty() || !self.conditions.is_empty()
    }

    /// Add a filter; if DuckDB rejects it, leave things as they were and say why.
    fn add_condition(&mut self, label: String, sql: String) {
        self.conditions.push((label, sql));
        let check = format!("SELECT count(*) AS n FROM {}{}", self.src, self.where_clause());
        if let Err(e) = count(&check) {
            self.conditions.pop();
            self.status = invalid_filter(&e);
            return;
        }
        self.requery();
    }

    fn order_clause(&self) -> String {
        match self.sort {
            Some((c, desc)) => order_sql(&self.cols[c].name, desc),
            None => String::new(),
        }
    }

    fn fetch(&mut self, offset: usize) -> Result<(), String> {
        let sql = format!("SELECT * FROM {}{}{} LIMIT {PAGE} OFFSET {offset}", self.src, self.where_clause(), self.order_clause());
        let rows = duck(&sql)?;
        self.cache = rows.arr().iter().map(|r| self.cols.iter().map(|c| r.get(&c.name).clone()).collect()).collect();
        self.cache_start = offset;
        // Widths from the names and this page's values.
        self.widths = self.cols.iter().enumerate().map(|(i, c)| {
            let longest = self.cache.iter().map(|r| width(&fmt(&r[i]))).max().unwrap_or(0);
            longest.max(width(&c.name) + 1).clamp(3, 32)
        }).collect();
        Ok(())
    }

    /// Make sure rows top..top+area are in the cache.
    fn ensure(&mut self, top: usize, area: usize) {
        let end = (top + area).min(self.total);
        if top >= self.cache_start && end <= self.cache_start + self.cache.len() {
            return;
        }
        let start = top.saturating_sub(PAGE / 4);
        if let Err(e) = self.fetch(start) {
            self.status = self.why(e);
        }
    }

    fn requery(&mut self) {
        let sql = format!("SELECT count(*) AS n FROM {}{}", self.src, self.where_clause());
        match count(&sql).and_then(|n| {
            self.total = n;
            self.fetch(0)
        }) {
            Ok(_) => {}
            Err(e) => self.status = self.why(e),
        }
        self.row = 0;
        self.top = 0;
    }

    fn render(&mut self, scr: &Screen) {
        let (cols, rows) = scr.size();
        let area = rows.saturating_sub(TOP + 1);
        // The body first: it may ask DuckDB for rows.
        let body = match self.mode {
            Mode::Data => {
                if self.row < self.top {
                    self.top = self.row;
                } else if self.row >= self.top + area {
                    self.top = self.row + 1 - area;
                }
                self.ensure(self.top, area);
                self.grid(cols, area)
            }
            Mode::Row(i) => {
                self.ensure(i, 1);
                self.row_view(i, cols, area)
            }
            Mode::Summary(..) => self.summary_view(cols, area),
        };
        let c = &self.colors;
        let mut f = String::from("\x1b[2J\x1b[H");
        let (all, ncols) = (thousands(self.all as u64), self.cols.len());
        let rows_word = if self.all == 1 { tr("row", "fila") } else { tr("rows", "filas") };
        let cols_word = if ncols == 1 { tr("column", "columna") } else { tr("columns", "columnas") };
        let count = if !self.filtered() {
            format!("{all} {rows_word} · {ncols} {cols_word}")
        } else {
            let shown = thousands(self.total as u64);
            trf!("{shown} of {all} {rows_word} · {ncols} {cols_word}", "{shown} de {all} {rows_word} · {ncols} {cols_word}")
        };
        f += &format!("\x1b[1;2H{}▸▸{RESET} {}{BOLD}{}{RESET}", c.accent, c.text, clip(&self.title, cols.saturating_sub(width(&count) + 8)));
        f += &format!("\x1b[1;{}H{}{count}{RESET}", cols.saturating_sub(width(&count)).max(1), c.muted);
        let mut info: Vec<String> = self.conditions.iter().map(|(label, _)| trf!("filter: {label}", "filtro: {label}")).collect();
        if !self.filter.is_empty() {
            info.push(trf!("searching “{}”", "buscando «{}»", self.filter));
        }
        if let Some((i, desc)) = self.sort {
            info.push(trf!("sort: {} {}", "orden: {} {}", self.cols[i].name, if desc { "▼" } else { "▲" }));
        }
        f += &format!("\x1b[2;2H{}{}{RESET}", c.dim, clip(&info.join(" · "), cols.saturating_sub(3)));
        f += &body;
        let hint = if let Some((is_filter, q)) = &self.typing {
            if *is_filter {
                trf!("filter: {q}▏  Enter applies · Esc cancels · e.g. mpg > 20 & cyl == 6, region %in% c(\"a\", \"b\")",
                     "filtro: {q}▏  Enter aplica · Esc cancela · p. ej. mpg > 20 & cyl == 6, region %in% c(\"a\", \"b\")")
            } else {
                trf!("/{q}▏  Enter to search · Esc cancels", "/{q}▏  Enter para buscar · Esc cancela")
            }
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            match self.mode {
                Mode::Data => tr(
                    "/ search · f filter · = equal to the cell · x clear filters · s sort · Enter row · i summary · m Mirador · Shift+wheel columns · q close",
                    "/ buscar · f filtrar · = igual a la celda · x quitar filtros · s ordenar · Enter fila · i resumen · m Mirador · Shift+rueda columnas · q cerrar",
                ).into(),
                _ => tr("↑↓ move · Esc back · q close", "↑↓ mover · Esc volver · q cerrar").into(),
            }
        };
        f += &format!("\x1b[{rows};1H\x1b[2K {}{}{RESET}", c.dim, clip(&hint, cols.saturating_sub(2)));
        out(&f);
    }

    fn row_view(&self, i: usize, cols: usize, area: usize) -> String {
        let c = &self.colors;
        let values = self.cache.get(i.wrapping_sub(self.cache_start)).cloned().unwrap_or_default();
        let name_w = self.cols.iter().map(|c| width(&c.name)).max().unwrap_or(4).min(30);
        let mut f = format!("\x1b[3;2H{}{} {}{RESET}", c.muted, tr("Row", "Fila"), thousands(i as u64 + 1));
        for (n, (col, v)) in self.cols.iter().zip(values.iter()).enumerate().skip(self.top).take(area) {
            let y = TOP + 1 + n - self.top;
            let value = match v { Value::Num(x) => format!("{x}"), other => fmt(other) };
            f += &format!("\x1b[{y};3H{}{BOLD}{:<name_w$}{RESET}  {}{}{RESET}  {}{}{RESET}", c.text, clip(&col.name, name_w),
                          if matches!(v, Value::Null) { &c.dim } else { &c.text }, clip(&value, cols.saturating_sub(name_w + 20)),
                          c.dim, clip(&col.ty, 14));
        }
        f
    }

    fn summary_view(&self, cols: usize, area: usize) -> String {
        let c = &self.colors;
        let Mode::Summary(scols, srows, top) = &self.mode else { return String::new() };
        let widths: Vec<usize> = (0..scols.len()).map(|i| {
            srows.iter().map(|r| width(&fmt(&r[i]))).max().unwrap_or(0).max(width(&scols[i].name)).clamp(3, 24)
        }).collect();
        let mut f = format!("\x1b[3;2H{}{}{RESET}", c.muted, tr("DuckDB's summary (SUMMARIZE)", "Resumen de DuckDB (SUMMARIZE)"));
        let mut x = 2;
        let mut header = String::new();
        for (i, col) in scols.iter().enumerate() {
            if x + widths[i] > cols {
                break;
            }
            header += &format!("{:<w$} ", clip(&col.name, widths[i]), w = widths[i]);
            x += widths[i] + 1;
        }
        f += &format!("\x1b[{};2H{}{BOLD}{header}{RESET}", TOP, c.text);
        for (n, r) in srows.iter().enumerate().skip(*top).take(area) {
            let mut line = String::new();
            let mut x = 2;
            for (i, v) in r.iter().enumerate() {
                if x + widths[i] > cols {
                    break;
                }
                let t = clip(&fmt(v), widths[i]);
                line += &if scols[i].numeric { format!("{:>w$} ", t, w = widths[i]) } else { format!("{:<w$} ", t, w = widths[i]) };
                x += widths[i] + 1;
            }
            f += &format!("\x1b[{};2H{}{line}{RESET}", TOP + 1 + n - top, c.text);
        }
        f
    }

    fn grid(&mut self, cols: usize, area: usize) -> String {
        let c = &self.colors;
        let gutter = format!("{}", self.total.max(1)).len() + 1;
        // Keep the current column in view.
        if self.col < self.first_col {
            self.first_col = self.col;
        }
        let fits = |from: usize, to: usize, widths: &[usize]| gutter + 1 + widths[from..=to].iter().map(|w| w + 2).sum::<usize>() <= cols;
        while self.first_col < self.col && !fits(self.first_col, self.col, &self.widths) {
            self.first_col += 1;
        }
        let mut shown = Vec::new();
        let mut x = gutter + 2;
        for i in self.first_col..self.cols.len() {
            if x + self.widths[i] > cols && !shown.is_empty() {
                break;
            }
            shown.push(i);
            x += self.widths[i] + 2;
        }
        let mut f = String::new();
        let mut header = format!("{:>gutter$} ", "");
        let mut x = gutter + 2;
        self.header_spans.clear();
        for &i in &shown {
            let w = self.widths[i];
            let mark = match self.sort { Some((s, true)) if s == i => "▼", Some((s, false)) if s == i => "▲", _ => "" };
            let name = clip(&format!("{}{mark}", self.cols[i].name), w);
            let cell = if self.cols[i].numeric { format!("{name:>w$}") } else { format!("{name:<w$}") };
            header += &if i == self.col { format!("{}{}{BOLD}{cell}{RESET}  ", c.sel_bg, c.accent) } else { format!("{}{BOLD}{cell}{RESET}  ", c.text) };
            self.header_spans.push((x, x + w, i));
            x += w + 2;
        }
        f += &format!("\x1b[3;1H{header}");
        let types: String = shown.iter().map(|&i| {
            let w = self.widths[i];
            let t = clip(&self.cols[i].ty.to_lowercase(), w);
            if self.cols[i].numeric { format!("{t:>w$}  ") } else { format!("{t:<w$}  ") }
        }).collect();
        f += &format!("\x1b[4;{}H{}{types}{RESET}", gutter + 2, c.dim);
        for n in 0..area {
            let r = self.top + n;
            if r >= self.total {
                break;
            }
            let Some(values) = self.cache.get(r.wrapping_sub(self.cache_start)) else { break };
            let selected = r == self.row;
            let mut line = format!("{}{:>gutter$}{RESET} ", c.dim, r + 1);
            for &i in &shown {
                let w = self.widths[i];
                let t = clip(&fmt(&values[i]), w);
                let cell = if self.cols[i].numeric { format!("{t:>w$}") } else { format!("{t:<w$}") };
                let color = if matches!(values[i], Value::Null) { &c.dim } else if selected && i == self.col { &c.accent } else { &c.text };
                line += &format!("{color}{cell}{RESET}{}  ", if selected { c.sel_bg.as_str() } else { "" });
            }
            let y = TOP + 1 + n;
            if selected {
                f += &format!("\x1b[{y};1H{}\x1b[2K{line}{}\x1b[K{RESET}", c.sel_bg, c.sel_bg);
            } else {
                f += &format!("\x1b[{y};1H{line}");
            }
        }
        if self.total == 0 {
            f += &format!("\x1b[{};4H{}{}{RESET}", TOP + 2, c.muted, tr("No rows match.", "Ninguna fila coincide."));
        }
        f
    }

    fn summary(&mut self) {
        match duck(&format!("SUMMARIZE SELECT * FROM {}", self.src)) {
            Ok(rows) => {
                let first = rows.arr().first().cloned().unwrap_or(Value::obj());
                let names: Vec<String> = match &first { Value::Obj(items) => items.iter().map(|(k, _)| k.clone()).collect(), _ => Vec::new() };
                let scols: Vec<Col> = names.iter().map(|n| Col {
                    name: n.clone(), ty: String::new(),
                    numeric: ["approx_unique", "avg", "std", "q25", "q50", "q75", "count", "null_percentage"].contains(&n.as_str()),
                }).collect();
                let srows = rows.arr().iter().map(|r| names.iter().map(|n| r.get(n).clone()).collect()).collect();
                self.mode = Mode::Summary(scols, srows, 0);
            }
            Err(e) => self.status = self.why(e),
        }
    }
}

// --------------------------------------------------------------------------- shared with Mirador's table view

/// The WHERE clause for the filters (their SQL, all of them) and a search for a
/// text in every column; empty when there is neither.
pub(crate) fn where_sql<C: AsRef<str>>(conditions: &[(String, String)], search: &str, cols: &[C]) -> String {
    let mut parts: Vec<String> = conditions.iter().map(|(_, sql)| format!("({sql})")).collect();
    let q = search.trim();
    if !q.is_empty() {
        let pattern = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let any: Vec<String> = cols.iter()
            .map(|c| format!("CAST({} AS VARCHAR) ILIKE {} ESCAPE '\\'", sql_ident(c.as_ref()), sql_lit(&pattern))).collect();
        parts.push(format!("({})", any.join(" OR ")));
    }
    if parts.is_empty() { String::new() } else { format!(" WHERE {}", parts.join(" AND ")) }
}

/// ORDER BY one column, missing values last.
pub(crate) fn order_sql(name: &str, desc: bool) -> String {
    format!(" ORDER BY {} {} NULLS LAST", sql_ident(name), if desc { "DESC" } else { "ASC" })
}

/// What DuckDB said about a filter it rejected, as the viewer shows it.
pub(crate) fn invalid_filter(error: &str) -> String {
    let why = error.lines().next().unwrap_or(error);
    trf!("invalid filter: {why}", "filtro no válido: {why}")
}

/// The filter that keeps the rows whose column `name` has the value `cell` (the
/// `=` key): what it shows (R style) and its SQL.
pub(crate) fn equal_condition(name: &str, cell: &Value) -> (String, String) {
    match cell {
        Value::Null => (format!("is.na({name})"), format!("{} IS NULL", sql_ident(name))),
        Value::Num(n) => (format!("{name} == {}", fmt(cell)), format!("{} = {n}", sql_ident(name))),
        Value::Bool(b) => (format!("{name} == {}", fmt(cell)), format!("{} = {b}", sql_ident(name))),
        Value::Str(t) => (format!("{name} == \"{t}\""), format!("CAST({} AS VARCHAR) = {}", sql_ident(name), sql_lit(t))),
        other => (format!("{name} == {}", other.to_json()), format!("CAST({} AS VARCHAR) = {}", sql_ident(name), sql_lit(&other.to_json()))),
    }
}

/// A column name as a filter starts with it: as is, or in `backticks`.
pub(crate) fn filter_name(name: &str) -> String {
    if plain(name) { name.to_string() } else { format!("`{name}`") }
}

/// A plain SQL identifier (no quoting needed).
fn plain(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A filter written the R way or the SQL way, as DuckDB SQL: `&` `|` `!` `%in% c(...)`,
/// `is.na(x)`, "text" in double quotes; column names with dots (or `backticks`) get quoted.
pub(crate) fn translate<C: AsRef<str>>(expr: &str, cols: &[C]) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        if c == '"' || c == '`' {
            // "text" is a value in R, unless it names a column; `name` is always a column.
            let end = chars[i + 1..].iter().position(|&x| x == c).map(|p| i + 1 + p).unwrap_or(chars.len());
            let inner: String = chars[i + 1..end.min(chars.len())].iter().collect();
            if c == '`' || cols.iter().any(|col| col.as_ref() == inner) {
                out += &sql_ident(&inner);
            } else {
                out += &sql_lit(&inner);
            }
            i = end + 1;
        } else if c == '\'' {
            let end = chars[i + 1..].iter().position(|&x| x == '\'').map(|p| i + 1 + p).unwrap_or(chars.len() - 1);
            out.extend(&chars[i..=end.min(chars.len() - 1)]);
            i = end + 1;
        } else if rest.starts_with("&&") || rest.starts_with("||") {
            out += if c == '&' { " AND " } else { " OR " };
            i += 2;
        } else if c == '&' || c == '|' {
            out += if c == '&' { " AND " } else { " OR " };
            i += 1;
        } else if c == '!' && chars.get(i + 1) != Some(&'=') {
            out += " NOT ";
            i += 1;
        } else if rest.starts_with("%in%") {
            out += " IN ";
            i += 4;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if chars.get(i) == Some(&'c') && chars.get(i + 1) == Some(&'(') {
                i += 1; // c( -> (
            }
        } else if rest.starts_with("is.na(") {
            let close = chars[i + 6..].iter().position(|&x| x == ')').map(|p| i + 6 + p).unwrap_or(chars.len());
            let inner: String = chars[i + 6..close.min(chars.len())].iter().collect();
            out += &format!("({} IS NULL)", translate(&inner, cols).trim());
            i = close + 1;
        } else if is_word(c) && (i == 0 || !is_word(chars[i - 1])) {
            let end = chars[i..].iter().position(|&x| !is_word(x)).map(|p| i + p).unwrap_or(chars.len());
            let word: String = chars[i..end].iter().collect();
            match word.as_str() {
                w if cols.iter().any(|col| col.as_ref() == w) && !plain(w) => out += &sql_ident(w),
                "NA" | "NULL" => out += "NULL",
                _ => out += &word,
            }
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.replace("NOT  (", "NOT (")
        .replace("!= NULL", "IS NOT NULL")
        .replace("== NULL", "IS NULL")
        .replace("= NULL", "IS NULL")
}

pub(crate) fn count(sql: &str) -> Result<usize, String> {
    Ok(duck(sql)?.arr().first().and_then(|r| r.get("n").num()).unwrap_or(0.0) as usize)
}

pub fn run(args: &[String], _all: &[String]) -> Result<(), String> {
    let file = args.first().cloned().or_else(|| std::env::var("CORRAL_FILE").ok()).ok_or(tr("the file is missing (an argument or CORRAL_FILE)", "falta el archivo (argumento o CORRAL_FILE)"))?;
    // Your own viewer, or VisiData if you prefer it (corral config table_viewer visidata).
    if let Ok(custom) = std::env::var("CORRAL_TABLE_VIEWER") {
        let argv: Vec<String> = custom.split_whitespace().map(|a| a.replace("{file}", &file)).collect();
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        if !custom.contains("{file}") {
            cmd.arg(&file);
        }
        return Err(format!("{}", cmd.exec()));
    }
    if env::config().get("table_viewer").str() == Some("visidata") && env::which("vd").is_some() {
        let err = Command::new("vd").arg("--config").arg(env::repo().join("lib/corral.visidatarc")).arg(&file).exec();
        return Err(err.to_string());
    }
    if env::which("duckdb").is_none() {
        message_screen(tr("DuckDB is missing", "Falta DuckDB"),
                       tr("The table viewer reads the file with DuckDB.\nInstall it, or use `corral config table_viewer visidata`.",
                          "El visor de tablas consulta el archivo con DuckDB.\nInstálalo o usa `corral config table_viewer visidata`."));
        return Ok(());
    }
    let title = std::env::var("CORRAL_TITLE").ok().filter(|t| !t.is_empty())
        .unwrap_or_else(|| file.rsplit('/').next().unwrap_or(&file).to_string());
    let mut v = match Viewer::open(&file, &title) {
        Ok(v) => v,
        Err(e) => {
            message_screen(tr("Couldn't open the table", "No pude abrir la tabla"), &e);
            return Ok(());
        }
    };
    let Some(scr) = Screen::enter(true) else { return Err(tr("not a terminal", "no hay una terminal").into()) };
    let mut size = scr.size();
    v.render(&scr);
    let mut spanish = crate::i18n::es();
    let mut dirty = false;
    loop {
        // Draw once the input that already arrived is handled: a burst of wheel
        // events or key repeat must not queue a redraw (and a query) for each one.
        if dirty && !scr.has_input() {
            v.render(&scr);
            dirty = false;
        }
        let key = scr.key(500);
        if scr.size() != size {
            size = scr.size();
            v.render(&scr);
        }
        let Some(key) = key else {
            // The language switched (corral config language): draw in the new one.
            if crate::i18n::es() != spanish {
                spanish = !spanish;
                v.render(&scr);
            }
            continue;
        };
        let (_, rows) = scr.size();
        let area = rows.saturating_sub(TOP + 1).max(1);
        v.status.clear();

        if v.typing.is_some() {
            match key.as_str() {
                "enter" => {
                    let (is_filter, text) = v.typing.take().unwrap_or_default();
                    if is_filter {
                        let text = text.trim().to_string();
                        if !text.is_empty() {
                            let sql = translate(&text, &v.cols);
                            v.add_condition(text, sql);
                        }
                    } else {
                        v.filter = text;
                        v.requery();
                    }
                }
                "esc" => v.typing = None,
                "backspace" => {
                    if let Some((_, q)) = v.typing.as_mut() {
                        q.pop();
                    }
                }
                k if k.chars().count() == 1 && !k.chars().next().unwrap().is_control() => {
                    if let Some((_, q)) = v.typing.as_mut() {
                        q.push_str(k);
                    }
                }
                _ => {}
            }
            dirty = true;
            continue;
        }

        match &mut v.mode {
            Mode::Row(_) | Mode::Summary(..) => {
                match key.as_str() {
                    "q" | "quit" => return Ok(()),
                    "esc" | "enter" | "i" | "left" | "h" => {
                        v.mode = Mode::Data;
                        v.top = v.row.saturating_sub(area / 2);
                    }
                    "down" | "j" => {
                        if let Mode::Summary(_, rows, top) = &mut v.mode { *top = (*top + 1).min(rows.len().saturating_sub(1)); } else { v.top += 1; }
                    }
                    "up" | "k" => {
                        if let Mode::Summary(_, _, top) = &mut v.mode { *top = top.saturating_sub(1); } else { v.top = v.top.saturating_sub(1); }
                    }
                    _ => {}
                }
                dirty = true;
                continue;
            }
            Mode::Data => {}
        }

        let last = v.total.saturating_sub(1);
        if let Some(rest) = key.strip_prefix("mouse:") {
            let nums: Vec<usize> = rest.split(':').filter_map(|x| x.parse().ok()).collect();
            if let [button, mx, my] = nums[..] {
                let last_col = v.cols.len().saturating_sub(1);
                match button {
                    64 => v.row = v.row.saturating_sub(3),
                    65 => v.row = (v.row + 3).min(last),
                    // Sideways: the horizontal wheel or a trackpad swipe (66, 67), or Shift + wheel (68, 69).
                    66 | 68 => v.col = v.col.saturating_sub(1),
                    67 | 69 => v.col = (v.col + 1).min(last_col),
                    0 if my == 3 => {
                        if let Some(&(_, _, i)) = v.header_spans.iter().find(|(a, b, _)| *a <= mx && mx < *b) {
                            v.col = i;
                            v.sort = match v.sort { Some((s, false)) if s == i => Some((i, true)), Some((s, true)) if s == i => None, _ => Some((i, false)) };
                            v.requery();
                        }
                    }
                    0 if my > TOP => {
                        let r = v.top + my - TOP - 1;
                        if r < v.total {
                            v.row = r;
                        }
                        if let Some(&(_, _, i)) = v.header_spans.iter().find(|(a, b, _)| *a <= mx && mx < *b) {
                            v.col = i;
                        }
                    }
                    _ => {}
                }
            }
            dirty = true;
            continue;
        }
        match key.as_str() {
            "q" | "quit" | "esc" => return Ok(()),
            "down" | "j" => v.row = (v.row + 1).min(last),
            "up" | "k" => v.row = v.row.saturating_sub(1),
            "pgdn" | " " => v.row = (v.row + area).min(last),
            "pgup" => v.row = v.row.saturating_sub(area),
            "home" | "g" => v.row = 0,
            "end" | "G" => v.row = last,
            "right" | "l" => v.col = (v.col + 1).min(v.cols.len().saturating_sub(1)),
            "left" | "h" => v.col = v.col.saturating_sub(1),
            "0" => v.col = 0,
            "$" => v.col = v.cols.len().saturating_sub(1),
            "s" => {
                let i = v.col;
                v.sort = match v.sort { Some((s, false)) if s == i => Some((i, true)), Some((s, true)) if s == i => None, _ => Some((i, false)) };
                v.requery();
            }
            "/" => v.typing = Some((false, String::new())), // an empty search clears it
            "f" => {
                // Start from the current column's name.
                v.typing = Some((true, format!("{} ", filter_name(&v.cols[v.col].name))));
            }
            "=" if v.total > 0 => {
                // Keep the rows with the value of the selected cell.
                v.ensure(v.row, 1);
                let cell = v.cache.get(v.row.wrapping_sub(v.cache_start)).map(|r| r[v.col].clone()).unwrap_or(Value::Null);
                let (label, sql) = equal_condition(&v.cols[v.col].name, &cell);
                v.add_condition(label, sql);
            }
            "x" => {
                v.conditions.clear();
                v.filter.clear();
                v.requery();
            }
            "enter" if v.total > 0 => {
                v.mode = Mode::Row(v.row);
                v.top = 0;
            }
            "i" => v.summary(),
            "m" => {
                // The same file in Mirador, in the browser; this viewer stays open.
                let path = std::fs::canonicalize(&file).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| file.clone());
                let session = std::env::var("CORRAL_SESSION").ok();
                v.status = tr("opening in Mirador…", "abriendo en Mirador…").into();
                v.render(&scr);
                v.status = match crate::plots::table_to_mirador(&path, &title, session.as_deref()) {
                    Ok(()) => tr("open in Mirador, in the browser · q closes this viewer", "abierta en Mirador, en el navegador · q cierra este visor").into(),
                    Err(e) => format!("Mirador: {e}"),
                };
            }
            _ => {}
        }
        dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<Col> {
        ["mpg", "cyl", "region", "activo", "Sepal.Length", "x"].iter()
            .map(|n| Col { name: n.to_string(), ty: String::new(), numeric: false }).collect()
    }

    fn sql(e: &str) -> String {
        translate(e, &cols()).split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn r_style_filters() {
        assert_eq!(sql("mpg > 20 & cyl == 6"), "mpg > 20 AND cyl == 6");
        assert_eq!(sql("cyl == 4 | cyl == 8"), "cyl == 4 OR cyl == 8");
        assert_eq!(sql("region %in% c(\"a\", \"b\")"), "region IN ('a', 'b')");
        assert_eq!(sql("is.na(x)"), "(x IS NULL)");
        assert_eq!(sql("!is.na(x)"), "NOT (x IS NULL)");
        assert_eq!(sql("!activo"), "NOT activo");
        assert_eq!(sql("Sepal.Length > 5"), "\"Sepal.Length\" > 5");
        assert_eq!(sql("region == \"norte\""), "region == 'norte'");
        assert_eq!(sql("`Sepal.Length` >= 5.5"), "\"Sepal.Length\" >= 5.5");
        assert_eq!(sql("x != NA"), "x IS NOT NULL");
        assert_eq!(sql("x == NA"), "x IS NULL");
    }

    #[test]
    fn equal_to_the_cell() {
        assert_eq!(equal_condition("region", &Value::Str("North".into())),
                   ("region == \"North\"".into(), "CAST(\"region\" AS VARCHAR) = 'North'".into()));
        assert_eq!(equal_condition("mpg", &Value::Num(21.0)).0, "mpg == 21");
        assert_eq!(equal_condition("x", &Value::Null), ("is.na(x)".into(), "\"x\" IS NULL".into()));
        assert_eq!(filter_name("Sepal.Length"), "`Sepal.Length`");
        assert_eq!(filter_name("mpg"), "mpg");
        let names = ["mpg".to_string(), "cyl".to_string()];
        assert_eq!(where_sql(&[("mpg > 20".into(), "mpg > 20".into())], "", &names), " WHERE (mpg > 20)");
        assert_eq!(translate("mpg > 20 & cyl == 6", &names), "mpg > 20  AND  cyl == 6");
    }

    #[test]
    fn sql_filters_pass_through() {
        assert_eq!(sql("region LIKE 'nor%' AND mpg BETWEEN 10 AND 20"), "region LIKE 'nor%' AND mpg BETWEEN 10 AND 20");
        assert_eq!(sql("x = NULL"), "x IS NULL");
    }
}
