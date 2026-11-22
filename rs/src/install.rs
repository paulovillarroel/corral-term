//! `corral install` and `corral uninstall`. install.sh downloads Corral (or
//! uses a clone), makes sure this program is built, and runs `corral install`
//! from there. First it checks everything Corral needs and says what to do
//! about what is missing; only when nothing required is missing does it
//! install. It never uses sudo and only writes inside HOME. Every file of the
//! user's it edits (herdr's config, ~/.Rprofile, Fresh's config) is backed up
//! first, and `corral uninstall` takes out what it added.

use crate::env;
use crate::i18n::tr;
use crate::json::{self, Value};
use crate::setup::{self, ARK_VERSION};
use crate::theme;
use crate::toml;
use crate::zip;
use std::io::IsTerminal;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HERDR_MIN: (u32, u32, u32) = (0, 9, 5);
const CRAN: &str = "https://cloud.r-project.org";
const KEYS_BEGIN: &str = "# >>> Corral: shortcuts (corral uninstall removes them) >>>";
/// The marker earlier versions wrote; install and uninstall still recognize it.
const KEYS_BEGIN_OLD: &str = "# >>> Corral: atajos (corral desinstalar los quita) >>>";
const KEYS_END: &str = "# <<< Corral <<<";
const RPROFILE_MARK: &str = "# Corral";
/// herdr action ids that were renamed: bindings to the old ones are moved to the new ones.
const RENAMED_ACTIONS: [(&str, &str); 4] = [
    ("\"corral.atajos\"", "\"corral.keys\""),
    ("\"corral.layout-codigo\"", "\"corral.layout-code\""),
    ("\"corral.tema-dark-2026\"", "\"corral.theme-dark-2026\""),
    ("\"corral.tema-herdr\"", "\"corral.theme-herdr\""),
];

/// A single-file program the installer downloads when it is missing:
/// name, shown name, version, what for (English, Spanish), URL, and the SHA-256 of
/// that version's package. The hashes live here, in Corral's own code (which the
/// installer checks), so each download is checked whether or not its project
/// publishes a checksum; a new version needs its new hash.
type Download = (&'static str, &'static str, &'static str, (&'static str, &'static str), &'static str, &'static str);

const DOWNLOADS: [Download; 3] = [
    ("fresh", "Fresh", "0.5.9", ("script editor", "editor de scripts"),
     "https://github.com/sinelaw/fresh/releases/download/v{v}/fresh-editor-x86_64-unknown-linux-musl.tar.gz",
     "97717593ded1f66946d9c598bbf9f2cdf9c53ed2b5d528c92d4e51b8137b3203"),
    ("duckdb", "DuckDB", "1.4.4", ("View() tables", "tablas de View()"),
     "https://github.com/duckdb/duckdb/releases/download/v{v}/duckdb_cli-linux-amd64.gz",
     "58882f65fcde335484857d595a6e7c1836eb2b125f78652fcfc813ad74ee348d"),
    ("air", "Air", "0.13.0", ("R code formatting", "formato del código de R"),
     "https://github.com/posit-dev/air/releases/download/{v}/air-x86_64-unknown-linux-gnu.tar.gz",
     "3909b12870df0b6a44e3f40376cd59aca373791c0abedd02dcbda7b136e4573c"),
];

// ----------------------------------------------------------------------- places

fn xdg(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| env::home().join(default))
}

fn data() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

fn state() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

fn bin() -> PathBuf {
    env::home().join(".local/bin")
}

fn dest() -> PathBuf {
    std::env::var_os("CORRAL_DIR").map(PathBuf::from).unwrap_or_else(|| data().join("corral-term"))
}

fn tools() -> PathBuf {
    data().join("corral/herramientas")
}

fn manifest() -> PathBuf {
    state().join("corral/instalacion.json")
}

fn rprofile() -> PathBuf {
    std::env::var_os("R_PROFILE_USER").map(PathBuf::from).unwrap_or_else(|| env::home().join(".Rprofile"))
}

fn fresh_dir() -> PathBuf {
    theme::config_home().join("fresh")
}

fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| env::home().join(".claude"))
}

/// Where Corral's skill goes. It follows the open Agent Skills standard
/// (agentskills.io): ~/.agents/skills is the shared place (Codex and others read
/// it), and Claude Code reads its own folder.
fn skill_places() -> Vec<(PathBuf, &'static str)> {
    let mut places = vec![(env::home().join(".agents/skills/corral-r"),
                           tr("~/.agents/skills (Codex and other agents)", "~/.agents/skills (Codex y otros agentes)"))];
    if which("claude").is_some() || claude_dir().is_dir() {
        places.push((claude_dir().join("skills/corral-r"), "~/.claude/skills (Claude Code)"));
    }
    places
}

// ----------------------------------------------------------------------- output

struct Style {
    green: &'static str,
    red: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    bold: &'static str,
    dim: &'static str,
    off: &'static str,
}

fn style() -> Style {
    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        Style { green: "\x1b[32m", red: "\x1b[31m", yellow: "\x1b[33m", cyan: "\x1b[36m", bold: "\x1b[1m", dim: "\x1b[2m", off: "\x1b[0m" }
    } else {
        Style { green: "", red: "", yellow: "", cyan: "", bold: "", dim: "", off: "" }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Ok,
    Missing,
    Warn,
    Download,
}

fn mark(s: &Style, state: State) -> String {
    let (m, c) = match state {
        State::Ok => ("✓", s.green),
        State::Missing => ("✗", s.red),
        State::Warn => ("!", s.yellow),
        State::Download => ("↓", s.cyan),
    };
    format!("{c}{m}{}", s.off)
}

fn step(text: &str, state: State) {
    println!("  {} {text}", mark(&style(), state));
}

