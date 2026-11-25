//! corral: Corral's command, all of it. It talks to herdr's socket directly
//! and has no dependencies beyond the C library.

#[macro_use]
mod i18n;
mod approve;
mod ark;
mod audit;
mod console;
mod editor;
mod env;
mod eval;
mod herdr;
mod install;
mod json;
mod layout;
mod lsp;
mod mirador;
mod plots;
mod send;
mod setup;
mod sha256;
mod shortcuts;
mod table;
mod term;
mod theme;
mod toml;
mod vars;
mod versions;
mod workspace;
mod zip;
mod zmtp;

use std::io::IsTerminal;

const HELP: &str = "corral: R and Python consoles, tables and plots next to your agents, in herdr.

Usage: corral                     opens the workspace here: in herdr, like ctrl+b a; elsewhere, herdr with it
       corral COMMAND [OPTIONS]   (corral COMMAND --help for its options)

Workspace
  layout       opens the workspace (ctrl+b a; --focus code, ctrl+b shift+s; --focus notebook)
  close        closes the script, the console and the variables; keeps the agent (ctrl+b shift+q)
  toggle       expands the script (or the agent) or the console (ctrl+b t)
  switch       switches the console between R and Python (ctrl+b y)
  script       adds the editor, or reads, saves or writes the draft
  console      marks a pane as the R or Python console (ctrl+b m)
  versions     the R version (rig) or Python version (uv) the consoles run
  keys         the shortcut palette (ctrl+b space); --list, --json, --change

Code and data
  send         sends code to the console (Ctrl+R, F5, ctrl+b enter)
  eval         runs code in your console and prints the result (for agents)
  audit        what agents ran in your consoles (--last N, --json)
  vars         the variables (window, --json for agents, --split)
  view         opens a table (parquet, csv…), --mirador for the browser
  plots        the plots (ctrl+b f)
  mirador      Mirador, the view in your browser (--open, --url, --stop)
  open         opens a local link in the browser

Setup
  config       reads or changes the settings (language, agent, theme, R console…)
  theme        applies herdr's theme to the editor, the viewers and Mirador
  ark          Ark's status, or --install
  doctor       checks that everything is in order
  install      checks the requirements and repairs the installation (--check)
  update       installs the latest version (--check only says if there is one)
  uninstall    removes Corral from herdr, R, the editor and your agents
";

const HELP_ES: &str = "corral: consolas de R y Python, tablas y gráficos junto a tus agentes, en herdr.

Uso: corral                     abre el espacio de trabajo aquí: en herdr, como ctrl+b a; afuera, herdr con él
     corral COMANDO [OPCIONES]   (corral COMANDO --help para sus opciones)

Espacio de trabajo
  layout       abre el espacio de trabajo (ctrl+b a; --focus code, ctrl+b shift+s; --focus notebook)
  close        cierra el script, la consola y las variables; deja al agente (ctrl+b shift+q)
  toggle       expande el script (o el agente) o la consola (ctrl+b t)
  switch       la consola pasa a R o a Python (ctrl+b y)
  script       agrega el editor, o lee, guarda o escribe el borrador
  console      marca un panel como consola de R o Python (ctrl+b m)
  versions     la versión de R (rig) o de Python (uv) de las consolas
  keys         la paleta de atajos (ctrl+b space); --list, --json, --change

Código y datos
  send         envía código a la consola (Ctrl+R, F5, ctrl+b enter)
  eval         ejecuta código en tu consola e imprime el resultado (para agentes)
  audit        lo que los agentes ejecutaron en tus consolas (--last N, --json)
  vars         las variables (ventana, --json para agentes, --split)
  view         abre una tabla (parquet, csv…), --mirador para el navegador
  plots        los gráficos (ctrl+b f)
  mirador      Mirador, la vista en el navegador (--open, --url, --stop)
  open         abre un enlace local en el navegador

Instalación y ajustes
  config       lee o cambia la configuración (idioma, agente, tema, consola de R…)
  theme        aplica el tema de herdr al editor, los visores y Mirador
  ark          estado de Ark, o --install
  doctor       revisa que esté todo en orden
  install      revisa los requisitos y repara la instalación (--check)
  update       instala la última versión (--check solo dice si hay una)
  uninstall    quita Corral de herdr, R, el editor y tus agentes
";

/// `corral` alone, typed in a terminal: in a herdr pane it opens the workspace, as
/// ctrl+b a does (it runs `corral layout`, without the JSON that prints for
/// agents); in any other terminal, herdr opens with it. With no terminal (an
/// agent's command, a script), `corral` prints the help.
fn launch() -> Result<(), String> {
    if !env::in_herdr() {
        return workspace::launch_herdr();
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = std::process::Command::new(exe).arg("layout")
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        // `corral layout` already said why on stderr.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    env::keep_state_private(); // what agents ran, your history: yours only
    let result = match args.first().map(String::as_str) {
        Some("send") => send::run(rest, &args),
        Some("eval") => eval::run(rest),
        Some("_approve") => approve::run(rest),
        Some("audit") => audit::run(rest),
        Some("_console") => console::run(rest),
        Some("_editor") => editor::run(rest),
        Some("keys") => shortcuts::run(rest, false),
        Some("_keys") => shortcuts::run(rest, true),
        Some("toggle") => layout::toggle(rest),
        Some("switch") => layout::switch(rest),
        Some("_lsp") => lsp::run(rest),
        Some("vars") => vars::run(rest, &args, false),
        Some("_vars") => vars::run(rest, &args, true),
        Some("_mirador") => mirador::serve(),
        Some("_table") => table::run(rest, &args),
        Some("theme") => theme::cmd_theme(rest),
        Some("layout") => workspace::layout(rest),
        Some("close") => workspace::close(rest),
        Some("script") => workspace::script(rest),
        Some("console") => workspace::mark_console(rest),
        Some("versions") => versions::run(rest, false),
        Some("_versions") => versions::run(rest, true),
        Some("plots") => plots::cmd_plots(rest),
        Some("_plots") => plots::cmd_plots_tui(rest),
        Some("_plot-ready") => plots::cmd_plot_ready(rest),
        Some("_show-object") => plots::cmd_show_object(rest),
        Some("view") => plots::cmd_view(rest),
        Some("open") => plots::cmd_open(rest),
        Some("mirador") => plots::cmd_mirador(rest),
        Some("ark") => setup::cmd_ark(rest),
        Some("doctor") => setup::cmd_doctor(rest),
        Some("install") => install::cmd_install(rest),
        Some("uninstall") => install::cmd_uninstall(rest),
        Some("update") => install::cmd_update(rest, false),
        Some("_update") => install::cmd_update(rest, true),
        Some("_update-check") => install::cmd_update_check(),
        Some("config") => theme::cmd_config(rest),
        Some("--version" | "-V") => {
            println!("corral {} ({})", env!("CARGO_PKG_VERSION"), env::repo().display());
            Ok(())
        }
        None if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => launch(),
        None | Some("--help" | "-h" | "help") => {
            print!("{}", i18n::tr(HELP, HELP_ES));
            Ok(())
        }
        Some(other) => Err(trf!("unknown command '{other}'; corral --help lists them", "no conozco el comando '{other}'; corral --help muestra los que hay")),
    };
    env::keep_state_private(); // also when this command created it
    if let Err(e) = result {
        eprintln!("corral: {e}");
        std::process::exit(1);
    }
}
