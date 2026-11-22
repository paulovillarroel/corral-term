"""One palette for every Corral window, taken from herdr's theme.

herdr is the source of truth: its `[theme] name` picks a base palette and its
`[theme.custom]` colours override it. From that palette Corral writes:

  ~/.config/corral/palette.json   read by the terminal viewers, VisiData and Mirador
  ~/.config/fresh/themes/corral.json  the editor theme (background left to the terminal)

herdr does not publish its theme colours, so the base palettes below follow
each theme's published colours.
"""

from __future__ import annotations

import json
import os
import tomllib
from pathlib import Path

CONFIG_HOME = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
HERDR_CONFIG = Path(os.environ.get("HERDR_CONFIG_PATH") or CONFIG_HOME / "herdr" / "config.toml")
PALETTE_FILE = CONFIG_HOME / "corral" / "palette.json"
FRESH_THEME = CONFIG_HOME / "fresh" / "themes" / "corral.json"
FRESH_CONFIG = CONFIG_HOME / "fresh" / "config.json"

# bg: page background (Mirador); surface: bars, menus; raised: current line, hover;
# selection; border; text; muted; dim; accent; then the named colours.
PALETTES: dict[str, dict[str, str]] = {
    "vesper": dict(bg="#101010", surface="#161616", raised="#1c1c1c", selection="#343434", border="#282828",
                   text="#ffffff", muted="#a0a0a0", dim="#5c5c5c", accent="#ffc799", green="#99ffe4",
                   yellow="#ffc799", red="#ff8080", blue="#a0a0a0", violet="#a0a0a0", orange="#ffc799",
                   cyan="#99ffe4"),
    "dracula": dict(bg="#282a36", surface="#21222c", raised="#343746", selection="#44475a", border="#343746",
                    text="#f8f8f2", muted="#bfbfbf", dim="#6272a4", accent="#bd93f9", green="#50fa7b",
                    yellow="#f1fa8c", red="#ff5555", blue="#8be9fd", violet="#ff79c6", orange="#ffb86c",
                    cyan="#8be9fd"),
    "catppuccin": dict(bg="#1e1e2e", surface="#181825", raised="#313244", selection="#45475a", border="#313244",
                       text="#cdd6f4", muted="#a6adc8", dim="#6c7086", accent="#f5c2e7", green="#a6e3a1",
                       yellow="#f9e2af", red="#f38ba8", blue="#89b4fa", violet="#cba6f7", orange="#fab387",
                       cyan="#94e2d5"),
    "tokyo-night": dict(bg="#1a1b26", surface="#16161e", raised="#292e42", selection="#33467c", border="#29293a",
                        text="#c0caf5", muted="#a9b1d6", dim="#565f89", accent="#7aa2f7", green="#9ece6a",
                        yellow="#e0af68", red="#f7768e", blue="#7aa2f7", violet="#bb9af7", orange="#ff9e64",
                        cyan="#7dcfff"),
    "nord": dict(bg="#2e3440", surface="#272c36", raised="#3b4252", selection="#434c5e", border="#3b4252",
                 text="#eceff4", muted="#d8dee9", dim="#616e88", accent="#88c0d0", green="#a3be8c",
                 yellow="#ebcb8b", red="#bf616a", blue="#81a1c1", violet="#b48ead", orange="#d08770",
                 cyan="#8fbcbb"),
    "gruvbox": dict(bg="#282828", surface="#1d2021", raised="#3c3836", selection="#504945", border="#3c3836",
                    text="#ebdbb2", muted="#bdae93", dim="#7c6f64", accent="#fe8019", green="#b8bb26",
                    yellow="#fabd2f", red="#fb4934", blue="#83a598", violet="#d3869b", orange="#fe8019",
                    cyan="#8ec07c"),
    "one-dark": dict(bg="#282c34", surface="#21252b", raised="#2c313a", selection="#3e4451", border="#3e4451",
                     text="#abb2bf", muted="#9da5b4", dim="#5c6370", accent="#61afef", green="#98c379",
                     yellow="#e5c07b", red="#e06c75", blue="#61afef", violet="#c678dd", orange="#d19a66",
                     cyan="#56b6c2"),
    "solarized": dict(bg="#002b36", surface="#00212b", raised="#073642", selection="#0a4a5c", border="#073642",
                      text="#93a1a1", muted="#839496", dim="#586e75", accent="#b58900", green="#859900",
                      yellow="#b58900", red="#dc322f", blue="#268bd2", violet="#6c71c4", orange="#cb4b16",
                      cyan="#2aa198"),
    "kanagawa": dict(bg="#1f1f28", surface="#16161d", raised="#2a2a37", selection="#2d4f67", border="#2a2a37",
                     text="#dcd7ba", muted="#c8c093", dim="#727169", accent="#7e9cd8", green="#98bb6c",
                     yellow="#e6c384", red="#ff5d62", blue="#7e9cd8", violet="#957fb8", orange="#ffa066",
                     cyan="#7aa89f"),
    "rose-pine": dict(bg="#191724", surface="#1f1d2e", raised="#26233a", selection="#403d52", border="#26233a",
                      text="#e0def4", muted="#908caa", dim="#6e6a86", accent="#ebbcba", green="#9ccfd8",
                      yellow="#f6c177", red="#eb6f92", blue="#31748f", violet="#c4a7e7", orange="#ebbcba",
                      cyan="#9ccfd8"),
}
# Corral's own themes, chosen with `corral config tema <name>`: they rule Corral's
# windows whatever herdr's theme is (herdr follows with matching [theme.custom]).
# syn_* keys give the editor its syntax colours; without them they come from the
# named colours above.
OWN_THEMES: dict[str, dict[str, str]] = {
    # VS Code "Dark 2026" (theme-defaults/themes/2026-dark.json), as in Positron.
    "dark-2026": dict(bg="#121314", surface="#191a1b", raised="#242526", selection="#245c73", border="#2a2b2c",
                      text="#bbbebf", muted="#8c8c8c", dim="#5f6264", accent="#3994bc", green="#7ee787",
                      yellow="#cd9731", red="#f44747", blue="#79c0ff", violet="#d2a8ff", orange="#ffa657",
                      cyan="#4ec9b0", line_number="#858889",
                      syn_keyword="#ff7b72", syn_string="#a5d6ff", syn_comment="#8b949e", syn_function="#d2a8ff",
                      syn_type="#ffa657", syn_variable="#c9d1d9", syn_variable_builtin="#79c0ff",
                      syn_constant="#79c0ff", syn_number="#79c0ff", syn_operator="#d4d4d4", syn_property="#79c0ff",
                      syn_attribute="#79c0ff"),
}
# herdr's side of each own theme: [theme.custom] colours that dress herdr to match.
OWN_HERDR: dict[str, dict[str, str]] = {
    "dark-2026": {
        "accent": "#3994bc", "panel_bg": "#121314", "sidebar_bg": "#191a1b", "surface_dim": "#191a1b",
        "active_row_bg": "#242526", "selection_bg": "#245c73", "surface0": "#202122", "surface1": "#2a2b2c",
        "overlay0": "#555555", "overlay1": "#858889", "text": "#bbbebf", "subtext0": "#8c8c8c",
        "mauve": "#d2a8ff", "green": "#7ee787", "yellow": "#cd9731", "red": "#f44747", "blue": "#79c0ff",
        "teal": "#4ec9b0", "peach": "#ffa657",
    },
}
CORRAL_CONFIG = CONFIG_HOME / "corral" / "config.json"