fn short(p: &Path) -> String {
    match p.strip_prefix(env::home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

// ----------------------------------------------------------------------- small helpers

fn which(name: &str) -> Option<PathBuf> {
    env::which(name).or_else(|| Some(bin().join(name)).filter(|p| p.is_file()))
}

fn version_of(program: &Path, arg: &str) -> String {
    let Ok(out) = Command::new(program).arg(arg).output() else { return String::new() };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find(|w| w.contains('.') && w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|w| w.trim_matches('.').to_string()).unwrap_or_default()
}

fn as_tuple(v: &str) -> (u32, u32, u32) {
    let mut it = v.split('.').map(|x| x.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.to_lowercase().contains("microsoft")).unwrap_or(false)
}

fn is_corral_dir(d: &Path) -> bool {
    d.join("rs/Cargo.toml").is_file() && d.join("herdr/herdr-plugin.toml").is_file()
}

fn backup(path: &Path, done: &mut Vec<String>) {
    if path.exists() {
        let copy = path.with_file_name(format!("{}.before-corral-{}", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), env::stamp()));
        if std::fs::copy(path, &copy).is_ok() {
            done.push(copy.to_string_lossy().into_owned());
        }
    }
}

/// A symlink at `at` pointing to `target`, replacing an older symlink or file.
fn link(target: &Path, at: &Path) -> Result<(), String> {
    if let Some(d) = at.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    if at.is_symlink() || at.is_file() {
        let _ = std::fs::remove_file(at);
    }
    symlink(target, at).map_err(|e| format!("{}: {e}", at.display()))
}

fn points_into(p: &Path, folder: &Path) -> bool {
    p.is_symlink() && match (std::fs::canonicalize(p), std::fs::canonicalize(folder)) {
        (Ok(a), Ok(b)) => a.starts_with(b),
        _ => false,
    }
}

fn install_r(pkgs: &[&str]) -> String {
    let list = pkgs.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    format!("Rscript -e 'install.packages(c({list}), repos = \"{CRAN}\")'")
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d == dir)).unwrap_or(false)
}

// ----------------------------------------------------------------------- checks

struct Check {
    group: &'static str, // required | installer | optional
    state: State,
    name: String,
    detail: String,
    fix: Vec<String>, // what to do, line by line; a line starting with two spaces is a command
}

fn check(group: &'static str, state: State, name: &str, detail: impl Into<String>, fix: &[&str]) -> Check {
    Check { group, state, name: name.into(), detail: detail.into(), fix: fix.iter().map(|s| s.to_string()).collect() }
}

fn checks(in_place: bool, src: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    let target = if in_place { src.to_path_buf() } else { dest() };

    // Required -------------------------------------------------------------
    out.push(check("required", State::Ok, tr("System", "Sistema"), format!("Linux x86_64{}", if is_wsl() { " (WSL)" } else { "" }), &[]));
    match which("herdr") {
        None => out.push(check("required", State::Missing, "herdr", tr("not installed", "no está instalado"),
                               &[tr("Install herdr, the terminal Corral lives in:", "Instala herdr, la terminal donde vive Corral:"),
                                 "  curl -fsSL https://herdr.dev/install.sh | sh"])),
        Some(h) => {
            let v = version_of(&h, "--version");
            let ok = !v.is_empty() && as_tuple(&v) >= HERDR_MIN;
            let update = [tr("Corral needs herdr 0.9.5 or newer. Update it:", "Corral necesita herdr 0.9.5 o más nuevo. Actualízalo:"), "  herdr update"];
            out.push(check("required", if ok { State::Ok } else { State::Missing }, "herdr",
                           if v.is_empty() { tr("unknown version", "versión desconocida").into() } else { v }, if ok { &[] } else { &update }));
        }
    }
    match which("Rscript") {
        None => out.push(check("required", State::Missing, "R", tr("not installed", "no está instalado"),
                               &[tr("Install R from https://cran.r-project.org (on Ubuntu: sudo apt install r-base).",
                                    "Instala R desde https://cran.r-project.org (en Ubuntu: sudo apt install r-base).")])),
        Some(r) => {
            out.push(check("required", State::Ok, "R", version_of(&r, "--version"), &[]));
            let pkgs = ["httpgd", "jsonlite", "nanoparquet", "arrow", "rlang"];
            let has = setup::r_packages(&r, &pkgs);
            let missing: Vec<&str> = ["httpgd", "jsonlite"].into_iter().filter(|p| !has[pkgs.iter().position(|x| x == p).unwrap_or(0)]).collect();
            if missing.is_empty() {
                out.push(check("required", State::Ok, tr("R packages", "Paquetes de R"), tr("httpgd and jsonlite", "httpgd y jsonlite"), &[]));
            } else {
                let cmd = format!("  {}", install_r(&missing));
                let list = missing.join(", ");
                out.push(check("required", State::Missing, tr("R packages", "Paquetes de R"), trf!("missing {list}", "faltan {list}"),
                               &[tr("Install the R packages (httpgd draws the plots, jsonlite carries the variables):",
                                    "Instala los paquetes de R (httpgd dibuja los gráficos, jsonlite lleva las variables):"), &cmd]));
            }
            let extra: Vec<&str> = [("nanoparquet", has[2] || has[3]), ("rlang", has[4])].into_iter().filter(|(_, ok)| !ok).map(|(p, _)| p).collect();
            if !extra.is_empty() {
                let cmd = format!("  {}", install_r(&extra));
                let list = extra.join(", ");
                out.push(check("optional", State::Warn, tr("More R packages", "Más paquetes de R"), trf!("missing {list}", "faltan {list}"),
                               &[tr("nanoparquet opens large tables faster; rlang avoids redrawing a plot:",
                                    "nanoparquet abre tablas grandes más rápido; rlang evita volver a dibujar un gráfico:"), &cmd]));
            }
        }
    }
    let target_used = std::fs::read_dir(&target).map(|mut d| d.next().is_some()).unwrap_or(false);
    if !in_place && target_used && !is_corral_dir(&target) {
        out.push(check("required", State::Missing, tr("Folder", "Carpeta"),
                       trf!("{} already exists and isn't Corral's", "{} ya existe y no es de Corral", short(&target)),
                       &[tr("Move it, or pick another folder with CORRAL_DIR=/other/folder before installing.",
                            "Muévela, o elige otra carpeta con CORRAL_DIR=/otra/carpeta antes de instalar.")]));
    }

    // What the installer brings -------------------------------------------------
    out.push(check("installer", State::Ok, "corral", trf!("{VERSION}, built", "{VERSION}, compilado"), &[]));
    for (name, shown, v, what, ..) in DOWNLOADS {
        match which(name) {
            Some(have) => {
                let found = version_of(&have, "--version");
                let found = if found.is_empty() { tr("installed", "instalado").to_string() } else { found };
                out.push(check("installer", State::Ok, shown, trf!("{found}, you already have it", "{found}, ya lo tienes"), &[]));
            }
            None => {
                let what = tr(what.0, what.1);
                out.push(check("installer", State::Download, shown, trf!("will download {v} ({what})", "descargo {v} ({what})"), &[]));
            }
        }
    }
    if data().join("corral/ark").join(ARK_VERSION).join("ark").is_file() {
        out.push(check("installer", State::Ok, "Ark", trf!("{ARK_VERSION}, you already have it", "{ARK_VERSION}, ya lo tienes"), &[]));
    } else {
        out.push(check("installer", State::Download, "Ark",
                       trf!("will download {ARK_VERSION} (Positron's R console)", "descargo {ARK_VERSION} (la consola de R de Positron)"), &[]));
    }

    // Optional -------------------------------------------------------------
    match which("uv") {
        Some(uv) => out.push(check("optional", State::Ok, "uv", trf!("{} (Python console)", "{} (consola de Python)", version_of(&uv, "--version")), &[])),
        None => out.push(check("optional", State::Warn, "uv", tr("without uv there is no Python console", "sin uv no hay consola de Python"),
                               &[tr("To use Python in Corral, install uv:", "Para usar Python en Corral instala uv:"),
                                 "  curl -LsSf https://astral.sh/uv/install.sh | sh"])),
    }
    let places: Vec<&str> = skill_places().iter().map(|(_, shown)| *shown).collect();
    let places = places.join(tr(" and ", " y "));
    out.push(check("optional", State::Ok, tr("Skill for agents", "Skill para agentes"), trf!("goes in {places}", "la dejo en {places}"), &[]));
    let browser = tr("Browser", "Navegador");
    if is_wsl() {
        out.push(check("optional", State::Ok, browser, tr("the Windows one, for Mirador", "el de Windows, para Mirador"), &[]));
    } else if env::which("xdg-open").is_some() {
        out.push(check("optional", State::Ok, browser, tr("xdg-open, for Mirador", "xdg-open, para Mirador"), &[]));
    } else {
        out.push(check("optional", State::Warn, browser, tr("no xdg-open, so Mirador only shows its address", "sin xdg-open, Mirador solo muestra su dirección"),
                       &[tr("Install xdg-utils (on Ubuntu: sudo apt install xdg-utils).", "Instala xdg-utils (en Ubuntu: sudo apt install xdg-utils).")]));
    }
    if on_path(&bin()) {
        out.push(check("optional", State::Ok, "PATH", trf!("{} is on the PATH", "{} está en el PATH", short(&bin())), &[]));
    } else {
        out.push(check("optional", State::Warn, "PATH",
                       trf!("{} isn't on the PATH; R and the editor wouldn't find corral", "{} no está en el PATH; R y el editor no encontrarían corral", short(&bin())),
                       &[tr("Add this line at the end of ~/.bashrc (or ~/.zshrc) and open a new terminal:",
                            "Agrega esta línea al final de ~/.bashrc (o ~/.zshrc) y abre una terminal nueva:"),
                         "  export PATH=\"$HOME/.local/bin:$PATH\""]));
    }
    let mine = bin().join("corral");
    if (mine.is_symlink() || mine.exists()) && !points_into(&mine, &target) {
        let real = std::fs::canonicalize(&mine).unwrap_or_else(|_| mine.clone());
        out.push(check("optional", State::Warn, tr("Another corral", "Otro corral"),
                       trf!("{} points to {}; I'll replace it with the new one", "{} apunta a {}; lo cambio por el nuevo", short(&mine), short(&real)), &[]));
    }
    if let Some(current) = env::which("corral") {
        if current.parent() != Some(bin().as_path()) && !points_into(&current, &target) {
            let fix = trf!("Delete it or put {} earlier on the PATH.", "Bórralo o pon {} antes en el PATH.", short(&bin()));
            out.push(check("optional", State::Warn, tr("Another corral", "Otro corral"),
                           trf!("there's one in {}; if it comes first on the PATH, it hides the new one",
                                "hay uno en {}; si va antes en el PATH, tapa al nuevo", short(&current)), &[&fix]));
        }
    }
    out
}

