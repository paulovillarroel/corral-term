# Corral panes and keyboard shortcuts

## Panes

A Corral layout has the agent pane on the left and, on the right, the script pane (Fresh) over the console, with the variables pane below them. The layout focused on code (`corral layout --focus code`; the user opens it with `ctrl+b shift+s`, the default one with `ctrl+b a`) swaps the script and the agent: script on the left, agent over the console. The notebook layout (`corral layout --focus notebook`, "Layout: notebook" in the palette) puts the script over the console on the left and the agent over the variables on the right. Panes that are not showing (the other language's console, or the pane hidden by expanding) wait in a hidden tab named "corral (hidden)" and keep running.

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
| `corral layout [--focus code\|notebook]` | Opens a Corral layout in a new tab; `--focus code` puts the script on the left, `--focus notebook` the script over the console on the left. With a layout already open in the workspace, it switches that one between agent first, code first and notebook in place, keeping the console, the draft and the agent. | Yes |
| `corral layout --default-sizes` | Forgets the pane sizes the user left in each layout and gives the open one Corral's. Corral remembers the sizes by itself; use this only when the user asks for the original sizes. | Yes |
| `corral close` | Closes script, console and variables; the R or Python session ends and the draft is discarded. | Yes, always |

`corral toggle` and `corral switch` change only what the user sees and can be undone with the same commands, so run them when asked. Without a value, `corral toggle` decides from the pane that has the focus (that is what `ctrl+b t` does for the user); from your pane always pass `script` (`agent` in the layout focused on code), `console` or `both`; the command refuses anything else.

## Shortcuts

`corral keys --json` lists every shortcut: `group`, `keys`, `description`, `id`, `runnable` (an action you can also run with its command), `changeable` and `in_effect`: `"active"` for what is in effect (the profile and each security setting the console in view runs by, the audit log's days, the agent), `"global"` for the global value when the console runs by a stricter one (its project's profile or its own sensitive mark), and `false` otherwise. `corral keys --list` prints the same for people. Read it before answering "which key does…": the user may have changed the defaults. Don't run `corral keys` without a flag: it opens the palette in a popup over the user's screen. The palette (`ctrl+b space`) groups them by part of the IDE (layouts and panes, script editor, console, variables, tables, plots and Mirador), then security (profiles, the project of the console in view, requests, results, audit log), the agent, appearance and language, and help; each description starts with its category (`Layout:`, `Editor:`, `Console:`, `Results:`, `Agent:`, `Theme:`…), and a group's name (`security`) filters too, so you can tell the user what to type in the palette.

The user can change one in the palette: `Tab` on it, then the new key (`Backspace` puts back Corral's). From the terminal:

```bash
corral keys --change ID KEY
corral keys --reset ID
```

- `herdr:<action>` (keys used in any pane): `KEY` is what comes after `ctrl+b`, like `t`, `shift+k` or `space`, or the whole shortcut (`"ctrl+b t"`). herdr takes it at once.
- `editor:<action>` (keys in the script pane): `KEY` is the whole shortcut, like `ctrl+e`, `f6` or `ctrl+shift+enter`. The user has to close the editor with `Ctrl+Q` and open the layout again.
- An action with no key yet (`keys` empty, like `herdr:r-version`) gets one the same way.
- Keys with other IDs (the R console's, the variables pane's, the tables' and Mirador's) are part of Corral and can't be changed this way.

The command refuses keys herdr already uses (`ctrl+b ?` lists them) and keys another shortcut has. Tell the user what it said; add `--force` only if they agree to take the key from the other shortcut. Corral remembers the keys it changes, so they stay when Corral updates; `--reset ID` gives a shortcut back Corral's key.

The files behind it, for reference: herdr's bindings are in `~/.config/herdr/config.toml` (`[[keys.command]]`), the editor's in `~/.config/fresh/config.json` (`keybindings`). Use `corral keys --change` instead of editing them.

## Where the user finds them

- `ctrl+b space`: the searchable list; Enter runs the actions.
- In Mirador: `?`.
- In the editor: `Ctrl+P`, then "Corral: keyboard shortcuts".
- `ctrl+b ?` shows herdr's own keys (panes, tabs, workspaces).
