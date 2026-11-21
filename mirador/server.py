#!/usr/bin/env python3
"""Mirador: local web view for Corral.

Serves one page (index.html) and a small JSON API over 127.0.0.1:
agents from herdr, R sessions and their variables (written by r/corral.R),
plots proxied from httpgd, and tables queried with the DuckDB CLI.
Every request needs the token stored in ~/.local/state/corral/mirador.json.
Standard library only.
"""

from __future__ import annotations

import hmac
import json
import os
import re
import secrets
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

VERSION = "0.1.0"
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / "lib"))
STATE_DIR = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local" / "state") / "corral"
SESSIONS_DIR = STATE_DIR / "sessions"
INFO_FILE = STATE_DIR / "mirador.json"
TABLES_FILE = STATE_DIR / "mirador-tables.json"
DEFAULT_PORT = int(os.environ.get("CORRAL_MIRADOR_PORT", "7317"))
ACTIVE_WINDOW_S = 6.0
MAX_PAGE = 500


# --------------------------------------------------------------------------- helpers


def pid_alive(pid: int | None) -> bool:
    if not pid:
        return False
    try:
        os.kill(int(pid), 0)
    except (OSError, ValueError):
        return False
    return True


def read_json(path: Path, default):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def herdr(*args: str):
    hb = os.environ.get("HERDR_BIN_PATH") or shutil.which("herdr")
    if not hb:
        return None
    try:
        proc = subprocess.run([hb, *args], capture_output=True, text=True, timeout=4)
        data = json.loads(proc.stdout or "{}")
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0 or "error" in data:
        return None
    return data.get("result", data)


def sql_lit(s: str) -> str:
    return "'" + str(s).replace("'", "''") + "'"


def sql_ident(s: str) -> str:
    return '"' + str(s).replace('"', '""') + '"'


def duck(sql: str) -> list[dict]:
    duckdb = shutil.which("duckdb")
    if not duckdb:
        raise RuntimeError("DuckDB no está en el PATH")
    proc = subprocess.run([duckdb, "-json", "-c", sql], capture_output=True, text=True, timeout=60)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or "DuckDB falló")
    out = proc.stdout.strip()
    return json.loads(out) if out else []


FONT_FALLBACKS = {
    "serif": "'Liberation Serif', 'Times New Roman', Georgia, serif",
    "mono": "'Liberation Mono', 'Cascadia Mono', Consolas, monospace",
    "sans": "'Liberation Sans', Arial, 'Segoe UI', Helvetica, sans-serif",
}


def fix_svg_fonts(svg: bytes) -> bytes:
    """httpgd names fonts installed in WSL (Liberation *); give Windows browsers fallbacks."""
    def repl(m: re.Match) -> str:
        fam = m.group(1).strip().strip("'\"")
        low = fam.lower()
        kind = "mono" if "mono" in low or "courier" in low else "serif" if "serif" in low and "sans" not in low else "sans"
        return f"font-family: '{fam}', {FONT_FALLBACKS[kind]}"
    return re.sub(r"font-family:\s*([^;\"<>]+)", repl, svg.decode("utf-8", "replace")).encode()


def table_source(path: str) -> str:
    low = path.lower()
    if low.endswith(".parquet"):
        return f"read_parquet({sql_lit(path)})"
    if low.endswith(".json") or low.endswith(".ndjson"):
        return f"read_json_auto({sql_lit(path)})"
    return f"read_csv_auto({sql_lit(path)})"


# --------------------------------------------------------------------------- state


