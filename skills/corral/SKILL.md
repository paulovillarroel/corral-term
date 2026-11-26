---
name: corral
description: Runs R, Python or SQL in the user's live console (the herdr panes labeled corral:r and corral:python) with `corral eval`, SQL on DuckDB over the session's data frames, so objects, plots and tables land in the session the user is looking at; also arranges the Corral panes (script, console, variables), reads or changes Corral's keyboard shortcuts, and answers questions about Corral from its settings and docs. Use whenever you need to run or inspect R or Python code or query data with SQL, explore or analyze data, fit models or make plots while working inside herdr (HERDR_ENV=1) and a Corral layout exists, for example (in any language) "run this in R", "do it in Python", "query it with SQL", "analyze this data", "make a plot", "what's in sales?", "fit a model", and when the user asks to show only the script or the console, switch to Python, or see or change a shortcut ("what's the shortcut for…?", "change the shortcut for…"), or asks how Corral works ("how do I change the language?"). Prefer it over Rscript or a separate python for anything that touches the user's data or session.
---

# Corral: the user's live R and Python sessions

The user works in herdr with a console under the script pane. It shows R (`corral:r`, the default) or Python (`corral:python`, IPython in the project's uv environment); the one not showing keeps running in a hidden tab. `corral eval` types your code into the console for the language you pass: it runs in their session, they watch it happen, and you get the printed output back. Objects you create stay in their session, plots go to their plot viewer (Mirador, `ctrl+b f`) and are saved as PNG for you, and tables open with `View()` in their table viewer.

R is the default. Add `--lang python` to every command for Python. Running code in a language brings that console into view, and the first Python request starts the Python console (`uv run --with ipython` in the project; it can take a few seconds).

## Check first

```bash
test "${HERDR_ENV:-}" = 1 && corral context --json
```

One call, nothing started: for each console (`consoles.r`, `consoles.python`) whether it is `running`, its `session`, `version`, `cwd`, whether it is `showing`, and whether SQL works there (`sql`); and the rules the user set, as they apply to that session: `access` (`on` runs, `ask` waits for their approval in a popup, `off` runs nothing) and `output` (`full`, `review` or `structure`), stricter when it is `sensitive`. `agents` gives the rules for a console not started yet. Plan around them: with `ask`, put related code in one request; with `structure`, work from shapes; with `off`, ask the user to run it. (`corral eval --check [--lang python]` still answers for one console.)

For R, `running: false` means there is no console, or R runs there without Corral active (Corral is opt-in per R session): tell the user they can open one with `ctrl+b a` or by typing `corral` in a terminal (workspace layout), or activate Corral in an existing R pane with `ctrl+b m` (or `corral_activate()` in that R). For Python, no console yet is normal (`starts_on_request`): `corral eval --lang python` starts it. Fall back to `Rscript` or a separate Python only if the user agrees.

If `corral` itself is not found, Corral is not installed: `corral install --check` (once installed) or `sh install.sh --check` from a clone lists what is missing and what to run; the install line is `curl -LsSf https://corral-term.dev/install.sh | sh`.

## Look before you run

```bash
corral vars --json
corral vars --json --lang python
```

Lists the objects in the session (name, kind, class, dimensions or preview, size) without typing anything in the console. Without `--lang` it lists the console that is showing. Use it instead of guessing object names, and before creating objects that could overwrite theirs.

## Run code

```bash
corral eval 'summary(sales)'

corral eval <<'EOF'
model <- lm(amount ~ region, data = sales)
summary(model)
EOF

corral eval --file analysis.R

corral eval --lang python <<'EOF'
import pandas as pd
sales = pd.read_csv("data/sales.csv")
sales.describe()
EOF
```

SQL runs on DuckDB in the console that is showing, R or Python: the session's data frames are tables by name, and the result lands in the session as `sql_result` (or the name a `-- name: x` line gives it). Prefer it for joins and aggregations over big tables:

```bash
corral eval --lang sql <<'EOF'
-- name: by_region
SELECT region, count(*) AS orders, sum(amount) AS total FROM sales GROUP BY region
EOF
```

You get the code echoed (`> ` in R, `>>> ` in Python) followed by what was printed. In R every top-level value prints, messages included, and warnings come after the output as `[corral] warning: ...`. In Python, as in IPython, only the last expression's value prints; use `print()` for the rest.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Ran without errors. |
| 1 | Error. The message is the last line; in R execution stopped at the failing expression, in Python the traceback shows where. Earlier statements already ran. |
| 2 | Console busy past `--wait-idle`, a half-typed line in the console, or `--timeout` reached (the code may still be running). |

## Plots

Each new plot is saved as PNG and reported as `[corral] plot N saved to <path>`. Read that file to see the plot before describing or adjusting it. The user sees it in Mirador (it opens on the first plot of a session; `ctrl+b f` brings it to the front). In Python, matplotlib figures (and plotnine or seaborn, which draw with it) go to Mirador at the end of each request; don't call `plt.savefig` to show them. The current figure stays open between requests, so later code can add to it and Mirador redraws the same plot: start each new plot with `plt.subplots()` or `plt.figure()`.

Every drawing stays in the user's plot history. When you redraw to fix a plot your previous `corral eval` drew (labels overlapping, wrong scale, a typo), add `--replace`: the new version replaces your draft, so the user sees one plot instead of near-identical copies.

```bash
corral eval --replace 'print(p)'
```

Don't use it when the user asked for a new or different plot, or wants to compare versions.

## The user's script

The Script pane above the console holds a temporary scratch file that is thrown away when the editor, its pane or the layout closes, unless saved. There is one per language (`draft.R`, `draft.py`) when the user moves between R and Python, and these commands act on the one for the console in view. It lives in the project's `.corral/` folder (git-ignored); files there are scratch, not part of the project, so don't touch them directly (use the commands below), cite or clean them up. Its current text, unsaved edits included:

```bash
corral script --read
```

To put code into it, pass the text on stdin. It goes at the end, on a new line, as one edit the user can undo with Ctrl+Z; their unsaved work stays:

```bash
corral script --write <<'EOF'
totals <- sales |> summarise(total = sum(amount))
EOF
```

Add `--replace` to replace the whole draft, only when the user asks for it (read it first: it may hold unsaved work). The draft stays temporary and is deleted when the editor closes, so code the user wants to keep belongs in a project file.

When the user asks you to save it, write it into their project (it refuses to overwrite unless you add `--force`, so ask first if the file exists):

```bash
corral script --save analysis/model.R
```

Don't save the scratch file on your own initiative; it is temporary on purpose.

## Panes, shortcuts, settings and docs

When the user asks to rearrange the Corral panes, or to see or change a keyboard shortcut, read [reference/panes-and-shortcuts.md](reference/panes-and-shortcuts.md) first. For a setting, a command's options or any question about how Corral works, read [reference/settings-and-docs.md](reference/settings-and-docs.md): it lists the settings, the R and Python versions the consoles run (rig, uv), which settings you must never change (`agents`, `agent_output`), and where Corral's docs are. The essentials:

```bash
corral toggle script            # the right column shows: script (agent, if focused on code), console or both
corral switch python            # the console shows: r or python
corral keys --json            # every shortcut, with the id used to change it
corral keys --change herdr:toggle shift+e
corral config theme dark-2026    # Corral's own theme (VS Code Dark 2026) for Corral and herdr; `herdr` goes back
corral config                    # the settings the user has set, as JSON
corral --help                    # every command
```

The user opens the same list with `ctrl+b space` (searchable; Enter runs the actions).

## Working in someone else's session

- Everything you run is visible and persists. Use clear names for objects meant to stay. For scratch work use `local({ ... })` in R, or a function in Python, so you don't clutter their environment.
- Ask before anything destructive or global: `rm(list = ls())`, `q()`, `setwd()`/`os.chdir()`, resetting `options()`, installing or removing packages (`uv add` changes their project), overwriting an object the user created, writing files outside the project.
- Keep output small: `head()`, `str()`, `summary()`, `dplyr::glimpse()`; `df.head()`, `df.info()`, `df.describe()`. Output over 20,000 characters is truncated and the full text path is printed.
- For long jobs pass `--timeout <seconds>` (default 300) and tell the user. After exit 2 the code may still be running; check later with `corral eval 'invisible(NULL)'` (or `corral eval --lang python 'None'`).
- If the console has a half-typed line, ask the user to finish or clear it. Never send keys to the console yourself.
- The user decides what you may do, and Corral enforces it (exit code 3 means it didn't run): with `agents ask` (the default, and always in a session marked with `corral_sensitive()`) each request waits until they approve it in a popup; if they refuse, don't retry or rephrase it unless they ask. With `agents off`, nothing runs: ask them to run it. With restricted output you get only whether it worked, the first line of an error and the shape of the objects; work from that, and never try to get values another way (printing them in errors, naming objects after them, encoding them in sizes, writing files you then read, or other tools). With review, the user sees each result first: if they keep one, you get its structure and a note; ask them what they found instead of running it again.
- Several agents can share one console: requests queue and run one at a time.
- To show a table to the user, run `View(df)` through `corral eval` (pandas, polars or pyarrow in Python); it opens in their table viewer, in the terminal. Only when the user asks for the browser, use `View(df, mirador = TRUE)` (`mirador=True` in Python) to open it in Mirador.

## When to still use Rscript or a separate Python

Package development checks (`devtools::test()`, `R CMD check`, `pytest`), or work the user explicitly wants isolated from their session.