/// Print the check; true when nothing required is missing.
fn report(rows: &[Check], again: &str) -> bool {
    let s = style();
    let width = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    for (group, title) in [("required", tr("Required", "Necesario")), ("installer", tr("Provided by the installer", "Lo pone el instalador")),
                           ("optional", tr("Optional", "Opcional"))] {
        println!("\n{}{title}{}", s.bold, s.off);
        for r in rows.iter().filter(|r| r.group == group) {
            println!("  {} {}{}  {}", mark(&s, r.state), r.name, " ".repeat(width - r.name.chars().count()), r.detail);
        }
    }
    let blocking: Vec<&Check> = rows.iter().filter(|r| r.state == State::Missing).collect();
    let optional: Vec<&Check> = rows.iter().filter(|r| r.state == State::Warn && !r.fix.is_empty()).collect();
    if !blocking.is_empty() {
        let n = blocking.len();
        let what = if n == 1 {
            tr("1 thing is missing before installing.", "Falta 1 cosa antes de instalar.").to_string()
        } else {
            trf!("{n} things are missing before installing.", "Faltan {n} cosas antes de instalar.")
        };
        println!("\n{}{}{what}{} {}\n", s.red, s.bold, s.off, tr("Nothing was changed.", "No cambié nada."));
        for (i, r) in blocking.iter().enumerate() {
            println!("  {}. {}", i + 1, r.fix[0]);
            for line in &r.fix[1..] {
                if let Some(cmd) = line.strip_prefix("  ") {
                    println!("     {}{cmd}{}", s.bold, s.off);
                } else {
                    println!("     {line}");
                }
            }
        }
        println!("\n  {}\n     {}{again}{}", tr("Then run the installation again:", "Después vuelve a correr la instalación:"), s.bold, s.off);
    }
    if !optional.is_empty() {
        println!("\n{}{}{}", s.dim, tr("Optional, whenever you like:", "Opcional, cuando quieras:"), s.off);
        for r in optional {
            println!("  • {}", r.fix[0]);
            for line in &r.fix[1..] {
                println!("    {}", line.trim_start());
            }
        }
    }
    blocking.is_empty()
}

// ----------------------------------------------------------------------- install steps

