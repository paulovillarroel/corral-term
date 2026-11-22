# Corral panes and keyboard shortcuts

## Panes

A Corral layout has the agent pane on the left and, on the right, the script pane (Fresh) over the console, with the variables pane below them. The layout focused on code (`corral layout --focus code`; the user opens it with `ctrl+b shift+a`, the default one with `ctrl+b a`) swaps the script and the agent: script on the left, agent over the console. Panes that are not showing (the other language's console, or the pane hidden by expanding) wait in a hidden tab named "corral (hidden)" and keep running.

| Command | Effect | Ask first? |
|---|---|---|
| `corral toggle script` | The script takes the whole right column; the console is hidden. | No |
| `corral toggle console` | The console takes the whole right column; the script is hidden. | No |
| `corral toggle both` | Script over console again, at the user's usual sizes. | No |
| `corral toggle agent` | Layout focused on code only: the agent takes the right column (instead of `script`). | No |
| `corral switch r` / `corral switch python` | The console spot shows that language (Python starts the first time). | No |
| `corral script [FILE]` | Adds the editor over the console in a layout that has none, with a new draft or that file. | Yes |
| `corral vars --json` | Reads the variables (no pane changes). | No |
| `corral vars` | Opens the variables pane as a popup over the user's screen. | Yes |
| `corral plots` / `corral mirador` | Brings the plot viewer to the front. | No |
| `corral layout [--focus code]` | Opens a new Corral layout in a new tab; `--focus code` puts the script on the left. | Yes |
| `corral close` | Closes script, console and variables; the R or Python session ends and the draft is discarded. | Yes, always |

`corral toggle` and `corral switch` change only what the user sees and can be undone with the same commands, so run them when asked. Without a value, `corral toggle` decides from the pane that has the focus (that is what `ctrl+b t` does for the user); from your pane always pass `script` (`agent` in the layout focused on code), `console` or `both`; the command refuses anything else.

## Shortcuts

`corral keys --json` lists every shortcut: `group`, `keys`, `description`, `id`, `runnable` (an action you can also run with its command) and `changeable`. `corral keys --list` prints the same for people. Read it before answering "which key does…": the user may have changed the defaults. The palette (`ctrl+b space`) groups them by part of the IDE (layouts and panes, script editor, console, variables, tables, plots and Mirador), then agents, appearance and language, and help; each description starts with its category (`Layout:`, `Editor:`, `Console:`, `Agents:`, `Theme:`…), so you can tell the user to type that category in the palette.

Change one with:

```bash
corral keys --change ID KEY
```

- `herdr:<action>` (keys used in any pane): `KEY` is what comes after `ctrl+b`, like `t`, `shift+k` or `space`, or the whole shortcut (`"ctrl+b t"`). herdr takes it at once.
- `editor:<action>` (keys in the script pane): `KEY` is the whole shortcut, like `ctrl+e`, `f6` or `ctrl+shift+enter`. The user has to close the editor with `Ctrl+Q` and open the layout again.
- Keys with other IDs (the R console's, the variables pane's, the tables' and Mirador's) are part of Corral and can't be changed this way.

The command refuses keys herdr already uses (`ctrl+b ?` lists them) and keys another shortcut has. Tell the user what it said; add `--force` only if they agree to take the key from the other shortcut. Before changing, note the current key from `--json`, so you can put it back if the user asks.

The files behind it, for reference: herdr's bindings are in `~/.config/herdr/config.toml` (`[[keys.command]]`), the editor's in `~/.config/fresh/config.json` (`keybindings`). Use `corral keys --change` instead of editing them.

## Where the user finds them

- `ctrl+b space`: the searchable list; Enter runs the actions.
- In Mirador: `?`.
- In the editor: `Ctrl+P`, then "Corral: keyboard shortcuts".
- `ctrl+b ?` shows herdr's own keys (panes, tabs, workspaces).