class Mirador:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.last_client = 0.0
        self.agents_cache: tuple[float, dict] = (0.0, {})
        self.agent_since: dict[str, tuple[str, float]] = {}
        self.tables: list[dict] = [t for t in read_json(TABLES_FILE, []) if Path(t.get("path", "")).is_file()]
        self.schema_cache: dict[str, list[dict]] = {}
        self.show_plot: dict | None = None

    # ---- agents (herdr)

    def agents(self) -> dict:
        now = time.time()
        with self.lock:
            stamp, cached = self.agents_cache
            if now - stamp < 1.0:
                return cached
        listed = herdr("agent", "list")
        spaces = herdr("workspace", "list")
        if listed is None:
            data = {"available": False, "agents": [], "workspaces": []}
        else:
            ws = {w["workspace_id"]: w for w in (spaces or {}).get("workspaces", [])}
            agents = []
            for a in listed.get("agents", []):
                key = f"{a.get('pane_id')}|{a.get('agent')}"
                status = a.get("agent_status") or "unknown"
                prev = self.agent_since.get(key)
                since = prev[1] if prev and prev[0] == status else now
                self.agent_since[key] = (status, since)
                agents.append({
                    "pane_id": a.get("pane_id"),
                    "kind": a.get("agent") or "",
                    "name": a.get("name") or a.get("display_agent") or a.get("agent") or "agente",
                    "title": a.get("terminal_title_stripped") or a.get("title") or a.get("terminal_title") or "",
                    "status": status,
                    "focused": bool(a.get("focused")),
                    "workspace_id": a.get("workspace_id"),
                    "workspace": (ws.get(a.get("workspace_id")) or {}).get("label") or a.get("workspace_id"),
                    "since": since,
                })
            data = {
                "available": True,
                "agents": agents,
                "workspaces": [
                    {"id": w["workspace_id"], "label": w.get("label"), "focused": bool(w.get("focused"))}
                    for w in ws.values()
                ],
            }
        with self.lock:
            self.agents_cache = (now, data)
        return data

    # ---- R sessions (written by r/corral.R)

    def sessions(self) -> list[dict]:
        out = []
        if not SESSIONS_DIR.is_dir():
            return out
        for d in sorted(SESSIONS_DIR.iterdir()):
            meta = read_json(d / "meta.json", None)
            if not meta:
                continue
            if not pid_alive(meta.get("pid")):
                shutil.rmtree(d, ignore_errors=True)
                continue
            vars_file = d / "vars.json"
            meta["vars_rev"] = vars_file.stat().st_mtime if vars_file.exists() else None
            meta["plots"] = self.httpgd(meta["id"], "state")
            out.append(meta)
        return out

    def session_dir(self, sid: str) -> Path:
        if not re.fullmatch(r"[A-Za-z0-9._-]+", sid or ""):
            raise ValueError("sesión inválida")
        return SESSIONS_DIR / sid

    def vars(self, sid: str) -> dict:
        return read_json(self.session_dir(sid) / "vars.json", {"vars": []})

    # ---- plots (httpgd)

    def httpgd_base(self, sid: str) -> tuple[str, str] | None:
        try:
            url = (self.session_dir(sid) / "plots-url").read_text().strip()
        except (OSError, ValueError):
            return None
        parsed = urllib.parse.urlparse(url)
        if parsed.hostname not in ("127.0.0.1", "localhost"):
            return None
        token = urllib.parse.parse_qs(parsed.query).get("token", [""])[0]
        return f"{parsed.scheme}://{parsed.netloc}", token

    def httpgd_raw(self, sid: str, path: str, params: dict | None = None) -> tuple[bytes, str] | None:
        base = self.httpgd_base(sid)
        if not base:
            return None
        url, token = base
        query = dict(params or {})
        if token:
            query["token"] = token
        try:
            with urllib.request.urlopen(f"{url}/{path}?{urllib.parse.urlencode(query)}", timeout=10) as resp:
                return resp.read(), resp.headers.get("Content-Type", "application/octet-stream")
        except OSError:
            return None

    def httpgd(self, sid: str, path: str, params: dict | None = None):
        raw = self.httpgd_raw(sid, path, params)
        if not raw:
            return None
        try:
            return json.loads(raw[0])
        except ValueError:
            return None

    # ---- tables (DuckDB)

    def add_table(self, path: str, title: str, session: str | None) -> dict:
        p = Path(path).expanduser().resolve()
        if not p.is_file():
            raise ValueError(f"no existe {p}")
        with self.lock:
            # A new View() of the same object replaces its tab (new id, so the page opens it).
            def same(t: dict) -> bool:
                return t["path"] == str(p) or (session and t.get("session") == session and t["title"] == title)
            for old in [t for t in self.tables if same(t)]:
                self.schema_cache.pop(old["id"], None)
            self.tables = [t for t in self.tables if not same(t)]
            entry = {
                "id": secrets.token_hex(4),
                "path": str(p),
                "title": title or p.name,
                "session": session,
                "added": time.time(),
            }
            self.tables.append(entry)
            self._save_tables()
            return entry

    def close_table(self, tid: str) -> None:
        with self.lock:
            self.tables = [t for t in self.tables if t["id"] != tid]
            self.schema_cache.pop(tid, None)
            self._save_tables()

    def _save_tables(self) -> None:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        TABLES_FILE.write_text(json.dumps(self.tables))

    def table(self, tid: str) -> dict:
        for t in self.tables:
            if t["id"] == tid:
                if not Path(t["path"]).is_file():
                    raise ValueError("el archivo ya no existe (¿se cerró la sesión de R?)")
                return t
        raise ValueError("tabla desconocida")

    def schema(self, tid: str) -> list[dict]:
        if tid not in self.schema_cache:
            src = table_source(self.table(tid)["path"])
            cols = duck(f"DESCRIBE SELECT * FROM {src}")
            self.schema_cache[tid] = [{"name": c["column_name"], "type": c["column_type"]} for c in cols]
        return self.schema_cache[tid]

    def where(self, tid: str, q: str) -> str:
        q = (q or "").strip()
        if not q:
            return ""
        pattern = "%" + q.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_") + "%"
        parts = [f"CAST({sql_ident(c['name'])} AS VARCHAR) ILIKE {sql_lit(pattern)} ESCAPE '\\'" for c in self.schema(tid)]
        return " WHERE " + " OR ".join(parts) if parts else ""

    def rows(self, tid: str, offset: int, limit: int, sort: str, desc: bool, q: str) -> dict:
        t = self.table(tid)
        cols = self.schema(tid)
        src = table_source(t["path"])
        where = self.where(tid, q)
        order = ""
        if sort and any(c["name"] == sort for c in cols):
            order = f" ORDER BY {sql_ident(sort)} {'DESC' if desc else 'ASC'} NULLS LAST"
        limit = max(1, min(int(limit), MAX_PAGE))
        offset = max(0, int(offset))
        rows = duck(f"SELECT * FROM {src}{where}{order} LIMIT {limit} OFFSET {offset}")
        total = duck(f"SELECT count(*) AS n FROM {src}{where}")[0]["n"]
        return {"columns": cols, "rows": rows, "total": total, "offset": offset}

    def summary(self, tid: str) -> list[dict]:
        return duck(f"SUMMARIZE SELECT * FROM {table_source(self.table(tid)['path'])}")

    def palette(self) -> dict | None:
        try:
            import corral_theme
            corral_theme.sync()
            return corral_theme.load()
        except Exception:
            return None

    def state(self) -> dict:
        with self.lock:
            # R deletes its temp files on exit; drop tables that went with it,
            # or whose session is gone (R killed before it could clean up).
            alive = [t for t in self.tables if Path(t["path"]).is_file()
                     and (not t.get("session") or (SESSIONS_DIR / t["session"]).is_dir())]
            if len(alive) != len(self.tables):
                self.tables = alive
                self._save_tables()
            tables = list(self.tables)
        return {
            "version": VERSION,
            "now": time.time(),
            **self.agents(),
            "sessions": self.sessions(),
            "tables": [{k: t[k] for k in ("id", "title", "session", "added")} for t in tables],
            "show_plot": self.show_plot,
            "palette": self.palette(),
        }