/// Copy a tree; `.git` and `__pycache__` stay out, and of rs/target only the built command.
fn copy_tree(from: &Path, to: &Path, root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        let (src, dst) = (entry.path(), to.join(&name));
        let rel = src.strip_prefix(root).unwrap_or(&src).to_path_buf();
        if name == ".git" || name == "__pycache__" {
            continue;
        }
        if rel.starts_with("rs/target") && rel != Path::new("rs/target") && rel != Path::new("rs/target/release")
            && rel != Path::new("rs/target/release/corral") {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            let target = std::fs::read_link(&src).map_err(|e| e.to_string())?;
            symlink(target, &dst).map_err(|e| e.to_string())?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst, root)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// Copy the downloaded Corral to its folder, replacing an earlier copy.
fn place(src: &Path) -> Result<PathBuf, String> {
    let d = dest();
    let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "corral-term".into());
    let (new, old) = (d.with_file_name(format!("{name}.nuevo")), d.with_file_name(format!("{name}.anterior")));
    for p in [&new, &old] {
        if is_corral_dir(p) {
            let _ = std::fs::remove_dir_all(p);
        }
    }
    copy_tree(src, &new, src)?;
    if d.exists() {
        std::fs::rename(&d, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&new, &d).map_err(|e| e.to_string())?;
    if is_corral_dir(&old) {
        let _ = std::fs::remove_dir_all(&old);
    }
    step(&trf!("Corral in {}", "Corral en {}", short(&d)), State::Ok);
    Ok(d)
}

fn download(name: &str, shown: &str, v: &str, what: &str, url_t: &str, sha: &str, done: &mut Value) {
    let url = url_t.replace("{v}", v);
    println!("  {}", trf!("… downloading {shown} {v} ({what})", "… descargando {shown} {v} ({what})"));
    let data = match setup::download(&url) {
        Ok(d) => d,
        Err(e) => return step(&trf!("couldn't download {shown}: {e}. Install it later and Corral will use it.",
                                    "no pude descargar {shown}: {e}. Instálalo después y Corral lo usará."), State::Warn),
    };
    if let Err(e) = setup::checked(&format!("{shown} {v}"), &data, sha) {
        return step(&e, State::Warn);
    }
    let program = if url.ends_with(".tar.gz") {
        zip::gunzip(&data).ok().and_then(|tar| zip::tar_member(&tar, name))
    } else {
        zip::gunzip(&data).ok()
    };
    let Some(program) = program else {
        return step(&trf!("{shown}: the download doesn't contain the program", "{shown}: el archivo descargado no trae el programa"), State::Warn);
    };
    let folder = tools().join(format!("{name}-{v}"));
    let _ = std::fs::remove_dir_all(&folder);
    let exe = folder.join(name);
    let written = std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&exe, program))
        .and_then(|_| std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)));
    if let Err(e) = written {
        return step(&format!("{shown}: {e}"), State::Warn);
    }
    if let Err(e) = link(&exe, &bin().join(name)) {
        return step(&e, State::Warn);
    }
    let mut list = done.get("herramientas").arr().to_vec();
    list.push(name.into());
    done.set("herramientas", Value::Arr(list));
    step(&trf!("{shown} {v} in {}, SHA-256 checked", "{shown} {v} en {}, SHA-256 comprobado", short(&bin().join(name))), State::Ok);
}

fn ark() {
    if data().join("corral/ark").join(ARK_VERSION).join("ark").is_file() {
        return step(&trf!("Ark {ARK_VERSION}, you already had it", "Ark {ARK_VERSION}, ya lo tenías"), State::Ok);
    }
    println!("  {}", trf!("… downloading Ark {ARK_VERSION} (the R console)", "… descargando Ark {ARK_VERSION} (la consola de R)"));
    match setup::install_ark(ARK_VERSION) {
        Ok(_) => step(&trf!("Ark {ARK_VERSION}, SHA-256 checked", "Ark {ARK_VERSION}, SHA-256 comprobado"), State::Ok),
        Err(e) => step(&trf!("{e}. The R console will be plain R; corral ark --install tries again.",
                             "{e}. La consola de R será R normal; corral ark --install lo reintenta."), State::Warn),
    }
}

fn herdr_out(args: &[&str]) -> Option<std::process::Output> {
    Command::new(which("herdr")?).args(args).output().ok()
}

fn herdr_plugin(dest: &Path) {
    let plugins = herdr_out(&["plugin", "list", "--json"])
        .and_then(|o| json::parse(String::from_utf8_lossy(&o.stdout).trim()).ok()).unwrap_or(Value::Null);
    let list = plugins.get("result").get("plugins").arr().to_vec();
    let mine = list.iter().find(|p| p.get("plugin_id").str().or(p.get("id").str()) == Some("corral"));
    if mine.is_some_and(|p| p.get("plugin_root").str().map(PathBuf::from) == Some(dest.join("herdr"))) {
        return step(tr("herdr plugin, already linked", "plugin de herdr, ya enlazado"), State::Ok);
    }
    if mine.is_some() {
        herdr_out(&["plugin", "unlink", "corral"]);
    }
    match herdr_out(&["plugin", "link", &dest.join("herdr").to_string_lossy()]) {
        Some(o) if o.status.success() => step(tr("herdr plugin linked", "plugin de herdr enlazado"), State::Ok),
        Some(o) => step(&trf!("couldn't link the herdr plugin: {}", "no pude enlazar el plugin de herdr: {}", String::from_utf8_lossy(&o.stderr).trim()),
                        State::Warn),
        None => step(tr("couldn't link the herdr plugin", "no pude enlazar el plugin de herdr"), State::Warn),
    }
}

/// Where Corral's marked block of keys starts (today's marker or the older one).
fn keys_block_start(text: &str) -> Option<usize> {
    [KEYS_BEGIN, KEYS_BEGIN_OLD].iter().filter_map(|m| text.find(m)).min()
}

/// Text without Corral's marked blocks of keys.
fn without_keys_block(text: &str) -> String {
    let mut text = text.to_string();
    while let Some(a) = keys_block_start(&text) {
        let Some(b) = text[a..].find(KEYS_END).map(|i| a + i) else { break };
        let head = text[..a].trim_end_matches('\n');
        let tail = text[b + KEYS_END.len()..].trim_start_matches('\n');
        text = format!("{head}\n{tail}");
    }
    text
}

/// Bindings to renamed herdr actions, moved to the new names.
fn with_renamed_actions(text: &str) -> String {
    RENAMED_ACTIONS.iter().fold(text.to_string(), |t, (old, new)| t.replace(old, new))
}

