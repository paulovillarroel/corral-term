//! corral: Corral's command, all of it. It talks to herdr's socket directly
//! and has no dependencies beyond the C library.

mod ark;
mod atajos;
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
mod table;
mod term;
mod theme;
mod toml;
mod vars;
mod workspace;
mod zip;
mod zmtp;

const HELP: &str = "corral: consolas de R y Python, tablas y gráficos junto a tus agentes, en herdr.

Uso: corral COMANDO [OPCIONES]   (corral COMANDO --help para sus opciones)

Espacio de trabajo
  layout       abre el espacio de trabajo (ctrl+b a; --foco codigo, ctrl+b shift+a)
  close        cierra el script, la consola y las variables; deja al agente (ctrl+b shift+c)
  toggle       expande el script (o el agente) o la consola (ctrl+b t)
  switch       la consola pasa a R o a Python (ctrl+b y)
  script       agrega el editor, o lee, guarda o escribe el borrador
  console      marca un panel como consola de R o Python (ctrl+b m)
  atajos       la paleta de atajos (ctrl+b space); --lista, --json, --cambiar

Código y datos
  send         envía código a la consola (Ctrl+R, F5, ctrl+b enter)
  eval         ejecuta código en tu consola e imprime el resultado (para agentes)
  vars         las variables (ventana, --json para agentes, --split)
  view         abre una tabla (parquet, csv…)
  plots        los gráficos (ctrl+b f)
  mirador      Mirador, la vista en el navegador (--open, --url, --stop)
  open         abre un enlace local en el navegador

Instalación y ajustes
  config       lee o cambia la configuración (tema, consola de R, servidor de Python…)
  theme        aplica el tema de herdr al editor, los visores y Mirador
  ark          estado de Ark, o --instalar
  doctor       revisa que esté todo en orden
  instalar     revisa los requisitos y repara la instalación (--revisar)
  desinstalar  quita Corral de herdr, R, el editor y Claude Code
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("send") => send::run(rest, &args),
        Some("eval") => eval::run(rest),
        Some("_console") => console::run(rest),
        Some("_editor") => editor::run(rest),
        Some("atajos") => atajos::run(rest, false),
        Some("_atajos") => atajos::run(rest, true),
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
        Some("plots") => plots::cmd_plots(rest),
        Some("_plots") => plots::cmd_plots_tui(rest),
        Some("_plot-ready") => plots::cmd_plot_ready(rest),
        Some("_show-object") => plots::cmd_show_object(rest),
        Some("view") => plots::cmd_view(rest),
        Some("open") => plots::cmd_open(rest),
        Some("mirador") => plots::cmd_mirador(rest),
        Some("ark") => setup::cmd_ark(rest),
        Some("doctor") => setup::cmd_doctor(rest),
        Some("instalar") => install::cmd_install(rest),
        Some("desinstalar") => install::cmd_uninstall(rest),
        Some("config") => theme::cmd_config(rest),
        Some("--version" | "-V") => {
            println!("corral {} ({})", env!("CARGO_PKG_VERSION"), env::repo().display());
            Ok(())
        }
        None | Some("--help" | "-h" | "ayuda") => {
            print!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!("no conozco el comando '{other}'; corral --help muestra los que hay")),
    };
    if let Err(e) = result {
        eprintln!("corral: {e}");
        std::process::exit(1);
    }
}
