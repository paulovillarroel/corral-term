//! `corral install` and `corral uninstall`. install.sh downloads Corral (or
//! uses a clone), makes sure this program is built, and runs `corral install`
//! from there. First it checks everything Corral needs and says what to do
//! about what is missing, offering to bring herdr and jsonlite itself;
//! only when nothing required is missing does it install. It never uses sudo
//! and only writes inside HOME. Every file of the
//! user's it edits (herdr's config, ~/.Rprofile, Fresh's config, and ~/.bashrc or
//! ~/.zshrc for the PATH) is backed up first, and `corral uninstall` takes out what
//! it added, except the PATH line.

use crate::env;
use crate::i18n::tr;
use crate::json::{self, Value};
use crate::setup::{self, PerSystem, ARK_VERSION, SYSTEM};
use crate::theme;
use crate::toml;
use crate::zip;
use std::io::IsTerminal;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
/// name, shown name, version, what for (English, Spanish), and for each system
/// (setup::SYSTEMS) the URL and the SHA-256 of that version's package. The hashes live
/// here, in Corral's own code (which the installer checks), so each download is checked
/// whether or not its project publishes a checksum; a new version needs its new hashes.
type Download = (&'static str, &'static str, &'static str, (&'static str, &'static str), PerSystem);

const DOWNLOADS: [Download; 3] = [
    ("fresh", "Fresh", "0.5.9", ("script editor", "editor de scripts"), [
        ("https://github.com/sinelaw/fresh/releases/download/v{v}/fresh-editor-x86_64-unknown-linux-musl.tar.gz",
         "97717593ded1f66946d9c598bbf9f2cdf9c53ed2b5d528c92d4e51b8137b3203"),
        ("https://github.com/sinelaw/fresh/releases/download/v{v}/fresh-editor-aarch64-apple-darwin.tar.xz",
         "aea215709bce0e1db25afe1040ff597e203e38a99a4d4302b9972e6a606a6f71"),
        ("https://github.com/sinelaw/fresh/releases/download/v{v}/fresh-editor-x86_64-apple-darwin.tar.xz",
         "9f1ff6bc9e62c5857c92c72f05d551a7604d8131951244e32b1fb28fd90372de"),
    ]),
    ("duckdb", "DuckDB", "1.5.6", ("View() tables", "tablas de View()"), [
        ("https://github.com/duckdb/duckdb/releases/download/v{v}/duckdb_cli-linux-amd64.gz",
         "cce2687a38681db644c248198b99ecd86170ff47e324c5f33e50c0042f142839"),
        ("https://github.com/duckdb/duckdb/releases/download/v{v}/duckdb_cli-osx-arm64.gz",
         "7f19ac71f0a4bde308a247ac556d93ee95c03adc327ec85eeab9113294f11dd5"),
        ("https://github.com/duckdb/duckdb/releases/download/v{v}/duckdb_cli-osx-amd64.gz",
         "388ce7e8a96307d040a27709a4c193f455ad1d3bbcd576c118bf1ae8158973e7"),
    ]),
    ("air", "Air", "0.13.0", ("R code formatting", "formato del código de R"), [
        ("https://github.com/posit-dev/air/releases/download/{v}/air-x86_64-unknown-linux-gnu.tar.gz",
         "3909b12870df0b6a44e3f40376cd59aca373791c0abedd02dcbda7b136e4573c"),
        ("https://github.com/posit-dev/air/releases/download/{v}/air-aarch64-apple-darwin.tar.gz",
         "43a063302643c49c2e0eb98162a4a3e9388be1113252ff647e817378c251354f"),
        ("https://github.com/posit-dev/air/releases/download/{v}/air-x86_64-apple-darwin.tar.gz",
         "f49a1659147224000da9d87b9a13dafc49170b7f40c5b6a90d4d874827c4329f"),
    ]),
];

/// herdr, when it is missing and you say yes: the version Corral was tested with, from
/// herdr's own GitHub release, checked against the SHA-256 written here like the
/// downloads above. It goes where herdr's own installer puts it (~/.local/bin/herdr),
/// as your herdr: `herdr update` keeps it current, and `corral uninstall` leaves it.
const HERDR: (&str, PerSystem) = ("0.9.5", [
    ("https://github.com/herdrdev/herdr/releases/download/v{v}/herdr-linux-x86_64",
     "14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d"),
    ("https://github.com/herdrdev/herdr/releases/download/v{v}/herdr-macos-aarch64",
     "fb77c2232a05a8035421382c894392f4174e4e38a18e48af3dd0290157c3a7e2"),
    ("https://github.com/herdrdev/herdr/releases/download/v{v}/herdr-macos-x86_64",
     "5fb0a908ec7f6745e3e245738b06bfad56c46391c391ab3c0955fbead01f1369"),
]);

/// uv, for the Python console, when it's missing and you say yes (uv_step): a release
/// at least three days old, from uv's GitHub release (on Linux the static musl build,
/// which runs on any Linux), checked against the SHA-256 written here. uv and uvx go
/// where uv's own installer puts them (~/.local/bin), as your uv; uninstalling Corral
/// leaves them.
const UV: (&str, PerSystem) = ("0.13.9", [
    ("https://github.com/astral-sh/uv/releases/download/{v}/uv-x86_64-unknown-linux-musl.tar.gz",
     "58ccf5f1792b8a5a88d01bc910ea131a8b3db4a319a17ad5fad75e0845a3e271"),
    ("https://github.com/astral-sh/uv/releases/download/{v}/uv-aarch64-apple-darwin.tar.gz",
     "5aeca67f2283f75235de40d0bd9c0a5bbc5fce23b6d48b3bf9fa1b2db08382ca"),
    ("https://github.com/astral-sh/uv/releases/download/{v}/uv-x86_64-apple-darwin.tar.gz",
     "617a3211261b0fc739562f0071aa34e9739797c510208ede8e2977c30903d041"),
]);

/// The R package Corral can't work without: jsonlite carries the variables (the
/// variables pane, `corral vars`, the plot index). httpgd, for plots, goes in too, but
/// Corral works without it, so it doesn't stop the install (r_httpgd).
const R_NEEDS: [&str; 1] = ["jsonlite"];

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
    data().join("corral/tools")
}

fn manifest() -> PathBuf {
    state().join("corral/install.json")
}

/// The install record's keys as Corral 0.1.7 and earlier named them, and their names now.
const OLD_KEYS: [(&str, &str); 10] = [
    ("carpeta", "folder"), ("en_el_lugar", "in_place"), ("fecha", "date"), ("respaldos", "backups"),
    ("herramientas", "tools"), ("herdr_creado", "herdr_created"), ("rprofile_creado", "rprofile_created"),
    ("fresh_creado", "fresh_created"), ("fresh_atajos_tuyos", "fresh_keys_user"), ("fresh_atajos_corral", "fresh_keys_corral"),
];

/// An install by Corral 0.1.7 or earlier: move its record (instalacion.json) and
/// its downloads (corral/herramientas) to their English names, and point the
/// links in ~/.local/bin at the new folder, so install and uninstall find them.
fn migrate_old_install() {
    let old = state().join("corral/instalacion.json");
    if old.exists() && !manifest().exists() {
        let mut done = Value::obj();
        if let Value::Obj(fields) = env::read_json(&old) {
            for (k, v) in fields {
                done.set(OLD_KEYS.iter().find(|(o, _)| *o == k).map_or(k.as_str(), |(_, n)| n), v);
            }
        }
        env::write_json(&manifest(), &done);
        if manifest().exists() {
            let _ = std::fs::remove_file(&old);
        }
    }
    let old = data().join("corral/herramientas");
    if !old.is_dir() || tools().exists() || std::fs::rename(&old, tools()).is_err() {
        return;
    }
    for (name, ..) in DOWNLOADS {
        let at = bin().join(name);
        let Ok(target) = std::fs::read_link(&at) else { continue };
        if let Ok(rest) = target.strip_prefix(&old) {
            let _ = link(&tools().join(rest), &at);
        }
    }
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

/// Corral's skill, by the name agents see. Until 0.1.18 it was corral-r, from when
/// Corral was R only; the installer takes the old links away (old_skill_links).
const SKILL: &str = "corral";
const OLD_SKILL: &str = "corral-r";

/// Where Corral's skill goes. It follows the open Agent Skills standard
/// (agentskills.io): ~/.agents/skills is the shared place (Codex, opencode and
/// others read it), and Claude Code and Gemini CLI read their own folders.
fn skill_places() -> Vec<(PathBuf, &'static str)> {
    let mut places = vec![(env::home().join(".agents/skills").join(SKILL),
                           tr("~/.agents/skills (Codex, opencode and other agents)", "~/.agents/skills (Codex, opencode y otros agentes)"))];
    if which("claude").is_some() || claude_dir().is_dir() {
        places.push((claude_dir().join("skills").join(SKILL), "~/.claude/skills (Claude Code)"));
    }
    if which("gemini").is_some() || gemini_dir().is_dir() {
        places.push((gemini_dir().join("skills").join(SKILL), "~/.gemini/skills (Gemini CLI)"));
    }
    places
}

fn gemini_dir() -> PathBuf {
    env::home().join(".gemini")
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

/// Whether your R has pak: the installer then installs R packages with it, as you
/// would. The checklist's probe sets it (checks); otherwise R is asked once.
static HAS_PAK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn has_pak() -> bool {
    *HAS_PAK.get_or_init(|| crate::versions::rscript().or_else(|| which("Rscript")).is_some_and(|r| setup::r_packages(&r, &["pak"])[0]))
}

/// The command to install these R packages yourself: pak's if your R has it, else
/// install.packages from CRAN.
fn install_r(pkgs: &[&str]) -> String {
    install_r_with(pkgs, has_pak())
}

fn install_r_with(pkgs: &[&str], pak: bool) -> String {
    let list = pkgs.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    if pak {
        format!("Rscript -e 'pak::pak(c({list}))'") // net: text
    } else {
        format!("Rscript -e 'install.packages(c({list}), repos = \"{CRAN}\")'") // net: text
    }
}

/// "a", "a and b", "a, b and c", in the user's language.
fn and_list(items: &[&str]) -> String {
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{}{}{last}", rest.join(", "), tr(" and ", " y ")),
        _ => items.join(""),
    }
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d == dir)).unwrap_or(false)
}

// ----------------------------------------------------------------------- checks

struct Check {
    group: &'static str, // required | installer | optional
    /// A required check's name for the stop ping (herdr, r…), the same in any language.
    id: &'static str,
    state: State,
    name: String,
    detail: String,
    fix: Vec<String>, // what to do, line by line; a line starting with two spaces is a command
    /// What is missing that the installer can bring itself, if you say yes (bring):
    /// a short name, and what it would do.
    brings: Option<(String, String)>,
}

fn check(group: &'static str, state: State, name: &str, detail: impl Into<String>, fix: &[&str]) -> Check {
    Check { group, id: "", state, name: name.into(), detail: detail.into(), fix: fix.iter().map(|s| s.to_string()).collect(), brings: None }
}

impl Check {
    fn id(mut self, id: &'static str) -> Check {
        self.id = id;
        self
    }

    fn brings(mut self, short: impl Into<String>, what: impl Into<String>) -> Check {
        self.brings = Some((short.into(), what.into()));
        self
    }
}

