//! corral, fast path. The commands that run on every keystroke (Ctrl+R, F5,
//! ctrl+b t, ctrl+b y) and the console launcher are handled here, talking to
//! herdr's socket directly; everything else, and the rare cases (starting a
//! console, reading the clipboard), goes to the Python version in bin/corral.

mod ark;
mod console;
mod editor;
mod env;
mod eval;
mod herdr;
mod json;
mod layout;
mod lsp;
mod mirador;
mod send;
mod sha256;
mod table;
mod term;
mod vars;
mod zmtp;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("send") => send::run(rest, &args),
        Some("eval") => eval::run(rest, &args),
        Some("_console") => console::run(rest, &args),
        Some("_editor") => editor::run(rest),
        Some("toggle") => layout::toggle(rest, &args),
        Some("switch") => layout::switch(rest, &args),
        Some("_lsp") => lsp::run(rest, &args),
        Some("vars") => vars::run(rest, &args, false),
        Some("_vars") => vars::run(rest, &args, true),
        Some("_mirador") => mirador::serve(),
        Some("_table") => table::run(rest, &args),
        Some("--version") => {
            println!("corral {} (Rust; el resto en {})", env!("CARGO_PKG_VERSION"), env::repo().join("bin/corral").display());
            Ok(())
        }
        _ => env::python_fallback(&args),
    };
    if let Err(e) = result {
        eprintln!("corral: {e}");
        std::process::exit(1);
    }
}
