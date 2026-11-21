"""Full-screen viewers that run inside herdr popups: plots and R variables.

Standard library only; images are drawn with chafa (symbols by default, or the
Kitty graphics protocol when the outer terminal supports it).
"""

from __future__ import annotations

import fcntl
import json
import os
import select
import shutil
import signal
import struct
import subprocess
import sys
import termios
import time
import tty
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Callable

RESET = "\x1b[0m"
BOLD = "\x1b[1m"


def rgb(r: int, g: int, b: int) -> str:
    return f"\x1b[38;2;{r};{g};{b}m"


def bg(r: int, g: int, b: int) -> str:
    return f"\x1b[48;2;{r};{g};{b}m"


ACCENT = rgb(224, 123, 83)
TEXT = rgb(233, 233, 233)
MUTED = rgb(155, 155, 155)
DIM = rgb(100, 100, 100)
BLUE = rgb(121, 168, 255)
VIOLET = rgb(178, 148, 255)
GREEN = rgb(79, 194, 124)
SEL_BG = bg(34, 34, 34)

KEYS = {
    "\x1b[A": "up", "\x1bOA": "up", "\x1b[B": "down", "\x1bOB": "down",
    "\x1b[C": "right", "\x1bOC": "right", "\x1b[D": "left", "\x1bOD": "left",
    "\x1b[H": "home", "\x1b[F": "end", "\x1b[5~": "pgup", "\x1b[6~": "pgdn",
    "\r": "enter", "\n": "enter", "\x1b": "esc", "\x7f": "backspace", "\x08": "backspace",
    "\x03": "quit",
}


def out(s: str) -> None:
    sys.stdout.write(s)
    sys.stdout.flush()


def clip(s: str, width: int) -> str:
    s = str(s)
    return s if len(s) <= width else (s[: max(0, width - 1)] + "…" if width > 0 else "")


class Screen:
    """Alternate screen, no echo, keys one at a time; restores the terminal on exit."""

    def __init__(self, kitty: bool = False, mouse: bool = False) -> None:
        self.kitty = kitty
        self.mouse = mouse

    def __enter__(self) -> "Screen":
        self.fd = sys.stdin.fileno()
        self.saved = termios.tcgetattr(self.fd)
        tty.setcbreak(self.fd)
        self.resized = True
        signal.signal(signal.SIGWINCH, self._on_resize)
        out("\x1b[?1049h\x1b[?25l" + ("\x1b[?1000h\x1b[?1006h" if self.mouse else ""))
        return self

    def __exit__(self, *exc) -> None:
        out(("\x1b[?1006l\x1b[?1000l" if self.mouse else "") + ("\x1b_Ga=d,q=2\x1b\\" if self.kitty else "")
            + "\x1b[0m\x1b[2J\x1b[?25h\x1b[?1049l")
        termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)

    def _on_resize(self, *_):
        self.resized = True

    def size(self) -> tuple[int, int, int, int]:
        """(cols, rows, pixel width, pixel height); pixels are 0 when unknown."""
        try:
            rows, cols, xpx, ypx = struct.unpack("HHHH", fcntl.ioctl(sys.stdout.fileno(), termios.TIOCGWINSZ, b"\0" * 8))
            return cols, rows, xpx, ypx
        except OSError:
            c, r = shutil.get_terminal_size()
            return c, r, 0, 0

    def key(self, timeout: float) -> str | None:
        try:
            ready, _, _ = select.select([self.fd], [], [], timeout)
        except InterruptedError:
            return None
        if not ready:
            return None
        data = os.read(self.fd, 64).decode("utf-8", "replace")
        if data.startswith("\x1b[<"):  # SGR mouse: ESC [ < button ; x ; y M|m
            try:
                b, x, y = data[3:-1].split(";")
                if data.endswith("M"):
                    return f"mouse:{int(b)}:{int(x)}:{int(y)}"
            except ValueError:
                pass
            return None
        if data == "\x1b[Z":
            return "backtab"
        return KEYS.get(data, data if len(data) == 1 else None)


