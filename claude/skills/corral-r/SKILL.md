---
name: corral-r
description: Run R or Python code in the user's live console (the herdr panes labeled corral:r and corral:python) with `corral eval`, so objects, plots and tables land in the session the user is looking at. Use whenever you need to run or inspect R or Python code, explore or analyze data, fit models or make plots while working inside herdr (HERDR_ENV=1) and a Corral layout exists, for example "corre esto en R", "hazlo en Python", "analiza estos datos", "haz un gráfico", "¿qué tiene ventas?", "ajusta un modelo". Prefer it over Rscript or a separate python for anything that touches the user's data or session.
---

# Corral: the user's live R and Python sessions

The user works in herdr with a console under the script pane. It shows R (`corral:r`, the default) or Python (`corral:python`, IPython in the project's uv environment); the one not showing keeps running in a hidden tab. `corral eval` types your code into the console for the language you pass: it runs in their session, they watch it happen, and you get the printed output back. Objects you create stay in their session, plots go to their plot viewer (Mirador, `ctrl+b f`) and are saved as PNG for you, and tables open with `View()` in their table viewer.

R is the default. Add `--lang python` to every command for Python. Running code in a language brings that console into view, and the first Python request starts the Python console (`uv run --with ipython` in the project; it can take a few seconds).

## Check first

```bash
test "${HERDR_ENV:-}" = 1 && corral eval --check
test "${HERDR_ENV:-}" = 1 && corral eval --check --lang python
```

Exit 0 prints the console and session as JSON (`active` tells whether it is the one showing). For R, anything else means there is no console, or R runs there without Corral active (Corral is opt-in per R session): tell the user they can open one with `ctrl+b a` (workspace layout), or activate Corral in an existing R pane with `ctrl+b m` (or `corral_activar()` in that R). For Python, no console yet is normal: `corral eval --lang python` starts it. Fall back to `Rscript` or a separate Python only if the user agrees.

## Look before you run

```bash
corral vars --json
corral vars --json --lang python
```

Lists the objects in the session (name, kind, class, dimensions or preview, size) without typing anything in the console. Without `--lang` it lists the console that is showing. Use it instead of guessing object names, and before creating objects that could overwrite theirs.

## Run code

```bash
corral eval 'summary(ventas)'

corral eval <<'EOF'
modelo <- lm(monto ~ region, data = ventas)
summary(modelo)
EOF

corral eval --file analisis.R

corral eval --lang python <<'EOF'
import pandas as pd
ventas = pd.read_csv("datos/ventas.csv")
ventas.describe()
EOF
```

You get the code echoed (`> ` in R, `>>> ` in Python) followed by what was printed. In R every top-level value prints, messages included, and warnings come after the output as `[corral] advertencia: ...`. In Python, as in IPython, only the last expression's value prints; use `print()` for the rest.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Ran without errors. |
| 1 | Error. The message is the last line; in R execution stopped at the failing expression, in Python the traceback shows where. Earlier statements already ran. |
| 2 | Console busy past `--wait-idle`, a half-typed line in the console, or `--timeout` reached (the code may still be running). |

## Plots

Each new plot is saved as PNG and reported as `[corral] gráfico N guardado en <path>`. Read that file to see the plot before describing or adjusting it. The user sees it in Mirador (it opens on the first plot of a session; `ctrl+b f` brings it to the front). In Python, matplotlib figures (and plotnine or seaborn, which draw with it) go to Mirador at the end of each request; don't call `plt.savefig` to show them.

Every drawing stays in the user's plot history. When you redraw to fix a plot your previous `corral eval` drew (labels overlapping, wrong scale, a typo), add `--reemplazar`: the new version replaces your draft, so the user sees one plot instead of near-identical copies.

```bash
corral eval --reemplazar 'print(grafico)'
```

Don't use it when the user asked for a new or different plot, or wants to compare versions.

## The user's script

The Script pane above the console holds a temporary scratch file that is thrown away when the layout closes, unless saved. It lives in the project's `.corral/` folder (git-ignored); files there are scratch, not part of the project, so don't edit, cite or clean them up. Its current text, unsaved edits included:

```bash
corral script --leer
```

When the user asks you to save it, write it into their project (it refuses to overwrite unless you add `--forzar`, so ask first if the file exists):

```bash
corral script --guardar analisis/modelo.R
```

Don't save the scratch file on your own initiative; it is temporary on purpose.

## Working in someone else's session

- Everything you run is visible and persists. Use clear names for objects meant to stay. For scratch work use `local({ ... })` in R, or a function in Python, so you don't clutter their environment.
- Ask before anything destructive or global: `rm(list = ls())`, `q()`, `setwd()`/`os.chdir()`, resetting `options()`, installing or removing packages (`uv add` changes their project), overwriting an object the user created, writing files outside the project.
- Keep output small: `head()`, `str()`, `summary()`, `dplyr::glimpse()`; `df.head()`, `df.info()`, `df.describe()`. Output over 20,000 characters is truncated and the full text path is printed.
- For long jobs pass `--timeout <seconds>` (default 300) and tell the user. After exit 2 the code may still be running; check later with `corral eval 'invisible(NULL)'` (or `corral eval --lang python 'None'`).
- If the console has a half-typed line, ask the user to finish or clear it. Never send keys to the console yourself.
- Several agents can share one console: requests queue and run one at a time.
- To show a table to the user, run `View(df)` through `corral eval` (pandas, polars or pyarrow in Python); it opens in their table viewer.

## When to still use Rscript or a separate Python

Package development checks (`devtools::test()`, `R CMD check`, `pytest`), or work the user explicitly wants isolated from their session.