fn checks(in_place: bool, src: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    let target = if in_place { src.to_path_buf() } else { dest() };

    // Required -------------------------------------------------------------
    out.push(check("required", State::Ok, tr("System", "Sistema"), format!("{}{}", setup::SYSTEMS[SYSTEM], if is_wsl() { " (WSL)" } else { "" }), &[]));
    match which("herdr") {
        None => out.push(check("required", State::Missing, "herdr", tr("not installed", "no está instalado"),
                               &[tr("Install herdr, the terminal Corral lives in:", "Instala herdr, la terminal donde vive Corral:"),
                                 "  curl -fsSL https://herdr.dev/install.sh | sh"]).id("herdr")
                     .brings("herdr", trf!("herdr {}, the terminal Corral lives in: from herdr's GitHub release, SHA-256 checked, into {}",
                                           "herdr {}, la terminal donde vive Corral: desde la versión publicada en GitHub, con su SHA-256 comprobado, en {}",
                                           HERDR.0, short(&bin())))),
        Some(h) => {
            let v = version_of(&h, "--version");
            let ok = !v.is_empty() && as_tuple(&v) >= HERDR_MIN;
            let update = [tr("Corral needs herdr 0.9.5 or newer. Update it:", "Corral necesita herdr 0.9.5 o más nuevo. Actualízalo:"), "  herdr update"];
            let shown = if v.is_empty() { tr("unknown version", "versión desconocida").to_string() } else { v.clone() };
            let row = check("required", if ok { State::Ok } else { State::Missing }, "herdr", shown.clone(), if ok { &[] } else { &update }).id("herdr_old");
            // Too old: herdr's own updater brings the latest, with your yes (bring).
            out.push(if ok { row } else {
                row.brings("herdr", trf!("herdr update, from {shown} to its latest version: herdr's own updater, which checks what it downloads",
                                         "herdr update, de {shown} a su última versión: el actualizador del propio herdr, que comprueba lo que descarga"))
            });
        }
    }
    match crate::versions::rscript().or_else(|| which("Rscript")) {
        None => out.push(check("required", State::Missing, "R", tr("not installed", "no está instalado"),
                               &[if cfg!(target_os = "macos") {
                                   tr("Install R from CRAN, the .pkg for your Mac (Apple silicon or Intel): https://cloud.r-project.org/bin/macosx/ (or with rig).",
                                      "Instala R desde CRAN, el .pkg para tu Mac (Apple silicon o Intel): https://cloud.r-project.org/bin/macosx/ (o con rig).")
                               } else {
                                   tr("Install R from https://cran.r-project.org (on Ubuntu: sudo apt install r-base).",
                                      "Instala R desde https://cran.r-project.org (en Ubuntu: sudo apt install r-base).")
                               }]).id("r")),
        Some(r) => {
            out.push(check("required", State::Ok, "R", version_of(&r, "--version"), &[]));
            let pkgs = ["httpgd", "jsonlite", "nanoparquet", "arrow", "rlang", "duckdb", "DBI", "pak"];
            let has = setup::r_packages(&r, &pkgs);
            let _ = HAS_PAK.set(has[7]);
            let missing: Vec<&str> = R_NEEDS.into_iter().filter(|p| !has[pkgs.iter().position(|x| x == p).unwrap_or(0)]).collect();
            if missing.is_empty() {
                out.push(check("required", State::Ok, tr("R packages", "Paquetes de R"), "jsonlite", &[]));
            } else {
                let cmd = format!("  {}", install_r(&missing));
                let (list, names) = (missing.join(", "), and_list(&missing));
                out.push(check("required", State::Missing, tr("R packages", "Paquetes de R"), trf!("missing {list}", "falta {list}"),
                               &[tr("Install jsonlite, which carries the variables to Corral:",
                                    "Instala jsonlite, que lleva las variables a Corral:"), &cmd]).id("r_packages")
                         .brings(names.clone(), if has[7] {
                             trf!("{names}, the R package that carries the variables: with pak, into your R library, without sudo",
                                  "{names}, el paquete de R que lleva las variables: con pak, en tu biblioteca de R, sin sudo")
                         } else {
                             trf!("{names}, the R package that carries the variables: into your R library, without sudo",
                                  "{names}, el paquete de R que lleva las variables: en tu biblioteca de R, sin sudo")
                         }));
            }
            // httpgd draws R's plots for Mirador and the agent. Corral works without it
            // (no plots there), so it doesn't stop the install: the installer puts it in
            // your R library, and if it can't, it says how (r_httpgd).
            if has[0] {
                out.push(check("installer", State::Ok, "httpgd", tr("you already have it (R's plots)", "ya lo tienes (los gráficos de R)"), &[]));
            } else {
                out.push(check("installer", State::Download, "httpgd",
                               tr("will install it in your R library (R's plots, in Mirador and for the agent)",
                                  "lo instalo en tu biblioteca de R (los gráficos de R, en Mirador y para el agente)"), &[]));
            }
            // nanoparquet even with arrow there: View() with arrow is slower, and in a
            // long session R's memory keeps growing with each table (bench/repeat.R).
            // The installer puts it in your R library when it's missing.
            if has[2] {
                out.push(check("installer", State::Ok, "nanoparquet", tr("you already have it (View()'s tables)", "ya lo tienes (las tablas de View())"), &[]));
            } else {
                out.push(check("installer", State::Download, "nanoparquet",
                               tr("will install it in your R library (big tables open faster in View(), R's memory stays steady)",
                                  "lo instalo en tu biblioteca de R (las tablas grandes abren más rápido en View() y la memoria de R se mantiene estable)"), &[]));
            }
            // SQL in R (corral_sql) runs on duckdb. Posit's package manager has it
            // compiled for the systems it builds for (seconds); compiling it takes a
            // long while, so elsewhere it is optional.
            if has[5] && has[6] {
                out.push(check("installer", State::Ok, "duckdb", tr("you already have it (SQL in R)", "ya lo tienes (SQL en R)"), &[]));
            } else if r_binaries() {
                out.push(check("installer", State::Download, "duckdb",
                               tr("will install it in your R library, already compiled (SQL in R)",
                                  "lo instalo en tu biblioteca de R, ya compilado (SQL en R)"), &[]));
            } else {
                let cmd = format!("  {}", install_r(&["duckdb", "DBI"]));
                out.push(check("optional", State::Warn, "duckdb", tr("not installed (SQL in R)", "no está instalado (SQL en R)"),
                               &[tr("SQL in R runs on duckdb; here it has to compile, which takes a while:",
                                    "SQL en R corre sobre duckdb; aquí hay que compilarlo, y tarda un rato:"), &cmd]));
            }
            if !has[4] {
                let cmd = format!("  {}", install_r(&["rlang"]));
                out.push(check("optional", State::Warn, tr("More R packages", "Más paquetes de R"), tr("missing rlang", "falta rlang"),
                               &[tr("rlang avoids redrawing a plot, and lets checkpoints see objects changed in place:",
                                    "rlang evita volver a dibujar un gráfico, y permite que los respaldos vean objetos cambiados en el lugar:"), &cmd]));
            }
        }
    }
    let target_used = std::fs::read_dir(&target).map(|mut d| d.next().is_some()).unwrap_or(false);
    if !in_place && target_used && !is_corral_dir(&target) {
        out.push(check("required", State::Missing, tr("Folder", "Carpeta"),
                       trf!("{} already exists and isn't Corral's", "{} ya existe y no es de Corral", short(&target)),
                       &[tr("Move it, or pick another folder with CORRAL_DIR=/other/folder before installing.",
                            "Muévela, o elige otra carpeta con CORRAL_DIR=/otra/carpeta antes de instalar.")]).id("folder"));
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
                               &[tr("For Python in Corral, the installer offers uv while it installs, in a terminal; or install it yourself:",
                                    "Para Python en Corral, el instalador ofrece uv mientras instala, en una terminal; o instálalo tú:"),
                                 "  curl -LsSf https://astral.sh/uv/install.sh | sh"])),
    }
    let places = and_list(&skill_places().iter().map(|(_, shown)| *shown).collect::<Vec<_>>());
    // The agent the layout opens: the one chosen, else the first one installed.
    let agent = tr("Agent", "Agente");
    let found: Vec<&str> = env::AGENTS.iter().filter(|(cmd, _)| env::which(cmd).is_some()).map(|(_, name)| *name).collect();
    match env::config().get("agent").str() {
        Some("none") => out.push(check("optional", State::Ok, agent, tr("none: the agent pane is a shell (corral config agent)", "ninguno: el panel del agente es una shell (corral config agent)"), &[])),
        Some(cmd) => out.push(check("optional", State::Ok, agent, &trf!("{}, your choice (corral config agent)", "{}, el que elegiste (corral config agent)", env::agent_name(cmd)), &[])),
        None if found.is_empty() => out.push(check("optional", State::Warn, agent,
            tr("none found (Claude Code, Codex, Gemini CLI, opencode): the agent pane will be a shell", "no encontré ninguno (Claude Code, Codex, Gemini CLI, opencode): el panel del agente será una shell"),
            &[tr("Install one; Corral opens the first it finds, and the palette (Agent: …) chooses another.", "Instala uno; Corral abre el primero que encuentre, y la paleta (Agente: …) elige otro.")])),
        None => {
            let more = if found.len() > 1 { trf!("; also {}: the palette chooses (Agent: …)", "; también {}: la paleta elige (Agente: …)", found[1..].join(", ")) } else { String::new() };
            out.push(check("optional", State::Ok, agent, &trf!("{} in the agent pane{more}", "{} en el panel del agente{more}", found[0]), &[]));
        }
    }
    out.push(check("optional", State::Ok, tr("Skill for agents", "Skill para agentes"), trf!("goes in {places}", "la dejo en {places}"), &[]));
    let browser = tr("Browser", "Navegador");
    if is_wsl() {
        out.push(check("optional", State::Ok, browser, tr("the Windows one, for Mirador", "el de Windows, para Mirador"), &[]));
    } else if cfg!(target_os = "macos") {
        out.push(check("optional", State::Ok, browser, tr("your default one (open), for Mirador", "el que tengas por defecto (open), para Mirador"), &[]));
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
                       &[if cfg!(target_os = "macos") {
                             tr("Add this line at the end of ~/.zshrc and open a new terminal:",
                                "Agrega esta línea al final de ~/.zshrc y abre una terminal nueva:")
                         } else {
                             tr("Add this line at the end of ~/.bashrc (or ~/.zshrc) and open a new terminal:",
                                "Agrega esta línea al final de ~/.bashrc (o ~/.zshrc) y abre una terminal nueva:")
                         },
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

/// Print the check; true when nothing required is missing. With `offer` (the
/// installer can ask you), what it can bring itself stays out of the instructions:
/// when that is all that's missing, the question comes next (bring); otherwise it
/// says it will bring it when you run it again.
fn report(rows: &[Check], again: &str, offer: bool) -> bool {
    let s = style();
    let width = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    for (group, title) in [("required", tr("Required", "Necesario")), ("installer", tr("Provided by the installer", "Lo pone el instalador")),
                           ("optional", tr("Optional", "Opcional"))] {
        println!("\n{}{title}{}", s.bold, s.off);
        for r in rows.iter().filter(|r| r.group == group) {
            println!("  {} {}{}  {}", mark(&s, r.state), r.name, " ".repeat(width - r.name.chars().count()), r.detail);
        }
    }
    let ok = missing_steps(rows, again, offer, true);
    let optional: Vec<&Check> = rows.iter().filter(|r| r.state == State::Warn && !r.fix.is_empty()).collect();
    if !optional.is_empty() {
        println!("\n{}{}{}", s.dim, tr("Optional, whenever you like:", "Opcional, cuando quieras:"), s.off);
        for r in optional {
            println!("  • {}", r.fix[0]);
            for line in &r.fix[1..] {
                println!("    {}", line.trim_start());
            }
        }
    }
    ok
}

/// What to do about what is missing, step by step; true when nothing is. With `offer`,
/// what the installer can bring itself is left out of the steps (see report).
/// `unchanged` says nothing was installed yet (false after bring installed part of it).
fn missing_steps(rows: &[Check], again: &str, offer: bool, unchanged: bool) -> bool {
    let s = style();
    let blocking: Vec<&Check> = rows.iter().filter(|r| r.state == State::Missing).collect();
    let (later, manual): (Vec<&Check>, Vec<&Check>) = blocking.iter().partition(|r| offer && r.brings.is_some());
    if !manual.is_empty() {
        let n = manual.len();
        let what = if n == 1 {
            tr("1 thing is missing before installing.", "Falta 1 cosa antes de instalar.").to_string()
        } else {
            trf!("{n} things are missing before installing.", "Faltan {n} cosas antes de instalar.")
        };
        let note = if unchanged { tr(" Nothing was changed.", " No cambié nada.") } else { "" };
        println!("\n{}{}{what}{}{note}\n", s.red, s.bold, s.off);
        for (i, r) in manual.iter().enumerate() {
            println!("  {}. {}", i + 1, r.fix[0]);
            for line in &r.fix[1..] {
                if let Some(cmd) = line.strip_prefix("  ") {
                    println!("     {}{cmd}{}", s.bold, s.off);
                } else {
                    println!("     {line}");
                }
            }
        }
        if !later.is_empty() {
            let names: Vec<&str> = later.iter().filter_map(|r| r.brings.as_ref().map(|b| b.0.as_str())).collect();
            println!("\n  {}", trf!("When you run it again, the installer can bring {} itself (it will ask you).",
                                    "Cuando lo vuelvas a correr, el instalador puede poner {} por su cuenta (te lo va a preguntar).", and_list(&names)));
        }
        println!("\n  {}\n     {}{again}{}", tr("Then run the installation again:", "Después vuelve a correr la instalación:"), s.bold, s.off);
    }
    blocking.is_empty()
}

/// Whether the installer can ask you something: a terminal, with /dev/tty to read the
/// answer from (the install line reads its script from a pipe). Not in CI or a log.
fn can_ask() -> bool {
    std::io::stdout().is_terminal() && std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").is_ok()
}

/// Enter, y, yes, s or sí: yes.
fn said_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes" | "s" | "si" | "sí")
}

/// A yes-or-no question on the terminal, where Enter says yes. The answer comes from
/// /dev/tty, since the install line reads its script from a pipe; None when there is
/// no terminal to ask in.
fn ask(question: &str) -> Option<bool> {
    use std::io::{BufRead, Write};
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()?;
    print!("{question} {} ", tr("[Y/n]", "[S/n]"));
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::BufReader::new(tty).read_line(&mut answer).ok()?;
    Some(said_yes(&answer))
}

/// When all that is missing is what the installer can bring (herdr, or a newer one,
/// and jsonlite), it offers to, and Enter says yes. None when it didn't bring anything
/// (it couldn't ask, or you said no); otherwise whether everything came.
fn bring(rows: &[Check]) -> Option<bool> {
    let missing: Vec<&Check> = rows.iter().filter(|r| r.state == State::Missing).collect();
    if missing.is_empty() || missing.iter().any(|r| r.brings.is_none()) {
        return None;
    }
    let s = style();
    println!("\n{}{}{}", s.bold, tr("The installer can bring what is missing:", "El instalador puede poner lo que falta:"), s.off);
    for r in &missing {
        println!("  • {}", r.brings.as_ref().map(|b| b.1.as_str()).unwrap_or_default());
    }
    if ask(tr("Install it now?", "¿Lo instalo ahora?"))? {
        println!();
    } else {
        return None;
    }
    let mut all = true;
    for r in &missing {
        all &= match r.id {
            "herdr" => bring_herdr(),
            "herdr_old" => bring_herdr_update(),
            "r_packages" => bring_r_packages(),
            _ => false,
        };
    }
    Some(all)
}

/// `herdr update`, herdr's own updater (it checks what it downloads), when your herdr
/// is older than Corral needs. Its output shows as it goes.
fn bring_herdr_update() -> bool {
    let Some(h) = which("herdr") else { return false };
    println!("  … herdr update");
    let ran = Command::new(&h).arg("update").stdin(Stdio::null()).status().is_ok_and(|s| s.success());
    let v = version_of(&h, "--version");
    if ran && !v.is_empty() && as_tuple(&v) >= HERDR_MIN {
        step(&trf!("herdr {v}, updated with herdr update", "herdr {v}, actualizado con herdr update"), State::Ok);
        true
    } else {
        step(tr("herdr update didn't leave a version Corral can use; run it yourself to see why",
                "herdr update no dejó una versión que Corral pueda usar; córrelo tú para ver por qué"), State::Missing);
        false
    }
}

/// The line that puts ~/.local/bin on the PATH, in bash or zsh.
const PATH_LINE: &str = "export PATH=\"$HOME/.local/bin:$PATH\"";

/// ~/.local/bin on the PATH, so a new terminal finds herdr and corral. When it isn't,
/// the installer offers to add PATH_LINE to your shell's startup file (~/.bashrc or
/// ~/.zshrc), after a copy of it, as with the other files it edits. Uninstalling
/// leaves the line: other tools (herdr, uv) live there too. True when the line is in
/// that file now, added or already there.
fn path_step(saved: &mut Vec<String>) -> bool {
    use std::io::Write;
    if on_path(&bin()) {
        return false;
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let rc = match Path::new(&shell).file_name().and_then(|n| n.to_str()) {
        Some("zsh") => ".zshrc",
        Some("bash") => ".bashrc",
        _ => return false,
    };
    let path = env::home().join(rc);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.lines().any(|l| l.trim() == PATH_LINE) {
        return true;
    }
    if !can_ask() {
        return false;
    }
    println!("\n  {}", trf!("{} isn't on your PATH, so a new terminal won't find herdr or corral.",
                            "{} no está en tu PATH, así que una terminal nueva no encontrará herdr ni corral.", short(&bin())));
    if ask(&format!("  {}", trf!("Add it in ~/{rc}?", "¿Lo agrego en ~/{rc}?"))) != Some(true) {
        return false;
    }
    backup(&path, saved);
    let gap = if text.is_empty() { "" } else if text.ends_with('\n') { "\n" } else { "\n\n" };
    let block = format!("{gap}# Corral's installer: ~/.local/bin on the PATH (herdr, corral)\n{PATH_LINE}\n");
    let written = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(block.as_bytes()));
    match written {
        Ok(()) => {
            step(&trf!("~/.local/bin on the PATH, in ~/{rc}", "~/.local/bin en el PATH, en ~/{rc}"), State::Ok);
            true
        }
        Err(e) => {
            step(&format!("~/{rc}: {e}"), State::Warn);
            false
        }
    }
}

/// The file called `name` in a .tar.xz (Fresh's package on macOS), already checked
/// against its SHA-256. Corral's own code reads gzip but not xz, so the system's tar
/// (macOS's reads xz) unpacks it into a new private folder, removed afterwards.
fn tar_xz_member(data: &[u8], name: &str) -> Option<Vec<u8>> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("corral-unpack-{}", env::random_hex(8)));
    // Made here, so nobody else made it first.
    std::fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;
    let archive = dir.join("package.tar.xz");
    let unpacked = std::fs::write(&archive, data).is_ok()
        && Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&dir)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    // The program is at the top or one folder down (fresh-editor-aarch64-apple-darwin/fresh).
    let found = unpacked.then(|| {
        std::iter::once(dir.join(name))
            .chain(std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path().join(name)))
            .find(|p| p.is_file()).and_then(|p| std::fs::read(p).ok())
    }).flatten();
    let _ = std::fs::remove_dir_all(&dir);
    found
}

