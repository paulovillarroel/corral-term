# Corral panes and keyboard shortcuts

## Panes

A Corral layout has the agent pane on the left and, on the right, the script pane (Fresh) over the console, with the variables pane below them. Panes that are not showing (the other language's console, or the pane hidden by expanding) wait in a hidden tab named "corral (oculto)" and keep running.

| Command | Effect | Ask first? |
|---|---|---|
| `corral toggle script` | The script takes the whole right column; the console is hidden. | No |
| `corral toggle consola` | The console takes the whole right column; the script is hidden. | No |
| `corral toggle ambos` | Script over console again, at the user's usual sizes. | No |
| `corral switch r` / `corral switch python` | The console spot shows that language (Python starts the first time). | No |
| `corral script [archivo]` | Adds the editor over the console in a layout that has none, with a new draft or that file. | Yes |
| `corral vars --json` | Reads the variables (no pane changes). | No |
| `corral vars` | Opens the variables pane as a popup over the user's screen. | Yes |
| `corral plots` / `corral mirador` | Brings the plot viewer to the front. | No |
| `corral layout` | Opens a new Corral layout in the current tab. | Yes |
| `corral close` | Closes script, console and variables; the R or Python session ends and the draft is discarded. | Yes, always |

`corral toggle` and `corral switch` change only what the user sees and can be undone with the same commands, so run them when asked. Without a value, `corral toggle` decides from the pane that has the focus (that is what `ctrl+b t` does for the user); from your pane always pass `script`, `consola` or `ambos`.

## Shortcuts

`corral atajos --json` lists every shortcut: `grupo`, `teclas`, `descripcion`, `id`, `ejecutable` (an action you can also run with its command) and `se_puede_cambiar`. `corral atajos --lista` prints the same for people. Read it before answering "which key does…": the user may have changed the defaults.

Change one with:

```bash
corral atajos --cambiar ID TECLA
```

- `herdr:<action>` (keys used in any pane): `TECLA` is what comes after `ctrl+b`, like `t`, `shift+k` or `space`, or the whole shortcut (`"ctrl+b t"`). herdr takes it at once.
- `editor:<action>` (keys in the script pane): `TECLA` is the whole shortcut, like `ctrl+e`, `f6` or `ctrl+shift+enter`. The user has to close the editor with `Ctrl+Q` and open the layout again.
- Other groups (console, variables, tables, Mirador) are part of Corral and can't be changed this way.

The command refuses keys herdr already uses (`ctrl+b ?` lists them) and keys another shortcut has. Tell the user what it said; add `--forzar` only if they agree to take the key from the other shortcut. Before changing, note the current key from `--json`, so you can put it back if the user asks.

The files behind it, for reference: herdr's bindings are in `~/.config/herdr/config.toml` (`[[keys.command]]`), the editor's in `~/.config/fresh/config.json` (`keybindings`). Use `corral atajos --cambiar` instead of editing them.

## Where the user finds them

- `ctrl+b space`: the searchable list; Enter runs the actions.
- In Mirador: `?`.
- In the editor: `Ctrl+P`, then "Corral: atajos de teclado".
- `ctrl+b ?` shows herdr's own keys (panes, tabs, workspaces).