/// Corral's keys in herdr's config, between two markers. Keys the user already
/// has (Corral's or their own) are left alone; on an update only new ones are added.
fn herdr_keys(dest: &Path, done: &mut Value, saved: &mut Vec<String>) {
    let path = theme::herdr_config();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let cut = without_keys_block(&text);
    let outside = with_renamed_actions(&cut);
    let example = dest.join("herdr/keys.example.toml");
    let Ok(current) = toml::parse(&outside) else {
        return step(&trf!("left {} alone: herdr can't read it; copy the shortcuts from {}",
                          "no toqué {}: herdr no lo puede leer; copia los atajos de {}", short(&path), short(&example)), State::Warn);
    };
    let mine = current.get("keys").get("command").arr().to_vec();
    let taken: Vec<&str> = mine.iter().filter_map(|k| k.get("key").str()).collect();
    let bound: Vec<&str> = mine.iter().filter_map(|k| k.get("command").str()).collect();
    let wanted = std::fs::read_to_string(&example).ok().and_then(|t| toml::parse(&t).ok()).unwrap_or(Value::Null);
    let wanted: Vec<Value> = wanted.get("keys").get("command").arr().iter()
        .filter(|k| !bound.contains(&k.get("command").str().unwrap_or(""))).cloned().collect();
    let skipped: Vec<String> = wanted.iter().filter_map(|k| k.get("key").str()).filter(|k| taken.contains(k)).map(String::from).collect();
    let adding: Vec<&Value> = wanted.iter().filter(|k| !taken.contains(&k.get("key").str().unwrap_or(""))).collect();
    let new_text = if adding.is_empty() {
        if outside.trim().is_empty() { String::new() } else { outside.trim_end_matches('\n').to_string() + "\n" }
    } else {
        let mut block = vec![String::new(), KEYS_BEGIN.to_string()];
        for k in &adding {
            block.push("[[keys.command]]".into());
            for f in ["key", "type", "command", "description"] {
                block.push(format!("{f} = {}", k.get(f).to_json()));
            }
            block.push(String::new());
        }
        *block.last_mut().expect("block") = KEYS_END.to_string();
        outside.trim_end_matches('\n').to_string() + "\n" + &block.join("\n") + "\n"
    };
    let block_before = keys_block_start(&text);
    if new_text == text || (adding.is_empty() && block_before.is_none() && outside == cut) {
        return step(tr("herdr shortcuts, already there", "atajos de herdr, ya estaban"), State::Ok);
    }
    if !path.exists() {
        done.set("herdr_creado", true.into());
    }
    backup(&path, saved);
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = theme::write_herdr(&new_text) {
        return step(&trf!("herdr rejected the shortcuts ({e}); I left its configuration as it was",
                          "herdr rechazó los atajos ({e}); dejé su configuración como estaba"), State::Warn);
    }
    let prefix = current.get("keys").get("prefix").str().unwrap_or("ctrl+b").to_string();
    let shown = |k: &str| k.replace("prefix+", &format!("{prefix} "));
    let before: Vec<String> = block_before.map(|a| with_renamed_actions(&text[a..]).lines()
        .filter_map(|l| l.strip_prefix("command = ")).map(String::from).collect()).unwrap_or_default();
    let new: Vec<String> = adding.iter().filter(|k| !before.contains(&k.get("command").to_json()))
        .filter_map(|k| k.get("key").str().map(shown)).collect();
    let mut keys = new.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if new.len() > 3 {
        keys += tr(" and the rest", " y los demás");
    }
    let note = if skipped.is_empty() { String::new() } else {
        let used = skipped.iter().map(|k| shown(k)).collect::<Vec<_>>().join(", ");
        trf!(" (except {used}, which you already use: corral keys --change gives it another key)",
             " (salvo {used}, que ya usabas: corral keys --change le da otra tecla)")
    };
    if keys.is_empty() {
        step(&trf!("herdr shortcuts, updated{note}", "atajos de herdr, actualizados{note}"), State::Ok);
    } else {
        step(&trf!("herdr shortcuts: {keys}{note}", "atajos de herdr: {keys}{note}"), State::Ok);
    }
}

fn rprofile_step(dest: &Path, done: &mut Value, saved: &mut Vec<String>) {
    let path = rprofile();
    let line = format!("if (interactive()) try(source(\"{}\"))  {RPROFILE_MARK}", dest.join("r/corral.R").display());
    let mut lines: Vec<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().map(String::from).collect();
    if lines.contains(&line) {
        return step(&trf!("{}, already loaded Corral", "{}, ya cargaba Corral", short(&path)), State::Ok);
    }
    let old: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains("source(") && l.contains("/r/corral.R")).map(|(i, _)| i).collect();
    if !path.exists() {
        done.set("rprofile_creado", true.into());
    }
    backup(&path, saved);
    if let Some(&first) = old.first() {
        lines[first] = line;
        for &i in old[1..].iter().rev() {
            lines.remove(i);
        }
    } else {
        lines.push(line);
    }
    match std::fs::write(&path, lines.join("\n") + "\n") {
        Ok(()) => step(&trf!("{} loads Corral (it only kicks in inside Corral's consoles)", "{} carga Corral (solo se activa en sus consolas)", short(&path)),
                       State::Ok),
        Err(e) => step(&format!("{}: {e}", short(&path)), State::Warn),
    }
}

fn combo(b: &Value) -> (String, Vec<String>) {
    let mut mods: Vec<String> = b.get("modifiers").arr().iter().filter_map(|m| m.str().map(String::from)).collect();
    mods.sort();
    (b.get("key").str().unwrap_or("").to_lowercase(), mods)
}

/// The user's settings with Corral's laid over them (Corral's keys replace the user's for the same key).
fn merge(user: &Value, ours: &Value) -> Value {
    let mut out = user.clone();
    let Value::Obj(items) = ours else { return out };
    for (k, v) in items {
        if k == "keybindings" {
            if let Value::Arr(mine) = v {
                let taken: Vec<_> = mine.iter().map(combo).collect();
                let mut list: Vec<Value> = user.get(k).arr().iter().filter(|b| !taken.contains(&combo(b))).cloned().collect();
                list.extend(mine.iter().cloned());
                out.set(k, Value::Arr(list));
                continue;
            }
        }
        if matches!(v, Value::Obj(_)) && matches!(user.get(k), Value::Obj(_)) {
            out.set(k, merge(user.get(k), v));
        } else {
            out.set(k, v.clone());
        }
    }
    out
}