/// herdr into ~/.local/bin, from its GitHub release and checked against HERDR's
/// SHA-256 before it is written.
fn bring_herdr() -> bool {
    let (v, files) = HERDR;
    let (url, sha) = files[SYSTEM];
    println!("  {}", trf!("… downloading herdr {v}", "… descargando herdr {v}"));
    // net: tools
    let data = match setup::download(&url.replace("{v}", v)) {
        Ok(d) => d,
        Err(e) => {
            step(&e, State::Missing);
            return false;
        }
    };
    if let Err(e) = setup::checked(&format!("herdr {v}"), &data, sha) {
        step(&e, State::Missing);
        return false;
    }
    let exe = match place_in_bin("herdr", &data) {
        Ok(exe) => exe,
        Err(e) => {
            step(&format!("herdr: {e}"), State::Missing);
            return false;
        }
    };
    step(&trf!("herdr {v} in {}, SHA-256 checked", "herdr {v} en {}, SHA-256 comprobado", short(&exe)), State::Ok);
    true
}

/// A program into ~/.local/bin, whole or not at all: written next to it as
/// `.<name>.part`, made executable, then renamed into place.
fn place_in_bin(name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let (exe, part) = (bin().join(name), bin().join(format!(".{name}.part")));
    let written = std::fs::create_dir_all(bin()).and_then(|_| std::fs::write(&part, data))
        .and_then(|_| std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&part, &exe));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written.map(|_| exe)
}

/// uv for the Python console, when it's missing: the installer offers it, in a
/// terminal, and Enter says yes. Without uv there is no Python console, and with it
/// python_ready, right after, gets Python and the console's packages once. No
/// terminal to ask in, or a no: the checklist's hint says how to add it later.
fn uv_step() {
    if which("uv").is_some() || !can_ask() {
        return;
    }
    println!("\n  {}", tr("uv isn't installed, so there's no Python console. With uv, it also gets Python and the console's packages, once (about 400 MB on disk).",
                          "uv no está instalado, así que no hay consola de Python. Con uv también trae Python y los paquetes de la consola, una vez (unos 400 MB en disco)."));
    if ask(&format!("  {}", trf!("Install uv {}?", "¿Instalo uv {}?", UV.0))) == Some(true) {
        bring_uv();
    }
}

/// uv and uvx into ~/.local/bin, from uv's GitHub release and checked against UV's
/// SHA-256 before anything is written.
fn bring_uv() -> bool {
    let (v, files) = UV;
    let (url, sha) = files[SYSTEM];
    println!("  {}", trf!("… downloading uv {v}", "… descargando uv {v}"));
    // net: tools
    let data = match setup::download(&url.replace("{v}", v)) {
        Ok(d) => d,
        Err(e) => {
            step(&e, State::Warn);
            return false;
        }
    };
    if let Err(e) = setup::checked(&format!("uv {v}"), &data, sha) {
        step(&e, State::Warn);
        return false;
    }
    let Ok(tar) = zip::gunzip(&data) else {
        step(&trf!("uv {v}: the download isn't a .tar.gz", "uv {v}: lo descargado no es un .tar.gz"), State::Warn);
        return false;
    };
    for name in ["uv", "uvx"] {
        let Some(program) = zip::tar_member(&tar, name) else {
            step(&trf!("uv {v}: the download doesn't contain {name}", "uv {v}: lo descargado no trae {name}"), State::Warn);
            return false;
        };
        if let Err(e) = place_in_bin(name, &program) {
            step(&format!("{name}: {e}"), State::Warn);
            return false;
        }
    }
    step(&trf!("uv {v} in {}, SHA-256 checked", "uv {v} en {}, SHA-256 comprobado", short(&bin().join("uv"))), State::Ok);
    true
}

