# Changelog

What changed in each version of Corral. The release workflow publishes each version's section as its release notes on GitHub, so add one here before you tag a version.

## Unreleased

- Corral's files are yours only. The state folder, `~/.local/state/corral`, holds the audit log, your console history, previews of your variables and the agents' requests, and Corral now keeps it at `700`: it fixes an existing install's folder, and everything already inside, the next time it runs, and `corral doctor` checks it. The audit log, the console history and the logs are created readable only by you. This matters on shared servers.
- The audit log keeps 90 days. `corral config audit_days` changes that (a number of days, or `forever`), and the palette has "Agents: keep the audit log 30 days", "90 days" and "a year".
- Less is left on disk: a console that starts deletes the folders of sessions that ended, an agent's requests and their full output go after a day, and Ark's and Mirador's logs start over when they pass 5 MB.
- Docs: the security model lists what stays on your disk, what each file holds and for how long.
- Tests run on every pull request. Live tests (`cargo test --test live -- --ignored`) start their own herdr and R and check what agents rely on: two agents at once take turns, timeouts, Ctrl+C, a crash in the middle of a request, and reading the variables while they change.
- The R console comes back by itself when R crashes: it starts a new session in the same pane and says so, instead of leaving the shell. A crash right after starting doesn't restart it again and again.
- The R console checks the signature of every message from Ark (HMAC-SHA256 with the session's key) and drops any that doesn't match; the editor's language server bridge does too. A connection to Ark that drops while R runs is opened again.
- Objects outlive a crash. Each R and Python console keeps a checkpoint of its objects, and if the session ends without you closing it (a crash, herdr or WSL closed), the next console in the project gets them back, packages and modules first, and says which it couldn't keep (connections). After a command that changed an object, a copy of the process writes what changed while you keep working: the prompt waits 2 to 7 ms in R and under half a millisecond in Python, and a command that changes nothing about 0.3 ms. Closing the console yourself starts clean, as before. `corral config checkpoints off` or the palette turns it off; `checkpoint_max_mb` (1024) caps the size of an object kept. The measurements are in the docs' Performance page.
- Live tests for the Python console too (objects kept, two agents at once, timeouts, Ctrl+C, a crash, the variables), and for R's restart and the editor's language server.

### Fixes

- A Python console that ended without closing (a crash) left its temporary folder, with the tables `View()` wrote, in /tmp for good. The folder's name now carries the process, and the next Python console removes those of processes that ended.
- When R crashed in the middle of an agent's request, the agent waited out its whole timeout (5 minutes by default) and was then told the code was still running. It now hears at once that the console ended. The next request used to find the dead session and say the console was busy; it now opens a new one.
- A kernel that stalled while Corral connected to it left the console waiting forever; Corral now gives up after 10 seconds. A damaged length in a message from the kernel could crash the console.
- Mirador: a session named `..` reached the folder above the sessions (only with Mirador's token).
- JSON nested thousands of levels deep crashed Corral instead of being refused. TOML: `\b`, `\f`, a line-ending backslash followed by spaces or CRLF, and quotes right before a closing `"""` are read as TOML 1.0 says.

## 0.1.10

The Notebook layout, layouts that switch in place and keep the sizes you leave, and a palette that picks the R or Python version and changes shortcuts.

- A third layout, Notebook: the script over the console on the left, like RStudio or Positron, and the agent over the variables on the right. "Layout: notebook" in the palette (Tab gives it a key) or `corral layout --focus notebook`. Like the other two, it rearranges an open layout in place, keeping the console's session, the draft and the agent.
- Each layout has its own sizes. The left column starts at about half the width when the agent is there and at 62 % when the script is (code first and notebook), since code needs the long lines. Panes you resize by hand keep their size: Corral notes the sizes when you switch layouts, press ctrl+b t or close Corral, and each layout opens with the ones you left, in any workspace. "Layout: back to Corral's sizes" in the palette (`corral layout --default-sizes`) forgets them.
- The editor's file explorer is 31 columns wide in every layout. It used to be 30 % of the pane, so it grew when you switched to code first or the notebook. A width you set in Fresh's configuration (`file_explorer.width`) stays.
- Docs: Windows users get a step-by-step guide to set up WSL with Ubuntu, R (with rig), herdr and Corral, for people who don't use Linux or terminals. The install page starts with it, and the landing's "Windows · WSL" tag and FAQ link to it.
- Change a shortcut from the palette: Tab on it, then press the new key; Backspace puts back Corral's. herdr takes it at once. From a terminal, `corral keys --reset ID` gives a shortcut back Corral's key.
- Choose the R version (with rig) or the Python version (with uv) from the palette: "Console: R version (rig)" and "Console: Python version (uv)" open a window with the installed versions, and Enter uses one; if that console is open, Corral asks whether to restart it. Corral runs the chosen R without changing rig's default, so the rest of the system keeps it. Inside a uv project the Python goes to the project's `.python-version` (`uv python pin`); outside one, to `uv run --python`. From a terminal or an agent: `corral versions r|python [VERSION] [--list|--json] [--restart]`.
- Easier keys to remember: `ctrl+b shift+s` (S for script) opens or switches to the layout focused on the code, instead of `ctrl+b shift+a`, and `ctrl+b shift+q` (Q for quit) closes Corral, instead of `ctrl+b shift+c`. `ctrl+b a` stays. Without shift, `ctrl+b s` and `ctrl+b q` are still herdr's own (settings and detach). The installer updates the shortcuts when you update Corral.
- Every command prints its usage with `-h` or `--help` and stops there, without touching herdr or your files. Before, only some did: `corral config --help` printed `null`, `corral keys --help` opened the palette, and `close`, `doctor`, `theme` and others simply ran.

### Fixes

- A shortcut changed with `corral keys --change` went back to Corral's key the next time Corral updated (herdr's), or got Corral's keys back next to yours (the editor's). Corral now keeps your keys in its config, under `keys`, and the installer puts them back.
- Switching between "agent first" and "code first" (ctrl+b a, ctrl+b shift+s) opened a whole new layout: a new R or Python session, an empty draft and a new agent, while the old ones were left running unlabeled in another tab. With a layout open, they now swap the agent and the script in place, and the console's objects, the draft and the agent stay; only closing Corral ends them.
- With rig, after `rig default` moved to another R, Corral's R console kept starting the previous one, while the editor and `corral doctor` used the new one. The console now follows it too.
- `corral uninstall --help` uninstalled Corral. Now it prints the usage, and `uninstall`, `close`, `doctor` and `theme` refuse any argument they don't know, so a typo like `-help` doesn't run them.