fn add_unique(done: &mut Value, key: &str, items: impl IntoIterator<Item = Value>) {
    let mut list = done.get(key).arr().to_vec();
    for i in items {
        if !list.contains(&i) {
            list.push(i);
        }
    }
    done.set(key, Value::Arr(list));
}

fn fresh(dest: &Path, done: &mut Value, saved: &mut Vec<String>) {
    let cfg = fresh_dir().join("config.json");
    let Some(ours) = std::fs::read_to_string(dest.join("fresh/config.json")).ok().and_then(|t| json::parse(&t).ok()) else {
        return step(tr("couldn't find Corral's editor configuration", "no encontré la configuración del editor de Corral"), State::Warn);
    };
    if cfg.exists() {
        match std::fs::read_to_string(&cfg).ok().and_then(|t| json::parse(&t).ok()) {
            Some(user @ Value::Obj(_)) => {
                let taken: Vec<_> = ours.get("keybindings").arr().iter().map(combo).collect();
                let theirs: Vec<Value> = user.get("keybindings").arr().iter()
                    .filter(|b| taken.contains(&combo(b)) && !ours.get("keybindings").arr().contains(b)).cloned().collect();
                add_unique(done, "fresh_atajos_tuyos", theirs);
                let added: Vec<Value> = ours.get("keybindings").arr().iter().filter(|b| !user.get("keybindings").arr().contains(b)).cloned().collect();
                add_unique(done, "fresh_atajos_corral", added);
                let merged = merge(&user, &ours);
                if merged != user {
                    backup(&cfg, saved);
                    let _ = std::fs::write(&cfg, merged.to_json_pretty(2));
                }
                step(tr("editor configuration (Fresh), merged with yours", "configuración del editor (Fresh), unida a la tuya"), State::Ok);
            }
            _ => step(&trf!("left {} alone: it isn't valid JSON; compare it with {}", "no toqué {}: no es JSON válido; compara con {}",
                            short(&cfg), short(&dest.join("fresh/config.json"))), State::Warn),
        }
    } else {
        let _ = std::fs::create_dir_all(fresh_dir());
        let _ = std::fs::write(&cfg, ours.to_json_pretty(2));
        add_unique(done, "fresh_atajos_corral", ours.get("keybindings").arr().to_vec());
        done.set("fresh_creado", true.into());
        step(tr("editor configuration (Fresh)", "configuración del editor (Fresh)"), State::Ok);
    }
    let _ = link(&dest.join("fresh/plugins/corral.ts"), &fresh_dir().join("plugins/corral.ts"));
}

/// Corral's skill (the Agent Skills standard) where agents look for skills.
fn skill(dest: &Path) {
    let source = dest.join("skills/corral-r");
    for (at, shown) in skill_places() {
        if at.exists() && !at.is_symlink() {
            step(&trf!("left your {} alone; Corral's skill is in {}", "dejé tu {}; la skill de Corral está en {}", short(&at), short(&source)), State::Warn);
            continue;
        }
        match link(&source, &at) {
            Ok(()) => step(&trf!("Corral's skill in {shown}", "skill de Corral en {shown}"), State::Ok),
            Err(e) => step(&e, State::Warn),
        }
    }
}

fn install(src: &Path, in_place: bool, again: &str) -> Result<(), String> {
    let s = style();
    println!("{}Corral {VERSION}{}: {}", s.bold, s.off,
             tr("checking what it needs, without changing anything yet.", "reviso lo que necesita, sin cambiar nada todavía."));
    if !report(&checks(in_place, src), again) {
        std::process::exit(1);
    }
    println!("\n{}{}{}", s.bold, tr("Installing", "Instalando"), s.off);
    let before = env::read_json(&manifest());
    let mut done = Value::obj();
    done.set("version", VERSION.into());
    let (y, mo, d, h, mi, _) = env::local_now();
    done.set("fecha", format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}").into());
    done.set("en_el_lugar", in_place.into());
    for key in ["fresh_atajos_tuyos", "fresh_atajos_corral"] {
        done.set(key, Value::Arr(before.get(key).arr().to_vec()));
    }
    for key in ["fresh_creado", "rprofile_creado", "herdr_creado"] {
        if before.get(key) == &Value::Bool(true) {
            done.set(key, true.into());
        }
    }
    let mut saved: Vec<String> = before.get("respaldos").arr().iter().filter_map(|v| v.str().map(String::from)).collect();
    let earlier = saved.len();

    let dest = if in_place { src.to_path_buf() } else { place(src)? };
    done.set("carpeta", dest.to_string_lossy().to_string().into());
    let exe = dest.join("rs/target/release/corral");
    if !exe.is_file() {
        return Err(trf!("{} is missing: build it with cargo build --release in {}", "falta {}: compílalo con cargo build --release en {}",
                        exe.display(), dest.join("rs").display()));
    }
    link(&exe, &bin().join("corral"))?;
    step(&trf!("corral command in {}", "comando corral en {}", short(&bin().join("corral"))), State::Ok);
    for (name, shown, v, what, url, sha) in DOWNLOADS {
        if which(name).is_none() {
            download(name, shown, v, tr(what.0, what.1), url, sha, &mut done);
        }
    }
    ark();
    herdr_plugin(&dest);
    herdr_keys(&dest, &mut done, &mut saved);
    rprofile_step(&dest, &mut done, &mut saved);
    fresh(&dest, &mut done, &mut saved);
    skill(&dest);
    theme::apply();
    step(tr("herdr's theme colors in the editor, the console and Mirador", "colores del tema de herdr en el editor, la consola y Mirador"), State::Ok);
    done.set("respaldos", Value::Arr(saved.iter().map(|b| Value::from(b.as_str())).collect()));
    env::write_json(&manifest(), &done);

    let (bold, off) = (s.bold, s.off);
    println!("\n{}{}{}{off} {}", s.green, bold, tr("Done.", "Listo."), tr("To get started:", "Para empezar:"));
    let mut n = 1;
    if !on_path(&bin()) {
        let line = "export PATH=\"$HOME/.local/bin:$PATH\"";
        println!("  {n}. {}", trf!("Add {bold}{line}{off} at the end of ~/.bashrc and open a new terminal.",
                                   "Agrega {bold}{line}{off} al final de ~/.bashrc y abre una terminal nueva."));
        n += 1;
    }
    println!("  {n}. {}", tr("Open herdr and go to a project folder.", "Abre herdr y entra a la carpeta de un proyecto."));
    println!("  {}. {}", n + 1, trf!("{bold}ctrl+b a{off} opens the workspace: the agent, the script, the R console and the variables.",
                                     "{bold}ctrl+b a{off} abre el espacio de trabajo: el agente, el script, la consola de R y las variables."));
    println!("  {}. {}", n + 2, trf!("{bold}ctrl+b space{off} shows every shortcut.", "{bold}ctrl+b space{off} muestra todos los atajos."));
    let copies: Vec<String> = saved[earlier..].iter().map(|b| short(Path::new(b))).collect();
    if !copies.is_empty() {
        println!("\n{}{}{}{}", s.dim, tr("Copies of the files I changed: ", "Copias de los archivos que cambié: "), copies.join(", "), s.off);
    }
    println!("{}{}{}", s.dim, tr("If something doesn't work: corral doctor. To remove it: corral uninstall.",
                                 "Si algo no anda: corral doctor. Para quitarlo: corral uninstall."), s.off);
    if env::config().get("language").is_null() {
        println!("{}{}{}", s.dim, tr("Corral speaks English; corral config language es switches it to Spanish.",
                                     "Corral habla inglés por defecto; corral config language es lo deja en español."), s.off);
    }
    count_install(before.get("version").str().is_some(), in_place);
    Ok(())
}