/// jsonlite, if it's missing, into your R library (r_install).
fn bring_r_packages() -> bool {
    let Some(r) = crate::versions::rscript().or_else(|| which("Rscript")) else { return false };
    let has = setup::r_packages(&r, &R_NEEDS);
    let missing: Vec<&str> = R_NEEDS.into_iter().zip(has).filter(|(_, h)| !h).map(|(p, _)| p).collect();
    if missing.is_empty() {
        return true;
    }
    let names = and_list(&missing);
    println!("  {}", trf!("… installing {names} in your R library (seconds; a few minutes if they have to compile)",
                          "… instalo {names} en tu biblioteca de R (segundos; unos minutos si hay que compilarlos)"));
    if r_install(&r, &missing) {
        step(&trf!("{names} in your R library", "{names} en tu biblioteca de R"), State::Ok);
        true
    } else {
        step(&trf!("couldn't install {names}", "no pude instalar {names}"), State::Missing);
        system_libs_step(&r, &missing);
        false
    }
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
    // net: tools
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
    } else if url.ends_with(".tar.xz") {
        tar_xz_member(&data, name)
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
    let mut list = done.get("tools").arr().to_vec();
    list.push(name.into());
    done.set("tools", Value::Arr(list));
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
    // Corral's shortcuts with the keys you changed (corral keys, Tab in the palette).
    let wanted: Vec<Value> = crate::shortcuts::apply_herdr(wanted.get("keys").get("command").arr().to_vec(), &crate::shortcuts::own_keys())
        .into_iter().filter(|k| !bound.contains(&k.get("command").str().unwrap_or(""))).collect();
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
        done.set("herdr_created", true.into());
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
        done.set("rprofile_created", true.into());
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
    let Some(mut ours) = std::fs::read_to_string(dest.join("fresh/config.json")).ok().and_then(|t| json::parse(&t).ok()) else {
        return step(tr("couldn't find Corral's editor configuration", "no encontré la configuración del editor de Corral"), State::Warn);
    };
    // With the keys you changed (corral keys, Tab in the palette).
    let keys = crate::shortcuts::apply_editor(ours.get("keybindings").arr().to_vec(), &crate::shortcuts::own_keys());
    ours.set("keybindings", Value::Arr(keys));
    if cfg.exists() {
        match std::fs::read_to_string(&cfg).ok().and_then(|t| json::parse(&t).ok()) {
            Some(user @ Value::Obj(_)) => {
                let taken: Vec<_> = ours.get("keybindings").arr().iter().map(combo).collect();
                let theirs: Vec<Value> = user.get("keybindings").arr().iter()
                    .filter(|b| taken.contains(&combo(b)) && !ours.get("keybindings").arr().contains(b)).cloned().collect();
                add_unique(done, "fresh_keys_user", theirs);
                let added: Vec<Value> = ours.get("keybindings").arr().iter().filter(|b| !user.get("keybindings").arr().contains(b)).cloned().collect();
                add_unique(done, "fresh_keys_corral", added);
                // Corral's explorer width is where it starts: one you set stays.
                if !user.get("file_explorer").get("width").is_null() {
                    let mut explorer = ours.get("file_explorer").clone();
                    explorer.remove("width");
                    ours.set("file_explorer", explorer);
                }
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
        add_unique(done, "fresh_keys_corral", ours.get("keybindings").arr().to_vec());
        done.set("fresh_created", true.into());
        step(tr("editor configuration (Fresh)", "configuración del editor (Fresh)"), State::Ok);
    }
    let _ = link(&dest.join("fresh/plugins/corral.ts"), &fresh_dir().join("plugins/corral.ts"));
}

/// The links an older Corral made under the skill's old name, in the places agents read.
fn old_skill_links() -> Vec<PathBuf> {
    old_skill_links_in(&[env::home().join(".agents/skills"), claude_dir().join("skills"), gemini_dir().join("skills")])
}

/// In these folders: links (never a folder of yours) that lead to a skills/corral-r folder.
fn old_skill_links_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter().map(|dir| dir.join(OLD_SKILL))
        .filter(|at| at.is_symlink() && std::fs::read_link(at).is_ok_and(|t| t.ends_with(Path::new("skills").join(OLD_SKILL))))
        .collect()
}

/// Corral's skill (the Agent Skills standard) where agents look for skills.
fn skill(dest: &Path) {
    let source = dest.join("skills").join(SKILL);
    // The old name goes, so agents don't see Corral's skill twice.
    for old in old_skill_links() {
        if std::fs::remove_file(&old).is_ok() {
            step(&trf!("{}: the skill's old name, taken away (now {SKILL})", "{}: el nombre antiguo de la skill, quitado (ahora {SKILL})", short(&old)), State::Ok);
        }
    }
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

/// With several agents installed and none chosen, ask once which one the agent pane
/// opens. Only in a terminal: the install line reads its script from a pipe, so the
/// answer comes from /dev/tty, and without one (CI, a log) nothing is asked. Enter
/// keeps Corral's choice, the first one installed. The install record remembers
/// that it asked, so an update doesn't ask again; the palette changes it later.
fn ask_agent(before: &Value, done: &mut Value) {
    use std::io::{BufRead, Write};
    if before.get("agent_asked") == &Value::Bool(true) {
        done.set("agent_asked", true.into());
        return;
    }
    if env::config().get("agent").str().is_some() {
        return;
    }
    let found: Vec<(&str, &str)> = env::AGENTS.iter().filter(|(cmd, _)| env::which(cmd).is_some()).copied().collect();
    if found.len() < 2 || !std::io::stdout().is_terminal() {
        return;
    }
    let Ok(tty) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty") else { return };
    let s = style();
    println!("\n  {}{}{}", s.bold, tr("Several agents are installed. Which one should the agent pane open?",
                                         "Hay varios agentes instalados. ¿Cuál abre el panel del agente?"), s.off);
    for (i, (_, name)) in found.iter().enumerate() {
        println!("    {}. {name}", i + 1);
    }
    print!("  {}", trf!("Number, or Enter for the first one installed ({}): ", "Número, o Enter para el primero instalado ({}): ", found[0].1));
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::BufReader::new(tty).read_line(&mut answer);
    done.set("agent_asked", true.into());
    match answer.trim().parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| found.get(i)) {
        Some((cmd, name)) => {
            let mut cfg = env::config();
            cfg.set("agent", (*cmd).into());
            theme::write_config(&cfg);
            step(&trf!("the agent pane opens {name} (the palette changes it: Agent: …)",
                       "el panel del agente abre {name} (la paleta lo cambia: Agente: …)"), State::Ok);
        }
        None => step(&trf!("the agent pane opens the first agent installed, {} now (the palette changes it: Agent: …)",
                           "el panel del agente abre el primer agente instalado, {} ahora (la paleta lo cambia: Agente: …)", found[0].1), State::Ok),
    }
}

/// The Python console's packages (IPython, cloudpickle, pyarrow, duckdb) into uv's cache,
/// so the first Python console starts at once instead of downloading them then.
fn python_ready() {
    let Some(uv) = which("uv") else { return };
    println!("  {}", tr("… preparing the Python console (uv downloads IPython, pyarrow and duckdb once)",
                        "… preparo la consola de Python (uv descarga IPython, pyarrow y duckdb una vez)"));
    // net: python-console: corral install fills uv's cache with the console's Python and packages
    let mut c = Command::new(uv);
    // As a console outside a project starts: in your home folder, with the same choice
    // of Python (a uv-managed one when uv would otherwise take the system's).
    c.current_dir(env::home()).args(["run", "--no-project"]).args(crate::versions::uv_python_args(&env::home()));
    for p in crate::console::PYTHON_PACKAGES {
        c.args(["--with", p]);
    }
    let ok = c.args(["python", "-c", "import IPython, cloudpickle, pyarrow, duckdb"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if ok {
        step(tr("Python console ready: IPython, cloudpickle, pyarrow and duckdb in uv's cache", "consola de Python lista: IPython, cloudpickle, pyarrow y duckdb en la caché de uv"), State::Ok);
    } else {
        step(tr("couldn't prepare the Python console; its first start downloads what it needs", "no pude preparar la consola de Python; su primer arranque descarga lo que necesita"), State::Warn);
    }
}

/// The system's release name (noble, jammy, bookworm…), from /etc/os-release.
fn linux_codename() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim_matches('"').to_string()).filter(|v| !v.is_empty());
    value("UBUNTU_CODENAME=").or_else(|| value("VERSION_CODENAME="))
}

/// The release Posit's package manager has R packages compiled for, if this is one.
fn p3m_codename() -> Option<String> {
    linux_codename().filter(|c| ["focal", "jammy", "noble", "bullseye", "bookworm"].contains(&c.as_str()))
}

/// R packages into your own R library: no sudo. From where r_repos_code says: by
/// default Posit's snapshot of three days ago, already compiled on the systems it
/// builds for (otherwise from source, which needs a compiler). With pak in your R it
/// installs them with pak, as you would, and with install.packages if pak couldn't.
/// True when R can load them all afterwards.
fn r_install(r: &Path, pkgs: &[&str]) -> bool {
    (has_pak() && run_r(r, &r_install_code(pkgs, true))) || run_r(r, &r_install_code(pkgs, false))
}

/// R code run without your .Rprofile and without output; true when it ends with status 0.
fn run_r(r: &Path, code: &str) -> bool {
    Command::new(r).args(["--no-init-file", "-e", code]).stdin(Stdio::null()).stdout(Stdio::null())
        .stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Whether R packages come already compiled here: on Linux, on a release Posit's package
/// manager builds for; on macOS, with an R from CRAN or rig (in /Library/Frameworks),
/// which takes CRAN's compiled packages for macOS. Homebrew's R compiles every package.
fn r_binaries() -> bool {
    if cfg!(target_os = "macos") {
        crate::versions::rscript().or_else(|| which("Rscript")).and_then(|r| std::fs::canonicalize(r).ok())
            .is_some_and(|r| r.starts_with("/Library/Frameworks/R.framework"))
    } else {
        p3m_codename().is_some()
    }
}

/// The R code r_install runs: into the first library of R_LIBS_USER (yours), from
/// Posit's compiled packages where there are some, then CRAN. pak runs with
/// pkg.sysreqs off: it may list system libraries, but it never installs them (that
/// takes sudo); system_libs_hint says which, when something doesn't load.
fn r_install_code(pkgs: &[&str], pak: bool) -> String {
    let list = pkgs.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    let install = if pak {
        "options(pkg.sysreqs = FALSE)\n  pak::pak(pkgs, lib = lib, upgrade = FALSE, ask = FALSE)" // net: r-packages
    } else {
        "install.packages(pkgs, lib = lib, quiet = TRUE)" // net: r-packages
    };
    let repos = r_repos_code(p3m_codename().as_deref());
    format!(r#"{R_LIB}
{repos}
pkgs <- c({list})
try({{
  {install}
}}, silent = TRUE)
quit(status = if (all(vapply(pkgs, requireNamespace, logical(1), lib.loc = c(lib, .libPaths()), quietly = TRUE))) 0 else 1)"#)
}

/// The start of the R code that installs packages: into the first library of R_LIBS_USER
/// (yours), and with R's User-Agent, by which Posit's server picks compiled packages.
const R_LIB: &str = r#"lib <- strsplit(Sys.getenv("R_LIBS_USER"), .Platform$path.sep)[[1]][1]
if (is.na(lib) || !nzchar(lib)) lib <- .libPaths()[1]
lib <- path.expand(lib)
dir.create(lib, recursive = TRUE, showWarnings = FALSE)
options(HTTPUserAgent = sprintf("R/%s R (%s)", getRversion(), paste(getRversion(), R.version$platform, R.version$arch, R.version$os)))"#;

/// httpgd on macOS. Posit's snapshot has it only as source there, which needs Xcode's
/// command line tools to compile, while CRAN has it compiled for macOS. So it comes from
/// CRAN's compiled packages, and only in the version the snapshot (or your repository)
/// has: nothing newer than the snapshot gets in. unigd, which it needs, goes in first
/// from the snapshot as usual (r_httpgd).
fn httpgd_from_cran_code() -> String {
    let repos = r_repos_code(None);
    format!(r#"{R_LIB}
{repos}
want <- tryCatch(available.packages(repos = repos, type = "source")["httpgd", "Version"], error = function(e) NA)
cran <- c(CRAN = "{CRAN}")
have <- tryCatch(available.packages(repos = cran, type = "binary")["httpgd", "Version"], error = function(e) NA)
if (!is.na(want) && identical(unname(want), unname(have))) {{
  try(install.packages("httpgd", lib = lib, repos = cran, type = "binary", dependencies = FALSE, quiet = TRUE), silent = TRUE) # net: r-packages
}}
quit(status = if (requireNamespace("httpgd", lib.loc = c(lib, .libPaths()), quietly = TRUE)) 0 else 1)"#)
}

/// The R code that picks where R packages come from, in this order:
/// 1. CORRAL_R_REPOS, if you set it: repository URLs, separated by spaces or commas.
/// 2. The repository R already has, when it isn't a public one: an institution's
///    mirror, set by its administrators in R's Rprofile.site (R runs here without your
///    own .Rprofile). In a network that blocks CRAN and Posit, it's the one that works.
/// 3. Otherwise Posit's package manager as it was three days ago: no package newer than
///    72 hours, the same versions for everyone installing that day, compiled for your
///    system when `codename` is one Posit builds for, and source packages otherwise.
///    Live CRAN isn't added: install.packages takes the newest version it finds, and
///    that would undo the snapshot.
fn r_repos_code(codename: Option<&str>) -> String {
    let binaries = codename.map(|c| format!("P3M = paste0(\"https://p3m.dev/cran/__linux__/{c}/\", day), ")).unwrap_or_default();
    format!(r#"own <- getOption("repos")
own <- own[!is.na(own) & nzchar(own) & own != "@CRAN@"]
public <- "cloud[.]r-project[.]org|cran[.]r-project[.]org|cran[.]rstudio[.]com|p3m[.]dev|packagemanager[.](posit|rstudio)[.]com"
set <- trimws(Sys.getenv("CORRAL_R_REPOS"))
day <- format(Sys.Date() - 3)
repos <- if (nzchar(set)) {{
  urls <- strsplit(set, "[[:space:],]+")[[1]]
  stats::setNames(urls, paste0("CORRAL", seq_along(urls)))
}} else if (length(own) && any(!grepl(public, own))) {{
  own
}} else {{
  c({binaries}CRAN = paste0("https://p3m.dev/cran/", day))
}}
options(repos = repos)"#)
}

/// With pak in your R: the commands that install the system libraries these packages
/// need (pak::pkg_sysreqs), each with sudo, for you to run; Corral never runs sudo
/// itself. None without pak, or when pak has nothing to say.
fn system_libs_hint(r: &Path, pkgs: &[&str]) -> Option<String> {
    if !has_pak() {
        return None;
    }
    let list = pkgs.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    let code = format!("sr <- tryCatch(pak::pkg_sysreqs(c({list})), error = function(e) NULL)
if (!is.null(sr)) cat(c(sr$pre_install, sr$install_scripts, sr$post_install), sep = \"\\n\")");
    let out = Command::new(r).args(["--no-init-file", "-e", &code]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    with_sudo(&String::from_utf8_lossy(&out.stdout))
}

/// pak's system commands (apt-get, dnf…), one per line, as one line to run with sudo.
/// Anything else pak printed is left out.
fn with_sudo(text: &str) -> Option<String> {
    const TOOLS: [&str; 8] = ["apt-get", "apt", "dnf", "yum", "zypper", "apk", "pacman", "sudo"];
    let lines: Vec<String> = text.lines().map(str::trim)
        .filter(|l| l.split_whitespace().next().is_some_and(|w| TOOLS.contains(&w)))
        .map(|l| if l.starts_with("sudo ") { l.to_string() } else { format!("sudo {l}") }).collect();
    (!lines.is_empty()).then(|| lines.join(" && "))
}

/// After R packages didn't install or load: what pak says they need from the system.
fn system_libs_step(r: &Path, pkgs: &[&str]) {
    if let Some(cmd) = system_libs_hint(r, pkgs) {
        println!("    {}", trf!("If a system library is missing, pak says what they need: {cmd}",
                                "Si falta una librería del sistema, pak indica lo que necesitan: {cmd}"));
    }
}

/// httpgd, which draws R's plots for Mirador and the agent. If it can't be installed
/// (it needs cairo and libpng from the system), Corral works without plots there, and
/// the step says how to add it later, with pak's list of what the system lacks.
/// Some("httpgd") when it tried and couldn't, for the install's ping.
fn r_httpgd() -> Option<&'static str> {
    let r = crate::versions::rscript().or_else(|| which("Rscript"))?;
    if setup::r_packages(&r, &["httpgd"])[0] {
        return None;
    }
    println!("  {}", tr("… installing httpgd in your R library, for R's plots (seconds; a few minutes if it has to compile)",
                        "… instalo httpgd en tu biblioteca de R, para los gráficos de R (segundos; unos minutos si hay que compilarlo)"));
    // On macOS, compiled from CRAN when R takes compiled packages; otherwise, or if that
    // didn't work, as any other package (compiled from source where there is no binary).
    let from_cran = cfg!(target_os = "macos") && r_binaries() && r_install(&r, &["unigd"]) && run_r(&r, &httpgd_from_cran_code());
    if from_cran || r_install(&r, &["httpgd"]) {
        step(tr("httpgd in your R library (R's plots in Mirador)", "httpgd en tu biblioteca de R (los gráficos de R en Mirador)"), State::Ok);
        None
    } else {
        step(&trf!("couldn't install httpgd: Corral works, but R's plots won't reach Mirador or the agent until you install it: {}",
                   "no pude instalar httpgd: Corral funciona, pero los gráficos de R no llegan a Mirador ni al agente hasta que lo instales: {}",
                   install_r(&["httpgd"])), State::Warn);
        system_libs_step(&r, &["httpgd"]);
        Some("httpgd")
    }
}

/// nanoparquet, which View() writes R's tables with (faster than arrow, and R's memory
/// stays steady in a long session). If it fails, View() keeps working with arrow or CSV.
/// Some("nanoparquet") when it tried and couldn't, for the install's ping.
fn r_nanoparquet() -> Option<&'static str> {
    let r = crate::versions::rscript().or_else(|| which("Rscript"))?;
    if setup::r_packages(&r, &["nanoparquet"])[0] {
        return None;
    }
    println!("  {}", tr("… installing nanoparquet in your R library (seconds; a minute or two if it has to compile)",
                        "… instalo nanoparquet en tu biblioteca de R (segundos; uno o dos minutos si hay que compilarlo)"));
    let ok = r_install(&r, &["nanoparquet"]);
    if ok {
        step(tr("nanoparquet in your R library", "nanoparquet en tu biblioteca de R"), State::Ok);
        None
    } else {
        step(&trf!("couldn't install nanoparquet; View() uses arrow or CSV meanwhile. To try again: {}",
                   "no pude instalar nanoparquet; mientras tanto View() usa arrow o CSV. Para reintentarlo: {}", install_r(&["nanoparquet"])), State::Warn);
        system_libs_step(&r, &["nanoparquet"]);
        Some("nanoparquet")
    }
}

/// duckdb and DBI, which SQL in R runs on (corral_sql), only where they come already
/// compiled: elsewhere compiling duckdb takes a long while, and the checklist says how.
/// Some("duckdb") when it tried and couldn't, for the install's ping.
fn r_duckdb() -> Option<&'static str> {
    let r = crate::versions::rscript().or_else(|| which("Rscript"))?;
    if setup::r_packages(&r, &["duckdb", "DBI"]).iter().all(|h| *h) || !r_binaries() {
        return None;
    }
    println!("  {}", tr("… installing duckdb and DBI in your R library, already compiled (SQL in R)",
                        "… instalo duckdb y DBI en tu biblioteca de R, ya compilados (SQL en R)"));
    if r_install(&r, &["duckdb", "DBI"]) {
        step(tr("duckdb in your R library (SQL in R)", "duckdb en tu biblioteca de R (SQL en R)"), State::Ok);
        None
    } else {
        step(&trf!("couldn't install duckdb; SQL in R needs it. To try again: {}",
                   "no pude instalar duckdb; SQL en R lo necesita. Para reintentarlo: {}", install_r(&["duckdb", "DBI"])), State::Warn);
        system_libs_step(&r, &["duckdb", "DBI"]);
        Some("duckdb")
    }
}

fn install(src: &Path, in_place: bool, again: &str) -> Result<(), String> {
    let s = style();
    println!("{}Corral {VERSION}{}: {}", s.bold, s.off,
             tr("checking what it needs, without changing anything yet.", "reviso lo que necesita, sin cambiar nada todavía."));
    let offer = can_ask();
    let mut rows = checks(in_place, src);
    if !report(&rows, again, offer) {
        // When all that's missing is what the installer can bring, it asks, brings it
        // with your yes and checks again; if you say no or something fails, the steps to
        // do it yourself. Anything else missing, report already gave its steps.
        let only_brought = offer && rows.iter().filter(|r| r.state == State::Missing).all(|r| r.brings.is_some());
        let mut brought = None;
        let ok = only_brought && {
            brought = bring(&rows);
            rows = checks(in_place, src);
            brought == Some(true) && rows.iter().all(|r| r.state != State::Missing)
        };
        if !ok {
            if only_brought {
                missing_steps(&rows, again, false, brought.is_none());
            }
            let missing: Vec<&str> = rows.iter().filter(|r| r.state == State::Missing)
                .map(|r| if r.id.is_empty() { "other" } else { r.id }).collect();
            count_stop(&missing, in_place);
            std::process::exit(1);
        }
        step(tr("everything needed is here now", "ya está todo lo necesario"), State::Ok);
    }
    println!("\n{}{}{}", s.bold, tr("Installing", "Instalando"), s.off);
    migrate_old_install();
    let before = env::read_json(&manifest());
    let mut done = Value::obj();
    done.set("version", VERSION.into());
    let (y, mo, d, h, mi, _) = env::local_now();
    done.set("date", format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}").into());
    done.set("in_place", in_place.into());
    for key in ["fresh_keys_user", "fresh_keys_corral"] {
        done.set(key, Value::Arr(before.get(key).arr().to_vec()));
    }
    for key in ["fresh_created", "rprofile_created", "herdr_created"] {
        if before.get(key) == &Value::Bool(true) {
            done.set(key, true.into());
        }
    }
    let mut saved: Vec<String> = before.get("backups").arr().iter().filter_map(|v| v.str().map(String::from)).collect();
    let earlier = saved.len();

    let dest = if in_place { src.to_path_buf() } else { place(src)? };
    done.set("folder", dest.to_string_lossy().to_string().into());
    let exe = dest.join("rs/target/release/corral");
    if !exe.is_file() {
        return Err(trf!("{} is missing: build it with cargo build --release in {}", "falta {}: compílalo con cargo build --release en {}",
                        exe.display(), dest.join("rs").display()));
    }
    link(&exe, &bin().join("corral"))?;
    step(&trf!("corral command in {}", "comando corral en {}", short(&bin().join("corral"))), State::Ok);
    for (name, shown, v, what, files) in DOWNLOADS {
        if which(name).is_none() {
            let (url, sha) = files[SYSTEM];
            download(name, shown, v, tr(what.0, what.1), url, sha, &mut done);
        }
    }
    ark();
    // The R packages it puts in by itself and couldn't, for the install's ping.
    let r_failed: Vec<&str> = [r_httpgd(), r_nanoparquet(), r_duckdb()].into_iter().flatten().collect();
    uv_step();
    python_ready();
    herdr_plugin(&dest);
    herdr_keys(&dest, &mut done, &mut saved);
    // With the sensitive profile, herdr's checks of herdr.dev stay off.
    crate::profile::sync_herdr(&env::config());
    rprofile_step(&dest, &mut done, &mut saved);
    fresh(&dest, &mut done, &mut saved);
    skill(&dest);
    ask_agent(&before, &mut done);
    let path_in_rc = path_step(&mut saved);
    theme::apply();
    step(tr("herdr's theme colors in the editor, the console and Mirador", "colores del tema de herdr en el editor, la consola y Mirador"), State::Ok);
    done.set("backups", Value::Arr(saved.iter().map(|b| Value::from(b.as_str())).collect()));
    env::write_json(&manifest(), &done);

    let (bold, off) = (s.bold, s.off);
    println!("\n{}{}{}{off} {}", s.green, bold, tr("Done.", "Listo."), tr("To get started:", "Para empezar:"));
    let mut n = 1;
    if path_in_rc {
        println!("  {n}. {}", tr("Open a new terminal, so it finds herdr and corral.", "Abre una terminal nueva, para que encuentre herdr y corral."));
        n += 1;
    } else if !on_path(&bin()) {
        println!("  {n}. {}", trf!("Add {bold}{PATH_LINE}{off} at the end of ~/.bashrc and open a new terminal.",
                                   "Agrega {bold}{PATH_LINE}{off} al final de ~/.bashrc y abre una terminal nueva."));
        n += 1;
    }
    println!("  {n}. {}", trf!("Go to a project folder and type {bold}corral{off}: herdr opens with the agent, the script, the R console and the variables.",
                               "Entra a la carpeta de un proyecto y escribe {bold}corral{off}: herdr se abre con el agente, el script, la consola de R y las variables."));
    println!("  {}. {}", n + 1, trf!("{bold}ctrl+b space{off} shows every shortcut.", "{bold}ctrl+b space{off} muestra todos los atajos."));
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
    count_install(before.get("version").str(), in_place, &r_failed);
    Ok(())
}

/// One anonymous ping after a successful install, so the project knows how many
/// people install Corral and from which countries: the version, the system (linux or
/// wsl), new install or update (and on an update, the version it replaced), published
/// package or clone, and where it runs in categories (ping_env). The site adds the
/// country from Vercel's geolocation and stores no IP (api/installed.js).
/// telemetry_off() says what turns it off; CORRAL_TELEMETRY_URL points it elsewhere
/// (tests). It waits 5 seconds at most and never fails the install.
fn count_install(previous: Option<&str>, in_place: bool, r_failed: &[&str]) {
    if telemetry_off() {
        return;
    }
    if send_ping(&ping_body(previous, is_wsl(), in_place, &ping_env(), r_failed)) {
        let s = style();
        println!("{}{}{}", s.dim, tr("Counted this install anonymously (version, system, country, the tools it found and any R package that didn't go in). corral config telemetry off turns it off.",
                                     "Conté esta instalación de forma anónima (versión, sistema, país, las herramientas que encontré y los paquetes de R que no se instalaron). corral config telemetry off lo desactiva."), s.off);
    }
}

/// One anonymous ping when the installation stops because something required is
/// missing, so the project learns what keeps people out: which checks failed, by
/// name (herdr, herdr_old, r, r_packages, folder), plus what an install's ping says. The
/// same switches turn it off; `--check` never sends it.
fn count_stop(missing: &[&str], in_place: bool) {
    if telemetry_off() {
        return;
    }
    if send_ping(&stop_body(missing, is_wsl(), in_place, &ping_env())) {
        let s = style();
        println!("\n{}{}{}", s.dim, tr("Counted this stop anonymously (what was missing, version, system, country and the tools it found). CORRAL_NO_TELEMETRY=1 turns it off.",
                                       "Conté esta detención de forma anónima (qué faltó, versión, sistema, país y las herramientas que encontré). CORRAL_NO_TELEMETRY=1 lo desactiva."), s.off);
    }
}

/// One anonymous ping when Corral is uninstalled: the version, the system and
/// published package or clone, so the project sees who leaves, and after which
/// version. The same switches turn it off (the settings stay after uninstalling).
fn count_uninstall(in_place: bool) {
    if telemetry_off() {
        return;
    }
    if send_ping(&uninstall_body(is_wsl(), in_place)) {
        let s = style();
        println!("{}{}{}", s.dim, tr("Counted this uninstall anonymously (version, system and country). CORRAL_NO_TELEMETRY=1 turns it off.",
                                     "Conté esta desinstalación de forma anónima (versión, sistema y país). CORRAL_NO_TELEMETRY=1 lo desactiva."), s.off);
    }
}

/// The install count is off with CORRAL_NO_TELEMETRY=1 or DO_NOT_TRACK=1, or with
/// `corral config telemetry off`, which holds where no .bashrc is read too (an
/// agent's shell, a script), and with the sensitive profile.
fn telemetry_off() -> bool {
    let set = |var: &str| std::env::var(var).is_ok_and(|v| !v.is_empty() && v != "0");
    set("CORRAL_NO_TELEMETRY") || set("DO_NOT_TRACK") || env::config().get("telemetry").str() == Some("off")
        || crate::profile::local_only(None)
}

/// POSTs one ping to the site (api/installed.js): 5 seconds at most, never an error.
fn send_ping(body: &str) -> bool {
    // net: install-count
    let Some(curl) = env::which("curl") else { return false };
    let url = std::env::var("CORRAL_TELEMETRY_URL").unwrap_or_else(|_| "https://corral-term.dev/api/installed".into());
    Command::new(curl)
        .args(["-fsS", "-m", "5", "-X", "POST", "-H", "Content-Type: application/json", "--data", body, &url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|st| st.success())
}

/// Where Corral runs, in categories that name no person, machine or path: the agent
/// the layout opens, the Ubuntu or Debian release, herdr's version, R's major
/// version, and whether uv is there for the Python console. Unknown values say so.
fn ping_env() -> Vec<(&'static str, String)> {
    let agent = agent_kind(env::config().get("agent").str(), |c| which(c).is_some());
    let distro = linux_codename().filter(|c| (1..=16).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_lowercase()))
        .unwrap_or_else(|| "other".into());
    let herdr = which("herdr").map(|h| version_of(&h, "--version"))
        .map(|v| if plain_version(&v) { v } else { "unknown".into() }).unwrap_or_else(|| "none".into());
    let r = crate::versions::rscript().or_else(|| which("Rscript")).map(|r| major_minor(&version_of(&r, "--version")))
        .unwrap_or_else(|| "none".into());
    let python = if which("uv").is_some() { "uv" } else { "none" };
    vec![("agent", agent), ("distro", distro), ("herdr", herdr), ("r", r), ("python", python.into())]
}

/// The agent as a category: claude, codex, gemini, opencode, none, or other for any
/// other command. The one chosen (corral config agent), else the first installed.
fn agent_kind(chosen: Option<&str>, installed: impl Fn(&str) -> bool) -> String {
    let cmd = match chosen {
        Some(c) => c.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string(),
        None => env::first_agent(installed).unwrap_or("none").to_string(),
    };
    match cmd.as_str() {
        "" | "none" => "none".into(),
        c if env::AGENTS.iter().any(|(a, _)| *a == c) => c.into(),
        _ => "other".into(),
    }
}

/// 0.9.5: digits and dots, two or three parts.
fn plain_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.len() <= 4 && p.chars().all(|c| c.is_ascii_digit()))
}

/// 4.6.1 → 4.6; anything else, unknown.
fn major_minor(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 2 && plain_version(&format!("{}.{}", parts[0], parts[1])) {
        format!("{}.{}", parts[0], parts[1])
    } else {
        "unknown".into()
    }
}

/// What every ping starts with: the version, the system, its kind and the source.
fn ping_base(kind: &str, wsl: bool, in_place: bool) -> Value {
    let system = if wsl { "wsl" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    json::obj(vec![("version", VERSION.into()), ("system", system.into()),
                   ("kind", kind.into()), ("source", (if in_place { "clone" } else { "release" }).into())])
}

fn with_env(mut v: Value, envs: &[(&str, String)]) -> Value {
    for (k, val) in envs {
        v.set(k, val.as_str().into());
    }
    v
}

/// The stop ping's JSON: the failed checks' ids, which Corral names itself.
fn stop_body(missing: &[&str], wsl: bool, in_place: bool, envs: &[(&str, String)]) -> String {
    let mut v = ping_base("stopped", wsl, in_place);
    v.set("missing", Value::Arr(missing.iter().map(|m| Value::from(*m)).collect()));
    with_env(v, envs).to_json()
}

/// The uninstall ping's JSON: what every ping says, nothing more.
fn uninstall_body(wsl: bool, in_place: bool) -> String {
    ping_base("uninstall", wsl, in_place).to_json()
}

/// The install ping's JSON. `previous` is the version install.json had, if any: then
/// it is an update, and the ping says which version it replaced when that reads as
/// one (digits and dots only).
fn ping_body(previous: Option<&str>, wsl: bool, in_place: bool, envs: &[(&str, String)], r_failed: &[&str]) -> String {
    let mut v = ping_base(if previous.is_some() { "update" } else { "new" }, wsl, in_place);
    if let Some(from) = previous.filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.')) {
        v.set("from", from.into());
    }
    // Which of the R packages it installs by itself didn't go in (httpgd, nanoparquet,
    // duckdb): where installs on Linux get stuck. Only those names.
    if !r_failed.is_empty() {
        v.set("r_failed", Value::Arr(r_failed.iter().map(|p| Value::from(*p)).collect()));
    }
    with_env(v, envs).to_json()
}

// The installer (install.sh) also passes --from, --in-place and --rerun.
fn install_usage() -> &'static str {
    tr("usage: corral install [--check]", "uso: corral install [--check]")
}

/// `corral install [--from FOLDER] [--in-place] [--check] [--rerun COMMAND]`.
pub fn cmd_install(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", install_usage());
        return Ok(());
    }
    let (mut src, mut in_place, mut only_check, mut again) = (None::<PathBuf>, false, false, "corral install".to_string());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => src = it.next().map(PathBuf::from),
            "--in-place" => in_place = true,
            "--check" => only_check = true,
            "--rerun" => again = it.next().cloned().unwrap_or(again),
            other => return Err(trf!("unknown option {other}\n{}", "opción desconocida {other}\n{}", install_usage())),
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
        let ok = report(&checks(in_place, &src), &again, false);
        if !ok {
            std::process::exit(1);
        }
        println!("\n{}{}{} {}{}{}{}", s.green, tr("Everything needed is here.", "Está todo lo necesario."), s.off,
                 tr("To install: ", "Para instalar: "), s.bold, again.replace(" --check", ""), s.off);
        return Ok(());
    }
    install(&src, in_place, &again)
}

