// Corral for Fresh: run R code from the editor in the shared console (pane corral:r).
//
//   Ctrl+R (or Ctrl+Enter)    selection, or the whole statement under the cursor
//                             (multi-line pipes and ggplot chains included), then
//                             move to the next statement
//   F5 (or Ctrl+Shift+Enter)  the whole file, with source(echo = TRUE)
//   Ctrl+L (or Alt+-)         " <- "
//   Ctrl+U (or Ctrl+Shift+M)  " |> "
//   Ctrl+S                    save; in the temporary scratch file, ask for a name in the project
//
// Messages and the palette's names follow Corral's language (English or Spanish).
//
// Code from a .py file goes to the Python console and code from a .R file to
// the R console; corral brings the right one into view (and starts Python the
// first time). Other files go to whichever console is showing.
//
// Through Warp and herdr, Ctrl+Enter arrives as plain Enter and Alt chords get
// split, so the first key of each pair is the one that works there.
// Keys are bound in ~/.config/fresh/config.json (see corral-term/fresh/config.json).

const editor = getEditor();

// Corral's language: CORRAL_LANG (Corral sets it when it opens the editor), else
// `language` in ~/.config/corral/config.json (`corral config language`), else English.
let langCache: [number, string] = [-Infinity, "en"];

function normalizeLang(value: unknown): string | null {
  const v = String(value || "").trim().toLowerCase();
  if (["en", "english", "inglés", "ingles"].includes(v)) return "en";
  if (["es", "spanish", "español", "espanol"].includes(v)) return "es";
  return null;
}

function lang(): string {
  const now = Date.now();
  if (now - langCache[0] < 2000) return langCache[1];
  let value = normalizeLang(editor.getEnv("CORRAL_LANG"));
  if (!value) {
    const config = editor.getEnv("XDG_CONFIG_HOME") || (editor.getEnv("HOME") || "") + "/.config";
    try {
      const text = editor.readFile(config + "/corral/config.json");
      if (text) value = normalizeLang(JSON.parse(text).language);
    } catch (e) {
      // No settings, or not readable: English.
    }
  }
  langCache = [now, value || "en"];
  return langCache[1];
}

// The text in the user's language.
function tr(en: string, es: string): string {
  return lang() === "es" ? es : en;
}

// Corral scratch files: <project>/.corral/<name>/draft.R (borrador.R in Spanish),
// or draft.py when the layout starts with Python, or the temp folder.
const SCRATCH_NAME = /^(draft|borrador)\.(R|py)$/;

function isScratch(path: string): boolean {
  return /\/(\.corral|corral\/drafts)\/[^/]+\/(draft|borrador)\.(R|py)$/.test(path || "");
}

// Any file inside a scratch folder (git-ignored, removed after two days).
function inScratchFolder(path: string): boolean {
  return /\/(\.corral|corral\/drafts)\/[^/]+\/[^/]+$/.test(path || "");
}

// The project a scratch file belongs to: the folder holding .corral/, else Fresh's folder.
function projectOf(path: string): string {
  const i = (path || "").indexOf("/.corral/");
  return i > 0 ? path.slice(0, i) : editor.getCwd();
}
const MAX_LINES = 400;

// Code without strings and comments, enough to count brackets and see how a line ends.
function codeOnly(line: string): string {
  let out = "";
  let quote: string | null = null;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quote) {
      if (c === "\\") { i++; continue; }
      if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") { quote = c; out += " "; continue; }
    if (c === "#") break;
    out += c;
  }
  return out;
}

function depthChange(code: string): number {
  let d = 0;
  for (const c of code) {
    if (c === "(" || c === "[" || c === "{") d++;
    else if (c === ")" || c === "]" || c === "}") d--;
  }
  return d;
}

// A line ending like this continues on the next one.
const CONTINUES = /(\+|-|\*|\/|\^|&|\||,|~|=|<-|\|>|%[^%\s]*%|\(|\[|\{)\s*$/;
// A line starting like this continues the previous one.
const CONTINUATION = /^\s*(\)|\]|\}|\|>|%[^%\s]*%|\+|&|\||,)/;

function isBlank(line: string): boolean {
  return codeOnly(line).trim() === "";
}