def themed_page() -> bytes:
    """index.html with herdr's palette (written by `corral theme`) as CSS variables."""
    page = (ROOT / "index.html").read_text(encoding="utf-8")
    try:
        import corral_theme
        p = corral_theme.load()
    except Exception:
        p = None
    if p:
        css = {"--bg": p["bg"], "--panel": p["surface"], "--side": p["surface"], "--side-2": p["raised"],
               "--line": p["border"], "--line-2": p["selection"], "--text": p["text"], "--muted": p["muted"],
               "--dim": p["dim"], "--accent": p["accent"], "--green": p["green"], "--amber": p["yellow"],
               "--red": p["red"], "--blue": p["blue"], "--violet": p["violet"]}
        style = "<style>:root{" + ";".join(f"{k}:{v}" for k, v in css.items()) + "}</style>"
        page = page.replace("</head>", style + "\n</head>", 1)
    return page.encode("utf-8")


# --------------------------------------------------------------------------- HTTP


class Handler(BaseHTTPRequestHandler):
    server_version = f"Mirador/{VERSION}"
    app: Mirador
    token: str
    port: int

    def log_message(self, fmt, *args):  # quiet unless debugging
        if os.environ.get("CORRAL_DEBUG"):
            super().log_message(fmt, *args)

    # ---- plumbing

    def _send(self, code: int, body: bytes, ctype: str, extra: dict | None = None) -> None:
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def _json(self, data, code: int = 200) -> None:
        self._send(code, json.dumps(data, default=str).encode(), "application/json; charset=utf-8")

    def _error(self, code: int, msg: str) -> None:
        self._json({"error": msg}, code)

    def _authorized(self, query: dict) -> bool:
        host = (self.headers.get("Host") or "").lower()
        if host not in (f"127.0.0.1:{self.port}", f"localhost:{self.port}"):
            return False
        given = self.headers.get("X-Corral-Token") or query.get("token", [""])[0]
        return hmac.compare_digest(given, self.token)

    def _body(self) -> dict:
        length = int(self.headers.get("Content-Length") or 0)
        if length > 1_000_000:
            raise ValueError("cuerpo demasiado grande")
        raw = self.rfile.read(length) if length else b"{}"
        return json.loads(raw or b"{}")

    # ---- routes

    def do_GET(self) -> None:
        url = urllib.parse.urlparse(self.path)
        q = urllib.parse.parse_qs(url.query)
        arg = lambda k, d="": q.get(k, [d])[0]  # noqa: E731
        if not self._authorized(q):
            return self._error(403, "token inválido; abre Mirador con `corral mirador`")
        try:
            if url.path in ("/", "/index.html"):
                return self._send(200, themed_page(), "text/html; charset=utf-8")
            if url.path == "/api/ping":
                # Only /api/state (the page polling) counts as an open window.
                active = time.time() - self.app.last_client < ACTIVE_WINDOW_S
                return self._json({"ok": True, "version": VERSION, "active": active, "pid": os.getpid()})
            if url.path == "/api/state":
                self.app.last_client = time.time()
                return self._json(self.app.state())
            if url.path == "/api/vars":
                return self._json(self.app.vars(arg("session")))
            if url.path == "/api/plots":
                return self._json(self.app.httpgd(arg("session"), "plots") or {"plots": []})
            if url.path == "/api/plot":
                params = {"width": arg("width", "720"), "height": arg("height", "576"),
                          "renderer": arg("renderer", "svg"), "zoom": arg("zoom", "1")}
                if arg("id"):
                    params["id"] = arg("id")
                else:
                    params["index"] = arg("index", "-1")
                raw = self.app.httpgd_raw(arg("session"), "plot", params)
                if not raw:
                    return self._error(404, "gráfico no disponible")
                body, ctype = raw
                if "svg" in ctype:
                    body = fix_svg_fonts(body)
                extra = {}
                if arg("download"):
                    ext = "png" if params["renderer"] == "png" else "svg"
                    extra["Content-Disposition"] = f'attachment; filename="grafico.{ext}"'
                return self._send(200, body, ctype, extra)
            if url.path == "/api/table/rows":
                return self._json(self.app.rows(arg("id"), int(arg("offset", "0")), int(arg("limit", "200")),
                                                 arg("sort"), arg("desc") == "1", arg("q")))
            if url.path == "/api/table/summary":
                return self._json({"columns": self.app.summary(arg("id"))})
            return self._error(404, "no encontrado")
        except (ValueError, RuntimeError) as exc:
            return self._error(400, str(exc))

    def do_POST(self) -> None:
        url = urllib.parse.urlparse(self.path)
        q = urllib.parse.parse_qs(url.query)
        if not self._authorized(q):
            return self._error(403, "token inválido")
        try:
            body = self._body()
            if url.path == "/api/tables":
                entry = self.app.add_table(body.get("path", ""), body.get("title", ""), body.get("session"))
                return self._json(entry)
            if url.path == "/api/show-plot":
                # Corral asks the open page to jump to a plot (from the variables pane).
                self.app.session_dir(body.get("session", ""))  # validates the id
                with self.app.lock:
                    seq = (self.app.show_plot or {}).get("seq", 0) + 1
                    self.app.show_plot = {"session": body["session"], "id": str(body.get("id", "")), "seq": seq}
                return self._json({"ok": True, "seq": seq})
            if url.path == "/api/opened":
                # corral just opened a window: count it as active while the page loads.
                self.app.last_client = time.time()
                return self._json({"ok": True})
            if url.path == "/api/tables/close":
                self.app.close_table(body.get("id", ""))
                return self._json({"ok": True})
            if url.path == "/api/agents/focus":
                pane = body.get("pane_id", "")
                if not re.fullmatch(r"[A-Za-z0-9:_-]+", pane):
                    raise ValueError("panel inválido")
                ok = herdr("agent", "focus", pane) is not None
                return self._json({"ok": ok})
            if url.path == "/api/view-var":
                sid, name = body.get("session", ""), body.get("name", "")
                meta = read_json(self.app.session_dir(sid) / "meta.json", {})
                known = {v.get("name") for v in self.app.vars(sid).get("vars", [])}
                if name not in known:
                    raise ValueError("objeto desconocido")
                if not meta.get("pane_id"):
                    raise ValueError("la sesión no está en un panel de herdr")
                code = "View(`" + name.replace("`", "\\`") + "`)"
                ok = herdr("pane", "run", meta["pane_id"], code) is not None
                return self._json({"ok": ok, "code": code})
            if url.path in ("/api/plots/remove", "/api/plots/clear"):
                sid = body.get("session", "")
                path = "clear" if url.path.endswith("clear") else "remove"
                params = {} if path == "clear" else {"index": str(body.get("index", -1))}
                return self._json({"ok": self.app.httpgd_raw(sid, path, params) is not None})
            return self._error(404, "no encontrado")
        except (ValueError, RuntimeError) as exc:
            return self._error(400, str(exc))


def load_token() -> str:
    info = read_json(INFO_FILE, {})
    return info.get("token") or secrets.token_urlsafe(18)


def serve() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    token = load_token()
    app = Mirador()
    try:
        httpd = ThreadingHTTPServer(("127.0.0.1", DEFAULT_PORT), Handler)
    except OSError:
        httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    httpd.daemon_threads = True
    port = httpd.server_address[1]
    Handler.app, Handler.token, Handler.port = app, token, port

    info = {"url": f"http://127.0.0.1:{port}/?token={token}", "port": port, "token": token,
            "pid": os.getpid(), "started": time.time(), "version": VERSION}
    tmp = INFO_FILE.with_suffix(".tmp")
    tmp.write_text(json.dumps(info))
    os.chmod(tmp, 0o600)
    tmp.replace(INFO_FILE)

    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=httpd.shutdown, daemon=True).start())
    print(f"Mirador en {info['url']}", flush=True)
    try:
        httpd.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()


if __name__ == "__main__":
    sys.exit(serve())
