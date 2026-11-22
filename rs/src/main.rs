//! corral, fast path. The commands that run on every keystroke (Ctrl+R, F5,
//! ctrl+b t, ctrl+b y) and the console launcher are handled here, talking to
//! herdr's socket directly; everything else, and the rare cases (starting a
//! console, reading the clipboard), goes to the Python version in bin/corral.

mod console;
mod env;
mod herdr;
mod json;
mod layout;
mod send;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("send") => send::run(rest, &args),
        Some("_console") => console::run(rest, &args),
        Some("toggle") => layout::toggle(rest, &args),
        Some("switch") => layout::switch(rest, &args),
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
