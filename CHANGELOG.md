# Changelog

What changed in each version of Corral. The release workflow publishes each version's section as its release notes on GitHub, so add one here before you tag a version.

## Unreleased

- A month without a look for a newer Corral doesn't go unnoticed. Without the daily check (the sensitive profile, or `update_check off`), after 30 days the foot of the variables pane says so ("No update check in 30 days: ctrl+b space → Update") and the palette's new entry, "Update: look for a newer version, without installing" (`corral update --check`, in a popup), says how long it has been. Every look that reaches GitHub counts, the daily one or yours (an attempt without a connection doesn't), so looking puts it away for another 30 days; it reads files on your disk and asks nobody.
- The sensitive profile keeps Corral and herdr to your machine. With it, Corral doesn't check daily for a newer version or send its install count (the installer included), and uv starts the Python console and its language servers only from what is already downloaded. herdr's two checks of herdr.dev, new versions and agent-detection updates, are turned off in its config; Corral marks the lines it writes, and another profile puts back exactly what you had. With `--project`, that folder's sessions and workspaces keep to the machine; herdr's checks follow the global profile. What you ask for on purpose still connects (`corral update`, `corral update --check`, `corral install`). It covers Corral and herdr, not the agent's model provider or your own code. A live test under strace checks that opening the workspace with the sensitive profile doesn't ask GitHub, though the daily check is on and due; without the change it does.
- The security page lists every connection Corral makes beyond your machine: when each happens, where it goes, what it carries and how to go without it. There are eight, and none carries your data or code: the daily question about new versions, the install count, Corral's package and its provenance, the programs and R packages `corral install` fetches, and what the Python console and its language servers fetch the first time. Each place in the code that can open a connection is marked with its row, and a test fails when one isn't marked or listed, or when a row is no longer in the code. A second test watches Corral run under strace, following every process through a console, `corral eval`, a plot, Mirador, the table viewer, SQL and the variables, and fails on any connection beyond the machine; its last run followed 764, all local. It found herdr's own two: herdr asks herdr.dev in the background for new versions and agent-detection updates. The page says how to turn them off (`[update] version_check = false` and `manifest_check = false` in herdr's config), and live tests now do.
- The security page lists every connection Corral makes beyond your machine: when each happens, where it goes, what it carries and how to go without it. There are eight, and none carries your data or code: the daily question about new versions, the install count, Corral's package and its provenance, the programs and R packages `corral install` fetches, and what the Python console and its language servers fetch the first time. Each place in the code that can open a connection is marked with its row, and a test fails when one isn't marked or listed, or when a row is no longer in the code.
- Mirador opens a plot sooner from the variables pane. Enter on a plot object with no Mirador window open used to wait for R to draw it, then launch the browser, then draw the plot two or three times: at the size the window opened with, again once the thumbnails showed, and again once Corral placed the window on the right. Now the window opens while R draws; the page draws the plot at the size the window will have on the right, with the thumbnails already in place, so R draws it once (in Chromium, one request where there were three); and only one window opens, where R's first plot and the variables pane could each open their own. If the window isn't placed within a few seconds, the plot fits it as before. A plot with many `ggrepel` labels takes R about half a second each time it is drawn, measured in an isolated session.
- Mirador's plot size menu opens again. Clicking it did nothing: a handler that keeps buttons from taking the focus (so Enter and space stay the table's) also caught the menu, which needs that click to open. Now it opens. Opened with the mouse, it gives the keys back to Mirador once you choose a size, or at the next key if you close it without choosing, so ← and → move between plots; reached with the keyboard, its arrows choose a size, as in any menu. Checked with real clicks and keys in Chromium: the menu opens, 800 × 500 redraws the plot at that size, and Window fits it to the window again.

## 0.2.1

A follow-up to 0.2.0's security work. Three ways an agent could get around the harness are closed, Ark's channels stay private in every case, and the palette's security entries say which profile and settings are in effect, for your work and for the console in view.

