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
// Through Warp and herdr, Ctrl+Enter arrives as plain Enter and Alt chords get
// split, so the first key of each pair is the one that works there.
// Keys are bound in ~/.config/fresh/config.json (see corral-term/fresh/config.json).

const editor = getEditor();

// Corral scratch files: <project>/.corral/<name>/borrador.R, or the temp folder.
function isScratch(path: string): boolean {
  return /\/(\.corral|corral\/borradores)\/[^/]+\/borrador\.R$/.test(path || "");
}

// Any file inside a scratch folder (git-ignored, removed after two days).
function inScratchFolder(path: string): boolean {
  return /\/(\.corral|corral\/borradores)\/[^/]+\/[^/]+$/.test(path || "");
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

async function runCorral(args: string[]): Promise<boolean> {
  const res = await editor.spawnProcess("corral", args);
  if (res.exit_code !== 0) {
    const msg = (res.stderr || res.stdout || "").trim().split("\n").pop() || "corral falló";
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

    if (cur.selection && cur.selection.end > cur.selection.start) {
      const text = await editor.getBufferText(buf, cur.selection.start, cur.selection.end);
      if (text.trim() && (await runCorral(["send", "--", text])))
        editor.setStatus("Corral: selección enviada a la consola");
      return;
    }
    if (cur.line === null) return;

    const lines = (await editor.getBufferText(buf)).split("\n");
    let line = cur.line;
    // On a blank or comment line, run the next statement (like Positron).
    while (line < lines.length - 1 && isBlank(lines[line])) line++;
    if (isBlank(lines[line])) return;

    const [first, last] = statementAt(lines, line);
    const text = lines.slice(first, last + 1).join("\n");
    const ok = await runCorral(["send", "--", text]);

    let next = last + 1;
    while (next < lines.length && isBlank(lines[next])) next++;
    if (next >= lines.length) next = Math.min(last + 1, lines.length - 1);
    const pos = await editor.getLineStartPosition(next);
    if (pos !== null) editor.setBufferCursor(buf, pos);
    if (ok) {
      const n = last - first + 1;
      editor.setStatus(`Corral: ${n} línea${n === 1 ? "" : "s"} enviada${n === 1 ? "" : "s"} a la consola`);
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
    if (await runCorral(["send", "--source", "--name", path, "--", text]))
      editor.setStatus("Corral: archivo completo enviado a la consola");
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

// Scratch files: Fresh does not let outside programs read unsaved buffers, so
// mirror each Corral scratch buffer to ".actual.R" next to it. `corral script
// --guardar/--leer` (and Claude) read that copy.
const mirrored: Record<string, string> = {};
let mirrorTicks = 0;
registerHandler("corral_mirror_scratch", async function () {
  try {
    // The active buffer is the one on screen (listBuffers can return other
    // windows' copies of the same file).
    const id = editor.getActiveBufferId();
    const path = editor.getBufferPath(id) || "";
    if (!isScratch(path) || !path.endsWith("borrador.R")) return;
    const text = await editor.getBufferText(id);
    mirrorTicks = (mirrorTicks + 1) % 20;  // rewrite now and then even if unchanged
    if (mirrored[path] === text && mirrorTicks !== 0) return;
    mirrored[path] = text;
    // writeFile only creates new files; replaceFile overwrites.
    editor.replaceFile(path.replace(/borrador\.R$/, ".actual.R"), text);
  } catch (e) {
    // A failed copy only delays the next one.
  }
});
editor.setInterval(1500, "corral_mirror_scratch");

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
    const answer = await editor.prompt("Guardar en el proyecto: ", project + "/");
    if (!answer || !answer.trim() || answer.trim() === project + "/") return;
    let target = answer.trim();
    if (!target.startsWith("/")) target = project + "/" + target;
    if (!/\.[A-Za-z0-9]+$/.test(target)) target += ".R";
    if (editor.fileExists(target)) {
      const confirm = await editor.prompt(`${target} ya existe. ¿Reemplazarlo? (s/n): `, "");
      if (!confirm || !/^s/i.test(confirm.trim())) {
        editor.setStatus("Corral: no se guardó");
        return;
      }
    }
    // The save is confirmed later (Fresh can still refuse it, e.g. when the file
    // is open in another Fresh): only claim success once the buffer moved there.
    if (!editor.saveBufferToPath(id, target)) {
      editor.setStatus("Corral: no pude guardar en " + target);
      return;
    }
    for (let i = 0; i < 10; i++) {
      await editor.delay(200);
      if (editor.getBufferPath(id) === target) {
        editor.setStatus("Corral: guardado en " + target);
        return;
      }
    }
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});

registerHandler("corral_insert_assign", async function () {
  editor.insertAtCursor(" <- ");
});

registerHandler("corral_insert_pipe", async function () {
  editor.insertAtCursor(" |> ");
});

// The editor follows herdr's theme: `corral theme` writes themes/corral.json.
try {
  editor.applyTheme("corral");
} catch (e) {
  // No Corral theme yet: keep whatever theme is configured.
}

// Live: when herdr's config changes (you switched its theme), regenerate the
// Corral palette; when the Corral theme file changes, reload it here.
const homeDir = editor.getEnv("HOME") || "";
const herdrConfig = editor.getEnv("HERDR_CONFIG_PATH")
  || ((editor.getEnv("XDG_CONFIG_HOME") || homeDir + "/.config") + "/herdr/config.toml");
let lastHerdrConfig: string | null = null;
let lastCorralTheme: string | null = null;
let syncing = false;
registerHandler("corral_theme_sync", async function () {
  if (syncing) return;
  syncing = true;
  try {
    const herdr = editor.readFile(herdrConfig);
    if (herdr !== null && lastHerdrConfig !== null && herdr !== lastHerdrConfig) {
      await editor.spawnProcess("corral", ["theme"]);
    }
    if (herdr !== null) lastHerdrConfig = herdr;
    const theme = editor.readFile(editor.getThemesDir() + "/corral.json");
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
editor.setInterval(2000, "corral_theme_sync");

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

// Fresh's own "Save as" (File menu) starts in the scratch folder. A file saved
// there under another name would stay hidden, git-ignored and be deleted in two
// days, so move it to the project with the same name.
registerHandler("corral_after_save", async function (args: { path: string; buffer_id: number }) {
  try {
    const path = args.path || "";
    const name = path.split("/").pop() || "";
    if (!inScratchFolder(path) || name === "borrador.R" || name.startsWith(".")) return;
    const target = projectOf(path) + "/" + name;
    if (editor.fileExists(target)) {
      editor.setStatus(`Corral: ${name} quedó en la carpeta del borrador porque ya existe en el proyecto; usa Ctrl+S para elegir otro nombre`);
      return;
    }
    if (!editor.saveBufferToPath(args.buffer_id, target)) return;
    for (let i = 0; i < 10; i++) {
      await editor.delay(200);
      if (editor.getBufferPath(args.buffer_id) === target) {
        await editor.spawnProcess("rm", ["-f", "--", path]);
        editor.setStatus("Corral: guardado en el proyecto, " + target);
        return;
      }
    }
  } catch (e) {
    editor.setStatus("Corral: " + String(e));
  }
});
editor.on("after_file_save", "corral_after_save");
editor.setInterval(1000, "corral_show_explorer");

editor.registerCommand("Corral: ejecutar línea o selección", "Envía el código a la consola de R", "corral_run_line", null);
editor.registerCommand("Corral: ejecutar archivo", "Ejecuta el archivo completo en la consola de R", "corral_run_file", null);
editor.registerCommand("Corral: guardar", "Guarda; en un borrador temporal, pregunta dónde", "corral_save", null);
editor.registerCommand("Corral: insertar <-", "Inserta el operador de asignación", "corral_insert_assign", null);
editor.registerCommand("Corral: insertar |>", "Inserta el pipe nativo", "corral_insert_pipe", null);