// ----------------------------------------------------------------------- update

const REPO: &str = "paulovillarroel/corral-term";
const INSTALL_LINE: &str = "curl -LsSf https://corral-term.dev/install.sh | sh";

fn update_usage() -> &'static str {
    tr("usage: corral update [--check]", "uso: corral update [--check]")
}

/// The tag of the latest published version (v0.1.12): where GitHub's "latest"
/// link leads, as the installer finds it.
fn latest_tag() -> Result<String, String> {
    // net: update-check: GitHub's "latest" link, for the daily check and corral update
    let curl = env::which("curl").ok_or(tr("curl isn't installed", "curl no está instalado"))?;
    let url = format!("https://github.com/{REPO}/releases/latest");
    let out = Command::new(curl).args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}", "--max-time", "20", &url])
        .output().map_err(|e| e.to_string())?;
    tag_of(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
        tr("couldn't find out the latest version: is there a connection to github.com?",
           "no pude saber cuál es la última versión: ¿hay conexión con github.com?").into()
    })
}

/// "https://github.com/…/releases/tag/v0.1.12" → "v0.1.12".
fn tag_of(url: &str) -> Option<String> {
    let tag = url.trim().rsplit_once("/tag/")?.1;
    let numbers = tag.strip_prefix('v')?;
    (numbers.split('.').count() == 3 && numbers.split('.').all(|n| n.parse::<u32>().is_ok())).then(|| tag.to_string())
}