/// One anonymous ping after a successful install, so the project knows how many
/// people install Corral and from which countries: the version, the system (linux or
/// wsl), new install or update, and published package or clone. The site adds the
/// country from Vercel's geolocation and stores no IP (api/installed.js).
/// CORRAL_NO_TELEMETRY=1 or DO_NOT_TRACK=1 turns it off; CORRAL_TELEMETRY_URL points it
/// elsewhere (tests). It waits 5 seconds at most and never fails the install.
fn count_install(update: bool, in_place: bool) {
    let set = |var: &str| std::env::var(var).is_ok_and(|v| !v.is_empty() && v != "0");
    if set("CORRAL_NO_TELEMETRY") || set("DO_NOT_TRACK") {
        return;
    }
    let Some(curl) = env::which("curl") else { return };
    let body = format!(r#"{{"version":"{VERSION}","system":"{}","kind":"{}","source":"{}"}}"#,
                       if is_wsl() { "wsl" } else { "linux" }, if update { "update" } else { "new" },
                       if in_place { "clone" } else { "release" });
    let url = std::env::var("CORRAL_TELEMETRY_URL").unwrap_or_else(|_| "https://corral-term.dev/api/installed".into());
    let sent = Command::new(curl)
        .args(["-fsS", "-m", "5", "-X", "POST", "-H", "Content-Type: application/json", "--data", &body, &url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|st| st.success());
    if sent {
        let s = style();
        println!("{}{}{}", s.dim, tr("Counted this install anonymously (version, system and country). CORRAL_NO_TELEMETRY=1 turns it off.",
                                     "Conté esta instalación de forma anónima (versión, sistema y país). CORRAL_NO_TELEMETRY=1 lo desactiva."), s.off);
    }
}

/// `corral install [--from FOLDER] [--in-place] [--check] [--rerun COMMAND]`.
pub fn cmd_install(args: &[String]) -> Result<(), String> {
    let (mut src, mut in_place, mut only_check, mut again) = (None::<PathBuf>, false, false, "corral install".to_string());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => src = it.next().map(PathBuf::from),
            "--in-place" => in_place = true,
            "--check" => only_check = true,
            "--rerun" => again = it.next().cloned().unwrap_or(again),
            other => return Err(trf!("unknown option {other}\nusage: corral install [--check]", "opción desconocida {other}\nuso: corral install [--check]")),
        }
    }
    let src = match src {
        Some(s) => std::fs::canonicalize(&s).map_err(|e| format!("{}: {e}", s.display()))?,
        None => {
            in_place = true; // repairing the installed copy
            env::repo()
        }
    };
    let in_place = in_place || std::fs::canonicalize(dest()).ok().as_deref() == Some(src.as_path());
    if only_check {
        let s = style();
        println!("{}Corral {VERSION}{}: {}", s.bold, s.off, tr("what it needs, without changing anything.", "lo que necesita, sin cambiar nada."));
        let ok = report(&checks(in_place, &src), &again);
        if !ok {
            std::process::exit(1);
        }
        println!("\n{}{}{} {}{}{}{}", s.green, tr("Everything needed is here.", "Está todo lo necesario."), s.off,
                 tr("To install: ", "Para instalar: "), s.bold, again.replace(" --check", ""), s.off);
        return Ok(());
    }
    install(&src, in_place, &again)
}

// ----------------------------------------------------------------------- uninstall