def message_screen(title: str, body: str) -> int:
    with Screen() as scr:
        while True:
            if scr.resized:
                scr.resized = False
                cols, rows, _, _ = scr.size()
                lines = [f"{TEXT}{BOLD}{title}{RESET}", "", *[f"{MUTED}{l}{RESET}" for l in body.split("\n")], "", f"{DIM}q para cerrar{RESET}"]
                top = max(0, (rows - len(lines)) // 2)
                out("\x1b[2J")
                for i, line in enumerate(lines):
                    out(f"\x1b[{top + i + 1};3H{line}")
            if scr.key(0.5) in ("q", "esc", "enter", "quit"):
                return 0


# --------------------------------------------------------------------------- plots


class Httpgd:
    def __init__(self, url: str) -> None:
        parsed = urllib.parse.urlparse(url)
        self.base = f"{parsed.scheme}://{parsed.netloc}"
        self.token = urllib.parse.parse_qs(parsed.query).get("token", [""])[0]

    def get(self, path: str, **params) -> bytes:
        if self.token:
            params["token"] = self.token
        with urllib.request.urlopen(f"{self.base}/{path}?{urllib.parse.urlencode(params)}", timeout=10) as r:
            return r.read()

    def json(self, path: str) -> dict:
        return json.loads(self.get(path))

    def title(self, plot_id: str) -> str:
        """Best guess at a plot's title: its largest text, if larger than the rest."""
        try:
            data = json.loads(self.get("plot", id=plot_id, width=720, height=480, renderer="json"))
        except (OSError, ValueError):
            return ""
        texts = [d for d in data.get("draw_calls", []) if d.get("type") == "text" and str(d.get("str", "")).strip()]
        if not texts:
            return ""
        sizes = sorted(d.get("fontsize", 0) for d in texts)
        best = max(texts, key=lambda d: (d.get("fontsize", 0), d.get("weight", 400), -d.get("y", 0)))
        if best.get("fontsize", 0) <= sizes[len(sizes) // 2] and best.get("weight", 400) <= 400:
            return ""  # no text stands out: no title
        return str(best["str"]).strip()


def plots_viewer(session_dir: Path, images: str = "symbols", open_browser: Callable[[], None] | None = None,
                 start_id: str | None = None) -> int:
    url_file = session_dir / "plots-url"
    if not url_file.is_file():
        return message_screen("Todavía no hay gráficos", "Dibuja algo en la consola de R, por ejemplo plot(1:10),\ny vuelve a abrir este visor con ctrl+b f.")
    if not shutil.which("chafa"):
        return message_screen("Falta chafa", "Instálalo con: sudo apt install chafa")
    gd = Httpgd(url_file.read_text().strip())
    tmp = session_dir / "view.png"
    ids: list[str] = []
    idx, upid, follow = -1, None, True
    status = ""

    def load() -> None:
        nonlocal ids, idx, upid
        data = gd.json("plots")
        ids = [p["id"] for p in data.get("plots", [])]
        upid = data.get("state", {}).get("upid")
        if follow or not (0 <= idx < len(ids)):
            idx = len(ids) - 1

    def render(scr: Screen) -> None:
        cols, rows, xpx, ypx = scr.size()
        area = max(4, rows - 2)
        out(("\x1b_Ga=d,q=2\x1b\\" if images == "kitty" else "") + "\x1b[2J\x1b[H")
        if not ids:
            out(f"\x1b[3;3H{MUTED}No hay gráficos en esta sesión. Dibuja algo en R.{RESET}")
        else:
            if images == "kitty":
                # Real pixels when the terminal reports them; otherwise ~9x19 px per cell.
                w, h, zoom = (xpx, int(ypx * area / rows), 1) if xpx and ypx else (cols * 9, area * 19, 1)
            else:
                # About 10x20 px per cell; zoom makes labels readable at character resolution.
                w, h, zoom = cols * 10, area * 20, 1.6
            try:
                tmp.write_bytes(gd.get("plot", id=ids[idx], width=int(w), height=int(h), renderer="png", zoom=zoom))
                fmt = "kitty" if images == "kitty" else "symbols"
                args = ["chafa", "-f", fmt, "--size", f"{cols}x{area}", "--animate", "off", str(tmp)]
                if fmt == "symbols":
                    # Block elements only: every terminal font has them (sextants are often missing).
                    args[1:1] = ["-c", "full", "--symbols", "block+border+space", "--color-space", "din99d", "-w", "9"]
                img = subprocess.run(args, capture_output=True, text=True, timeout=20).stdout
                out(img)
            except (OSError, subprocess.SubprocessError) as exc:
                out(f"\x1b[3;3H{ACCENT}No pude dibujar el gráfico: {exc}{RESET}")
        pos = f"{idx + 1}/{len(ids)}" if ids else "0/0"
        live = f"{GREEN}●{RESET} " if follow else ""
        hint = "←/→ historial · o abrir en el navegador · q salir"
        footer = f" {ACCENT}◔{RESET} {TEXT}Gráfico {pos}{RESET}  {live}{DIM}{clip(hint, cols - 20)}{RESET}"
        if status:
            footer += f"  {MUTED}{status}{RESET}"
        out(f"\x1b[{rows};1H\x1b[2K{footer}")

    try:
        load()
        if start_id in ids:
            idx, follow = ids.index(start_id), ids.index(start_id) == len(ids) - 1
    except OSError:
        return message_screen("httpgd no responde", "¿Cerraste R o el dispositivo gráfico?\nDibuja de nuevo y vuelve a abrir el visor.")

    with Screen(kitty=images == "kitty") as scr:
        last_poll = 0.0
        while True:
            if scr.resized:
                scr.resized = False
                render(scr)
            key = scr.key(0.3)
            if key in ("q", "esc", "quit"):
                return 0
            if key in ("left", "h") and idx > 0:
                idx -= 1
                follow = False
                render(scr)
            elif key in ("right", "l") and idx < len(ids) - 1:
                idx += 1
                follow = idx == len(ids) - 1
                render(scr)
            elif key == "end" and ids:
                idx, follow = len(ids) - 1, True
                render(scr)
            elif key == "o" and open_browser:
                status = "abriendo…"
                render(scr)
                open_browser()
                status = ""
            elif key is None and time.time() - last_poll > 1:
                last_poll = time.time()
                try:
                    state = gd.json("state")
                except OSError:
                    continue
                if state.get("upid") != upid:
                    try:
                        load()
                    except OSError:
                        continue
                    render(scr)


# --------------------------------------------------------------------------- variables

# Tabs of the variables viewer: (id, label, kinds shown, colour, icon per kind)
TABS = [
    ("data", "Datos", ["data"]),
    ("plots", "Gráficos", ["plotobj", "plot"]),
    ("values", "Valores", ["value"]),
    ("functions", "Funciones", ["function"]),
    ("other", "Otros", ["other"]),
]
ICONS = {"data": (BLUE, "▦"), "plot": (ACCENT, "◔"), "plotobj": (ACCENT, "◇"),
         "value": (DIM, "•"), "function": (VIOLET, "ƒ"), "other": (DIM, "◇")}
SUBTITLES = {"plotobj": "Objetos", "plot": "Historial"}
MAX_PLOTS = 12


def fmt_bytes(n) -> str:
    if not isinstance(n, (int, float)):
        return ""
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" or n >= 10 else f"{n:.1f} {unit}"
        n /= 1024
    return ""


def vars_viewer(session_dir: Path | None, run_in_console: Callable[[str, str], bool] | None,
                resolve: Callable[[], Path | None] | None = None, keep_open: bool = False,
                open_plot: Callable[[Path, str], str] | None = None,
                show_object: Callable[[Path, str], str] | None = None) -> int:
    """List the objects of an R session; refreshes as they change.

    run_in_console(pane_id, code) runs code in the session's console.
    resolve() finds the current session again (a pane that outlives R restarts).
    keep_open: stay open after Enter (a permanent pane rather than a popup).
    """
    if session_dir is None and resolve is None:
        return message_screen("No hay una sesión de R con Corral", "Abre R en un panel de herdr.")
    sel, top, query, typing, status = 0, 0, "", False, ""
    rev, items, meta = None, [], {}
    plots: list[dict] = []
    plot_state: dict = {"url": None, "upid": None, "titles": {}}

    def load_plots() -> bool:
        """Refresh the plot list from httpgd when it changed. True if it changed."""
        nonlocal plots
        url_file = session_dir / "plots-url" if session_dir else None
        url = url_file.read_text().strip() if url_file and url_file.is_file() else None
        if url != plot_state["url"]:
            plot_state.update(url=url, upid=None, titles={})
            if not url and plots:
                plots = []
                return True
        if not url:
            return False
        gd = Httpgd(url)
        try:
            upid = gd.json("state").get("upid")
            if upid == plot_state["upid"]:
                return False
            ids = [p["id"] for p in gd.json("plots").get("plots", [])]
        except (OSError, ValueError):
            return False
        plot_state["upid"] = upid
        recent = ids[-MAX_PLOTS:]
        for pid in recent:
            if pid not in plot_state["titles"]:
                plot_state["titles"][pid] = gd.title(pid)
        plots = [{"name": plot_state["titles"].get(pid) or f"Gráfico {ids.index(pid) + 1}", "kind": "plot",
                  "preview": f"{ids.index(pid) + 1}/{len(ids)}", "class": "gráfico", "plot_id": pid,
                  "order": -ids.index(pid)} for pid in recent]
        return True

    def files() -> tuple[Path, Path] | None:
        return (session_dir / "vars.json", session_dir / "meta.json") if session_dir else None

    def load() -> bool:
        """Reload when the session or its variables changed. True if something changed."""
        nonlocal rev, items, meta, session_dir, sel
        f = files()
        if resolve:
            # Follow the console of this workspace: a new R, or a console that
            # registered after this pane started, takes over.
            found = resolve()
            if found != session_dir:
                session_dir, rev, items, meta, sel = found, None, [], {}, 0
                plot_state.update(url=None, upid=None, titles={})
                f = files()
        if f is None or not f[0].is_file():
            changed = bool(items) or rev is not None
            rev, items = None, []
            return changed
        mtime = f[0].stat().st_mtime
        if mtime == rev:
            return False
        rev = mtime
        try:
            items = json.loads(f[0].read_text()).get("vars", [])
            meta = json.loads(f[1].read_text()) if f[1].is_file() else {}
        except (OSError, ValueError):
            items = []
        return True

    tab = 0
    tab_chosen = False  # once the user picks a tab, new objects never move them

    def matches(v: dict) -> bool:
        q = query.lower()
        return not q or q in v["name"].lower() or q in str(v.get("class", "")).lower()

    def in_tab(i: int) -> list[dict]:
        kinds = TABS[i][2]
        shown = [v for v in items + plots if v.get("kind") in kinds and matches(v)]
        return sorted(shown, key=lambda v: (kinds.index(v["kind"]), v.get("order", 0), v["name"].lower()))

    def visible() -> list[dict]:
        return in_tab(tab)

    tab_spans: list[tuple[int, int]] = []  # columns of each tab label, for mouse clicks
    LIST_TOP = 5

    def render(scr: Screen) -> None:
        nonlocal top, tab_spans
        cols, rows, _, _ = scr.size()
        wide = cols >= 70
        vs = visible()
        out("\x1b[2J\x1b[H")
        where = " · ".join(x for x in [meta.get("pane_id"), "R " + str(meta.get("r_version", "")) if meta else "",
                                         str(meta.get("cwd", "")).replace(str(Path.home()), "~")] if x)
        n_vars = sum(1 for v in items if matches(v))
        count = f"{n_vars} objetos" + (f" · {len(plots)} gráficos" if plots else "")
        out(f"\x1b[1;2H{ACCENT}▸▸{RESET} {TEXT}{BOLD}Variables{RESET}  {DIM}{clip(where, max(0, cols - len(count) - 19))}{RESET}")
        out(f"\x1b[1;{max(1, cols - len(count))}H{MUTED}{count}{RESET}")

        # Tab bar
        x = 2
        tab_spans = []
        bar = ""
        for i, (_, label, _kinds) in enumerate(TABS):
            n = len(in_tab(i))
            text = f" {label} {n} "
            if i == tab:
                piece = f"{SEL_BG}{ACCENT}{BOLD}{text}{RESET}"
            elif n == 0:
                piece = f"{DIM}{text}{RESET}"
            else:
                piece = f"{MUTED}{text}{RESET}"
            tab_spans.append((x, x + len(text) - 1))
            bar += piece + " "
            x += len(text) + 1
        out(f"\x1b[3;2H{bar}")
        underline = tab_spans[tab]
        out(f"\x1b[4;1H{DIM}{'─' * cols}{RESET}")
        out(f"\x1b[4;{underline[0]}H{ACCENT}{'━' * (underline[1] - underline[0] + 1)}{RESET}")

        if rev is None:
            msg = "Esperando una sesión de R con Corral en la consola…" if resolve else "Esta sesión no publica variables (¿falta jsonlite?)."
            out(f"\x1b[6;3H{MUTED}{clip(msg, cols - 4)}{RESET}")

        lines: list[tuple[str, int | None]] = []  # (text, index into vs or None)
        last_kind = None
        multi = len({v["kind"] for v in vs}) > 1
        for i, v in enumerate(vs):
            kind = v.get("kind")
            if multi and kind != last_kind and kind in SUBTITLES:
                if lines:
                    lines.append(("", None))
                lines.append((f"{DIM}{SUBTITLES[kind].upper()}{RESET}", None))
            last_kind = kind
            color, icon = ICONS.get(kind, (DIM, "•"))
            preview = str(v.get("preview") or "")
            size = fmt_bytes(v.get("size"))
            meta_txt = (f"{v.get('class') or ''}{' · ' + size if size else ''}" if kind != "plot" else "") if wide else ""
            right = clip(preview, max(8, cols // 3))
            name = clip(v["name"], max(6, cols - len(right) - len(meta_txt) - 10))
            pad = cols - 6 - len(name) - len(right) - (len(meta_txt) + 2 if meta_txt else 0) - 1
            tail = f"  {DIM}{meta_txt}{RESET}" if meta_txt else ""
            body = f"{color}{icon}{RESET} {TEXT}{BOLD}{name}{RESET}{' ' * max(1, pad)}{MUTED}{right}{RESET}{tail}"
            lines.append((body, i))

        area = rows - LIST_TOP - 1
        sel_line = next((n for n, (_, i) in enumerate(lines) if i == sel), 0)
        if sel_line < top:
            top = max(0, sel_line - 1)
        elif sel_line >= top + area:
            top = sel_line - area + 1
        row_items.clear()
        for row, (text, i) in enumerate(lines[top: top + area]):
            y = row + LIST_TOP + 1
            row_items[y] = i
            if i is not None and i == sel:
                out(f"\x1b[{y};1H{SEL_BG}\x1b[2K {ACCENT}▸{RESET}{SEL_BG} {text}{SEL_BG}\x1b[K{RESET}")
            else:
                out(f"\x1b[{y};1H   {text}")
        if rev is not None and not vs:
            empty = "Ningún objeto coincide." if query else f"No hay {TABS[tab][1].lower()} en la sesión."
            out(f"\x1b[{LIST_TOP + 2};4H{MUTED}{empty}{RESET}")

        current = vs[sel] if 0 <= sel < len(vs) else None
        kind = current.get("kind") if current else None
        action = {"data": "Enter tabla", "plot": "Enter ver", "plotobj": "Enter dibujar"}.get(kind, "Enter str()")
        hint = (f"/{query}▏  Enter o Esc para terminar" if typing
                else status or f"←/→ pestañas · ↑↓ mover · {action} · / buscar" + ("" if keep_open else " · q salir"))
        out(f"\x1b[{rows};1H\x1b[2K {DIM}{clip(hint, cols - 2)}{RESET}")

    row_items: dict[int, int | None] = {}

    def first_useful_tab() -> int:
        for i in range(len(TABS)):
            if in_tab(i):
                return i
        return 0

    def act(v: dict) -> str:
        """Enter on an item. Returns a status message."""
        if v.get("kind") == "plot" and open_plot and session_dir:
            return f"{v['name']}: abierto en {open_plot(session_dir, v['plot_id'])}"
        if v.get("kind") == "plotobj" and show_object and session_dir:
            return f"{v['name']}: {show_object(session_dir, v['name'])}"
        if run_in_console and meta.get("pane_id"):
            name = "`" + v["name"].replace("`", "\\`") + "`"
            code = f"View({name})" if v.get("kind") == "data" else f"str({name})"
            ok = run_in_console(meta["pane_id"], code)
            return f"{code} enviado a la consola" if ok else "no pude enviar a la consola"
        return ""

    load()
    load_plots()
    tab = first_useful_tab()
    with Screen(mouse=True) as scr:
        last_check = 0.0
        while True:
            if scr.resized:
                scr.resized = False
                render(scr)
            key = scr.key(0.3)
            vs = visible()
            if key is None:
                if time.time() - last_check > 1:
                    last_check = time.time()
                    try:
                        changed = load()
                        if load_plots() or changed:
                            if not tab_chosen and not visible():
                                tab = first_useful_tab()
                            sel = min(sel, max(0, len(visible()) - 1))
                            render(scr)
                    except OSError:
                        pass
                continue
            status = ""
            if typing:
                if key in ("enter", "esc"):
                    typing = False
                elif key == "backspace":
                    query = query[:-1]
                elif len(key) == 1 and key.isprintable():
                    query += key
                sel = 0
                render(scr)
                continue
            if key.startswith("mouse:"):
                _, button, mx, my = key.split(":")
                button, mx, my = int(button), int(mx), int(my)
                if button == 64:      # wheel up
                    sel = max(sel - 3, 0)
                elif button == 65:    # wheel down
                    sel = min(sel + 3, len(vs) - 1)
                elif button == 0 and my == 3:
                    for i, (a, b) in enumerate(tab_spans):
                        if a <= mx <= b:
                            tab, sel, top, tab_chosen = i, 0, 0, True
                elif button == 0 and row_items.get(my) is not None:
                    clicked = row_items[my]
                    if clicked == sel and 0 <= sel < len(vs):
                        status = "…"
                        render(scr)
                        status = act(vs[sel])
                        if not keep_open:
                            return 0
                    sel = clicked
                render(scr)
                continue
            if key in ("q", "quit") or (key == "esc" and not keep_open):
                return 0
            if key == "esc":
                query = ""
            elif key in ("right", "l", "\t"):
                tab, sel, top, tab_chosen = (tab + 1) % len(TABS), 0, 0, True
            elif key in ("left", "h", "backtab"):
                tab, sel, top, tab_chosen = (tab - 1) % len(TABS), 0, 0, True
            elif key in "12345" and len(key) == 1:
                tab, sel, top, tab_chosen = int(key) - 1, 0, 0, True
            elif key in ("down", "j"):
                sel = min(sel + 1, len(vs) - 1)
            elif key in ("up", "k"):
                sel = max(sel - 1, 0)
            elif key == "pgdn":
                sel = min(sel + 10, len(vs) - 1)
            elif key == "pgup":
                sel = max(sel - 10, 0)
            elif key == "/":
                typing, query, sel = True, "", 0
            elif key == "enter" and 0 <= sel < len(vs):
                status = "…"
                render(scr)
                status = act(vs[sel])
                if not keep_open:
                    return 0
            render(scr)