/// What `corral update` does with the installed version, the latest one and where
/// Corral is installed.
#[derive(Debug, PartialEq)]
enum Next {
    UpToDate,
    Ahead,
    Install,
    FromClone,
}

fn next_step(have: &str, latest: &str, from_clone: bool) -> Next {
    match as_tuple(have).cmp(&as_tuple(latest)) {
        std::cmp::Ordering::Equal => Next::UpToDate,
        std::cmp::Ordering::Greater => Next::Ahead,
        std::cmp::Ordering::Less if from_clone => Next::FromClone,
        std::cmp::Ordering::Less => Next::Install,
    }
}

/// `corral update [--check]`: install the latest published version, with the same
/// line as the first time, which checks the package's SHA-256 and, with the GitHub
/// CLI, its build provenance. `--check` only says whether there is a newer one. A
/// Corral installed from a copy of the repository is updated there, with git: this
/// says how instead of replacing it. From the palette it runs in a popup
/// (`corral _update`), which waits for Enter before closing.
pub fn cmd_update(args: &[String], in_popup: bool) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", update_usage());
        return Ok(());
    }
    if let Some(bad) = args.iter().find(|a| a.as_str() != "--check") {
        return Err(trf!("unknown option {bad}\n{}", "opción desconocida {bad}\n{}", update_usage()));
    }
    // From the palette there is no terminal to show the installation: a popup has one.
    if !in_popup && std::env::var_os("HERDR_PLUGIN_ID").is_some() {
        return crate::vars::open_popup(if args.iter().any(|a| a == "--check") { "update-look" } else { "update" }, &[]);
    }
    let result = update(args.iter().any(|a| a == "--check"));
    if in_popup {
        if let Err(e) = &result {
            println!("\n{e}");
        }
        println!("\n{}", tr("Press Enter to close.", "Presiona Enter para cerrar."));
        let _ = std::io::stdin().read_line(&mut String::new());
        return Ok(());
    }
    result
}

