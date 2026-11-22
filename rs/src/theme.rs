//! One palette for every Corral window, taken from herdr's theme, and
//! `corral theme` / `corral config`.
//!
//! herdr is the source of truth: its `[theme] name` picks a base palette and its
//! `[theme.custom]` colours override it. Corral's own themes (`corral config theme
//! dark-2026`) take over until another theme is chosen in herdr. From the palette
//! Corral writes ~/.config/corral/palette.json (the terminal viewers, VisiData
//! and Mirador read it) and ~/.config/fresh/themes/corral.json (the editor; its
//! background is left to the terminal). herdr does not publish its theme colours,
//! so the base palettes below follow each theme's published colours.

use crate::i18n::tr;
use crate::env;
use crate::json::{self, obj, Value};
use crate::toml;
use std::path::PathBuf;
use std::process::Command;
use std::time::UNIX_EPOCH;

const KEYS: [&str; 16] = ["bg", "surface", "raised", "selection", "border", "text", "muted", "dim", "accent", "green",
                          "yellow", "red", "blue", "violet", "orange", "cyan"];

/// bg: page background (Mirador); surface: bars, menus; raised: current line, hover;
/// selection; border; text; muted; dim; accent; then the named colours (order of KEYS).
const PALETTES: &[(&str, [&str; 16])] = &[
    ("vesper", ["#101010", "#161616", "#1c1c1c", "#343434", "#282828", "#ffffff", "#a0a0a0", "#5c5c5c", "#ffc799",
                "#99ffe4", "#ffc799", "#ff8080", "#a0a0a0", "#a0a0a0", "#ffc799", "#99ffe4"]),
    ("dracula", ["#282a36", "#21222c", "#343746", "#44475a", "#343746", "#f8f8f2", "#bfbfbf", "#6272a4", "#bd93f9",
                 "#50fa7b", "#f1fa8c", "#ff5555", "#8be9fd", "#ff79c6", "#ffb86c", "#8be9fd"]),
    ("catppuccin", ["#1e1e2e", "#181825", "#313244", "#45475a", "#313244", "#cdd6f4", "#a6adc8", "#6c7086", "#f5c2e7",
                    "#a6e3a1", "#f9e2af", "#f38ba8", "#89b4fa", "#cba6f7", "#fab387", "#94e2d5"]),
    ("tokyo-night", ["#1a1b26", "#16161e", "#292e42", "#33467c", "#29293a", "#c0caf5", "#a9b1d6", "#565f89", "#7aa2f7",
                     "#9ece6a", "#e0af68", "#f7768e", "#7aa2f7", "#bb9af7", "#ff9e64", "#7dcfff"]),
    ("nord", ["#2e3440", "#272c36", "#3b4252", "#434c5e", "#3b4252", "#eceff4", "#d8dee9", "#616e88", "#88c0d0",
              "#a3be8c", "#ebcb8b", "#bf616a", "#81a1c1", "#b48ead", "#d08770", "#8fbcbb"]),
    ("gruvbox", ["#282828", "#1d2021", "#3c3836", "#504945", "#3c3836", "#ebdbb2", "#bdae93", "#7c6f64", "#fe8019",
                 "#b8bb26", "#fabd2f", "#fb4934", "#83a598", "#d3869b", "#fe8019", "#8ec07c"]),
    ("one-dark", ["#282c34", "#21252b", "#2c313a", "#3e4451", "#3e4451", "#abb2bf", "#9da5b4", "#5c6370", "#61afef",
                  "#98c379", "#e5c07b", "#e06c75", "#61afef", "#c678dd", "#d19a66", "#56b6c2"]),
    ("solarized", ["#002b36", "#00212b", "#073642", "#0a4a5c", "#073642", "#93a1a1", "#839496", "#586e75", "#b58900",
                   "#859900", "#b58900", "#dc322f", "#268bd2", "#6c71c4", "#cb4b16", "#2aa198"]),
    ("kanagawa", ["#1f1f28", "#16161d", "#2a2a37", "#2d4f67", "#2a2a37", "#dcd7ba", "#c8c093", "#727169", "#7e9cd8",
                  "#98bb6c", "#e6c384", "#ff5d62", "#7e9cd8", "#957fb8", "#ffa066", "#7aa89f"]),
    ("rose-pine", ["#191724", "#1f1d2e", "#26233a", "#403d52", "#26233a", "#e0def4", "#908caa", "#6e6a86", "#ebbcba",
                   "#9ccfd8", "#f6c177", "#eb6f92", "#31748f", "#c4a7e7", "#ebbcba", "#9ccfd8"]),
];

