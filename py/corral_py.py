"""Corral in IPython: the Python side of a Corral session.

`corral` starts the Python console with `uv run --with ipython ipython -i py/startup.py`,
which calls activate(). Like r/corral.R, the session publishes itself for the
variables pane, Mirador and agents:

  ~/.local/state/corral/sessions/<id>/meta.json   which pane, which Python, where
  .../vars.json                                    the objects in the namespace, after every cell
  .../plots-url                                    the figures, served with httpgd's HTTP API
  .../eval/<id>.py                                 code sent by `corral eval`

Figures are served with the same endpoints as R's httpgd (/state, /plots, /plot,
/remove, /clear), so Mirador and `corral` treat both languages alike.

Only the standard library is required; pandas, polars, pyarrow, numpy and
matplotlib are used when the session has them.
"""

from __future__ import annotations

import atexit
import functools
import inspect
import io
import json
import os
import platform
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import types
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

STATE_DIR = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local" / "state") / "corral"
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
SHORT = {"pandas": "pd.", "polars": "pl.", "numpy": "np.", "pyarrow": "pa."}
HIDDEN = {"In", "Out", "exit", "quit", "get_ipython", "open", "View"}

_session: Session | None = None


# --------------------------------------------------------------------------- helpers


def write_json_atomic(path: Path, data) -> None:
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(data, ensure_ascii=False, default=str), encoding="utf-8")
    os.replace(tmp, path)


def corral_bin() -> str | None:
    return os.environ.get("CORRAL_BIN") or shutil.which("corral")


def in_herdr() -> bool:
    return os.environ.get("HERDR_ENV") == "1"


def short_class(x) -> str:
    t = type(x)
    return SHORT.get(t.__module__.split(".")[0], "") + t.__name__


def clip(text: str, n: int = 80) -> str:
    text = " ".join(str(text).split())
    return text if len(text) <= n else text[: n - 1] + "…"


# --------------------------------------------------------------------------- objects


def is_plot(x) -> bool:
    t = type(x)
    mod = t.__module__
    if mod.startswith("plotnine") and t.__name__ == "ggplot":
        return True
    if mod.startswith("seaborn") and t.__name__ in ("FacetGrid", "PairGrid", "JointGrid"):
        return True
    return mod.startswith("matplotlib.figure") and t.__name__ in ("Figure", "SubFigure")


def plot_preview(x) -> str:
    mod = type(x).__module__.split(".")[0]
    title = ""
    try:
        if mod == "plotnine":
            title = x.labels.get("title", "") if hasattr(x.labels, "get") else ""
        else:
            fig = getattr(x, "figure", x)
            title = fig._suptitle.get_text() if getattr(fig, "_suptitle", None) else (
                fig.axes[0].get_title() if fig.axes else "")
    except Exception:
        title = ""
    return f"{mod} · {title}" if title else mod


def is_table(x) -> bool:
    t = type(x)
    mod = t.__module__.split(".")[0]
    if mod in ("pandas", "polars") and t.__name__ == "DataFrame":
        return True
    if mod == "pyarrow" and t.__name__ == "Table":
        return True
    return mod == "numpy" and t.__name__ == "ndarray" and getattr(x, "ndim", 0) == 2


def size_of(x) -> int | None:
    try:
        mod = type(x).__module__.split(".")[0]
        if mod == "pandas":
            usage = x.memory_usage(index=True, deep=False)
            return int(usage.sum()) if hasattr(usage, "sum") else int(usage)
        if mod == "polars" and hasattr(x, "estimated_size"):
            return int(x.estimated_size())
        if mod in ("numpy", "pyarrow") and hasattr(x, "nbytes"):
            return int(x.nbytes)
        return sys.getsizeof(x)
    except Exception:
        return None


def describe(name: str, x) -> dict:
    kind, preview, size, fingerprint = "other", "", None, None
    if is_plot(x):
        kind, preview, fingerprint = "plotobj", plot_preview(x), str(id(x))
    elif is_table(x):
        kind, size = "data", size_of(x)
        preview = " × ".join(str(n) for n in x.shape)
    elif isinstance(x, (types.FunctionType, types.BuiltinFunctionType, types.MethodType, functools.partial)):
        kind = "function"
        try:
            preview = "def" + str(inspect.signature(x))
        except (TypeError, ValueError):
            preview = "def(…)"
    elif x is None or isinstance(x, (bool, int, float, complex, str, bytes)) or type(x).__module__ == "numpy" and getattr(x, "ndim", None) == 0:
        kind, preview, size = "value", clip(repr(x)), size_of(x)
    elif isinstance(x, (list, tuple, set, frozenset, dict)) or (
            type(x).__name__ in ("ndarray", "Series") and type(x).__module__.split(".")[0] in ("numpy", "pandas", "polars")):
        kind, size = "value", size_of(x)
        preview = f"{short_class(x)} [{len(x)}]"
    else:
        preview, size = short_class(x), size_of(x)
    return {"name": name, "kind": kind, "class": short_class(x), "preview": preview,
            "size": size if kind != "plotobj" else None, "hash": fingerprint}