// [first, last] lines of the top-level statement that contains `line`.
function statementAt(lines: string[], line: number): [number, number] {
  const depth: number[] = [0];
  for (let i = 0; i < lines.length; i++) depth.push(depth[i] + depthChange(codeOnly(lines[i])));

  let first = line;
  while (first > 0 && line - first < MAX_LINES) {
    const prev = codeOnly(lines[first - 1]);
    const inside = depth[first] > 0;
    if (inside || CONTINUES.test(prev) || CONTINUATION.test(codeOnly(lines[first]))) first--;
    else break;
  }
  let last = line;
  while (last < lines.length - 1 && last - first < MAX_LINES) {
    const insideAfter = depth[last + 1] > depth[first];
    const next = lines[last + 1];
    if (insideAfter || CONTINUES.test(codeOnly(lines[last])) || CONTINUATION.test(codeOnly(next))) last++;
    else break;
  }
  return [first, last];
}

// Indentation after Enter, the way Air writes R: one level inside an open
// bracket (counted from the line that opened it), one level for the lines that
// continue a pipe or operator chain, and back to the start of the statement
// once it is complete. Fresh's own rules can't express the last two.
const OPERATOR_END = /(\|>|%[^%\s]*%|\+|-|\*|\/|\^|&|\||~|=)\s*$/;
const INDENT_UNIT = 2;

function leading(line: string): number {
  return line.length - line.replace(/^[ \t]+/, "").length;
}

// Columns for the line after `p` (the last non-blank line before the cursor).
function indentAfter(lines: string[], p: number): number {
  const [first] = statementAt(lines.slice(0, p + 1), p);
  const open: number[] = [];
  for (let i = first; i <= p; i++) {
    for (const c of codeOnly(lines[i])) {
      if (c === "(" || c === "[" || c === "{") open.push(leading(lines[i]));
      else if ((c === ")" || c === "]" || c === "}") && open.length) open.pop();
    }
  }
  const continues = OPERATOR_END.test(codeOnly(lines[p]));
  const base = open.length ? open[open.length - 1] + INDENT_UNIT : leading(lines[first]);
  return base + (continues ? INDENT_UNIT : 0);
}

let fixingIndent = false;
registerHandler("corral_indent", async function (args: { buffer_id: number; text: string }) {
  if (fixingIndent) return;
  try {
    // Only a plain Enter: a newline plus the indentation Fresh chose.
    if (!/^\r?\n[ \t]*$/.test(args.text || "")) return;
    const id = args.buffer_id;
    if (id !== editor.getActiveBufferId()) return;
    if (!/\.[Rr]$/.test(editor.getBufferPath(id) || "")) return;
    const cursor = editor.getCursorPosition();
    const lines = (await editor.getBufferText(id, 0, cursor)).split("\n");
    const current = lines[lines.length - 1];
    if (!/^[ \t]*$/.test(current)) return;
    let p = lines.length - 2;
    while (p >= 0 && isBlank(lines[p])) p--;
    if (p < 0) return;
    const want = " ".repeat(indentAfter(lines, p));
    if (want === current) return;
    fixingIndent = true;
    const start = cursor - current.length;
    if (await editor.replaceRange(id, start, cursor, want)) editor.setBufferCursor(id, start + want.length);
  } catch (e) {
    // Leave Fresh's indentation as it is.
  } finally {
    fixingIndent = false;
  }
});
editor.on("after_insert", "corral_indent");

// Which console a file's code goes to; null means the one showing.
function langOf(path: string): string | null {
  if (/\.py$/i.test(path)) return "python";
  if (/\.sql$/i.test(path)) return "sql";
  if (/\.(R|r|Rprofile)$/.test(path)) return "r";
  return null;
}

function langArgs(buf: number): string[] {
  const lang = langOf(editor.getBufferPath(buf) || "");
  return lang ? ["--lang", lang] : [];
}

// SQL: the lines where a statement ends, at a ; outside strings, quoted names and
// comments (a string can span lines).
function sqlEnds(lines: string[]): number[] {
  const ends: number[] = [];
  let quote: string | null = null;
  let block = false;
  for (let n = 0; n < lines.length; n++) {
    const l = lines[n];
    let ended = false;
    for (let i = 0; i < l.length; i++) {
      const c = l[i];
      if (block) {
        if (c === "*" && l[i + 1] === "/") { block = false; i++; }
        continue;
      }
      if (quote) {
        if (c === quote) quote = null;
        continue;
      }
      if (c === "'" || c === '"') { quote = c; continue; }
      if (c === "-" && l[i + 1] === "-") break;
      if (c === "/" && l[i + 1] === "*") { block = true; i++; continue; }
      if (c === ";") ended = true;
    }
    if (ended) ends.push(n);
  }
  return ends;
}