const DEFAULT_THEME: &str = "catppuccin"; // herdr's own default

/// Corral's own themes: the palette (KEYS order), extra keys (syn_* are the
/// editor's syntax colours), and herdr's side ([theme.custom] that dresses herdr to match).
struct Own {
    name: &'static str,
    colours: [&'static str; 16],
    extra: &'static [(&'static str, &'static str)],
    herdr: &'static [(&'static str, &'static str)],
}

const OWN_THEMES: &[Own] = &[Own {
    // VS Code "Dark 2026" (theme-defaults/themes/2026-dark.json), as in Positron.
    name: "dark-2026",
    colours: ["#121314", "#191a1b", "#242526", "#245c73", "#2a2b2c", "#bbbebf", "#8c8c8c", "#5f6264", "#3994bc",
              "#7ee787", "#cd9731", "#f44747", "#79c0ff", "#d2a8ff", "#ffa657", "#4ec9b0"],
    extra: &[("line_number", "#858889"), ("syn_keyword", "#ff7b72"), ("syn_string", "#a5d6ff"),
             ("syn_comment", "#8b949e"), ("syn_function", "#d2a8ff"), ("syn_type", "#ffa657"),
             ("syn_variable", "#c9d1d9"), ("syn_variable_builtin", "#79c0ff"), ("syn_constant", "#79c0ff"),
             ("syn_number", "#79c0ff"), ("syn_operator", "#d4d4d4"), ("syn_property", "#79c0ff"),
             ("syn_attribute", "#79c0ff")],
    herdr: &[("accent", "#3994bc"), ("panel_bg", "#121314"), ("sidebar_bg", "#191a1b"), ("surface_dim", "#191a1b"),
             ("active_row_bg", "#242526"), ("selection_bg", "#245c73"), ("surface0", "#202122"), ("surface1", "#2a2b2c"),
             ("overlay0", "#555555"), ("overlay1", "#858889"), ("text", "#bbbebf"), ("subtext0", "#8c8c8c"),
             ("mauve", "#d2a8ff"), ("green", "#7ee787"), ("yellow", "#cd9731"), ("red", "#f44747"), ("blue", "#79c0ff"),
             ("teal", "#4ec9b0"), ("peach", "#ffa657")],
}];

/// herdr [theme.custom] tokens -> palette keys.
const CUSTOM_TOKENS: &[(&str, &str)] = &[
    ("accent", "accent"), ("sidebar_bg", "surface"), ("active_row_bg", "raised"), ("selection_bg", "selection"),
    ("surface_dim", "surface"), ("panel_bg", "bg"), ("text", "text"), ("green", "green"), ("yellow", "yellow"),
    ("red", "red"), ("mauve", "violet"), ("peach", "orange"),
];

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| env::home().join(".config"))
}

pub fn herdr_config() -> PathBuf {
    std::env::var_os("HERDR_CONFIG_PATH").map(PathBuf::from).unwrap_or_else(|| config_home().join("herdr/config.toml"))
}

pub fn palette_file() -> PathBuf {
    config_home().join("corral/palette.json")
}

pub fn corral_config_file() -> PathBuf {
    config_home().join("corral/config.json")
}

fn fresh_theme_file() -> PathBuf {
    config_home().join("fresh/themes/corral.json")
}

fn fresh_config() -> PathBuf {
    config_home().join("fresh/config.json")
}

fn own(name: &str) -> Option<&'static Own> {
    OWN_THEMES.iter().find(|t| t.name == name)
}