DEFAULT_THEME = "catppuccin"  # herdr's own default

# herdr [theme.custom] tokens -> palette keys
CUSTOM_TOKENS = {
    "accent": "accent", "sidebar_bg": "surface", "active_row_bg": "raised", "selection_bg": "selection",
    "surface_dim": "surface", "panel_bg": "bg", "text": "text", "green": "green", "yellow": "yellow",
    "red": "red", "mauve": "violet", "peach": "orange",
}


def herdr_theme() -> tuple[str, dict]:
    """(theme name, custom overrides) from herdr's config.toml."""
    try:
        cfg = tomllib.loads(HERDR_CONFIG.read_text())
    except (OSError, tomllib.TOMLDecodeError):
        return DEFAULT_THEME, {}
    theme = cfg.get("theme") or {}
    return str(theme.get("name") or DEFAULT_THEME), dict(theme.get("custom") or {})


def corral_config() -> dict:
    try:
        cfg = json.loads(CORRAL_CONFIG.read_text())
    except (OSError, ValueError):
        return {}
    return cfg if isinstance(cfg, dict) else {}


def own_theme() -> str | None:
    """The Corral theme chosen with `corral config tema`, while it is still on:
    choosing another theme in herdr afterwards turns it off (see follow_herdr)."""
    cfg = corral_config()
    chosen = cfg.get("tema")
    if chosen not in OWN_THEMES:
        return None
    if cfg.get("tema_herdr") not in (None, herdr_theme()[0]):
        return None
    return chosen


# --------------------------------------------------------------------------- herdr's [theme.custom]