fn update(check: bool) -> Result<(), String> {
    let s = style();
    let done = env::read_json(&manifest());
    let have = done.get("version").str().unwrap_or(VERSION).to_string();
    // Only a record that says so is a published package; without one, or from a
    // clone, nothing is replaced.
    let from_clone = done.get("in_place") != &Value::Bool(false);
    let tag = latest_tag()?;
    let latest = tag.trim_start_matches('v');
    record_check(latest);
    match next_step(&have, latest, from_clone) {
        Next::UpToDate => {
            println!("{}", trf!("Corral {have} is the latest version.", "Corral {have} es la última versión."));
            Ok(())
        }
        Next::Ahead => {
            println!("{}", trf!("You have Corral {have}, newer than the latest published one ({latest}).",
                                "Tienes Corral {have}, más nuevo que el último publicado ({latest})."));
            Ok(())
        }
        Next::FromClone => {
            let folder = done.get("folder").str().map(String::from).unwrap_or_else(|| env::repo().display().to_string());
            println!("{}", trf!("Corral {latest} is out; you have {have}.", "Salió Corral {latest}; tienes {have}."));
            println!("{}", trf!("This Corral runs from a copy of the repository, so update it there:",
                                "Este Corral corre desde una copia del repositorio, así que actualízalo ahí:"));
            println!("  {}git -C {folder} pull{}\n  {}sh {folder}/install.sh{}", s.bold, s.off, s.bold, s.off);
            Ok(())
        }
        Next::Install if check => {
            println!("{}", trf!("Corral {latest} is out; you have {have}. corral update installs it.",
                                "Salió Corral {latest}; tienes {have}. corral update lo instala."));
            Ok(())
        }
        Next::Install => {
            println!("{}", trf!("Updating Corral {have} to {latest}, with the install line:",
                                "Actualizo Corral {have} a {latest}, con la línea de instalación:"));
            println!("  {}{INSTALL_LINE}{}\n", s.dim, s.off);
            let ok = Command::new("sh").args(["-c", INSTALL_LINE]).env("CORRAL_VERSION", &tag)
                .status().map(|st| st.success()).unwrap_or(false);
            if !ok {
                return Err(trf!("the update didn't finish; Corral {have} is still installed",
                                "la actualización no terminó; sigue instalado Corral {have}"));
            }
            println!("\n{}", tr("Consoles already open keep running the version they started with; the next ones you open use the new one.",
                                "Las consolas ya abiertas siguen con la versión con que partieron; las próximas que abras usan la nueva."));
            Ok(())
        }
    }
}

// ----------------------------------------------------------------------- uninstall

