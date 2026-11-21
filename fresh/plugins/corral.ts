// Corral for Fresh: run R code from the editor in the shared console (pane corral:r).
//
//   Ctrl+R (or Ctrl+Enter)    selection, or the whole statement under the cursor
//                             (multi-line pipes and ggplot chains included), then
//                             move to the next statement
//   F5 (or Ctrl+Shift+Enter)  the whole file, with source(echo = TRUE)
//   Ctrl+L (or Alt+-)         " <- "
//   Ctrl+U (or Ctrl+Shift+M)  " |> "
//   Ctrl+S                    save; in a temporary scratch file, "Save as"
//
// Through Warp and herdr, Ctrl+Enter arrives as plain Enter and Alt chords get
// split, so the first key of each pair is the one that works there.
// Keys are bound in ~/.config/fresh/config.json (see corral-term/fresh/config.json).

const editor = getEditor();
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
    if (!path.includes("/corral/borradores/") || !path.endsWith("borrador.R")) return;
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

// Ctrl+S: a Corral scratch file is temporary, so saving it means choosing a real place.
registerHandler("corral_save", async function () {
  const path = editor.getBufferPath(editor.getActiveBufferId()) || "";
  editor.executeAction(path.includes("/corral/borradores/") ? "save_as" : "save");
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
    if (!path.includes("/corral/borradores/") || explorerShown[path]) return;
    explorerShown[path] = true;
    editor.executeAction("focus_file_explorer");
    await editor.delay(50);
    editor.executeAction("focus_editor");
  } catch (e) {
    // The folder view is a convenience; never break the editor over it.
  }
});
editor.on("buffer_activated", "corral_show_explorer");
editor.setInterval(1000, "corral_show_explorer");

editor.registerCommand("Corral: ejecutar línea o selección", "Envía el código a la consola de R", "corral_run_line", null);
editor.registerCommand("Corral: ejecutar archivo", "Ejecuta el archivo completo en la consola de R", "corral_run_file", null);
editor.registerCommand("Corral: guardar", "Guarda; en un borrador temporal, pregunta dónde", "corral_save", null);
editor.registerCommand("Corral: insertar <-", "Inserta el operador de asignación", "corral_insert_assign", null);
editor.registerCommand("Corral: insertar |>", "Inserta el pipe nativo", "corral_insert_pipe", null);