def has_content(fig) -> bool:
    """Something drawn: a figure that only has empty axes (plt.subplots()) waits for data."""
    try:
        return bool(fig.images or fig.texts or getattr(fig, "_suptitle", None)
                    or any(ax.has_data() for ax in fig.axes))
    except Exception:
        return bool(getattr(fig, "axes", None))


def figure_of(x):
    """The matplotlib figure for a plot object, and whether we created it."""
    mod = type(x).__module__
    if mod.startswith("plotnine"):
        return x.draw(), True
    if mod.startswith("seaborn"):
        return getattr(x, "figure", None) or x.fig, False
    if hasattr(x, "savefig"):
        return x, False
    if hasattr(x, "get_figure"):
        return x.get_figure(), False
    raise TypeError(f"{short_class(x)} no es un gráfico")


# --------------------------------------------------------------------------- plots (httpgd's API)


class PlotServer:
    """Rendered figures behind httpgd's endpoints, on 127.0.0.1 with a token."""

    def __init__(self, session: Session) -> None:
        self.session = session
        self.pages: list[dict] = []
        self.next_id = 0
        self.upid = 0
        self.lock = threading.Lock()
        self.token = secrets.token_urlsafe(12)
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}/live?token={self.token}"
        for folder in (session.dir, STATE_DIR):
            (folder / "plots-url").write_text(self.url + "\n")

    def state(self) -> dict:
        return {"upid": self.upid, "hsize": len(self.pages), "active": True}

    @staticmethod
    def render(fig) -> tuple[bytes, bytes | None]:
        png = io.BytesIO()
        fig.savefig(png, format="png", dpi=144, bbox_inches="tight", facecolor=fig.get_facecolor())
        svg = io.BytesIO()
        try:
            fig.savefig(svg, format="svg", bbox_inches="tight", facecolor=fig.get_facecolor())
            svg_bytes = svg.getvalue() if svg.tell() < 4_000_000 else None  # huge scatter plots: PNG only
        except Exception:
            svg_bytes = None
        # savefig leaves the figure marked as changed; only real changes should redraw it.
        fig.stale = False
        return png.getvalue(), svg_bytes

    def add(self, fig) -> str:
        png, svg = self.render(fig)
        with self.lock:
            pid = str(self.next_id)
            self.next_id += 1
            self.pages.append({"id": pid, "png": png, "svg": svg})
            self.upid += 1
            first = len(self.pages) == 1 and self.next_id == 1
        if first:
            self.session.first_plot()
        return pid

    def update(self, pid: str, fig) -> bool:
        """Draw a figure again over its page (like adding to a plot in R's httpgd)."""
        png, svg = self.render(fig)
        with self.lock:
            for page in self.pages:
                if page["id"] == pid:
                    page.update(png=png, svg=svg)
                    self.upid += 1
                    return True
        return False

    def has(self, pid: str | None) -> bool:
        with self.lock:
            return pid is not None and any(p["id"] == pid for p in self.pages)

    def number(self, pid: str) -> int:
        with self.lock:
            return next((i + 1 for i, p in enumerate(self.pages) if p["id"] == pid), 0)

    def find(self, query: dict) -> int | None:
        with self.lock:
            if "id" in query:
                return next((i for i, p in enumerate(self.pages) if p["id"] == query["id"][0]), None)
            try:
                i = int(query.get("index", ["-1"])[0])
            except ValueError:
                return None
            i = i if i >= 0 else len(self.pages) + i
            return i if 0 <= i < len(self.pages) else None

    def _handler(self):
        server = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):  # quiet
                pass

            def _send(self, code: int, body: bytes, ctype: str) -> None:
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)

            def _json(self, data) -> None:
                self._send(200, json.dumps(data).encode(), "application/json")

            def do_GET(self) -> None:
                url = urlparse(self.path)
                query = parse_qs(url.query)
                if query.get("token", [""])[0] != server.token and self.headers.get("X-HTTPGD-TOKEN") != server.token:
                    return self._send(401, b"token", "text/plain")
                path = url.path.rstrip("/")
                if path == "/state":
                    return self._json(server.state())
                if path == "/plots":
                    with server.lock:
                        plots = [{"id": p["id"]} for p in server.pages]
                    return self._json({"state": server.state(), "plots": plots})
                if path == "/plot":
                    i = server.find(query)
                    if i is None:
                        return self._send(404, b"no plot", "text/plain")
                    page = server.pages[i]
                    if query.get("renderer", ["svg"])[0].startswith("svg") and page["svg"]:
                        return self._send(200, page["svg"], "image/svg+xml")
                    return self._send(200, page["png"], "image/png")
                if path in ("/remove", "/clear"):
                    with server.lock:
                        if path == "/clear":
                            server.pages.clear()
                        else:
                            i = None
                            if "id" in query:
                                i = next((k for k, p in enumerate(server.pages) if p["id"] == query["id"][0]), None)
                            else:
                                try:
                                    i = int(query.get("index", ["-1"])[0])
                                    i = i if i >= 0 else len(server.pages) + i
                                except ValueError:
                                    i = None
                            if i is not None and 0 <= i < len(server.pages):
                                del server.pages[i]
                        server.upid += 1
                    return self._json(server.state())
                if path in ("", "/live"):
                    return self._send(200, b"Corral (Python): los graficos se ven en Mirador.", "text/plain")
                self._send(404, b"not found", "text/plain")

        return Handler

    def close(self) -> None:
        try:
            self.httpd.shutdown()
        except Exception:
            pass