pub fn own_names() -> Vec<&'static str> {
    OWN_THEMES.iter().map(|t| t.name).collect()
}

/// (theme name, custom overrides) from herdr's config.toml.
pub fn herdr_theme() -> (String, Value) {
    let cfg = std::fs::read_to_string(herdr_config()).ok().and_then(|t| toml::parse(&t).ok()).unwrap_or_else(Value::obj);
    let theme = cfg.get("theme");
    let name = theme.get("name").str().filter(|n| !n.is_empty()).unwrap_or(DEFAULT_THEME).to_string();
    let custom = match theme.get("custom") {
        v @ Value::Obj(_) => v.clone(),
        _ => Value::obj(),
    };
    (name, custom)
}

fn write_config(cfg: &Value) {
    let path = corral_config_file();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, cfg.to_json_pretty(1).trim_end());
}

/// The theme stored in config.json ("tema" and "tema_herdr" are the names from before).
fn stored(cfg: &Value) -> (Option<&str>, Option<&str>) {
    (cfg.get("theme").str().or_else(|| cfg.get("tema").str()),
     cfg.get("theme_herdr").str().or_else(|| cfg.get("tema_herdr").str()))
}

fn forget_theme(cfg: &mut Value) {
    for k in ["theme", "theme_herdr", "tema", "tema_herdr"] {
        cfg.remove(k);
    }
}

/// The Corral theme chosen with `corral config theme`, while it is still on:
/// choosing another theme in herdr afterwards turns it off (see follow_herdr).
pub fn own_theme() -> Option<&'static str> {
    let cfg = env::config();
    let (name, base) = stored(&cfg);
    let chosen = own(name?)?;
    match base {
        Some(base) if base != herdr_theme().0 => None,
        _ => Some(chosen.name),
    }
}

// ----------------------------------------------------------------------- herdr's [theme.custom]

