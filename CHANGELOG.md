# Changelog

What changed in each version of Corral. The release workflow publishes each version's section as its release notes on GitHub, so add one here before you tag a version.

## Unreleased

- Agents ask by default: each `corral eval` waits in a popup until you approve it. A missing or mistyped `agents` setting also asks; only `corral config agents on` lets agents run without asking. If you had set `on`, nothing changes for you.
- Start with R or with Python: "Corral: start with R" and "Corral: start with Python" in the palette (`corral config start r|python`), or `corral layout --lang python` for one layout. A Python layout opens the Python console and a `draft.py`, so Ctrl+R sends to Python; `corral script` follows the console in view.
- Docs: Why Corral, a new page with the idea behind Corral, where the name comes from and where it fits in a data scientist's work; the panes of a layout, with a diagram; and the draft is deleted when you close Corral unless you save it.

### Fixes

- The notice for `agent_output structure` no longer says agents never see values; it says to keep `agents ask` with it.
- Docs: `corral layout --focus` takes `code`, not `codigo`.

## 0.1.6

A long session doesn't fill the disk.

- `View()` no longer fills the disk in a long session. A new `View()` of the same object replaces its temporary copy, and each session keeps the copies of its latest 20 tables, up to 2 GB in all; before, every copy stayed until R or Python closed (twenty `View()` of ten million rows left 3.4 GB). A file you open with `corral view` is never deleted.
- The table viewer says so when its file was replaced by a newer `View()`, instead of showing DuckDB's error.

## 0.1.5

Big tables open faster.

- `View()` in R writes the table with nanoparquet first, and with arrow when nanoparquet isn't installed or can't write a column (such as `POSIXlt`). Ten million rows now open in about 1.1 s instead of 1.9 s.
- In a long session, R's memory stays steady: 20 `View()` of ten million rows left R at 1.6 GB with nanoparquet, against 6.4 GB with arrow. `corral doctor` and the installer suggest nanoparquet even when arrow is installed.
- A `difftime` column shows in the table viewer as a number in its own units: 1.5 hours shows 1.5.
- `bench/view.R` measures how long `View()` takes with a big table, `bench/writers.R` compares nanoparquet with arrow, and `bench/repeat.R` follows R's memory over 20 `View()` in one session.

### Fixes

- Site and docs: the time to open ten million rows counts writing the Parquet file. The 34 ms quoted before was only the first screen, after the file was written.
- Docs: the configuration page no longer says `agent_output structure` never shows values; it says to keep `agents ask` with it.

## 0.1.4

Every download checked.

- Fresh, DuckDB, Air and Ark are always checked against SHA-256s written into Corral's code for the versions it installs; before, only when their projects published a checksum, and Ark never. A version of Ark you choose yourself is checked against the SHA-256 GitHub reports for it.
- The installer checks the package's build provenance attestation when the GitHub CLI is there (`gh` 2.49 or later, logged in), and stops if it doesn't hold.
- `CORRAL_URL` (a package from a mirror or a test) needs `CORRAL_SHA256`; installing it unchecked takes `CORRAL_SHA256=skip`.

### Fixes

- Docs: the install page no longer describes the old fallback to `main`, or the repository as private.
- Docs: what restricted output doesn't stop (an agent that writes code to get values out), and why it pairs with `ask`.
- Docs: herdr's own memory and CPU, measured apart; with it, Corral uses about 200 MB.

## 0.1.3

Safer installs.

- The installer installs a published version (the latest, or the one in `CORRAL_VERSION`), says which, and checks the package's SHA-256 before opening it.
- Each release now ships a `.sha256` file and a build provenance attestation, which proves the package was built by the project's GitHub workflow; check it with `gh attestation verify`.

### Fixes

- The installer no longer falls back to downloading `main` and building it when a release download fails: it stops and says why.
- The whole script runs from one function called on its last line, so a download cut halfway runs nothing.
- Docs: an agent's `--timeout` only stops the agent waiting; the code keeps running in your console.

## 0.1.2

Agent controls, enforced in Corral's own code whatever the agent was told.

- `corral config agents on|ask|off`: with `ask`, every request from an agent waits for your approval in a popup that shows the exact code; with `off`, nothing runs.
- `corral config agent_output full|review|structure`: with `review` you see each result first and decide whether the agent gets it; with `structure` the agent gets whether it worked and the shape of the objects, without printed values.
- `corral_sensitive()` in R or Python makes one session stricter: every request needs approval.
- `corral audit`: a local log of every agent request (who, what code, whether it ran, what it returned).
- All of these are in the command palette.
- Docs: a Security model page, and the agent harness explained on the site.

## 0.1.1

- MIT license.
- An anonymous install count: one ping when an installation finishes (version, system, country; never the IP). `CORRAL_NO_TELEMETRY=1` or `DO_NOT_TRACK=1` turns it off.
- The install line moves to `curl -LsSf https://corral-term.dev/install.sh | sh`.
- The site and docs live at [corral-term.dev](https://corral-term.dev), now also comfortable on phones.

## 0.1.0

- Corral speaks English by default; Spanish is one setting away (`corral config language es`, or the palette). Commands and options are always English, and the script editor (Fresh) follows Corral's language.
- Tables in Mirador, when you ask for them (`m` in the table viewer or the variables pane, or `View(df, mirador = TRUE)`), with the same search, filters (R or SQL), sorting, row view and keys as the terminal viewer, on the same DuckDB queries.
- The agent skill moves to `skills/`, following the open Agent Skills standard, for any agent.
- A documentation site with the architecture, the command palette, what you need before installing, and an FAQ.

### Fixes

- Mirador's variables sidebar is gone: clicking a variable there opened it in the terminal, which was confusing.

## 0.0.1

First published version.

- A terminal-native workspace for R and Python inside herdr: your agent, the script (Fresh), the console and the variables in one layout.
- R runs in Ark, Positron's R console; Python in IPython, through uv.
- Variables pane, a fast table viewer on DuckDB, and Mirador for plots in the browser.
- `corral eval` and an agent skill, so agents run code in your live session, where you see it.
- One Rust binary with no dependencies, and an installer that checks everything first and never uses `sudo`.
