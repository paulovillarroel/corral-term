//! `corral _editor FILE -- EDITOR ARGS…`: the script pane. It runs the editor
//! and, when the editor ends (you quit it, or the pane or the layout closes),
//! throws the scratch file away: a draft is for writing something quick, and
//! what you want to keep you save into the project with Ctrl+S.

use crate::i18n::tr;
use std::path::Path;
use std::process::Command;

extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}
const SIGHUP: i32 = 1;
const SIGTERM: i32 = 15;
const SIG_IGN: usize = 1;

pub fn run(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral _editor FILE -- EDITOR…", "uso: corral _editor ARCHIVO -- EDITOR…");
    let split = args.iter().position(|a| a == "--").ok_or(usage)?;
    let file = args[..split].first().ok_or(usage)?;
    let cmd = &args[split + 1..];
    let program = cmd.first().ok_or(tr("the editor command is missing", "falta el comando del editor"))?;
    // Corral's editor plugin reads the language setting itself, so it follows a switch live.
    let mut child = Command::new(program).args(&cmd[1..]).spawn()
        .map_err(|e| trf!("could not open {program}: {e}", "no pude abrir {program}: {e}"))?;
    // Closing the pane hangs up the editor and this process together: this one
    // stays (after starting the editor, so the editor doesn't inherit it) to clean up.
    // SAFETY: setting signal dispositions to "ignore" for this process only.
    unsafe {
        signal(SIGHUP, SIG_IGN);
        signal(SIGTERM, SIG_IGN);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    discard(Path::new(file));
    std::process::exit(status.code().unwrap_or(0))
}

/// Remove a scratch file and Corral's live copy of it, then its folder if
/// nothing else is left there (a file you saved there by hand stays).
pub fn discard(file: &Path) {
    let Some(name) = file.file_name().and_then(|n| n.to_str()) else { return };
    if !crate::workspace::is_scratch_name(name) {
        return;
    }
    let Some(dir) = file.parent() else { return };
    let in_scratch = dir.parent().is_some_and(|p| p.file_name().is_some_and(|n| n == ".corral") || p.ends_with("corral/borradores"));
    if !in_scratch {
        return;
    }
    for name in [name, ".actual.R", ".pedido.json", ".pedido.tmp", ".respuesta.json"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    let _ = std::fs::remove_dir(dir); // fails, as it should, if something else is there
}