/// Lines of the [theme.custom] table (with Corral's comment above it, if any).
fn custom_span(lines: &[String]) -> Option<(usize, usize)> {
    let mut start = lines.iter().position(|l| l.trim() == "[theme.custom]")?;
    let end = (start + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
    while start > 0 && (lines[start - 1].starts_with("# Corral") || lines[start - 1].starts_with("# VS Code \"Dark 2026\"")) {
        start -= 1;
    }
    Some((start, end))
}

/// Write herdr's config, only if herdr accepts it, and have herdr reload it.
pub fn write_herdr(text: &str) -> Result<(), String> {
    let path = herdr_config();
    let tmp = path.with_extension(format!("toml.corral-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let check = herdr_bin().map(|bin| Command::new(bin).args(["config", "check"]).env("HERDR_CONFIG_PATH", &tmp).output());
    let ok = match check {
        Some(Ok(out)) => out.status.success(),
        _ => toml::parse(text).is_ok(), // no herdr to ask: at least it must read as TOML
    };
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(tr("herdr would not accept the configuration; I left it as it was", "herdr no aceptaría la configuración; la dejé como estaba").into());
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(bin) = herdr_bin() {
        let _ = Command::new(bin).args(["server", "reload-config"]).output(); // not running: it reads the file when it starts
    }
    Ok(())
}

fn herdr_bin() -> Option<PathBuf> {
    std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from).or_else(|| env::which("herdr"))
}

/// Which own theme these [theme.custom] colours are (Corral wrote them), if any.
fn is_own_custom(custom: &Value) -> Option<&'static Own> {
    let Value::Obj(items) = custom else { return None };
    OWN_THEMES.iter().find(|t| {
        items.len() == t.herdr.len()
            && t.herdr.iter().all(|(k, v)| custom.get(k).str().map(|s| s.to_lowercase()) == Some(v.to_string()))
    })
}

fn herdr_lines() -> Vec<String> {
    std::fs::read_to_string(herdr_config()).unwrap_or_default().lines().map(String::from).collect()
}

/// Put the theme's colours in herdr's [theme.custom]. False when the user has
/// colours of their own there (they are left alone).
fn dress_herdr(theme: &Own) -> Result<bool, String> {
    let custom = herdr_theme().1;
    if !matches!(&custom, Value::Obj(i) if i.is_empty()) && is_own_custom(&custom).is_none() {
        return Ok(false);
    }
    let mut lines = herdr_lines();
    if let Some((a, b)) = custom_span(&lines) {
        lines.drain(a..b);
    }
    let head = match lines.iter().position(|l| l.trim() == "[theme]") {
        Some(h) => h,
        None => {
            lines.extend([String::new(), "[theme]".into()]);
            lines.len() - 1
        }
    };
    let mut end = (head + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
    while end > head + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut block = vec![String::new(), format!("# Corral: tema {} (se quita solo al elegir otro tema en herdr)", theme.name),
                         "[theme.custom]".to_string()];
    block.extend(theme.herdr.iter().map(|(k, v)| format!("{k} = \"{v}\"")));
    block.push(String::new());
    lines.splice(end..end, block);
    write_herdr(&(lines.join("\n").trim_end_matches('\n').to_string() + "\n"))?;
    Ok(true)
}

/// Take Corral's colours out of herdr's [theme.custom] (never the user's own).
fn undress_herdr() -> Result<(), String> {
    if is_own_custom(&herdr_theme().1).is_none() {
        return Ok(());
    }
    let mut lines = herdr_lines();
    if let Some((a, b)) = custom_span(&lines) {
        lines.drain(a..b);
        write_herdr(&(lines.join("\n").trim_end_matches('\n').to_string() + "\n"))?;
    }
    Ok(())
}

/// `corral config theme`: an own theme for Corral and herdr, or None to follow
/// herdr. Returns whether herdr took the colours too.
fn choose(theme: Option<&'static Own>) -> Result<bool, String> {
    let mut cfg = env::config();
    let dressed = match theme {
        None => {
            undress_herdr()?;
            forget_theme(&mut cfg);
            false
        }
        Some(t) => {
            let dressed = dress_herdr(t)?;
            forget_theme(&mut cfg);
            cfg.set("theme", t.name.into());
            cfg.set("theme_herdr", herdr_theme().0.into());
            dressed
        }
    };
    write_config(&cfg);
    apply();
    Ok(dressed)
}

/// Another theme was chosen in herdr since the own theme went on: drop it.
fn follow_herdr() {
    let mut cfg = env::config();
    let (name, base) = stored(&cfg);
    let on = name.and_then(own).is_some();
    if on && base.is_some_and(|base| base != herdr_theme().0) {
        let _ = undress_herdr();
        forget_theme(&mut cfg);
        write_config(&cfg);
    }
}

/// The palette Corral uses: its own theme if one is chosen, else herdr's theme
/// with its custom colours applied.
pub fn palette() -> Value {
    let mut p = Value::obj();
    if let Some(t) = own_theme().and_then(own) {
        for (k, v) in KEYS.iter().zip(t.colours) {
            p.set(k, v.into());
        }
        for (k, v) in t.extra {
            p.set(k, (*v).into());
        }
        p.set("name", t.name.into());
        p.set("base", t.name.into());
        return p;
    }
    let (name, custom) = herdr_theme();
    let base = PALETTES.iter().find(|(n, _)| *n == name)
        // Light or unknown variants: their dark sibling, else herdr's default.
        .or_else(|| PALETTES.iter().find(|(n, _)| name.starts_with(n)))
        .or_else(|| PALETTES.iter().find(|(n, _)| *n == DEFAULT_THEME)).expect("default palette");
    for (k, v) in KEYS.iter().zip(base.1) {
        p.set(k, v.into());
    }
    for (token, key) in CUSTOM_TOKENS {
        if let Some(v) = custom.get(token).str().filter(|v| v.starts_with('#') && v.len() == 7) {
            p.set(key, v.to_lowercase().into());
        }
    }
    p.set("name", name.as_str().into());
    p.set("base", base.0.into());
    p
}

pub fn rgb(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let part = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0);
    [part(0), part(2), part(4)]
}

/// Nearest xterm-256 colour index (for VisiData).
pub fn xterm256(hex: &str) -> u8 {
    let [r, g, b] = rgb(hex).map(i32::from);
    let steps = [0, 95, 135, 175, 215, 255];
    let nearest = |v: i32| (0..6).min_by_key(|&i| (steps[i] - v).abs()).unwrap_or(0);
    let ci = [nearest(r), nearest(g), nearest(b)];
    let cube = 16 + 36 * ci[0] + 6 * ci[1] + ci[2];
    let cube_rgb = [steps[ci[0]], steps[ci[1]], steps[ci[2]]];
    let gray_level = (((r + g + b) as f64 / 3.0 - 8.0) / 10.0).round().clamp(0.0, 23.0) as i32;
    let gray_rgb = [8 + 10 * gray_level; 3];
    let dist = |c: [i32; 3]| (c[0] - r).pow(2) + (c[1] - g).pow(2) + (c[2] - b).pow(2);
    if dist(cube_rgb) <= dist(gray_rgb) { cube as u8 } else { (232 + gray_level) as u8 }
}

/// A Fresh theme from the palette. Backgrounds of the text area stay "Default"
/// so the editor shows the terminal background, like every other herdr pane.
fn fresh_theme(p: &Value) -> Value {
    let c = |k: &str| -> Value {
        let hex = p.get(k).str().unwrap_or("#808080");
        Value::Arr(rgb(hex).iter().map(|v| Value::Num(*v as f64)).collect())
    };
    let d = || Value::Str("Default".into());
    let styled = |k: &str, m: &str| obj(vec![("color", c(k)), ("modifier", Value::Arr(vec![m.into()]))]);
    let vesper = p.get("base").str() == Some("vesper");
    let own_syntax: Vec<(String, Value)> = match p {
        Value::Obj(items) => items.iter().filter_map(|(k, v)| {
            let name = k.strip_prefix("syn_")?;
            v.str().map(|hex| (name.to_string(), Value::Arr(rgb(hex).iter().map(|x| Value::Num(*x as f64)).collect())))
        }).collect(),
        _ => Vec::new(),
    };
    let syntax = if !own_syntax.is_empty() {
        Value::Obj(own_syntax)
    } else {
        obj(vec![
            ("keyword", styled(if vesper { "muted" } else { "violet" }, "italic")),
            ("string", c("green")),
            ("comment", styled("dim", "italic")),
            ("function", c(if vesper { "accent" } else { "blue" })),
            ("type", c(if vesper { "text" } else { "yellow" })),
            ("variable", c("text")),
            ("variable_builtin", c("red")),
            ("constant", c("orange")),
            ("operator", c("muted")),
            ("number", c("orange")),
            ("property", c(if vesper { "text" } else { "cyan" })),
            ("attribute", styled("violet", "dim")),
        ])
    };
    let line_number = if p.get("line_number").str().is_some() { c("line_number") } else { c("dim") };
    let mut editor = obj(vec![
        ("bg", d()), ("fg", c("text")), ("cursor", c("accent")), ("inactive_cursor", c("dim")),
        ("selection_bg", c("selection")), ("current_line_bg", c("raised")), ("line_number_fg", line_number),
        ("line_number_bg", d()), ("current_line_number_fg", c("accent")), ("gutter_separator_fg", c("border")),
        ("diff_add_bg", c("raised")), ("diff_remove_bg", c("raised")), ("diff_add_highlight_bg", c("selection")),
        ("diff_remove_highlight_bg", c("selection")), ("diff_modify_bg", c("raised")),
        ("whitespace_indicator_fg", c("border")), ("whitespace_indicator_selected_fg", c("muted")),
        ("bracket_match_fg", c("accent")), ("ruler_bg", c("raised")),
    ]);
    for (i, k) in ["accent", "blue", "green", "violet", "yellow", "red"].iter().enumerate() {
        editor.set(&format!("indent_rainbow_{}", i + 1), c(k));
        editor.set(&format!("bracket_rainbow_{}", i + 1), c(k));
    }
    let ui_pairs: &[(&str, &str)] = &[
        ("tab_active_fg", "text"), ("tab_active_bg", "-"), ("tab_inactive_fg", "dim"), ("tab_inactive_bg", "-"),
        ("tab_separator_bg", "-"), ("tab_separator_fg", "border"), ("tab_close_hover_fg", "red"),
        ("tab_hover_bg", "raised"), ("tab_drop_zone_bg", "selection"), ("tab_drop_zone_border", "accent"),
        ("menu_bg", "-"), ("menu_fg", "muted"), ("menu_active_bg", "raised"), ("menu_active_fg", "text"),
        ("menu_dropdown_bg", "surface"), ("menu_dropdown_fg", "text"), ("menu_highlight_bg", "selection"),
        ("menu_highlight_fg", "text"), ("menu_border_fg", "border"), ("menu_separator_fg", "border"),
        ("menu_hover_bg", "raised"), ("menu_hover_fg", "text"), ("menu_disabled_fg", "dim"), ("menu_disabled_bg", "surface"),
        ("status_bar_fg", "muted"), ("status_bar_bg", "-"), ("status_separator_fg", "border"),
        ("status_separator_bg", "-"), ("status_palette_fg", "bg"), ("status_palette_bg", "accent"),
        ("status_error_indicator_fg", "bg"), ("status_error_indicator_bg", "red"),
        ("status_error_indicator_hover_fg", "bg"), ("status_error_indicator_hover_bg", "red"),
        ("status_warning_indicator_fg", "bg"), ("status_warning_indicator_bg", "yellow"),
        ("status_warning_indicator_hover_fg", "bg"), ("status_warning_indicator_hover_bg", "yellow"),
        ("prompt_fg", "text"), ("prompt_bg", "surface"), ("prompt_selection_fg", "bg"), ("prompt_selection_bg", "accent"),
        ("popup_border_fg", "border"), ("popup_bg", "surface"), ("popup_selection_bg", "selection"),
        ("popup_selection_fg", "text"), ("popup_text_fg", "text"), ("list_drop_target_bg", "selection"),
        ("text_input_selection_bg", "selection"), ("settings_selected_bg", "selection"),
        ("settings_selected_fg", "text"), ("suggestion_bg", "surface"), ("suggestion_selected_bg", "selection"),
        ("help_bg", "surface"), ("help_fg", "text"), ("help_key_fg", "accent"), ("help_separator_fg", "border"),
        ("help_indicator_fg", "bg"), ("help_indicator_bg", "accent"), ("inline_code_bg", "raised"),
        ("split_separator_fg", "border"), ("split_separator_hover_fg", "accent"),
        ("scrollbar_track_fg", "-"), ("scrollbar_thumb_fg", "border"), ("scrollbar_track_hover_fg", "raised"),
        ("scrollbar_thumb_hover_fg", "dim"), ("compose_margin_bg", "-"), ("blame_header_fg", "muted"),
        ("blame_header_bg", "surface"), ("semantic_highlight_bg", "raised"), ("tour_step_bg", "raised"),
        ("terminal_bg", "-"), ("terminal_fg", "-"),
        ("file_explorer_bg", "-"), ("file_explorer_fg", "muted"), ("file_explorer_directory_fg", "text"),
        ("file_explorer_border_fg", "border"), ("file_explorer_border_focused_fg", "accent"),
        ("file_explorer_indicator_fg", "dim"), ("file_explorer_pinned_bg", "raised"), ("file_status_added_fg", "green"),
    ];
    let ui = Value::Obj(ui_pairs.iter().map(|(k, v)| (k.to_string(), if *v == "-" { d() } else { c(v) })).collect());
    obj(vec![
        ("name", "corral".into()),
        ("editor", editor),
        ("ui", ui),
        ("search", obj(vec![("match_bg", c("selection")), ("match_fg", c("text")), ("current_match_bg", c("accent")),
                            ("current_match_fg", c("bg")), ("label_bg", c("accent")), ("label_fg", c("bg"))])),
        ("diagnostic", obj(vec![("error_fg", c("red")), ("error_bg", d()), ("warning_fg", c("yellow")), ("warning_bg", d()),
                                ("info_fg", c("blue")), ("info_bg", d()), ("hint_fg", c("dim")), ("hint_bg", d())])),
        ("syntax", syntax),
        ("chrome", obj(vec![("gutter_separator", false.into()), ("file_explorer_border", false.into()),
                            ("tab_separator", "▏".into()), ("status_bar_separator", "│".into())])),
    ])
}

/// Write the palette and the editor theme; point Fresh at it. Returns the palette.
pub fn apply() -> Value {
    follow_herdr();
    let mut p = palette();
    let x256 = match &p {
        Value::Obj(items) => Value::Obj(items.iter().filter_map(|(k, v)| {
            v.str().filter(|s| s.starts_with('#')).map(|s| (k.clone(), Value::Num(xterm256(s) as f64)))
        }).collect()),
        _ => Value::obj(),
    };
    p.set("x256", x256);
    p.set("source_mtime", Value::Num(sources_mtime()));
    env::write_json(&palette_file(), &p);
    let fresh = fresh_config();
    if fresh.parent().is_some_and(|d| d.exists()) {
        let _ = std::fs::create_dir_all(config_home().join("fresh/themes"));
        let _ = std::fs::write(fresh_theme_file(), fresh_theme(&p).to_json_pretty(1));
        if let Some(mut cfg) = std::fs::read_to_string(&fresh).ok().and_then(|t| json::parse(&t).ok()) {
            if matches!(cfg, Value::Obj(_)) && cfg.get("theme").str() != Some("corral") {
                cfg.set("theme", "corral".into());
                let _ = std::fs::write(&fresh, cfg.to_json_pretty(2));
            }
        }
    }
    p
}

fn mtime(path: &std::path::Path) -> Option<f64> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

/// When herdr's config (or Corral's theme choice) last changed.
pub fn sources_mtime() -> f64 {
    [herdr_config(), corral_config_file()].iter().filter_map(|f| mtime(f)).fold(0.0, f64::max)
}

/// `corral theme [--show]`.
pub fn cmd_theme(args: &[String]) -> Result<(), String> {
    let show = args.iter().any(|a| a == "--show");
    let p = if show { palette() } else { apply() };
    let name = p.get("name").str().unwrap_or("").to_string();
    let base = p.get("base").str().unwrap_or("").to_string();
    if own_theme().is_some() {
        println!("{}", trf!("Corral theme: {name} (corral config theme herdr goes back to herdr's)", "tema de Corral: {name} (corral config theme herdr vuelve al de herdr)"));
    } else {
        let custom = match herdr_theme().1 {
            Value::Obj(i) => i.len(),
            _ => 0,
        };
        let mut line = trf!("herdr theme: {name}", "tema de herdr: {name}");
        if base != name {
            line += &format!(" (base {base})");
        }
        if custom > 0 {
            line += &trf!(", with {custom} colors of your own", ", con {custom} colores propios");
        }
        println!("{line}");
    }
    if show {
        for k in ["accent", "text", "muted", "dim", "selection", "green", "yellow", "red"] {
            println!("  {k:<10} {}", p.get(k).str().unwrap_or(""));
        }
        return Ok(());
    }
    println!("{}: {}", tr("palette", "paleta"), palette_file().display());
    println!("{}", trf!("Fresh theme: {} (shows when the editor restarts)", "tema de Fresh: {} (se ve al reiniciar el editor)", fresh_theme_file().display()));
    Ok(())
}

/// `corral config [KEY [VALUE]]`: read or change Corral's settings.
pub fn cmd_config(args: &[String]) -> Result<(), String> {
    let mut cfg = env::config();
    if args.len() == 1 && args[0] == "language" {
        println!("{}", crate::i18n::resolve());
        return Ok(());
    }
    let (Some(key), Some(value)) = (args.first(), args.get(1)) else {
        let shown = match args.first() {
            Some(k) => cfg.get(k).clone(),
            None => cfg,
        };
        println!("{}", shown.to_json());
        return Ok(());
    };
    // From the palette (a plugin action) there is no terminal to print to.
    let say = |title: &str, body: &str| {
        if std::env::var_os("HERDR_PLUGIN_ID").is_some() {
            env::notify(title, body);
        } else {
            println!("{title}: {body}");
        }
    };
    if key == "language" {
        let lang = crate::i18n::normalize(value).ok_or_else(|| trf!("language takes: {}", "language acepta: {}", crate::i18n::LANGUAGES.join(", ")))?;
        cfg.set("language", lang.into());
        write_config(&cfg);
        if lang == "es" {
            say("Corral en español", "Todo cambia ahora; el editor de scripts, al volver a abrirlo. Los comandos siguen en inglés.");
        } else {
            say("Corral in English", "Everything switches now; the script editor when you open it again.");
        }
        return Ok(());
    }
    if key == "theme" {
        // A theme for Corral and herdr together; choosing another theme in herdr drops it.
        let names = own_names();
        if value != "herdr" && !names.contains(&value.as_str()) {
            let mut all: Vec<&str> = names.clone();
            all.push("herdr");
            all.sort();
            return Err(trf!("theme takes: {}", "theme acepta: {}", all.join(", ")));
        }
        if value == "herdr" {
            choose(None)?;
            say(tr("herdr theme", "Tema de herdr"), &trf!("Corral follows {}", "Corral sigue {}", herdr_theme().0));
        } else if choose(own(value))? {
            say(&trf!("{value} theme", "Tema {value}"), tr("in Corral and herdr; choose another theme in herdr (ctrl+b s) and Corral follows it",
                                                          "en Corral y en herdr; al elegir otro tema en herdr (ctrl+b s), Corral lo sigue"));
        } else {
            say(&trf!("{value} theme", "Tema {value}"), tr("in Corral; herdr keeps your [theme.custom] colors", "en Corral; herdr mantiene tus colores de [theme.custom]"));
        }
        return Ok(());
    }
    let allowed: &[(&str, &[&str])] = &[("agents", &["off", "ask", "on"]), ("agent_output", &["full", "review", "structure"]),
                                         ("images", &["auto", "kitty", "symbols"]), ("mirador_window", &["right", "free"]),
                                         ("ark_frontend", &["corral", "jupyter"]), ("python_lsp", &["ty", "pylsp"])];
    if let Some((_, values)) = allowed.iter().find(|(k, _)| k == key) {
        if !values.contains(&value.as_str()) {
            let mut v = values.to_vec();
            v.sort();
            return Err(trf!("{key} takes: {}", "{key} acepta: {}", v.join(", ")));
        }
    }
    cfg.set(key, value.as_str().into());
    write_config(&cfg);
    // The security settings come from the palette too: say what they mean.
    let meaning = match (key.as_str(), value.as_str()) {
        ("agents", "on") => Some((tr("Agents: on", "Agentes: activados"), tr("They run code in your console without asking.", "Ejecutan código en tu consola sin preguntar."))),
        ("agents", "ask") => Some((tr("Agents: ask", "Agentes: preguntan"), tr("Each request waits until you approve it.", "Cada petición espera hasta que la apruebes."))),
        ("agents", "off") => Some((tr("Agents: off", "Agentes: desactivados"), tr("They can't run code in your consoles.", "No pueden ejecutar código en tus consolas."))),
        ("agent_output", "full") => Some((tr("Agent output: full", "Salida para agentes: completa"), tr("They see what their code prints, and new plots.", "Ven lo que imprime su código, y los gráficos nuevos."))),
        ("agent_output", "review") => Some((tr("Agent output: review", "Salida para agentes: revisar"), tr("You see each result first, and decide whether they get it.", "Ves cada resultado primero y decides si lo reciben."))),
        ("agent_output", "structure") => Some((tr("Agent output: structure", "Salida para agentes: estructura"), tr("They see whether it worked and the shape of objects, never values.", "Ven si funcionó y la forma de los objetos, nunca valores."))),
        _ => None,
    };
    match meaning {
        Some((title, body)) => say(title, body),
        None => println!("{key} = {value}"),
    }
    Ok(())
}
