---
name: corral-r
description: Run R code in the user's live R console (the herdr pane labeled corral:r) with `corral eval`, so objects, plots and tables land in the session the user is looking at. Use whenever you need to run or inspect R code, explore or analyze data, fit models or make plots while working inside herdr (HERDR_ENV=1) and a Corral console exists, for example "corre esto en R", "analiza estos datos", "haz un gráfico", "¿qué tiene ventas?", "ajusta un modelo". Prefer it over Rscript for anything that touches the user's data or session.
---

# Corral: the user's live R session

The user works in herdr with an R console in a pane labeled `corral:r`. `corral eval` types your code into that console: it runs in their global environment, they watch it happen, and you get the printed output back. Objects you create stay in their session, plots go to their plot viewer (`ctrl+b f`) and are saved as PNG for you, and tables open with `View()` in their table viewer.

## Check first

```bash
test "${HERDR_ENV:-}" = 1 && corral eval --check
```

Exit 0 prints the console and session as JSON. Anything else means there is no console, or R runs there without Corral active (Corral is opt-in per R session). Tell the user they can open one with `ctrl+b a` (workspace layout), or activate Corral in an existing R pane with `ctrl+b m` (or `corral_activar()` in that R). Fall back to `Rscript` only if they agree.

## Look before you run

```bash
corral vars --json
```

Lists the objects in the global environment (name, kind, class, dimensions or preview, size) without typing anything in the console. Use it instead of guessing object names, and before creating objects that could overwrite theirs.

## Run code

```bash
corral eval 'summary(ventas)'

corral eval <<'EOF'
modelo <- lm(monto ~ region, data = ventas)
summary(modelo)
EOF

corral eval --file analisis.R
```

You get the code echoed with `> ` prompts, followed by what R printed, messages included. Warnings come after the output as `[corral] advertencia: ...`.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Ran without errors. |
| 1 | R error. The message is the last line; execution stopped at the failing expression, earlier ones already ran. |
| 2 | Console busy past `--wait-idle`, a half-typed line in the console, or `--timeout` reached (the code may still be running). |

## Plots

Each new plot is saved as PNG and reported as `[corral] gráfico N guardado en <path>`. Read that file to see the plot before describing or adjusting it. The user sees it in Mirador (it opens on the first plot of a session; `ctrl+b f` brings it to the front).

## The user's script

The Script pane above the console holds a temporary scratch file that is thrown away when the layout closes, unless saved. Its current text, unsaved edits included:

```bash
corral script --leer
```

When the user asks you to save it, write it into their project (it refuses to overwrite unless you add `--forzar`, so ask first if the file exists):

```bash
corral script --guardar analisis/modelo.R
```

Don't save the scratch file on your own initiative; it is temporary on purpose.

## Working in someone else's session

- Everything you run is visible and persists. Use clear names for objects meant to stay, and `local({ ... })` for scratch work so you don't clutter their environment.
- Ask before anything destructive or global: `rm(list = ls())`, `q()`, `setwd()`, resetting `options()`, installing or removing packages, overwriting an object the user created, writing files outside the project.
- Keep output small: `head()`, `str()`, `summary()`, `dplyr::glimpse()`. Output over 20,000 characters is truncated and the full text path is printed.
- For long jobs pass `--timeout <seconds>` (default 300) and tell the user. After exit 2 the code may still be running; check later with `corral eval 'invisible(NULL)'`.
- If the console has a half-typed line, ask the user to finish or clear it. Never send keys to the console yourself.
- Several agents can share one console: requests queue and run one at a time.
- To show a table to the user, run `View(df)` through `corral eval`; it opens in their table viewer.

## When to still use Rscript

Package development checks (`devtools::test()`, `R CMD check`), or work the user explicitly wants isolated from their session.