/// `corral uninstall`: take out what the installer added (and the user's own Corral lines it recognizes).
pub fn cmd_uninstall(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral uninstall", "uso: corral uninstall");
    if env::wants_help(args) {
        println!("{usage}");
        return Ok(());
    }
    // Nothing else is an option: a typo must not remove Corral.
    if let Some(bad) = args.first() {
        return Err(trf!("unknown option {bad}\n{usage}", "opción desconocida {bad}\n{usage}"));
    }
    let s = style();
    migrate_old_install();
    let done = env::read_json(&manifest());
    let dest = done.get("folder").str().map(PathBuf::from).unwrap_or_else(env::repo);
    let in_place = done.get("in_place") != &Value::Bool(false);
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
                if created("herdr_created") && cut.trim().is_empty() {
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
            if !kept.is_empty() || !created("rprofile_created") {
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
    if user.is_some() && created("fresh_created") {
        backup(&cfg, &mut saved);
        let _ = std::fs::remove_file(&cfg);
        step(&trf!("{}, which Corral had created", "{}, que había creado Corral", short(&cfg)), State::Ok);
        user = None;
    }
    if let Some(user) = user {
        let mut clean = user.clone();
        if let Value::Arr(list) = user.get("keybindings") {
            let ours = done.get("fresh_keys_corral").arr();
            let mut kept: Vec<Value> = list.iter()
                .filter(|b| !b.get("action").str().unwrap_or("").starts_with("corral_") && !ours.contains(b)).cloned().collect();
            for b in done.get("fresh_keys_user").arr() {
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
    for at in [bin().join("corral"), fresh_dir().join("plugins/corral.ts"), claude_dir().join("skills").join(SKILL),
               env::home().join(".agents/skills").join(SKILL), gemini_dir().join("skills").join(SKILL)].into_iter().chain(old_skill_links()) {
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
    count_uninstall(in_place);
    Ok(())
}

// ------------------------------------------------------------- new versions

/// How often opening the workspace asks GitHub for the latest version.
const CHECK_EVERY: u64 = 24 * 60 * 60;

fn update_state() -> PathBuf {
    env::state_dir().join("update-check.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether a check is due: never asked, a day since, or a clock that went back.
fn check_due(checked: Option<f64>, now: u64) -> bool {
    match checked {
        Some(c) if c >= 0.0 && (c as u64) <= now => now - c as u64 >= CHECK_EVERY,
        _ => true,
    }
}

/// The latest version the daily check found, while it is newer than the Corral you
/// have: the variables pane and the palette show it until you update, since herdr's
/// toast, the other way it is told, is off unless you turn it on (`[ui.toast] delivery`
/// is "off" by default). It reads two small files and never asks GitHub. Nothing with
/// `corral config update_check off`.
pub fn newer_version() -> Option<String> {
    if env::config().get("update_check").str() == Some("off") {
        return None;
    }
    let latest = env::read_json(&update_state()).get("latest").str().map(String::from);
    let installed = env::read_json(&manifest()).get("version").str().map(String::from);
    newer_than_installed(latest.as_deref(), installed.as_deref(), VERSION)
}

/// `latest` when it is newer than both the installed version and this binary's: the
/// pane that shows it may still be the old binary after `corral update` replaced it.
fn newer_than_installed(latest: Option<&str>, installed: Option<&str>, this: &str) -> Option<String> {
    let latest = latest?;
    let have = installed.map(as_tuple).unwrap_or((0, 0, 0)).max(as_tuple(this));
    (as_tuple(latest) > have).then(|| latest.to_string())
}

/// Whether to tell about `latest`: newer than this corral, and not told yet.
fn worth_telling(have: &str, latest: &str, told: Option<&str>) -> bool {
    as_tuple(latest) > as_tuple(have) && told != Some(latest)
}

/// A look for the latest version that reached GitHub: when (`looked`, which the reminder
/// counts from), and what it found. The daily check and `corral update` (with --check too)
/// both record it; `checked`, the daily check's attempt, also counts toward the next one.
fn record_check(latest: &str) {
    let path = update_state();
    let mut st = env::read_json(&path);
    let now = now_secs() as f64;
    st.set("checked", now.into());
    st.set("looked", now.into());
    st.set("latest", latest.into());
    env::write_json(&path, &st);
}

/// After this many days without a look for a newer version, Corral says so.
const REMIND_AFTER_DAYS: u64 = 30;

/// Days since Corral last looked for a newer version, once they are REMIND_AFTER_DAYS or
/// more: when the daily check doesn't run (the sensitive profile, update_check off) or
/// can't reach GitHub, nothing else would say a newer one is out. Without a look yet, it
/// counts from the installation. It reads two small files and asks nobody.
pub fn check_reminder() -> Option<u64> {
    let installed = std::fs::metadata(manifest()).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    // A look that reached GitHub: an attempt that didn't (`checked`) leaves the count.
    let looked = env::read_json(&update_state()).get("looked").num().map(|c| c as u64);
    reminder_days(looked.or(installed), now_secs())
}

fn reminder_days(last: Option<u64>, now: u64) -> Option<u64> {
    let days = now.saturating_sub(last?) / 86_400;
    (days >= REMIND_AFTER_DAYS).then_some(days)
}

/// Opening the workspace in `folder` calls this. At most once a day it starts `corral
/// _update-check` on its own, which asks GitHub, so the layout never waits for the
/// network. `corral config update_check off` stops it, and so does the sensitive
/// profile, for all your work or for the folder's project.
pub fn check_for_update_later(folder: &Path) {
    if env::config().get("update_check").str() == Some("off") || crate::profile::local_only(Some(folder)) {
        return;
    }
    if !check_due(env::read_json(&update_state()).get("checked").num(), now_secs()) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = Command::new(exe).arg("_update-check")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// `corral _update-check`: asks GitHub which version is the latest (the redirect
/// install.sh follows, which sends nothing about you) and, when it is newer than
/// this one and Corral hasn't said so yet, says so once in herdr.
pub fn cmd_update_check() -> Result<(), String> {
    let path = update_state();
    let mut st = env::read_json(&path);
    // First the time, so two layouts opened at once ask only once.
    st.set("checked", (now_secs() as f64).into());
    env::write_json(&path, &st);
    let tag = latest_tag()?;
    let latest = tag.trim_start_matches('v').to_string();
    st.set("latest", latest.as_str().into());
    st.set("looked", (now_secs() as f64).into());
    if worth_telling(VERSION, &latest, st.get("told").str()) {
        let shown = crate::herdr::call("notification.show", crate::json::obj(vec![
            ("title", trf!("Corral {latest} is out", "Salió Corral {latest}").into()),
            ("body", tr("corral update installs it, or Update in the palette (ctrl+b space).",
                        "corral update la instala, o Actualizar en la paleta (ctrl+b space).").into()),
        ]));
        if shown.is_ok() {
            st.set("told", latest.as_str().into());
        }
    }
    env::write_json(&path, &st);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh comes as a .tar.xz on macOS: the program comes out of its folder in the package.
    #[test]
    fn a_program_comes_out_of_a_tar_xz() {
        let dir = std::env::temp_dir().join(format!("corral-test-xz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fresh-editor-aarch64-apple-darwin")).unwrap();
        std::fs::write(dir.join("fresh-editor-aarch64-apple-darwin/fresh"), b"the program").unwrap();
        std::fs::write(dir.join("fresh-editor-aarch64-apple-darwin/README.md"), b"not it").unwrap();
        let made = Command::new("tar").args(["-cJf", "package.tar.xz", "fresh-editor-aarch64-apple-darwin"]).current_dir(&dir).status().unwrap();
        assert!(made.success());
        let data = std::fs::read(dir.join("package.tar.xz")).unwrap();
        assert_eq!(tar_xz_member(&data, "fresh").as_deref(), Some(&b"the program"[..]));
        assert_eq!(tar_xz_member(&data, "missing"), None);
        assert_eq!(tar_xz_member(b"not a package", "fresh"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The system a ping names when it isn't WSL.
    const SYS: &str = if cfg!(target_os = "macos") { "macos" } else { "linux" };

    /// httpgd on macOS comes compiled from CRAN, but only in the version the snapshot
    /// (or your repository) has, and without pulling newer dependencies from CRAN.
    #[test]
    fn httpgd_on_macos_comes_from_cran_only_in_the_snapshots_version() {
        let code = httpgd_from_cran_code();
        assert!(code.contains(r#"available.packages(repos = repos, type = "source")["httpgd", "Version"]"#), "{code}");
        assert!(code.contains(&format!(r#"cran <- c(CRAN = "{CRAN}")"#)), "{code}");
        assert!(code.contains("identical(unname(want), unname(have))"), "{code}");
        assert!(code.contains(r#"install.packages("httpgd", lib = lib, repos = cran, type = "binary", dependencies = FALSE"#), "{code}");
        assert!(code.contains(r#"CRAN = paste0("https://p3m.dev/cran/", day)"#), "the snapshot decides the version: {code}");
        assert!(code.starts_with(R_LIB));
    }

    /// A month without a look for a newer version, and Corral says how long it has been;
    /// before that, or with no date at all, nothing.
    #[test]
    fn a_month_without_a_look_is_said() {
        let day = 86_400;
        let now = 1_800_000_000;
        assert_eq!(reminder_days(Some(now - 29 * day), now), None);
        assert_eq!(reminder_days(Some(now - 30 * day), now), Some(30));
        assert_eq!(reminder_days(Some(now - 45 * day - 10), now), Some(45));
        assert_eq!(reminder_days(None, now), None);
        // A clock behind the last look counts as a look just made.
        assert_eq!(reminder_days(Some(now + day), now), None);
    }

    #[test]
    fn a_newer_version_is_shown_until_it_is_installed() {
        assert_eq!(newer_than_installed(Some("0.1.24"), Some("0.1.23"), "0.1.23").as_deref(), Some("0.1.24"));
        // Installed by `corral update` while this (old) pane runs: no longer news.
        assert_eq!(newer_than_installed(Some("0.1.24"), Some("0.1.24"), "0.1.23"), None);
        // A build ahead of what was installed, or of what is published.
        assert_eq!(newer_than_installed(Some("0.1.24"), Some("0.1.23"), "0.1.25"), None);
        assert_eq!(newer_than_installed(Some("0.1.24"), None, "0.1.23").as_deref(), Some("0.1.24"));
        assert_eq!(newer_than_installed(Some("0.1.10"), Some("0.1.9"), "0.1.9").as_deref(), Some("0.1.10"));
        // A new minor version is newer than any patch of the one before.
        assert_eq!(newer_than_installed(Some("0.2.0"), Some("0.1.29"), "0.1.29").as_deref(), Some("0.2.0"));
        assert_eq!(newer_than_installed(Some("0.1.30"), Some("0.2.0"), "0.2.0"), None);
        assert_eq!(newer_than_installed(None, Some("0.1.23"), "0.1.23"), None);
    }

    #[test]
    fn the_ping_says_which_version_an_update_replaced() {
        let none: [(&str, String); 0] = [];
        assert_eq!(ping_body(None, true, false, &none, &[]),
                   format!(r#"{{"version":"{VERSION}","system":"wsl","kind":"new","source":"release"}}"#));
        assert_eq!(ping_body(Some("0.1.15"), false, true, &none, &[]),
                   format!(r#"{{"version":"{VERSION}","system":"{SYS}","kind":"update","source":"clone","from":"0.1.15"}}"#));
        // A version that doesn't read as one is left out, and it is still an update.
        let odd = ping_body(Some(r#"0.1"}x"#), true, false, &none, &[]);
        assert!(odd.contains(r#""kind":"update""#) && !odd.contains("from"), "{odd}");
        assert!(!ping_body(Some(""), true, false, &none, &[]).contains("from"));
    }

    #[test]
    fn the_ping_carries_where_corral_runs_in_categories() {
        let envs = [("agent", "claude".to_string()), ("distro", "noble".to_string()), ("herdr", "0.9.5".to_string()),
                    ("r", "4.6".to_string()), ("python", "uv".to_string())];
        assert_eq!(ping_body(None, true, false, &envs, &[]),
                   format!(r#"{{"version":"{VERSION}","system":"wsl","kind":"new","source":"release","agent":"claude","distro":"noble","herdr":"0.9.5","r":"4.6","python":"uv"}}"#));
        // The R packages that didn't go in.
        assert!(ping_body(None, true, false, &envs, &["httpgd", "duckdb"]).contains(r#""r_failed":["httpgd","duckdb"]"#),
                "{}", ping_body(None, true, false, &envs, &["httpgd", "duckdb"]));
        assert!(!ping_body(None, true, false, &envs, &[]).contains("r_failed"));
        assert_eq!(uninstall_body(false, false), format!(r#"{{"version":"{VERSION}","system":"{SYS}","kind":"uninstall","source":"release"}}"#));
        let installed = |c: &str| c == "codex";
        assert_eq!(agent_kind(None, installed), "codex");
        assert_eq!(agent_kind(None, |_: &str| false), "none");
        assert_eq!(agent_kind(Some("none"), installed), "none");
        assert_eq!(agent_kind(Some("/usr/local/bin/claude --model x"), installed), "claude");
        assert_eq!(agent_kind(Some("my-own-agent --flag"), installed), "other");
        assert_eq!(major_minor("4.6.1"), "4.6");
        assert_eq!(major_minor("R version"), "unknown");
        assert!(plain_version("0.9.5") && plain_version("1.4") && !plain_version("0.9.5-beta") && !plain_version("1.2.3.4"));
    }

    #[test]
    fn only_corral_s_own_links_under_the_old_skill_name_go() {
        let base = std::env::temp_dir().join(format!("corral-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b, c) = (base.join("agents"), base.join("claude"), base.join("gemini"));
        for d in [&a, &b, &c] {
            std::fs::create_dir_all(d).unwrap();
        }
        symlink("/somewhere/corral-term/skills/corral-r", a.join(OLD_SKILL)).unwrap(); // Corral's: goes
        std::fs::create_dir_all(b.join(OLD_SKILL)).unwrap(); // a folder of yours: stays
        symlink("/elsewhere/my-skill", c.join(OLD_SKILL)).unwrap(); // a link of yours: stays
        assert_eq!(old_skill_links_in(&[a.clone(), b, c]), vec![a.join(OLD_SKILL)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn new_versions_are_checked_once_a_day_and_told_once() {
        assert!(check_due(None, 1_000_000));
        assert!(!check_due(Some(1_000_000.0), 1_000_000 + 3600));
        assert!(check_due(Some(1_000_000.0), 1_000_000 + CHECK_EVERY));
        assert!(check_due(Some(2_000_000.0), 1_000_000)); // the clock went back
        assert!(worth_telling("0.1.17", "0.1.18", None));
        assert!(!worth_telling("0.1.17", "0.1.18", Some("0.1.18")));
        assert!(worth_telling("0.1.17", "0.1.19", Some("0.1.18")));
        assert!(!worth_telling("0.1.17", "0.1.17", None));
        assert!(!worth_telling("0.1.18", "0.1.17", None)); // a build ahead of the latest
        assert!(worth_telling("0.1.9", "0.1.10", None)); // not compared as text
    }

    #[test]
    fn the_stop_ping_says_what_was_missing() {
        let none: [(&str, String); 0] = [];
        assert_eq!(stop_body(&["herdr", "r_packages"], true, false, &none),
                   format!(r#"{{"version":"{VERSION}","system":"wsl","kind":"stopped","source":"release","missing":["herdr","r_packages"]}}"#));
    }

    #[test]
    fn update_reads_the_latest_tag() {
        assert_eq!(tag_of("https://github.com/paulovillarroel/corral-term/releases/tag/v0.1.12\n").as_deref(), Some("v0.1.12"));
        assert_eq!(tag_of("https://github.com/paulovillarroel/corral-term/releases"), None);
        assert_eq!(tag_of("https://github.com/x/releases/tag/nightly"), None);
        assert_eq!(tag_of(""), None);
    }

    #[test]
    fn update_knows_what_to_do() {
        assert_eq!(next_step("0.1.12", "0.1.12", false), Next::UpToDate);
        assert_eq!(next_step("0.1.11", "0.1.12", false), Next::Install);
        assert_eq!(next_step("0.1.9", "0.1.12", false), Next::Install); // not compared as text
        assert_eq!(next_step("0.1.11", "0.1.12", true), Next::FromClone);
        assert_eq!(next_step("0.2.0", "0.1.12", false), Next::Ahead);
    }

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
        for (name, _, _, _, files) in DOWNLOADS {
            for (url, sha) in files {
                assert!(hex(sha), "{name}: {url}");
            }
        }
    }

    #[test]
    fn renamed_actions() {
        let text = "command = \"corral.atajos\"\ncommand = \"corral.layout-codigo\"\ncommand = \"corral.layout\"\n";
        assert_eq!(with_renamed_actions(text), "command = \"corral.keys\"\ncommand = \"corral.layout-code\"\ncommand = \"corral.layout\"\n");
    }

    #[test]
    fn the_herdr_it_brings_is_one_corral_accepts() {
        let (v, files) = HERDR;
        assert!(as_tuple(v) >= HERDR_MIN, "{v}");
        for (url, sha) in files {
            assert!(url.starts_with("https://github.com/herdrdev/herdr/releases/download/") && url.contains("{v}"), "{url}");
            assert!(sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{sha}");
        }
    }

    #[test]
    fn r_packages_go_in_with_pak_when_your_r_has_it() {
        assert_eq!(install_r_with(&["httpgd", "jsonlite"], true), r#"Rscript -e 'pak::pak(c("httpgd", "jsonlite"))'"#);
        assert_eq!(install_r_with(&["rlang"], false), format!(r#"Rscript -e 'install.packages(c("rlang"), repos = "{CRAN}")'"#));
        let pak = r_install_code(&["httpgd"], true);
        assert!(pak.contains("pak::pak(pkgs, lib = lib, upgrade = FALSE, ask = FALSE)") && pak.contains("pkg.sysreqs = FALSE"), "{pak}");
        let base = r_install_code(&["httpgd"], false);
        assert!(base.contains("install.packages(pkgs, lib = lib, quiet = TRUE)") && !base.contains("pak"), "{base}");
    }

    #[test]
    fn r_packages_come_from_your_repos_or_a_snapshot_at_least_three_days_old() {
        let code = r_repos_code(Some("noble"));
        assert!(code.contains(r#"Sys.getenv("CORRAL_R_REPOS")"#) && code.contains("any(!grepl(public, own))"), "{code}");
        assert!(code.contains("day <- format(Sys.Date() - 3)"), "{code}");
        assert!(code.contains(r#"P3M = paste0("https://p3m.dev/cran/__linux__/noble/", day)"#), "{code}");
        assert!(code.contains(r#"CRAN = paste0("https://p3m.dev/cran/", day)"#), "{code}");
        // No live repository next to the snapshot: the newest version would win.
        assert!(!code.contains("/latest") && !code.contains(r#"= "https://cloud.r-project.org""#), "{code}");
        assert!(!r_repos_code(None).contains("__linux__"));
        assert!(r_install_code(&["jsonlite"], false).contains("options(repos = repos)"));
    }

    #[test]
    fn pak_s_system_commands_come_with_sudo_and_nothing_else() {
        let text = "✔ Updated metadata database\napt-get -y update\napt-get -y install libcairo2-dev libpng-dev\n\n";
        assert_eq!(with_sudo(text).as_deref(), Some("sudo apt-get -y update && sudo apt-get -y install libcairo2-dev libpng-dev"));
        assert_eq!(with_sudo("sudo dnf install -y cairo-devel\n").as_deref(), Some("sudo dnf install -y cairo-devel"));
        assert_eq!(with_sudo("ℹ nothing to install\n"), None);
    }

    #[test]
    fn the_uv_it_offers_is_pinned_and_checked() {
        let (v, files) = UV;
        assert!(plain_version(v), "{v}");
        for ((url, sha), ending) in files.iter().zip(["x86_64-unknown-linux-musl.tar.gz", "aarch64-apple-darwin.tar.gz", "x86_64-apple-darwin.tar.gz"]) {
            assert!(url.starts_with("https://github.com/astral-sh/uv/releases/download/{v}/") && url.ends_with(ending), "{url}");
            assert!(sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{sha}");
        }
    }

    #[test]
    fn enter_says_yes_to_bringing_what_is_missing() {
        for answer in ["", "\n", "y", "Yes\n", "s", "sí\n", " SI "] {
            assert!(said_yes(answer), "{answer:?}");
        }
        for answer in ["n", "no\n", "N", "q"] {
            assert!(!said_yes(answer), "{answer:?}");
        }
    }

    #[test]
    fn what_it_can_bring_still_counts_as_missing() {
        let herdr = || check("required", State::Missing, "herdr", "", &["Install herdr:", "  curl …"]).id("herdr").brings("herdr", "herdr 0.9.5");
        let fine = || check("required", State::Ok, "System", "", &[]);
        assert!(!missing_steps(&[herdr(), fine()], "again", true, true));
        assert!(!missing_steps(&[herdr(), fine()], "again", false, false));
        assert!(missing_steps(&[fine()], "again", true, true));
        assert_eq!(and_list(&["herdr"]), "herdr");
        assert!(and_list(&["a", "b", "c"]).starts_with("a, b"));
    }
}