## 0.1.9

Agents know Corral's settings and docs, and the docs say where Corral runs.

- Agents know Corral's settings and docs: the skill lists every `corral config` key, points to the docs that come with Corral and to `corral --help`, and tells agents never to change `agents` or `agent_output` themselves.
- Docs: the install page says how to read the installer in Spanish: `curl -LsSf https://corral-term.dev/install.sh | CORRAL_LANG=es sh`.
- Site and docs: the macOS version is in development, and the landing shows where Corral runs with a tag for each system under the install line.

### Fixes

- The skill told agents to pass `agente`, `consola` or `ambos` to `corral toggle`, which refuses them; it now says `agent`, `console` and `both`.
- Docs: `corral toggle` takes `console`, `both` and `agent` (the page still said `consola`, `ambos` and `agente`), and `corral help` lists the commands, not `corral ayuda`.
- The comment Corral writes above its theme in herdr's config is in English, and the docs' examples use English names.

## 0.1.8

Corral's files move to English names, and nothing is lost on upgrade.

- Corral's own files have English names: the R console's history is `~/.local/state/corral/history-r.jsonl`, and drafts outside a project go to `$XDG_RUNTIME_DIR/corral/drafts/`. Nothing is lost on upgrade: the history, the layout state and the installer's record move to their new names the first time Corral reads them, so `corral uninstall` still undoes an earlier install.
- An editor that was open while you updated needs to be closed (Ctrl+Q) and opened again before `corral script --write` reaches its draft; until then it says the editor did not answer.

## 0.1.7

Agents ask first, and R and Python share one session.

- Agents ask by default: each `corral eval` waits in a popup until you approve it. A missing or mistyped `agents` setting also asks; only `corral config agents on` lets agents run without asking. If you had set `on`, nothing changes for you.
- Start with R or with Python: "Layout: start with R" and "Layout: start with Python" in the palette (`corral config start r|python`), or `corral layout --lang python` for one layout. A Python layout opens the Python console and a `draft.py`, so Ctrl+R sends to Python; `corral script` follows the console in view.
- One draft per language in a session that goes back and forth: switching the console (ctrl+b y) opens the other language's draft in another tab, and switching back shows the first again. Both are temporary and are deleted together when the editor closes; `corral script --read/--write/--save` act on the one for the console in view.
- The palette (ctrl+b space) is grouped by part of the IDE and by what it does: layouts and panes, the script editor, the console, the variables, tables, plots and Mirador, then agents, appearance and language, and help. Each entry starts with its category (`Layout:`, `Editor:`, `Console:`, `Agents:`, `Theme:`…), so typing one lists just those.
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