def _custom_span(lines: list[str]) -> tuple[int, int] | None:
    """Lines of the [theme.custom] table (with Corral's comment above it, if any)."""
    start = next((i for i, l in enumerate(lines) if l.strip() == "[theme.custom]"), None)
    if start is None:
        return None
    end = next((i for i in range(start + 1, len(lines)) if lines[i].lstrip().startswith("[")), len(lines))
    while start > 0 and lines[start - 1].startswith(("# Corral", '# VS Code "Dark 2026"')):
        start -= 1
    return start, end


def _write_herdr(lines: list[str]) -> None:
    text = "\n".join(lines).rstrip("\n") + "\n"
    tomllib.loads(text)  # never leave herdr with a broken config
    HERDR_CONFIG.write_text(text)
    import subprocess
    try:
        subprocess.run(["herdr", "server", "reload-config"], capture_output=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        pass  # herdr not running: it reads the file when it starts


def is_own_custom(custom: dict) -> str | None:
    """Which own theme these [theme.custom] colours are (Corral wrote them), if any."""
    norm = {k: str(v).lower() for k, v in custom.items()}
    return next((t for t, colours in OWN_HERDR.items() if norm == colours), None)


def dress_herdr(theme: str) -> bool:
    """Put the theme's colours in herdr's [theme.custom]. False when the user has
    colours of their own there (they are left alone)."""
    _, custom = herdr_theme()
    if custom and not is_own_custom(custom):
        return False
    lines = HERDR_CONFIG.read_text().splitlines() if HERDR_CONFIG.exists() else []
    span = _custom_span(lines)
    if span:
        del lines[span[0]:span[1]]
    head = next((i for i, l in enumerate(lines) if l.strip() == "[theme]"), None)
    if head is None:
        lines += ["", "[theme]"]
        head = len(lines) - 1
    end = next((i for i in range(head + 1, len(lines)) if lines[i].lstrip().startswith("[")), len(lines))
    while end > head + 1 and not lines[end - 1].strip():
        end -= 1
    block = ["", f"# Corral: tema {theme} (se quita solo al elegir otro tema en herdr)", "[theme.custom]"]
    block += [f'{k} = "{v}"' for k, v in OWN_HERDR[theme].items()] + [""]
    lines[end:end] = block
    _write_herdr(lines)
    return True


def undress_herdr() -> None:
    """Take Corral's colours out of herdr's [theme.custom] (never the user's own)."""
    _, custom = herdr_theme()
    if not custom or not is_own_custom(custom):
        return
    lines = HERDR_CONFIG.read_text().splitlines()
    span = _custom_span(lines)
    if span:
        del lines[span[0]:span[1]]
        _write_herdr(lines)


def choose(theme: str | None) -> bool:
    """`corral config tema`: an own theme for Corral and herdr, or None to follow herdr.
    Returns whether herdr took the colours too."""
    cfg = corral_config()
    if theme is None:
        undress_herdr()
        cfg.pop("tema", None)
        cfg.pop("tema_herdr", None)
        dressed = False
    else:
        dressed = dress_herdr(theme)
        cfg["tema"] = theme
        cfg["tema_herdr"] = herdr_theme()[0]
    CORRAL_CONFIG.parent.mkdir(parents=True, exist_ok=True)
    CORRAL_CONFIG.write_text(json.dumps(cfg, indent=1, ensure_ascii=False))
    apply()
    return dressed


def follow_herdr() -> None:
    """Another theme was chosen in herdr since the own theme went on: drop it."""
    cfg = corral_config()
    if cfg.get("tema") in OWN_THEMES and cfg.get("tema_herdr") not in (None, herdr_theme()[0]):
        undress_herdr()
        cfg.pop("tema", None)
        cfg.pop("tema_herdr", None)
        CORRAL_CONFIG.write_text(json.dumps(cfg, indent=1, ensure_ascii=False))


def palette() -> dict:
    """The palette Corral uses: its own theme if one is chosen, else herdr's
    theme with its custom colours applied."""
    own = own_theme()
    if own:
        return dict(OWN_THEMES[own], name=own, base=own)
    name, custom = herdr_theme()
    base = name
    if base not in PALETTES:
        # Light or unknown variants: their dark sibling, else herdr's default.
        base = next((k for k in PALETTES if name.startswith(k)), DEFAULT_THEME)
    pal = dict(PALETTES[base])
    for token, key in CUSTOM_TOKENS.items():
        value = custom.get(token)
        if isinstance(value, str) and value.startswith("#") and len(value) == 7:
            pal[key] = value.lower()
    pal["name"] = name
    pal["base"] = base
    return pal


def rgb(hex_color: str) -> list[int]:
    h = hex_color.lstrip("#")
    return [int(h[i:i + 2], 16) for i in (0, 2, 4)]


def xterm256(hex_color: str) -> int:
    """Nearest xterm-256 colour index (for VisiData)."""
    r, g, b = rgb(hex_color)
    steps = [0, 95, 135, 175, 215, 255]

    def nearest(v):
        return min(range(6), key=lambda i: abs(steps[i] - v))

    ci = (nearest(r), nearest(g), nearest(b))
    cube = 16 + 36 * ci[0] + 6 * ci[1] + ci[2]
    cube_rgb = (steps[ci[0]], steps[ci[1]], steps[ci[2]])
    gray_level = min(23, max(0, round((sum((r, g, b)) / 3 - 8) / 10)))
    gray = 232 + gray_level
    gray_rgb = (8 + 10 * gray_level,) * 3

    def dist(c):
        return sum((a - b) ** 2 for a, b in zip(c, (r, g, b)))

    return cube if dist(cube_rgb) <= dist(gray_rgb) else gray


def fresh_theme(p: dict) -> dict:
    """A Fresh theme from the palette. Backgrounds of the text area stay 'Default'
    so the editor shows the terminal background, like every other herdr pane."""
    c = {k: rgb(v) for k, v in p.items() if isinstance(v, str) and v.startswith("#")}
    D = "Default"
    vesper_like = p["base"] == "vesper"
    if "syn_keyword" in c:
        syntax = {k[4:]: c[k] for k in c if k.startswith("syn_")}
    else:
        syntax = None
    syntax = syntax or {
        "keyword": {"color": c["muted"] if vesper_like else c["violet"], "modifier": ["italic"]},
        "string": c["green"],
        "comment": {"color": c["dim"], "modifier": ["italic"]},
        "function": c["accent"] if vesper_like else c["blue"],
        "type": c["text"] if vesper_like else c["yellow"],
        "variable": c["text"],
        "variable_builtin": c["red"],
        "constant": c["orange"],
        "operator": c["muted"],
        "number": c["orange"],
        "property": c["text"] if vesper_like else c["cyan"],
        "attribute": {"color": c["violet"], "modifier": ["dim"]},
    }
    rainbow = [c["accent"], c["blue"], c["green"], c["violet"], c["yellow"], c["red"]]
    editor = {
        "bg": D, "fg": c["text"], "cursor": c["accent"], "inactive_cursor": c["dim"],
        "selection_bg": c["selection"], "current_line_bg": c["raised"], "line_number_fg": c.get("line_number", c["dim"]),
        "line_number_bg": D, "current_line_number_fg": c["accent"], "gutter_separator_fg": c["border"],
        "diff_add_bg": c["raised"], "diff_remove_bg": c["raised"], "diff_add_highlight_bg": c["selection"],
        "diff_remove_highlight_bg": c["selection"], "diff_modify_bg": c["raised"],
        "whitespace_indicator_fg": c["border"], "whitespace_indicator_selected_fg": c["muted"],
        "bracket_match_fg": c["accent"], "ruler_bg": c["raised"],
    }
    for i, col in enumerate(rainbow, 1):
        editor[f"indent_rainbow_{i}"] = col
        editor[f"bracket_rainbow_{i}"] = col
    ui = {
        "tab_active_fg": c["text"], "tab_active_bg": D, "tab_inactive_fg": c["dim"], "tab_inactive_bg": D,
        "tab_separator_bg": D, "tab_separator_fg": c["border"], "tab_close_hover_fg": c["red"],
        "tab_hover_bg": c["raised"], "tab_drop_zone_bg": c["selection"], "tab_drop_zone_border": c["accent"],
        "menu_bg": D, "menu_fg": c["muted"], "menu_active_bg": c["raised"], "menu_active_fg": c["text"],
        "menu_dropdown_bg": c["surface"], "menu_dropdown_fg": c["text"], "menu_highlight_bg": c["selection"],
        "menu_highlight_fg": c["text"], "menu_border_fg": c["border"], "menu_separator_fg": c["border"],
        "menu_hover_bg": c["raised"], "menu_hover_fg": c["text"], "menu_disabled_fg": c["dim"],
        "menu_disabled_bg": c["surface"],
        "status_bar_fg": c["muted"], "status_bar_bg": D, "status_separator_fg": c["border"],
        "status_separator_bg": D, "status_palette_fg": c["bg"], "status_palette_bg": c["accent"],
        "status_error_indicator_fg": c["bg"], "status_error_indicator_bg": c["red"],
        "status_error_indicator_hover_fg": c["bg"], "status_error_indicator_hover_bg": c["red"],
        "status_warning_indicator_fg": c["bg"], "status_warning_indicator_bg": c["yellow"],
        "status_warning_indicator_hover_fg": c["bg"], "status_warning_indicator_hover_bg": c["yellow"],
        "prompt_fg": c["text"], "prompt_bg": c["surface"], "prompt_selection_fg": c["bg"],
        "prompt_selection_bg": c["accent"],
        "popup_border_fg": c["border"], "popup_bg": c["surface"], "popup_selection_bg": c["selection"],
        "popup_selection_fg": c["text"], "popup_text_fg": c["text"], "list_drop_target_bg": c["selection"],
        "text_input_selection_bg": c["selection"], "settings_selected_bg": c["selection"],
        "settings_selected_fg": c["text"], "suggestion_bg": c["surface"], "suggestion_selected_bg": c["selection"],
        "help_bg": c["surface"], "help_fg": c["text"], "help_key_fg": c["accent"], "help_separator_fg": c["border"],
        "help_indicator_fg": c["bg"], "help_indicator_bg": c["accent"], "inline_code_bg": c["raised"],
        "split_separator_fg": c["border"], "split_separator_hover_fg": c["accent"],
        "scrollbar_track_fg": D, "scrollbar_thumb_fg": c["border"], "scrollbar_track_hover_fg": c["raised"],
        "scrollbar_thumb_hover_fg": c["dim"], "compose_margin_bg": D, "blame_header_fg": c["muted"],
        "blame_header_bg": c["surface"], "semantic_highlight_bg": c["raised"], "tour_step_bg": c["raised"],
        "terminal_bg": D, "terminal_fg": D,
        "file_explorer_bg": D, "file_explorer_fg": c["muted"], "file_explorer_directory_fg": c["text"],
        "file_explorer_border_fg": c["border"], "file_explorer_border_focused_fg": c["accent"],
        "file_explorer_indicator_fg": c["dim"], "file_explorer_pinned_bg": c["raised"],
        "file_status_added_fg": c["green"],
    }
    return {
        "name": "corral",
        "editor": editor,
        "ui": ui,
        "search": {"match_bg": c["selection"], "match_fg": c["text"], "current_match_bg": c["accent"],
                   "current_match_fg": c["bg"], "label_bg": c["accent"], "label_fg": c["bg"]},
        "diagnostic": {"error_fg": c["red"], "error_bg": D, "warning_fg": c["yellow"], "warning_bg": D,
                       "info_fg": c["blue"], "info_bg": D, "hint_fg": c["dim"], "hint_bg": D},
        "syntax": syntax,
        "chrome": {"gutter_separator": False, "file_explorer_border": False, "tab_separator": "▏",
                   "status_bar_separator": "│"},
    }


def apply() -> dict:
    """Write the palette and the editor theme; point Fresh at it. Returns the palette."""
    follow_herdr()
    p = palette()
    p["x256"] = {k: xterm256(v) for k, v in p.items() if isinstance(v, str) and v.startswith("#")}
    p["source_mtime"] = herdr_mtime()
    PALETTE_FILE.parent.mkdir(parents=True, exist_ok=True)
    PALETTE_FILE.write_text(json.dumps(p, indent=1))
    if FRESH_CONFIG.parent.exists():
        FRESH_THEME.parent.mkdir(parents=True, exist_ok=True)
        FRESH_THEME.write_text(json.dumps(fresh_theme(p), indent=1, ensure_ascii=False))
        try:
            cfg = json.loads(FRESH_CONFIG.read_text())
        except (OSError, ValueError):
            cfg = None
        if isinstance(cfg, dict) and cfg.get("theme") != "corral":
            cfg["theme"] = "corral"
            FRESH_CONFIG.write_text(json.dumps(cfg, indent=2, ensure_ascii=False) + "\n")
    return p


def herdr_mtime() -> float:
    """When herdr's config (or Corral's theme choice) last changed."""
    times = []
    for f in (HERDR_CONFIG, CORRAL_CONFIG):
        try:
            times.append(f.stat().st_mtime)
        except OSError:
            pass
    return max(times, default=0.0)


def sync() -> bool:
    """Re-apply when herdr's config changed since the palette was written. True if it did."""
    current = load()
    if current and current.get("source_mtime") == herdr_mtime():
        return False
    apply()
    return True


def palette_mtime() -> float:
    try:
        return PALETTE_FILE.stat().st_mtime
    except OSError:
        return 0.0


def load() -> dict | None:
    """The palette last written by apply(), for the viewers."""
    try:
        return json.loads(PALETTE_FILE.read_text())
    except (OSError, ValueError):
        return None