# --------------------------------------------------------------------------- session


class Session:
    def __init__(self, shell) -> None:
        self.shell = shell
        pane = os.environ.get("HERDR_PANE_ID", "")
        self.pane = pane
        self.id = (re.sub(r"[^A-Za-z0-9._-]", "-", pane) if pane else "pid") + f"-{os.getpid()}"
        self.dir = STATE_DIR / "sessions" / self.id
        self.dir.mkdir(parents=True, exist_ok=True)
        self.started = time.time()
        self.cwd = ""
        self.tmp = Path(tempfile.mkdtemp(prefix="corral-py-"))
        self.rendered: dict[int, str] = {}  # figures sent during this cell: id(figure) -> plot id
        self.pages_before = 0
        self.plots = PlotServer(self)
        self.write_meta()
        atexit.register(self.close)

    # ---- registry

    def write_meta(self) -> None:
        self.cwd = os.getcwd()
        write_json_atomic(self.dir / "meta.json", {
            "id": self.id, "pid": os.getpid(), "lang": "python",
            "pane_id": self.pane or None,
            "workspace_id": os.environ.get("HERDR_WORKSPACE_ID") or None,
            "python_version": platform.python_version(), "executable": sys.executable,
            "cwd": self.cwd, "started": self.started,
        })

    def snapshot(self) -> None:
        if os.getcwd() != self.cwd:
            self.write_meta()
        ns = self.shell.user_ns
        hidden = set(getattr(self.shell, "user_ns_hidden", {}))
        out = []
        for name in sorted(ns):
            if name.startswith("_") or name in HIDDEN or name in hidden:
                continue
            x = ns[name]
            if isinstance(x, types.ModuleType):
                continue
            try:
                out.append(describe(name, x))
            except Exception:
                out.append({"name": name, "kind": "other", "class": type(x).__name__,
                            "preview": "", "size": None, "hash": None})
        write_json_atomic(self.dir / "vars.json", {"updated": time.time(), "vars": out})

    # ---- cells

    def before_cell(self, info=None) -> None:
        self.rendered.clear()
        self.pages_before = len(self.plots.pages)

    def after_cell(self, result=None) -> None:
        try:
            self.flush_figures()
            # Variables holding a figure that has a page: Enter on them shows that drawing.
            drawn = {}
            for name, value in list(self.shell.user_ns.items()):
                if name.startswith("_") or not is_plot(value) or type(value).__module__.startswith("plotnine"):
                    continue
                pid = getattr(getattr(value, "figure", None) or value, "_corral_page", None)
                if self.plots.has(pid):
                    drawn[str(id(value))] = pid
            value = getattr(result, "result", None)
            if value is not None and is_plot(value) and len(self.plots.pages) > self.pages_before:
                drawn[str(id(value))] = self.plots.pages[-1]["id"]
            if drawn:
                self.remember(drawn)
        except Exception:
            pass
        try:
            self.snapshot()
        except Exception:
            pass

    # ---- figures

    def add_figure(self, fig, force: bool = False) -> str | None:
        """Show a figure in Mirador: a new page the first time, the same page when it changed.

        The page is remembered on the figure itself, so adding a title or a legend
        in a later line redraws that plot instead of making another one.
        """
        if id(fig) in self.rendered and not force:
            return self.rendered[id(fig)]
        pid = getattr(fig, "_corral_page", None)
        if self.plots.has(pid):
            if force or fig.stale:
                self.plots.update(pid, fig)
        else:
            pid = self.plots.add(fig)
            fig._corral_page = pid
        self.rendered[id(fig)] = pid
        return pid

    def flush_figures(self, close_all: bool = False) -> None:
        """Send the open pyplot figures that have something drawn to Mirador.

        The current figure stays open, so the next lines can keep adding to it
        (like Positron); older ones are closed. plt.show() closes them all.
        """
        plt = sys.modules.get("matplotlib.pyplot")
        if plt is None:
            return
        from matplotlib._pylab_helpers import Gcf
        active = Gcf.get_active()
        for manager in list(Gcf.get_all_fig_managers()):
            fig = manager.canvas.figure
            if has_content(fig):
                self.add_figure(fig)
            if close_all or manager is not active:
                plt.close(fig)

    def remember(self, pairs: dict[str, str]) -> None:
        """Which drawing each plot object made (by id), so the variables pane can show it again."""
        f = self.dir / "plotobj-map.json"
        try:
            drawn = json.loads(f.read_text())
        except (OSError, ValueError):
            drawn = {}
        drawn.update(pairs)
        write_json_atomic(f, drawn)

    def first_plot(self) -> None:
        if os.environ.get("CORRAL_OPEN_PLOTS", "1") == "0" or not in_herdr():
            return
        corral = corral_bin()
        if corral:
            env = dict(os.environ, CORRAL_SESSION=self.id)
            subprocess.Popen([corral, "_plot-ready"], env=env, stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)

    def close(self) -> None:
        self.plots.close()
        shutil.rmtree(self.dir, ignore_errors=True)
        shutil.rmtree(self.tmp, ignore_errors=True)