// [first, last] lines of the SQL statement at `line`: from the line after the
// previous ; to the line with its own, comments above it included (-- name: x).
function sqlStatementAt(lines: string[], line: number): [number, number] {
  const ends = sqlEnds(lines);
  const before = ends.filter((e) => e < line);
  let first = before.length ? before[before.length - 1] + 1 : 0;
  const after = ends.find((e) => e >= line);
  let last = after === undefined ? lines.length - 1 : after;
  while (first < last && lines[first].trim() === "") first++;
  while (last > first && lines[last].trim() === "") last--;
  return [first, last];
}

// Python: brackets and triple-quoted strings carried from one line to the next.
function pyScan(raw: string, state: { depth: number; triple: string | null }): void {
  let i = 0;
  while (i < raw.length) {
    if (state.triple) {
      const end = raw.indexOf(state.triple, i);
      if (end < 0) return;
      i = end + 3;
      state.triple = null;
      continue;
    }
    const c = raw[i];
    if (c === "#") return;
    if (raw.startsWith('"""', i) || raw.startsWith("'''", i)) {
      state.triple = raw.substr(i, 3);
      i += 3;
      continue;
    }
    if (c === '"' || c === "'") {
      let j = i + 1;
      while (j < raw.length && raw[j] !== c) j += raw[j] === "\\" ? 2 : 1;
      i = j + 1;
      continue;
    }
    if (c === "(" || c === "[" || c === "{") state.depth++;
    else if (c === ")" || c === "]" || c === "}") state.depth = Math.max(0, state.depth - 1);
    i++;
  }
}