- Three ways round the harness, closed. An agent's `corral send` (it typed code into the console with no approval, no output mode and no audit entry, even with `agents off`) now refuses from a pane herdr sees an agent in, or a shell outside herdr without a terminal, and the audit log records the attempt; your keys, the palette and your terminal send as before. `corral config projects …`, which replaced every project's profile without asking, is refused: a project's profile is set with `corral profile --project`. With `agents off`, `corral vars --json` lists nothing, where an agent could still read names and previews. Found in a review of the docs against the code; a live test checks the three, and fails without the change.
- Ark's channels are private in every case. When Corral's folder has a path too long for a socket address, the sockets go in a new folder of their own with a short path, made 700, under `$XDG_RUNTIME_DIR` (usually `/run/user/UID`), `$TMPDIR` or `/tmp`, where every folder above it belongs to you or root and others can't write to it (unless it is sticky, as `/tmp` is), and it goes when the console ends. The Jupyter console frontend (`ark_frontend jupyter`) uses Unix sockets too: its client names them `PATH-N` where Ark binds `PATH:N`, so Corral links one to the other on fixed ports before it starts. Before, both cases fell back to TCP on 127.0.0.1, where another user on the machine could read Ark's output. Now Ark never runs on TCP: if no private folder can be made, the console doesn't start and says why. A short folder the Jupyter frontend leaves (nothing of Corral's sees that console end) is removed by the next console. Tested with jupyter-console and with a state folder whose path is too long: R answers, and Ark has no TCP socket open.
- The console makes Corral's folder private before Ark's sockets go in it, also when it is the first thing that creates it.
- Ark's sockets are cleaned up in three more cases. A console that is starting no longer takes the socket folder of a jupyter-console starting at the same moment; a console that runs for days keeps its short folder, since cleanup finds it through its note even after its connection file is gone; and the Jupyter frontend's links go once its pane is closed, where before they stayed. A short folder whose note can't be written is removed, and the console says which note it couldn't write.
- The palette's security entries are in a Security group of their own, and say what is in effect. The three profiles come first, each one a mix of the two settings below it, and "● active" marks the one in effect; a mix that is none of them shows as "Profile: custom (…)". When the console in view runs by stricter rules than the global ones (its project's profile, or its own sensitive mark), the profile and settings it runs by are marked "● active" and the global ones "● global". The project entries name the folder of the console in view ("Project clinic: sensitive in every session") and show only with a console open in the workspace you are in, since otherwise there was no folder for them to act on, or another workspace's; "remove its profile" shows only when that folder has one, and says what applies after it. The audit log's days and the agent ("Agent: …", a group of its own, following `CORRAL_AGENT` as the layout does) are marked too, and `corral keys --json` gives each entry's state as `in_effect` (`"active"`, `"global"` or `false`). Opening the palette takes about 1.5 ms more, to read what is in effect and the console's rules.
- Mirador starts once, however many things ask for it at the same moment (the first plots of R and Python, the palette, `corral open`). Before, two could start: every caller then said "Mirador didn't start" while a server ran where `corral mirador --stop` couldn't reach it. Its address file is private from the moment it is written.

## 0.2.0

A security release. Corral is for data work, often on records about people, and the last versions are about keeping that data where it belongs: Ark talks to Corral over private sockets, keys come only from the system's random source, a threat model says what is protected and what isn't, and now turning checkpoints off deletes the copies of your objects: at once for consoles that have ended, at its next command for one still open, naming any it couldn't delete.