# --------------------------------------------------------------------------- what the user and agents call


def View(x, title: str | None = None) -> None:
    """Open a table (pandas, polars, pyarrow, a 2-D numpy array) in Corral's table viewer."""
    session = _require()
    if title is None:
        title = next((k for k, v in session.shell.user_ns.items() if v is x and not k.startswith("_")),
                     short_class(x))
    path = write_table(x, session.tmp)
    corral = corral_bin()
    if not corral:
        raise RuntimeError("`corral` no está en el PATH")
    args = [corral, "view", str(path), "--title", str(title), "--session", session.id]
    if in_herdr():
        subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, start_new_session=True)
    else:
        subprocess.run(args)


def write_table(x, folder: Path) -> Path:
    stamp = f"{time.strftime('%H%M%S')}-{secrets.token_hex(3)}"
    t = type(x)
    mod = t.__module__.split(".")[0]
    if mod == "polars":
        if t.__name__ == "LazyFrame":
            x = x.collect()
        if t.__name__ == "Series":
            x = x.to_frame()
        f = folder / f"{stamp}.parquet"
        x.write_parquet(f)
        return f
    if mod == "pyarrow":
        import pyarrow.parquet as pq
        f = folder / f"{stamp}.parquet"
        pq.write_table(x, f)
        return f
    if mod != "pandas":
        try:
            import pandas as pd
        except ImportError:
            raise TypeError("View() necesita un DataFrame de pandas, polars o pyarrow") from None
        x = pd.DataFrame(x)
    elif t.__name__ == "Series":
        x = x.to_frame()
    # Keep a meaningful index (names, dates) as a column.
    if type(x.index).__name__ != "RangeIndex" or x.index.name:
        x = x.reset_index()
    x.columns = [str(c) for c in x.columns]
    f = folder / f"{stamp}.parquet"
    try:
        x.to_parquet(f, index=False)
    except Exception:  # no pyarrow/fastparquet, or types parquet can't hold
        f = folder / f"{stamp}.csv"
        x.to_csv(f, index=False)
    return f


def show_plot(obj) -> None:
    """Draw a plot object again for Mirador (Enter on it in the variables pane)."""
    session = _require()
    fig, created = figure_of(obj)
    session.rendered.pop(id(fig), None)
    pid = session.add_figure(fig, force=True)
    if pid is not None:
        session.remember({str(id(obj)): pid})
    if created:
        plt = sys.modules.get("matplotlib.pyplot")
        if plt:
            plt.close(fig)