/// `corral uninstall`: take out what the installer added (and the user's own Corral lines it recognizes).
pub fn cmd_uninstall(_args: &[String]) -> Result<(), String> {
    let s = style();
    let done = env::read_json(&manifest());
    let dest = done.get("carpeta").str().map(PathBuf::from).unwrap_or_else(env::repo);
    let in_place = done.get("en_el_lugar") != &Value::Bool(false);
    let created = |k: &str| done.get(k) == &Value::Bool(true);
    let mut saved: Vec<String> = Vec::new();
    let mut fresh_note = false;
    println!("{}", trf!("{}Removing Corral{} from {}", "{}Quitando Corral{} de {}", s.bold, s.off, short(&dest)));

    if which("herdr").is_some() {
        herdr_out(&["plugin", "unlink", "corral"]);
        step(tr("herdr plugin", "plugin de herdr"), State::Ok);
        let path = theme::herdr_config();
        if let Ok(text) = std::fs::read_to_string(&path) {
            let cut = without_keys_block(&text);
            if cut != text {
                if created("herdr_creado") && cut.trim().is_empty() {
                    let _ = std::fs::remove_file(&path); // only Corral's lines were there
                } else {
                    backup(&path, &mut saved);
                    let _ = std::fs::write(&path, cut);
                }
                herdr_out(&["server", "reload-config"]);
                step(tr("herdr shortcuts", "atajos de herdr"), State::Ok);
            } else if text.contains("command = \"corral.") {
                step(&trf!("the installer didn't add your Corral shortcuts in {}; delete them by hand",
                           "tus atajos de Corral en {} no los agregó el instalador; bórralos a mano", short(&path)), State::Warn);
            }
        }
    }
    let rp = rprofile();
    if let Ok(text) = std::fs::read_to_string(&rp) {
        let lines: Vec<&str> = text.lines().collect();
        let kept: Vec<&str> = lines.iter().copied().filter(|l| !(l.contains("source(") && l.contains("/r/corral.R"))).collect();
        if kept.len() != lines.len() {
            if !kept.is_empty() || !created("rprofile_creado") {
                backup(&rp, &mut saved);
                let _ = std::fs::write(&rp, if kept.is_empty() { String::new() } else { kept.join("\n") + "\n" });
            } else {
                let _ = std::fs::remove_file(&rp); // only Corral's line was there
            }
            step(&short(&rp), State::Ok);
        }
    }
    let cfg = fresh_dir().join("config.json");
    let mut user = std::fs::read_to_string(&cfg).ok().and_then(|t| json::parse(&t).ok()).filter(|v| matches!(v, Value::Obj(_)));
    if user.is_some() && created("fresh_creado") {
        backup(&cfg, &mut saved);
        let _ = std::fs::remove_file(&cfg);
        step(&trf!("{}, which Corral had created", "{}, que había creado Corral", short(&cfg)), State::Ok);
        user = None;
    }
    if let Some(user) = user {
        let mut clean = user.clone();
        if let Value::Arr(list) = user.get("keybindings") {
            let ours = done.get("fresh_atajos_corral").arr();
            let mut kept: Vec<Value> = list.iter()
                .filter(|b| !b.get("action").str().unwrap_or("").starts_with("corral_") && !ours.contains(b)).cloned().collect();
            for b in done.get("fresh_atajos_tuyos").arr() {
                if !kept.contains(b) {
                    kept.push(b.clone());
                }
            }
            clean.set("keybindings", Value::Arr(kept));
        }
        if let Value::Obj(servers) = user.get("lsp") {
            let is_ours = |v: &Value| v.get("command").str() == Some("corral") || v.arr().iter().any(|s| s.get("command").str() == Some("corral"));
            clean.set("lsp", Value::Obj(servers.iter().filter(|(_, v)| !is_ours(v)).cloned().collect()));
        }
        if clean.get("theme").str() == Some("corral") {
            clean.remove("theme");
        }
        if clean != user {
            backup(&cfg, &mut saved);
            let _ = std::fs::write(&cfg, clean.to_json_pretty(2));
            step(tr("Corral's shortcuts, theme and language servers in the editor", "atajos, tema y servidores de lenguaje de Corral en el editor"), State::Ok);
            fresh_note = true;
        }
    }
    let _ = std::fs::remove_file(fresh_dir().join("themes/corral.json"));
    for at in [bin().join("corral"), fresh_dir().join("plugins/corral.ts"), claude_dir().join("skills/corral-r"),
               env::home().join(".agents/skills/corral-r")] {
        if points_into(&at, &dest) && std::fs::remove_file(&at).is_ok() {
            step(&short(&at), State::Ok);
        }
    }
    for (name, ..) in DOWNLOADS {
        let at = bin().join(name);
        if points_into(&at, &tools()) {
            let _ = std::fs::remove_file(at);
        }
    }
    for (folder, what) in [(tools(), tr("downloaded programs (Fresh, DuckDB, Air)", "programas descargados (Fresh, DuckDB, Air)")),
                           (data().join("corral/ark"), "Ark"),
                           (data().join("corral/jupyter"), tr("Ark's kernel definition", "definición del kernel de Ark"))] {
        if folder.exists() && std::fs::remove_dir_all(&folder).is_ok() {
            step(what, State::Ok);
        }
    }
    let _ = std::fs::remove_dir(data().join("corral")); // only when nothing else is left there
    if !in_place && is_corral_dir(&dest) && std::fs::remove_dir_all(&dest).is_ok() {
        step(&short(&dest), State::Ok);
    }
    let _ = std::fs::remove_file(manifest());

    println!("\n{}{}{}{}", s.green, s.bold, tr("Done.", "Listo."), s.off);
    if in_place {
        println!("  {}", trf!("The folder {} is yours (a copy of the repository), so I left it.",
                              "La carpeta {} es tuya (una copia del repositorio), así que la dejé.", short(&dest)));
    }
    println!("  {}", trf!("Your settings and sessions are still in {} and {}; delete them if you're not coming back.",
                          "Tus ajustes y sesiones siguen en {} y {}; bórralos si no vas a volver.",
                          short(&theme::config_home().join("corral")), short(&state().join("corral"))));
    if fresh_note {
        println!("  {}", tr("The editor keeps general settings Corral put there (VS Code-style keys, format on save); \
                             the copy below has your configuration from before.",
                            "El editor conserva ajustes generales que puso Corral (teclado estilo VS Code, formato al guardar); \
                             la copia de abajo tiene tu configuración de antes."));
    }
    if !saved.is_empty() {
        println!("{}{}{}{}", s.dim, tr("Copies of the files I changed: ", "Copias de los archivos que cambié: "),
                 saved.iter().map(|b| short(Path::new(b))).collect::<Vec<_>>().join(", "), s.off);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_block_old_and_new_markers() {
        let block = |begin: &str| format!("[keys]\nprefix = \"ctrl+b\"\n\n{begin}\n[[keys.command]]\nkey = \"prefix+t\"\n{KEYS_END}\n\n# mine\n");
        for begin in [KEYS_BEGIN, KEYS_BEGIN_OLD] {
            let text = block(begin);
            assert_eq!(keys_block_start(&text), text.find(begin));
            assert_eq!(without_keys_block(&text), "[keys]\nprefix = \"ctrl+b\"\n# mine\n");
        }
        let both = format!("{}{}", block(KEYS_BEGIN_OLD), block(KEYS_BEGIN));
        assert!(!without_keys_block(&both).contains("prefix+t"));
        assert_eq!(without_keys_block("a = 1\n"), "a = 1\n");
    }

    #[test]
    fn every_download_has_a_sha256() {
        let hex = |h: &str| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        for (name, _, _, _, _, sha) in DOWNLOADS {
            assert!(hex(sha), "{name}");
        }
    }

    #[test]
    fn renamed_actions() {
        let text = "command = \"corral.atajos\"\ncommand = \"corral.layout-codigo\"\ncommand = \"corral.layout\"\n";
        assert_eq!(with_renamed_actions(text), "command = \"corral.keys\"\ncommand = \"corral.layout-code\"\ncommand = \"corral.layout\"\n");
    }
}