// [first, last] lines of the top-level Python statement that contains `line`:
// a def, class, loop or with block with everything indented under it, a call
// spread over several lines, a decorated function, an if with its elif/else.
function pyStatementAt(lines: string[], line: number): [number, number] {
  const starts: boolean[] = [];
  const state = { depth: 0, triple: null as string | null };
  let continued = false;
  let decorated = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const blank = raw.trim() === "" || raw.trim().startsWith("#");
    starts.push(!state.triple && state.depth === 0 && !continued && !decorated && !blank &&
      !/^\s/.test(raw) && !/^[)\]}]/.test(raw) && !/^(else|elif|except|finally)\b/.test(raw));
    pyScan(raw, state);
    if (!blank) {
      continued = !state.triple && /\\\s*$/.test(raw);
      decorated = /^@/.test(raw);
    }
  }
  let first = line;
  while (first > 0 && !starts[first]) first--;
  let next = line + 1;
  while (next < lines.length && !starts[next]) next++;
  let last = next - 1;
  while (last > line && (lines[last].trim() === "" || /^#/.test(lines[last]))) last--;
  return [first, last];
}

// A selection from inside a block, without its common indentation.
function dedent(text: string): string {
  const lines = text.split("\n");
  const widths = lines.filter((l) => l.trim()).map((l) => l.length - l.replace(/^[ \t]+/, "").length);
  const cut = widths.length ? Math.min(...widths) : 0;
  return cut ? lines.map((l) => l.slice(Math.min(cut, l.length - l.replace(/^[ \t]+/, "").length))).join("\n") : text;
}

async function runCorral(args: string[]): Promise<boolean> {
  const res = await editor.spawnProcess("corral", args);
  if (res.exit_code !== 0) {
    const msg = (res.stderr || res.stdout || "").trim().split("\n").pop() || tr("corral failed", "corral falló");
    editor.setStatus("Corral: " + msg.replace(/^corral:\s*/, ""));
    return false;
  }
  return true;
}

registerHandler("corral_run_line", async function () {
  try {
    const buf = editor.getActiveBufferId();
    const cur = editor.getPrimaryCursor();
    if (!cur) return;

    const lang = langArgs(buf);
    const python = lang[1] === "python";
    const sql = lang[1] === "sql";
    // In SQL a comment above a statement belongs to it (-- name: x): only empty lines are skipped.
    const blank = sql ? (l: string) => l.trim() === "" : isBlank;
    if (cur.selection && cur.selection.end > cur.selection.start) {
      let text = await editor.getBufferText(buf, cur.selection.start, cur.selection.end);
      if (python) text = dedent(text);
      if (text.trim() && (await runCorral(["send", ...lang, "--", text])))
        editor.setStatus(tr("Corral: selection sent to the console", "Corral: selección enviada a la consola"));
      return;
    }
    if (cur.line === null) return;

    const lines = (await editor.getBufferText(buf)).split("\n");
    let line = cur.line;
    // On a blank or comment line, run the next statement (like Positron).
    while (line < lines.length - 1 && blank(lines[line])) line++;
    if (blank(lines[line])) return;

    const [first, last] = sql ? sqlStatementAt(lines, line) : python ? pyStatementAt(lines, line) : statementAt(lines, line);
    const text = lines.slice(first, last + 1).join("\n");
    const ok = await runCorral(["send", ...lang, "--", text]);

    let next = last + 1;
    while (next < lines.length && blank(lines[next])) next++;
    if (next >= lines.length) next = Math.min(last + 1, lines.length - 1);
    const pos = await editor.getLineStartPosition(next);
    if (pos !== null) editor.setBufferCursor(buf, pos);
    if (ok) {
      const n = last - first + 1;
      const s = n === 1 ? "" : "s";
      editor.setStatus(tr(`Corral: ${n} line${s} sent to the console`, `Corral: ${n} línea${s} enviada${s} a la consola`));
    }
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

registerHandler("corral_run_file", async function () {
  try {
    const buf = editor.getActiveBufferId();
    const text = await editor.getBufferText(buf);
    if (!text.trim()) return;
    const path = editor.getBufferPath(buf) || "script";
    const lang = langArgs(buf);
    // A .sql file goes as one corral_sql() call; R and Python files are run as a copy.
    const how = lang[1] === "sql" ? [] : ["--source", "--name", path];
    if (await runCorral(["send", ...lang, ...how, "--", text]))
      editor.setStatus(tr("Corral: whole file sent to the console", "Corral: archivo completo enviado a la consola"));
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

// Scratch files: Fresh does not let outside programs read unsaved buffers, so
// mirror each Corral scratch buffer to ".live.R" next to it (".live.py" for a
// Python draft, so the two drafts of a session don't overwrite each other). `corral script
// --save/--read` (and Claude) read that copy. It is written when typing
// pauses for 0.4 s (at least every 2 s while typing goes on) and when the
// buffer shows up.
const mirrored: Record<string, string> = {};
const dirty: Record<number, boolean> = {};
let mirrorTimer = -1;
let dirtySince = 0;

function scratchCopy(id: number): [string, string] | null {
  const path = editor.getBufferPath(id) || "";
  if (!isScratch(path)) return null;
  return [path, path.replace(/(draft|borrador)\.(R|py)$/, ".live.$2")];
}

async function mirror(id: number): Promise<void> {
  const paths = scratchCopy(id);
  if (!paths) return;
  const [path, copy] = paths;
  const text = await editor.getBufferText(id);
  if (mirrored[path] === text && editor.fileExists(copy)) return;
  mirrored[path] = text;
  // writeFile only creates new files; replaceFile overwrites.
  editor.replaceFile(copy, text);
}

registerHandler("corral_scratch_modified", function (args: { buffer_id: number }) {
  if (!scratchCopy(args.buffer_id)) return;
  dirty[args.buffer_id] = true;
  const now = Date.now();
  if (mirrorTimer >= 0) {
    if (now - dirtySince >= 2000) return; // typing goes on: let the pending write happen
    editor.clearInterval(mirrorTimer);
  } else {
    dirtySince = now;
  }
  mirrorTimer = editor.setTimeout(400, "corral_mirror_scratch");
});
editor.on("buffer_modified", "corral_scratch_modified");

registerHandler("corral_mirror_scratch", async function () {
  mirrorTimer = -1;
  for (const key of Object.keys(dirty)) {
    delete dirty[Number(key)];
    try {
      await mirror(Number(key));
    } catch (e) {
      // A failed copy is retried on the next edit.
    }
  }
});

// Ctrl+S: a Corral scratch file is temporary, so saving it means choosing a place
// in the project. Fresh's own "Save as" would start in the scratch folder, so ask
// for the name here, starting from the project folder.
registerHandler("corral_save", async function () {
  try {
    const id = editor.getActiveBufferId();
    const path = editor.getBufferPath(id) || "";
    if (!isScratch(path)) {
      editor.executeAction("save");
      return;
    }
    const project = projectOf(path);
    const answer = await editor.prompt(tr("Save in the project: ", "Guardar en el proyecto: "), project + "/");
    if (!answer || !answer.trim() || answer.trim() === project + "/") return;
    let target = answer.trim();
    if (!target.startsWith("/")) target = project + "/" + target;
    if (!/\.[A-Za-z0-9]+$/.test(target)) target += ".R";
    if (editor.fileExists(target)) {
      const confirm = await editor.prompt(tr(`${target} already exists. Replace it? (y/n): `, `${target} ya existe. ¿Reemplazarlo? (s/n): `), "");
      if (!confirm || !/^[sy]/i.test(confirm.trim())) {
        editor.setStatus(tr("Corral: not saved", "Corral: no se guardó"));
        return;
      }
    }
    // The save is confirmed later (Fresh can still refuse it, e.g. when the file
    // is open in another Fresh): only claim success once the buffer moved there.
    if (!editor.saveBufferToPath(id, target)) {
      editor.setStatus(tr("Corral: could not save to ", "Corral: no pude guardar en ") + target);
      return;
    }
    for (let i = 0; i < 10; i++) {
      await editor.delay(200);
      if (editor.getBufferPath(id) === target) {
        editor.setStatus(tr("Corral: saved to ", "Corral: guardado en ") + target);
        return;
      }
    }
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

// <- and |> are R's; in a Python or SQL file the keys do nothing.
registerHandler("corral_insert_assign", async function () {
  if (["python", "sql"].includes(langOf(editor.getBufferPath(editor.getActiveBufferId()) || "") || "")) return;
  editor.insertAtCursor(" <- ");
});

registerHandler("corral_insert_pipe", async function () {
  if (["python", "sql"].includes(langOf(editor.getBufferPath(editor.getActiveBufferId()) || "") || "")) return;
  editor.insertAtCursor(" |> ");
});

// The editor follows herdr's theme: `corral theme` writes themes/corral.json.
try {
  editor.applyTheme("corral");
} catch (e) {
  // No Corral theme yet: keep whatever theme is configured.
}

// Live: when herdr's config changes (you switched its theme), regenerate the
// Corral palette; when the Corral theme file changes, reload it here. Both
// folders are watched (a file replaced by a rename would lose a file watch).
const homeDir = editor.getEnv("HOME") || "";
const herdrConfig = editor.getEnv("HERDR_CONFIG_PATH")
  || ((editor.getEnv("XDG_CONFIG_HOME") || homeDir + "/.config") + "/herdr/config.toml");
const themesDir = editor.getThemesDir();
let lastHerdrConfig: string | null = null;
let lastCorralTheme: string | null = null;
let syncing = false;
let themePending = false;
registerHandler("corral_theme_sync", async function () {
  themePending = false;
  if (syncing) {
    // Busy with the previous change: look again once it is done.
    themePending = true;
    editor.setTimeout(300, "corral_theme_sync");
    return;
  }
  syncing = true;
  try {
    const herdr = editor.readFile(herdrConfig);
    if (herdr !== null && lastHerdrConfig !== null && herdr !== lastHerdrConfig) {
      await editor.spawnProcess("corral", ["theme"]);
    }
    if (herdr !== null) lastHerdrConfig = herdr;
    const theme = editor.readFile(themesDir + "/corral.json");
    if (theme !== null && lastCorralTheme !== null && theme !== lastCorralTheme) {
      editor.reloadThemes();
      editor.applyTheme("corral");
    }
    if (theme !== null) lastCorralTheme = theme;
  } catch (e) {
    // Try again on the next tick.
  } finally {
    syncing = false;
  }
});
const watches: Record<number, string> = {};  // handle -> the file it is for
registerHandler("corral_theme_watch", function (args: { handle: number; path: string }) {
  const file = watches[args.handle];
  if (!file || !(args.path === file || args.path.endsWith("/" + file.split("/").pop()))) return;
  if (themePending) return;
  themePending = true;
  editor.setTimeout(150, "corral_theme_sync");  // one sync for a burst of writes
});
editor.on("path_changed", "corral_theme_watch");
(async function () {
  let watching = 0;
  for (const file of [herdrConfig, themesDir + "/corral.json"]) {
    try {
      watches[await editor.watchPath(file.slice(0, file.lastIndexOf("/")))] = file;
      watching++;
    } catch (e) {
      // Folder missing: covered by the slow check below.
    }
  }
  editor.setTimeout(10, "corral_theme_sync");  // remember the current contents
  if (watching < 2) editor.setInterval(5000, "corral_theme_sync");
})();

// Open the folder view the first time a Corral scratch file shows up, then
// give the focus back to the code.
const explorerShown: Record<string, boolean> = {};
registerHandler("corral_show_explorer", async function () {
  try {
    const path = editor.getBufferPath(editor.getActiveBufferId()) || "";
    if (!isScratch(path) || explorerShown[path]) return;
    explorerShown[path] = true;
    editor.executeAction("focus_file_explorer");
    await editor.delay(50);
    editor.executeAction("focus_editor");
  } catch (e) {
    // The folder view is a convenience; never break the editor over it.
  }
});
editor.on("buffer_activated", "corral_show_explorer");
editor.on("after_file_open", "corral_show_explorer");
editor.on("ready", "corral_show_explorer");
editor.setTimeout(800, "corral_show_explorer");  // the file Fresh opened at start

// A scratch buffer that shows up gets its copy right away, and its folder is
// watched for text sent from the terminal (`corral script --write`).
registerHandler("corral_scratch_shown", async function (args?: { buffer_id: number }) {
  try {
    const id = args && typeof args.buffer_id === "number" ? args.buffer_id : editor.getActiveBufferId();
    await mirror(id);
    const paths = scratchCopy(id);
    if (paths) await watchScratch(paths[0]);
  } catch (e) {
    // Retried on the next edit.
  }
});
editor.on("buffer_activated", "corral_scratch_shown");
editor.on("after_file_open", "corral_scratch_shown");
editor.setTimeout(800, "corral_scratch_shown");

// Text sent to the draft from the terminal: Fresh doesn't let other programs
// change an unsaved buffer, so `corral script --write` leaves a request in
// the draft's folder (.request.json) and this applies it to the buffer as one
// edit (Ctrl+Z undoes it), then answers in .reply.json. "open" (from
// `corral switch`) opens the other language's draft in a tab, or shows it.
const scratchWatches: Record<number, string> = {}; // watch handle -> draft folder
const watchedFolders: Record<string, boolean> = {};
let lastRequest = "";

async function watchScratch(path: string): Promise<void> {
  const folder = path.slice(0, path.lastIndexOf("/"));
  if (watchedFolders[folder]) return;
  watchedFolders[folder] = true;
  try {
    scratchWatches[await editor.watchPath(folder)] = folder;
  } catch (e) {
    delete watchedFolders[folder];
  }
}

registerHandler("corral_scratch_request", async function (args: { handle: number; path: string }) {
  const folder = scratchWatches[args.handle];
  if (!folder || !args.path.endsWith("/.request.json")) return;
  let req: { id?: string; mode?: string; text?: string; file?: string };
  try {
    req = JSON.parse(editor.readFile(folder + "/.request.json") || "");
  } catch (e) {
    return; // not there anymore, or still being written
  }
  if (!req.id || req.id === lastRequest) return;
  lastRequest = req.id;
  let ok = false;
  let msg = "";
  try {
    const names = req.file && SCRATCH_NAME.test(req.file) ? [req.file] : ["draft.R", "borrador.R", "draft.py", "borrador.py"];
    const id = names.map((name) => editor.findBufferByPath(folder + "/" + name)).find((b) => b) || 0;
    if (req.mode === "open") {
      // The draft of the console now in view: show its tab, or open it next to the other one.
      ok = id ? editor.showBuffer(id) : editor.openFile(folder + "/" + names[0], null, null);
      if (!ok) msg = tr("the editor could not open the draft", "el editor no pudo abrir el borrador");
    } else if (!id) {
      msg = tr("the draft is not open in the editor (if you saved it in the project, edit that file)",
        "el borrador no está abierto en el editor (si lo guardaste en el proyecto, edita ese archivo)");
    } else {
      const len = editor.getBufferLength(id);
      let text = String(req.text || "");
      if (req.mode === "replace") {
        ok = await editor.replaceRange(id, 0, len, text);
      } else {
        // At the end, on a line of its own.
        const last = len > 0 ? await editor.getBufferText(id, len - 1, len) : "\n";
        if (!text.endsWith("\n")) text += "\n";
        ok = await editor.replaceRange(id, len, len, (last === "\n" ? "" : "\n") + text);
      }
      if (ok) {
        await mirror(id); // the plugin's own edits fire no buffer_modified
        editor.setStatus(tr("Corral: text from the terminal was written to the draft (Ctrl+Z undoes it)",
          "Corral: se escribió en el borrador desde la terminal (Ctrl+Z lo deshace)"));
      }
      else msg = tr("the editor did not accept the change", "el editor no aceptó el cambio");
    }
  } catch (e) {
    msg = String(e);
  }
  editor.replaceFile(folder + "/.reply.json", JSON.stringify({ id: req.id, ok, msg }));
});
editor.on("path_changed", "corral_scratch_request");

// Fresh's own "Save as" (File menu) starts in the scratch folder. A file saved
// there under another name would stay hidden, git-ignored and be deleted in two
// days, so move it to the project with the same name.
registerHandler("corral_after_save", async function (args: { path: string; buffer_id: number }) {
  try {
    const path = args.path || "";
    const name = path.split("/").pop() || "";
    if (!inScratchFolder(path) || SCRATCH_NAME.test(name) || name.startsWith(".")) return;
    const target = projectOf(path) + "/" + name;
    if (editor.fileExists(target)) {
      editor.setStatus(tr(`Corral: ${name} stayed in the draft's folder because it already exists in the project; use Ctrl+S to choose another name`,
        `Corral: ${name} quedó en la carpeta del borrador porque ya existe en el proyecto; usa Ctrl+S para elegir otro nombre`));
      return;
    }
    if (!editor.saveBufferToPath(args.buffer_id, target)) return;
    for (let i = 0; i < 10; i++) {
      await editor.delay(200);
      if (editor.getBufferPath(args.buffer_id) === target) {
        await editor.spawnProcess("rm", ["-f", "--", path]);
        editor.setStatus(tr("Corral: saved in the project, ", "Corral: guardado en el proyecto, ") + target);
        return;
      }
    }
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});
editor.on("after_file_save", "corral_after_save");

// The shortcut palette (also ctrl+b space in herdr): every Corral key, searchable.
registerHandler("corral_shortcuts", async function () {
  try {
    await editor.spawnProcess("corral", ["keys"]);
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

// The palette's names, in the language the editor started with.
editor.registerCommand(tr("Corral: keyboard shortcuts", "Corral: atajos de teclado"),
  tr("All of Corral's shortcuts, searchable (ctrl+b space)", "Todos los atajos de Corral, con búsqueda (ctrl+b space)"), "corral_shortcuts", null);
editor.registerCommand(tr("Corral: run line or selection", "Corral: ejecutar línea o selección"),
  tr("Sends the code to the console (R or Python, depending on the file)", "Envía el código a la consola (R o Python, según el archivo)"), "corral_run_line", null);
editor.registerCommand(tr("Corral: run file", "Corral: ejecutar archivo"),
  tr("Runs the whole file in the console (R or Python, depending on the file)", "Ejecuta el archivo completo en la consola (R o Python, según el archivo)"), "corral_run_file", null);
editor.registerCommand(tr("Corral: save", "Corral: guardar"),
  tr("Saves; for a temporary draft, asks where", "Guarda; en un borrador temporal, pregunta dónde"), "corral_save", null);
editor.registerCommand(tr("Corral: insert <-", "Corral: insertar <-"),
  tr("Inserts the assignment operator", "Inserta el operador de asignación"), "corral_insert_assign", null);
editor.registerCommand(tr("Corral: insert |>", "Corral: insertar |>"),
  tr("Inserts the native pipe", "Inserta el pipe nativo"), "corral_insert_pipe", null);