class _Tee(io.TextIOBase):
    def __init__(self, stream, sink) -> None:
        self.stream, self.sink = stream, sink

    def write(self, s: str) -> int:
        self.stream.write(s)
        if self.sink is not None:  # something may keep this stream after the eval ends
            self.sink.write(ANSI.sub("", s))
        return len(s)

    def flush(self) -> None:
        self.stream.flush()
        if self.sink is not None:
            self.sink.flush()

    def isatty(self) -> bool:
        return self.stream.isatty()

    @property
    def encoding(self):
        return getattr(self.stream, "encoding", "utf-8")

    def fileno(self) -> int:
        return self.stream.fileno()


def corral_eval(rid: str) -> None:
    """Run code sent by `corral eval` in this namespace, echoed, and record the outcome."""
    session = _require()
    folder = session.dir / "eval"
    path = lambda ext: folder / f"{rid}{ext}"  # noqa: E731
    if not path(".py").exists():
        return
    status = {"status": "ok", "error": None, "warnings": []}

    def done() -> None:
        status["finished"] = time.time()
        write_json_atomic(path(".json"), status)

    try:
        deadline = float(path(".deadline").read_text())
    except (OSError, ValueError):
        deadline = None
    if deadline and time.time() > deadline:
        print("# corral eval: la petición venció antes de empezar; no se ejecutó")
        status["status"] = "expired"
        return done()

    code = path(".py").read_text(encoding="utf-8")
    print("# corral eval")
    with open(path(".out"), "w", encoding="utf-8") as sink:
        out, err = sys.stdout, sys.stderr
        tees = _Tee(out, sink), _Tee(err, sink)
        sys.stdout, sys.stderr = tees
        try:
            for i, line in enumerate(code.rstrip("\n").splitlines()):
                cont = i > 0 and (not line.strip() or line[:1] in " \t)]}" or line.lstrip().startswith(
                    ("else", "elif", "except", "finally", "case ")))
                print(("... " if cont else ">>> ") + line)
            result = session.shell.run_cell(code, store_history=False)
            error = result.error_before_exec or result.error_in_exec
            if error is not None:
                status["status"] = "error"
                status["error"] = f"{type(error).__name__}: {error}"
        finally:
            sys.stdout.flush()
            sys.stdout, sys.stderr = out, err
            for tee in tees:
                tee.sink = None
    done()


def flush_figures() -> None:
    """plt.show(): the figures go to Mirador and are closed."""
    if _session:
        _session.flush_figures(close_all=True)


def _require() -> Session:
    if _session is None:
        raise RuntimeError("Corral no está activo en este Python")
    return _session


# --------------------------------------------------------------------------- start


def activate(shell=None) -> None:
    global _session
    if _session is not None:
        print("Corral ya está activo en esta sesión.")
        return
    if shell is None:
        try:
            from IPython import get_ipython
            shell = get_ipython()
        except ImportError:
            shell = None
    if shell is None:
        print("Corral necesita IPython: corral lo abre con `uv run --with ipython ipython`.")
        return

    # IPython takes the startup script's folder off sys.path when the script ends,
    # and matplotlib imports corral_mpl later: keep this folder on the path.
    sys.path.append(os.path.dirname(os.path.abspath(__file__)))

    # Figures go to Mirador; set before matplotlib is imported, or switch now.
    os.environ["MPLBACKEND"] = "module://corral_mpl"
    if "matplotlib.pyplot" in sys.modules:
        sys.modules["matplotlib.pyplot"].switch_backend("module://corral_mpl")

    _session = Session(shell)
    shell.events.register("pre_run_cell", _session.before_cell)
    shell.events.register("post_run_cell", _session.after_cell)

    # A figure as the last value of a cell goes to Mirador; the console says which.
    def figure_text(fig, p, cycle):
        pid = _session.add_figure(fig)
        n = _session.plots.number(pid) if pid is not None else 0
        p.text(f"<Figure en Mirador{f' (gráfico {n})' if n else ''}>")

    try:
        shell.display_formatter.formatters["text/plain"].for_type_by_name("matplotlib.figure", "Figure", figure_text)
    except Exception:
        pass

    helpers = {"View": View, "_corral_eval": corral_eval, "_corral_show": show_plot}
    shell.user_ns.update(helpers)
    shell.user_ns_hidden.update(helpers)
    _session.snapshot()
    venv = os.environ.get("VIRTUAL_ENV") or (sys.prefix if sys.prefix != sys.base_prefix else "")
    where = f" · {Path(venv).name}" if venv and "/.cache/" not in venv else ""
    print(f"Corral activo · Python {platform.python_version()}{where}")
