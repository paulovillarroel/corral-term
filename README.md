<p align="center"><img src="site/assets/favicon.svg" width="56" alt=""></p>

<h1 align="center">Corral</h1>

<p align="center"><b>An agentic Data Science IDE that opens in milliseconds.</b><br>
An IDE in your terminal for the data science workflow, in R and Python, next to your agents.<br>
Written in Rust. DuckDB for your data.</p>

<p align="center">
  <a href="https://corral-term.dev/">Website</a> ·
  <a href="https://corral-term.dev/docs/">Documentation</a> ·
  <a href="https://corral-term.dev/docs/architecture.html">Architecture</a>
</p>

<p align="center"><img src="site/assets/shots/corral-layout.webp" width="860" alt="A Corral layout in herdr: Claude Code on the left runs code in the R console; on the right, the script editor, the R console and the variables pane"></p>

- **Script, console, variables, tables and plots** in a [herdr TUI](https://herdr.dev/tui/) workspace: R on [Ark](https://github.com/posit-dev/ark), Python in IPython.
- **Big tables at once.** `View()` asks DuckDB only for what's on screen: ten million rows show up in about 34 ms.
- **Agents in your session.** Any agent runs code in your console with `corral eval`, guided by an [Agent Skills](https://agentskills.io) skill.
- **Light.** R ready in 0.45 s and 155 MB in use, against 13.3 s and 3.2 GB for Positron ([how it was measured](https://corral-term.dev/docs/performance.html)).

## Install

```bash
curl -LsSf https://corral-term.dev/install.sh | sh
```

You need Linux x86_64 or Windows with WSL, herdr TUI 0.9.5 or newer, and R with `httpgd` and `jsonlite`. The installer checks everything before it changes anything and brings the rest ([what you need](https://corral-term.dev/docs/install.html#before-you-install)).

Then open herdr in a project folder and press <kbd>ctrl+b</kbd> <kbd>a</kbd>. <kbd>ctrl+b</kbd> <kbd>space</kbd> lists every shortcut.

Version 0.1.0 · Linux and WSL; macOS not yet · [MIT license](LICENSE).