- Turning checkpoints off now leaves no copy of your objects behind. `corral config checkpoints off` (or the palette's "Console: don't keep checkpoints…") deletes at once the checkpoints of consoles that have ended, closed or crashed, and names any it couldn't delete; each running R or Python deletes its own at its next command. Before, a running console kept its copy until it closed, and the startup cleanup of old copies ran only with checkpoints on, so a crash's copy could stay on disk indefinitely. Turned on again, the next copy is a whole one, plots and figures included. A copy being written when they are turned off doesn't leave its folder behind. A live test turns them off with two consoles running and two crashed copies, and fails without the change.
- An R plot whose copy httpgd didn't answer (R just started, or busy) is copied on the next try, instead of waiting for another plot to be drawn.

## 0.1.29

On a shared machine, other users can no longer read your R session: Ark talks to Corral over Unix sockets in its private folder (TCP stays only with the Jupyter console frontend or a folder path too long for a socket). Keys only from the system's random source, and a threat model in the docs.

- Ark now listens on Unix sockets in Corral's own folder (`~/.local/state/corral/ark`, which only you can open) instead of TCP ports on 127.0.0.1. On TCP, any user on the same machine could subscribe to Ark's output channel (IOPub) without the key and read, live, the code each session ran and everything R printed: the key signs each message but doesn't hide it. A test reproduced it, reading a value printed by R from a second process that never saw the key. Only shared machines were exposed; on a laptop or WSL with one user there was no one else to read it. TCP stays as a fallback when the folder's path is too long for a socket (over about 100 characters), and for the Jupyter console frontend (`ark_frontend jupyter`), whose client names the sockets differently. The console removes Ark's sockets with its connection file when it ends or starts R again; those a console that was killed leaves go once their connection file does (two days) and nothing answers on them. A live test checks that the connection file says `ipc`, that the sockets are in the private folder and that Ark has no TCP port open.
- Ark's key and a new Mirador token come only from the system's random source (`/dev/urandom`); if it can't be read, Ark doesn't start, and Mirador doesn't when it has to make a token, with a message saying why. Before, a fallback used the clock, fine for file names but guessable for a key. Mirador keeps the token it already has in `mirador.json`; delete that file for a new one.
- Python's plot server compares its token in constant time (`secrets.compare_digest`), and a token that isn't ASCII gets a 401 too.
- The workflows' actions are pinned to a commit, with the version in a comment, since a tag can be moved and the release workflow's output is what gets attested.
- The docs have a Threat model page, in Harness and security: what Corral protects and from whom, each threat with what stops it and what is left, the channels between Corral's parts and how each is closed, and the supply chain. The harness page no longer says flatly that an agent can't loosen the controls: that holds for Corral's own commands, not for an agent with an unrestricted shell. The Security model says the files Corral keeps aren't encrypted and what the 700 folder does and doesn't keep out, and that the audit log is an operational record, not a tamper-evident one.

## 0.1.28

A request that moves into a sensitive project gets its rules at once.

- A project's profile is now judged by where the session really is, read from the process (`/proc/PID/cwd`), not only from what R and Python note after each command. Before, an agent's request that did `setwd()` into a sensitive project and read a file there got the values back under the old rules, and so could the next request if it came straight away, before R had noted the new folder. Now that request's own result comes back as the project allows (only its structure, with a note saying so) and the session stays sensitive; the next request asks. It also counts for a request that runs past its timeout, or whose console ends: `corral eval` looks at the folder while it waits. Reported in an outside review; a live test reproduces it (in R, and in Python through a symlink and past the timeout) and fails without the change.
- The security docs say what a project's profile follows (sessions by their folder, not files: code reading a project's file by absolute path from elsewhere isn't covered) and that in a team each person sets it in their own Corral.
- The variables pane names the profile as one: `profile: open` (`perfil: abierto`) instead of the bare word, which read like the state of something else.
- The docs have a search box (at the top of the menu, `/` to reach it) and a section of their own, Harness and security: The harness (new: the life of a request, who decides what), Security profiles (new), Working with agents and Security model.

## 0.1.27

Security profiles, for all your work or for one project.

- `corral profile open|standard|sensitive` sets the harness in one step: open (agents run without asking, full results), standard (they ask; the default) or sensitive (they ask, results come back without values, every session is sensitive). With `--project`, the profile is for one project folder and every session that starts or runs in it, so a project with clinical data is always sensitive without anyone remembering `corral_sensitive()`; a session that started there stays sensitive if it moves. The stricter of the global and the project's profile wins, so a project can only tighten. Projects' profiles live in Corral's settings (`projects`), not in the repository. Tightening never asks; a looser profile, or removing a project's, from an agent's pane waits for the user's approval, as `corral config` does.
- The profile in effect shows in the variables pane's header, in the accent colour when it is sensitive; `corral context` gives it to the agent; `corral profile` shows the global one, the project's and the rules that result. In the palette: "Agents: open profile", "standard profile", "sensitive profile", "this project is sensitive" and "this project follows the global profile".
- `corral eval`, `corral vars --json` and `corral context` now take their rules from one place, so they can't disagree. Working them out takes about 2 µs per request. R and Python note in their session file the folder each session started in. A live test marks a project sensitive and checks that requests then wait for approval, that values' previews go, that the session is marked and the pane says so, and that an agent can't remove the profile.

## 0.1.26

`corral context`: what your agent knows before it runs anything. After a crash, R says why and what came back.

- `corral context --json` gives an agent, in one call, where it is and what it may do: for each console, R and Python, whether it runs, its session, version and folder, whether it is in view and whether SQL works there; and the rules you set (`access`: on, ask or off; `output`: full, review or structure) as they apply to that session, stricter in one marked sensitive. Before, an agent learned the rules by being refused (exit 3) or by getting only structure back, and that SQL didn't work in R by trying. It reads files Corral already writes and herdr's list of panes, starts no console (Python included), and takes about 2 ms, less than the two `corral eval --check` calls it replaces. The rules it reports come from the same function `corral eval` applies. R and Python now note in their session file whether SQL works there, looked up once at start without loading duckdb.
- In the palette, "Agents: what your agent sees" shows the same in words, in a window, so you see exactly what the agent knows. `corral context` without `--json` prints it in a terminal.
- Corral's skill checks with `corral context --json` first and plans around the rules: related code in one request when each one asks, shapes instead of values with restricted output. `corral eval --check` still works.
- When R crashes, the console says why, in words, and what was running: "R ended unexpectedly: the system ran out of memory (signal 9), while running: big <- merge(…)". Out of memory when the system's count of processes it ended for lack of memory (`oom_kill` in /proc/vmstat) went up between R's start and its end, stopped from outside (`kill -9`) when it didn't, a crash in R or a package for signal 11, aborted for 6; for an agent's request, its code rather than the call that runs it. Before, it said only "signal 9". The count is read once when R starts and once when it ends: nothing while you work.
- What came back is one line, said before anything typed meanwhile runs: "Back in 0.4 s with 5 objects (big, check, f, m, v), 1 package (tools) and 4 plots, first in Mirador (ctrl+b f)". It names the packages attached again, which came back without a word before, and how long R took to be back. Before, the restore message could come after the first command that followed the crash (an agent's request, say): R writes it once it has restored, and the console now waits for it, up to 3 s, before running what was typed meanwhile. A console that opens on a checkpoint later (after `wsl --shutdown`, say) still says "Restored … from the R that ended without being closed (date)". Live tests kill R while a request runs and check the line, and that the notice comes before the next command's output.

## 0.1.25

A new version shows in Corral's own window: at the foot of the variables pane and in the palette.

- When the daily check finds a newer Corral, the foot of the variables pane says so in the accent colour ("Corral 0.1.26 is out: ctrl+b space → Update") until you install it, and the palette's Update entry names the version. Before, the only notice was a herdr toast, and herdr's toasts are off by default (`[ui.toast] delivery = "off"`), so it wasn't seen: in a capture with herdr's default settings nothing reached the screen. The pane reads two small files every 10 s (6.5 µs), never the network, and drops the notice once the new version is installed, even while it is still the old binary. A live test checks the pane and the palette, and fails without the change.

## 0.1.24

Plots survive a crash too: the newest 30 come back first in Mirador, with the objects.

- The plots you drew come back after a crash, like the objects. Each console keeps its newest 30 in its checkpoint, as images (an SVG up to 2 MB and a PNG), and the next console in the project shows them first, in Mirador and in the terminal's viewer, before its own: "[corral] Restored 4 plots from the R that ended without being closed (…), first in Mirador (ctrl+b f)." A plot removed in Mirador doesn't come back, and an R plot whose SVG is over 20 MB (a scatter of more than about 200,000 points) is left out. "Console: don't keep checkpoints of the objects and plots" in the palette turns it off with the rest.
- In R, one small process per session, `corral _keep-plots`, writes them: it starts with the first plot, watches the file R rewrites when its plots change, and asks httpgd over HTTP, as Mirador does. A command that draws waits about 0.1 ms more; the watcher uses about 1 MB and next to no CPU at rest, and R's memory grows the same as without it. It isn't done in the checkpoint's forked copy, where httpgd's server thread can hold a lock at the moment of the fork and the copy then never ends, nor by starting a process from R for each plot, which forks R too (8 ms with 400 to 650 MB in use). Python's figures are already images in memory, and the checkpoint's copy writes them.
- Live tests kill R after two plots, and Python after two matplotlib figures, and check that the new console says so, that Mirador lists them first and serves them, and that a new plot comes after them. `plots_checkpoint_cost_report` measures the cost. The R test fails without the change.

## 0.1.23

The installer asks less of you and brings more: herdr, jsonlite, uv and the PATH line, R packages from a 3-day-old snapshot or your institution's mirror, with pak if you use it. Corral now works in projects with renv.

- When uv is missing, the installer offers it, in a terminal: uv 0.13.9 from its GitHub release, checked against a SHA-256 in Corral's code, into `~/.local/bin` as uv's own installer does, so the Python console works from the first install. It says what that costs once: Python and the console's packages, about 400 MB on disk. Say no, or run it without a terminal, and it goes on without Python, as before. Uninstalling Corral leaves uv.
- The Python console, outside a uv project and without a `.venv`, now runs on a Python uv manages instead of the system's. With Ubuntu's Python, the packages apt put in it (numpy 1) came before the console's, and pyarrow, built for numpy 2, couldn't be imported, so `View()` of a pandas table failed. uv downloads that Python the first time. In a project, or with your own `.venv` or `UV_PYTHON_PREFERENCE`, nothing changes.
- Corral now works in projects with renv. R reads the project's `.Rprofile` there, not yours, so Corral didn't load, and renv's library hid jsonlite and httpgd: the console was plain R, without variables, `View()` or plots in Mirador. Corral's R consoles now start with `R_PROFILE_USER` set to `r/profile.R`, which reads the profile R would have read and then loads Corral, and Corral loads its packages from the project's library or, when the project doesn't have them, from yours, without adding yours to `.libPaths()`. In a session without its own library nothing changes. A live test makes an renv project and checks it; it fails without the change.
- R packages the installer installs come from Posit's package manager as it was three days ago, instead of its latest: no package newer than 72 hours, and the same versions for everyone installing that day. If R already has a repository that isn't a public one (an institution's mirror, set in R's `Rprofile.site`), the installer uses it instead, since in a network that blocks CRAN and Posit it's the one that works; before, the installer replaced it. `CORRAL_R_REPOS` sets the repositories by hand.
- The install's anonymous ping says which of the R packages the installer puts in by itself didn't go in (httpgd, nanoparquet or duckdb), now that a missing httpgd no longer stops the install and so sends no stop ping. The counter accepts only those three names and leaves the field out otherwise, and its totals count each one.
- When your R has pak, the installer installs R packages with it, as you would: httpgd and jsonlite, nanoparquet, and duckdb with DBI. The commands it shows to do it yourself use pak too. pak runs with `pkg.sysreqs` off, so it never installs system libraries (that takes sudo). If a package arrives but doesn't load, the installer shows the `apt-get` line `pak::pkg_sysreqs()` gives, for you to run. Without pak, or if pak fails, it uses `install.packages()` as before.
- After installing part of what was missing and stopping on the rest, the installer no longer says "Nothing was changed".
- httpgd no longer stops the install. It draws R's plots, but Corral works without it, so it moves next to nanoparquet: the installer puts it in your R library by itself, and if it can't (it needs cairo and libpng from the system), it warns that R's plots won't reach Mirador or the agent until you install it, with the command and pak's list of system libraries. jsonlite, which carries the variables, is still required and still offered.
- When `~/.local/bin` isn't on the PATH, the installer offers to add the line to `~/.bashrc` (or `~/.zshrc`), after saving a copy, so a new terminal finds herdr and corral. Uninstalling leaves the line, since other tools live there too.
- When herdr is older than 0.9.5, the installer offers to run `herdr update` instead of stopping. A herdr you already have is never replaced by the version Corral pins.

## 0.1.22

The installer brings herdr and R's two packages when they are missing.

- The installer offers to install herdr and the R packages httpgd and jsonlite when they are missing, instead of stopping. One question, Enter says yes. herdr 0.9.5 comes from herdr's GitHub release, checked against a SHA-256 written into Corral's code, and goes in `~/.local/bin` as your herdr: `herdr update` keeps it current and `corral uninstall` leaves it. The packages go in your R library, without sudo. Say no, or run it without a terminal, and it stops with the commands as before. When something else is missing too, such as R, it gives the steps for that and says it will offer the rest next time.

## 0.1.21

`corral vars --json` follows the console in view.

- `corral vars --json` without `--lang` lists the console in view, as the docs and Corral's skill say. Before, it always listed R, so an agent working with Python in view got R's objects. A live test switches between the two consoles and checks it.

## 0.1.20

The anonymous install count says where Corral runs, counts uninstalls, and warns the project when a published package doesn't check out.

- The anonymous install ping says a little more about where Corral runs, in coarse categories that name no person, machine or path: the agent the layout opens (`claude`, `codex`, `gemini`, `opencode`, another or none), the Ubuntu or Debian release (`noble`, `bookworm`…), herdr's version, R's major version (`4.6`) and whether uv is there. The site checks each field against a fixed list or pattern before it stores anything.
- Uninstalling sends one ping too, with only the version, the system and whether Corral came from the package or a clone, so the project sees who leaves and after which version.
- When a download doesn't check out (its SHA-256 doesn't match, the version has none published, or the build provenance attestation doesn't verify), install.sh sends a ping naming the check and the version: an alarm that a published package may be broken or replaced. A package from `CORRAL_URL` sends nothing.
- The same switches turn all of it off: `corral config telemetry off`, `CORRAL_NO_TELEMETRY=1` or `DO_NOT_TRACK=1`. `--check` still sends nothing.

## 0.1.19

Closing Corral leaves no tabs behind, SQL in R splits like DuckDB, the audit log names SQL, and the skill is called `corral`.

- Corral's skill is called `corral` now, not `corral-r`: it covers R, Python and SQL. The installer takes away the links an older Corral made under the old name, so agents don't see the skill twice; a folder of yours with that name stays.
- Closing Corral (ctrl+b shift+q) closes what the layout opened, so no tabs are left behind: the agent when herdr says it is idle (`claude --continue` picks the conversation up again), and the terminal tab when nothing runs there. An agent at work or waiting for you, or a command running, stays, and the message says so; a tab you opened Corral from is never closed.
- SQL in R splits statements the way DuckDB does: `$$…$$` and `$tag$…$tag$` strings, `E'…'` strings with backslashes, and comments inside comments no longer cut a query in two. A live test runs one SQL file in R and in Python and checks they give the same results.
- SQL from an agent is recorded as SQL. The audit log keeps `lang: "sql"` and the console it ran in (`console`), and the approval and review popups say SQL; `code` is still exactly what ran in the console. Before, such a request showed as R or Python.

## 0.1.18

SQL in your R or Python session, tables twice as fast, and `corral` opening herdr from any terminal.

- SQL, first-class. In a `.sql` file, Ctrl+R runs the statement under the cursor and F5 the whole file, on DuckDB in the console in view, R or Python. The session's data frames are tables under their own names, read without a copy, and the result is a data frame in the session: `sql_result`, or the name a `-- name: x` line gives it. Files work as tables too (`FROM 'data/*.parquet'`). Agents run SQL with `corral eval --lang sql`, through the same harness, and the palette runs any selection as SQL. DuckDB loads with the first query (about 0.2 s, once per session), so a session that never runs SQL doesn't pay for it; after that, a `GROUP BY` over a million rows takes about 20 ms. The installer puts duckdb in your R library, already compiled on Ubuntu and Debian, and in the Python console.
- Tables open about twice as fast. The table viewer and Mirador keep one DuckDB open while you use a table, instead of starting one for every query. With 10 million rows (`bench/view.R`, median of 10 runs, before with DuckDB 1.4.4):
  - Parquet from nanoparquet: the first screen in 14 ms instead of 30, a page from the middle in 8 ms instead of 16, and every row sorted in 25 ms instead of 35.
  - Parquet from arrow: 14 ms instead of 33, 15 ms instead of 25, and 35 ms instead of 49.
  - CSV, without either: 348 ms instead of 1.3 s, 753 ms instead of 705 ms, and 254 ms instead of 854 ms.
  DuckDB uses about 35 MB for the first screen and up to about 100 MB after sorting ten million rows. It closes 30 seconds after the last query, and with the viewer, so nothing stays open at rest.
- DuckDB 1.5.6, the latest 1.x. DuckDB 2.0.1 starts three times slower (22 ms against 8 ms), and with one DuckDB per query it took the first screen of ten million rows from 33 ms to 82 ms, so Corral stays on 1.x for now. Corral also runs DuckDB without `AI_AGENT`, since DuckDB 2 changes how it answers when that is set.
- Closing Corral (ctrl+b shift+q) takes you back to the tab you opened it from, instead of leaving you on the agent. The agent keeps running in its tab.
- `corral` opens herdr too. Typed in any terminal outside herdr, it opens herdr on the folder's workspace, with the layout, and starts herdr's server first when none is running. Typing it again in the same folder goes back to that workspace.
- A new layout opens a tab named `terminal` next to it, with a shell in the project folder, since the layout has none of its own. There is one per workspace, and when `corral` creates the workspace, its own shell tab becomes the terminal tab. `corral config terminal_tab off`, or the palette, leaves it out.
- Corral tells you when there is a new version. Opening the workspace asks GitHub which version is the latest, at most once a day and in the background, and herdr shows a notice once per new version. `corral config update_check off`, or the palette, stops it. Besides the install count, this is the only request Corral makes, and it carries nothing about you.
- The install count turns off in the config: `corral config telemetry off`, or the palette, holds in every shell, where `CORRAL_NO_TELEMETRY=1` in `.bashrc` doesn't reach an agent's shell.
- When the installation stops because something required is missing, the ping says so, with the requirements by name (herdr, R, R packages…), so the project learns what keeps people out. install.sh does the same when it stops before Corral is on the machine (macOS, another processor). `--check` still sends nothing.

## 0.1.17

Corral opens with one word: type `corral` in herdr.

- `corral` alone, typed in a herdr pane, opens the workspace, like ctrl+b a. Outside herdr, or when an agent runs it without a terminal, it still shows the list of commands, which now says so. The installer's last steps say it too.
- On an update, the anonymous install ping also says which version it replaced (just the number, like `0.1.16`), so the project can see which versions people update from. Nothing else was added to it, and `CORRAL_NO_TELEMETRY=1` still turns it off.

## 0.1.16

Tables open as fast in Python as in R from the first day: the installer adds nanoparquet for R, and the Python console comes with pyarrow, ready to start.

- The installer installs nanoparquet when R doesn't have it, instead of suggesting it: `View()` opens big tables faster and R's memory stays steady. It goes in your own R library, without sudo, already compiled from Posit's package manager on the Ubuntu and Debian releases it builds for (seconds), from CRAN otherwise.
- The Python console comes with pyarrow, so `View()` saves pandas tables as parquet: a million rows in 0.16 s and 18 MB, instead of 0.95 s and 46 MB as CSV, and dates stay dates. A version your project pins wins, and the console starts as fast. With uv, the installer puts IPython, cloudpickle and pyarrow in uv's cache, so the first Python console starts at once.

## 0.1.15

The layout opens the agent you have, the installer asks which one when you have several, and a shell that asks something as it starts no longer swallows Corral's commands.

- The layout opens the agent you have. It opened Claude Code unless you set another, so with opencode, Codex or Gemini CLI the agent pane said command not found. Now it opens the first one installed of Claude Code, Codex, Gemini CLI and opencode, or a shell when there is none, with a notice saying how to choose. The palette chooses one ("Agents: use … in the agent pane", or none), `corral config agent` takes any command, and the installer says which agent it found. With several and none chosen, the installer asks once which one to open, in a terminal; Enter leaves it to Corral.
- Gemini CLI finds Corral's skill: the installer also puts it in `~/.gemini/skills` when Gemini CLI is installed.

### Fixes

- A shell that asks something as it starts, like zsh's compinit about insecure folders, took the first letter of the command Corral typed as its answer: the panes said `command not found: orral` and nothing started. Corral now types its commands after a space, which answers such a question instead, and the commands start whole. The Troubleshooting page says how to fix the zsh warning itself.

## 0.1.14

Mirador keeps plots at a size you choose, whatever the window.

- Mirador keeps plots at a size you choose. A menu next to SVG and PNG draws R plots at 800 × 500, 1000 × 650, 1200 × 800 or any size you type, whatever the window, instead of stretching them to fit a maximized one; **Window** fits them as before. A smaller window shows them whole and scaled down, and SVG, PNG and Copy use that size. It is a setting, `corral config plot_size 1000x650`, so the palette has it ("Plots: Mirador draws them at…") and every browser shares it. Python figures keep their `figsize`.

## 0.1.13

`corral update`, from a terminal or the palette, and a clone that builds itself again after a `git pull`.

- `corral update` installs the latest version when there is a newer one, with the same install line as the first time (it checks the package's SHA-256 and its build provenance). `corral update --check` only says whether there is one, and the palette has "Update: install the latest version of Corral", in a popup. A Corral installed from a copy of the repository isn't replaced: it says to `git pull` there and run `sh install.sh`.

### Fixes

- From a copy of the repository, `sh install.sh` after a `git pull` to a new version kept the old `corral`: it only built one when there was none. It now builds when the binary isn't the code's version.

## 0.1.12

A harness that shows exactly what runs, keeps a sensitive session sensitive after a crash, asks an agent even in a terminal of its own, and records every request; and live tests in CI.

- The approval popup shows exactly what runs. Control and invisible characters in an agent's code, which a terminal acts on or draws as nothing, show as `␛`, `␍` or `<U+202E>` under a warning, and long lines wrap instead of being cut. Before, an escape in a comment could make the popup show other code than the code that ran. The console prints such code written out too, before its echo, and the review popup shows a result's text without its colors.
- A sensitive session comes back sensitive after a crash: its objects return into a session marked like the one that crashed, before anything is loaded, and the console says so.
- Loosening the harness asks when the command comes from a pane herdr sees an agent in, even when the agent runs its commands in a terminal of its own.
- The audit log keeps requests that never got to run (no console, a busy one, a turn that didn't come), with why.
- Docs: the security page lists how an agent set on it still gets past the harness, and the agent permissions that hold.
- The live tests run in GitHub Actions, by hand before each version.

### Fixes

- A Python console that crashed sometimes didn't come back: the terminal left stray input on the shell's line, the shell refused the command that starts the console, and the next request waited three minutes.

## 0.1.11

Objects that outlive a crash, an R console that comes back by itself, Corral's files kept private to you, and a harness that asks before an agent loosens it.

- Corral's files are yours only. The state folder, `~/.local/state/corral`, holds the audit log, your console history, previews of your variables and the agents' requests, and Corral now keeps it at `700`: it fixes an existing install's folder, and everything already inside, the next time it runs, and `corral doctor` checks it. The audit log, the console history and the logs are created readable only by you. This matters on shared servers.
- The audit log keeps 90 days. `corral config audit_days` changes that (a number of days, or `forever`), and the palette has "Agents: keep the audit log 30 days", "90 days" and "a year".
- Less is left on disk: a console that starts deletes the folders of sessions that ended, an agent's requests and their full output go after a day, and Ark's and Mirador's logs start over when they pass 5 MB.
- Docs: the security model lists what stays on your disk, what each file holds and for how long.
- Tests run on every pull request. Live tests (`cargo test --test live -- --ignored`) start their own herdr and R and check what agents rely on: two agents at once take turns, timeouts, Ctrl+C, a crash in the middle of a request, and reading the variables while they change.
- The R console comes back by itself when R crashes: it starts a new session in the same pane and says so, instead of leaving the shell. A crash right after starting doesn't restart it again and again.
- The R console checks the signature of every message from Ark (HMAC-SHA256 with the session's key) and drops any that doesn't match; the editor's language server bridge does too. A connection to Ark that drops while R runs is opened again.
- Objects outlive a crash. Each R and Python console keeps a checkpoint of its objects, and if the session ends without you closing it (a crash, herdr or WSL closed), the next console in the project gets them back, packages and modules first, and says which it couldn't keep (connections). After a command that changed an object, a copy of the process writes what changed while you keep working: the prompt waits 2 to 7 ms in R and under half a millisecond in Python, and a command that changes nothing about 0.3 ms. Closing the console yourself starts clean, as before. `corral config checkpoints off` or the palette turns it off; `checkpoint_max_mb` (1024) caps the size of an object kept. The measurements are in the docs' Performance page.
- Live tests for the Python console too (objects kept, two agents at once, timeouts, Ctrl+C, a crash, the variables), and for R's restart and the editor's language server.
- The harness asks before an agent loosens it. `corral config` run from a shell without a terminal, as an agent's is, that gives agents more room (`agents on`, a looser `agent_output`, fewer `audit_days`) opens a popup for you to allow or refuse, and the audit log keeps the request. From the palette or your terminal nothing changes, and tightening never asks.
- Corral's popups ignore keys for their first half second, so an Enter meant for another pane can't approve an agent's code, and they name the agent that asks (Claude Code, Codex…) and its pane.

### Fixes

- With restricted results (`agent_output structure` or `review`), a request past its timeout told the agent the path of the file with the full output, the values it wasn't meant to see. It now only says the user sees the output in the console.
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
