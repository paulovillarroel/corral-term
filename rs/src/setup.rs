//! `corral ark` (Ark's status, or install a version) and `corral doctor`.

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::plots;
use crate::theme;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Ark version Corral was tested with, and the SHA-256 of its Linux package.
pub const ARK_VERSION: &str = "0.1.252";
const ARK_SHA256: &str = "7dda5d05b6c4d67e7ae74e70b6bec5fd3bc526b19e222fa78306e14972cf4495";

/// Check a download against the SHA-256 it should have, before anything uses it.
pub fn checked(what: &str, data: &[u8], expected: &str) -> Result<(), String> {
    let actual = crate::sha256::sha256_hex(data);
    if actual == expected.to_ascii_lowercase() {
        return Ok(());
    }
    Err(trf!("{what} doesn't match its SHA-256, so I didn't install it (expected {expected}, got {actual})",
             "{what} no coincide con su SHA-256, así que no lo instalé (esperado {expected}, obtenido {actual})"))
}

/// The SHA-256 GitHub reports for a file of a release (its `digest`), when it has one.
// net: tools: Ark's SHA-256, for a version Corral doesn't carry one for
fn github_digest(repo: &str, tag: &str, file: &str) -> Option<String> {
    let body = download(&format!("https://api.github.com/repos/{repo}/releases/tags/{tag}")).ok()?;
    let release = crate::json::parse(&String::from_utf8_lossy(&body)).ok()?;
    let asset = release.get("assets").arr().iter().find(|a| a.get("name").str() == Some(file))?;
    asset.get("digest").str()?.strip_prefix("sha256:").map(String::from)
}

/// Download `url` (with curl, which the install line needs anyway).
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    // net: tools: what corral install downloads (setup::download)
    let out = Command::new("curl").args(["-fsSL", "--retry", "2", "--connect-timeout", "20", url]).output()
        .map_err(|e| trf!("couldn't run curl: {e}", "no pude ejecutar curl: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let why = if why.is_empty() { tr("curl failed", "curl falló").to_string() } else { why };
        return Err(trf!("couldn't download {url}: {why}", "no pude descargar {url}: {why}"));
    }
    Ok(out.stdout)
}

/// Install Ark `version` under ~/.local/share/corral/ark/<version>. The version Corral
/// was tested with is checked against the SHA-256 written above, in Corral's own code;
/// another one, against the SHA-256 GitHub reports for it, and refused if there is none.
pub fn install_ark(version: &str) -> Result<PathBuf, String> {
    let file = format!("ark-{version}-linux-x64.zip");
    let url = format!("https://github.com/posit-dev/ark/releases/download/{version}/{file}");
    let expected = if version == ARK_VERSION {
        ARK_SHA256.to_string()
    } else {
        // net: tools
        github_digest("posit-dev/ark", version, &file).ok_or_else(|| trf!(
            "GitHub has no SHA-256 for Ark {version}, so I won't install it; Ark {ARK_VERSION} is the one Corral was tested with",
            "GitHub no tiene un SHA-256 para Ark {version}, así que no lo instalo; Ark {ARK_VERSION} es la que Corral probó"))?
    };
    let zip = download(&url)?;
    checked(&format!("Ark {version}"), &zip, &expected)?;
    let target = env::data_dir().join("ark").join(version);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    crate::zip::extract(&zip, &target)?;
    let ark = target.join("ark");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ark, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", ark.display()))?;
    Ok(target)
}

fn ark_usage() -> &'static str {
    tr("usage: corral ark [--install [VERSION]]", "uso: corral ark [--install [VERSIÓN]]")
}

/// `corral ark [--install [VERSION]]`.
pub fn cmd_ark(args: &[String]) -> Result<(), String> {
    if env::wants_help(args) {
        println!("{}", ark_usage());
        return Ok(());
    }
    match args.first().map(String::as_str) {
        None => {}
        Some("--install") => {
            let version = args.get(1).map(String::as_str).unwrap_or(ARK_VERSION);
            println!("{}", trf!("downloading Ark {version} from posit-dev/ark's published releases",
                                "descargando Ark {version} desde las versiones publicadas de posit-dev/ark"));
            let target = install_ark(version)?;
            println!("{}", trf!("Ark {version} installed in {}", "Ark {version} instalado en {}", target.display()));
        }
        Some(other) => return Err(trf!("unknown option {other}\n{}", "opción desconocida {other}\n{}", ark_usage())),
    }
    let cfg = env::config();
    let ark = console::ark_binary(&cfg);
    let jupyter = env::which("jupyter-console");
    let mut rows = vec![
        ("Ark", ark.as_ref().map(|p| p.display().to_string())
            .unwrap_or_else(|| trf!("not installed (corral ark --install {ARK_VERSION})", "no instalado (corral ark --install {ARK_VERSION})"))),
        ("jupyter-console", jupyter.map(|p| p.display().to_string())
            .unwrap_or_else(|| tr("not installed (optional: corral config ark_frontend jupyter)",
                                  "no instalado (opcional: corral config ark_frontend jupyter)").into())),
        (tr("R console", "consola de R"), format!("{}  (corral config r_console ark|r)", console::r_flavor(&cfg))),
        (tr("Ark frontend", "frente de Ark"),
         if console::jupyter_frontend(&cfg) { "jupyter-console".into() } else { tr("Corral's console (Rust)", "consola de Corral (Rust)").into() }),
    ];
    let spec = env::data_dir().join("jupyter/kernels/ark/kernel.json");
    if spec.is_file() {
        rows.push(("kernel", spec.display().to_string()));
    }
    let width = rows.iter().map(|(label, _)| label.chars().count()).max().unwrap_or(0) + 1;
    for (label, value) in rows {
        println!("{:<width$} {value}", format!("{label}:"));
    }
    Ok(())
}

/// Which of these R packages load (`requireNamespace`), without the user's .Rprofile.
pub fn r_packages(rscript: &Path, pkgs: &[&str]) -> Vec<bool> {
    let list = pkgs.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    let probe = format!("cat(sapply(c({list}), requireNamespace, quietly = TRUE))");
    let out = Command::new(rscript).args(["--no-init-file", "-e", &probe]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let found: Vec<bool> = text.split_whitespace().map(|w| w == "TRUE").collect();
    pkgs.iter().enumerate().map(|(i, _)| found.get(i).copied().unwrap_or(false)).collect()
}

/// `corral doctor`: what Corral needs and how it is set up, one line each.
pub fn cmd_doctor(args: &[String]) -> Result<(), String> {
    let usage = tr("usage: corral doctor", "uso: corral doctor");
    if env::wants_help(args) {
        println!("{usage}");
        return Ok(());
    }
    if let Some(bad) = args.first() {
        return Err(trf!("unknown option {bad}\n{usage}", "opción desconocida {bad}\n{usage}"));
    }
    let mut rows: Vec<(bool, String, String)> = Vec::new();
    let mut check = |ok: bool, name: &str, detail: String| rows.push((ok, name.to_string(), detail));

    let not_on_path = || tr("not on the PATH", "no está en el PATH").to_string();
    let yes = || tr("yes", "sí").to_string();
    let herdr_bin = env::which("herdr");
    check(herdr_bin.is_some(), "herdr", herdr_bin.as_ref().map(|p| p.display().to_string()).unwrap_or_else(not_on_path));
    let inside = env::in_herdr();
    check(inside, tr("inside herdr", "dentro de herdr"),
          if inside { yes() } else { tr("no (corral works the same, without panes)", "no (corral funciona igual, sin paneles)").into() });
    if herdr_bin.is_some() {
        let label = tr("plugin linked", "plugin enlazado");
        match herdr::cli(&["plugin", "list", "--json"]) {
            Ok(list) => {
                let linked = list.get("plugins").arr().iter()
                    .any(|p| p.get("plugin_id").str() == Some("corral") || p.get("id").str() == Some("corral"));
                check(linked, label, if linked { yes() } else { format!("herdr plugin link {}", env::repo().join("herdr").display()) });
            }
            Err(e) => check(false, label, e),
        }
    }
    let uv = env::which("uv");
    check(uv.is_some(), tr("uv (Python console)", "uv (consola de Python)"), uv.map(|p| p.display().to_string())
        .unwrap_or_else(|| tr("not on the PATH; https://docs.astral.sh/uv/", "no está en el PATH; https://docs.astral.sh/uv/").into()));
    let cfg = env::config();
    let ark = console::ark_binary(&cfg);
    check(ark.is_some(), tr("Ark (R console)", "Ark (consola de R)"), ark.map(|p| p.display().to_string())
        .unwrap_or_else(|| tr("corral ark --install  (without Ark, the console is plain R)", "corral ark --install  (sin Ark, la consola es R normal)").into()));
    if console::jupyter_frontend(&cfg) {
        let j = env::which("jupyter-console");
        check(j.is_some(), tr("jupyter-console (for Ark)", "jupyter-console (para Ark)"),
              j.map(|p| p.display().to_string()).unwrap_or_else(|| "uv tool install jupyter-console".into()));
    }
    check(true, tr("R console", "consola de R"),
          if console::r_flavor(&cfg) == "ark" { "Ark".into() } else { tr("plain R (corral config r_console)", "R normal (corral config r_console)").into() });
    let rscript = crate::versions::rscript();
    let r_detail = match (&rscript, cfg.get("r_version").str()) {
        (Some(p), Some(v)) => trf!("{} (R {v}, corral versions r)", "{} (R {v}, corral versions r)", p.display()),
        (Some(p), None) => p.display().to_string(),
        (None, _) => crate::versions::r_binary().err().unwrap_or_else(not_on_path),
    };
    check(rscript.is_some(), "R", r_detail);
    if let Some(r) = &rscript {
        let has = r_packages(r, &["httpgd", "jsonlite", "arrow", "nanoparquet"]);
        check(has[0], "R: httpgd", tr("plots", "gráficos").into());
        check(has[1], "R: jsonlite", tr("variables in Mirador", "variables en Mirador").into());
        let parquet = if has[3] {
            Some("nanoparquet".to_string())
        } else if has[2] {
            Some(tr("arrow; nanoparquet is faster and keeps R's memory steady", "arrow; nanoparquet es más rápido y mantiene estable la memoria de R").into())
        } else {
            None
        };
        let ok = parquet.is_some();
        check(ok, "R: parquet", parquet.unwrap_or_else(|| {
            tr("install nanoparquet; until then View() uses CSV", "instala nanoparquet; mientras tanto View() usa CSV").into()
        }));
    }
    let wanted = theme::palette().get("name").str().unwrap_or("").to_string();
    let applied = env::read_json(&theme::palette_file());
    let same = applied.get("name").str() == Some(wanted.as_str());
    let source = if theme::own_theme().is_some() { tr("Corral theme", "tema de Corral") } else { tr("from herdr", "desde herdr") };
    let fix = if same { "" } else { tr("; run `corral theme`", "; corre `corral theme`") };
    check(same, tr("theme", "tema"), format!("{wanted} {source}{fix}"));
    let state = env::state_dir();
    let private = env::is_private(&state);
    check(private, tr("Corral's files", "archivos de Corral"), if private {
        trf!("{}, only you can open it", "{}, solo tú puedes abrirla", state.display())
    } else {
        trf!("{} is open to others: chmod 700 {}", "{} está abierta a otros: chmod 700 {}", state.display(), state.display())
    });
    let (mode, why) = plots::image_mode();
    let how = if mode == "kitty" { tr("sharp (Kitty)", "nítidos (Kitty)") } else { tr("with characters", "con caracteres") };
    check(mode == "kitty", tr("plots in herdr", "gráficos en herdr"), format!("{how}; {why}"));
    let vd = env::which("vd");
    check(vd.is_some(), "VisiData", vd.map(|p| p.display().to_string())
        .unwrap_or_else(|| tr("optional; tables show in Corral's viewer", "opcional; las tablas se ven con el visor de Corral").into()));
    let duck = env::which("duckdb");
    check(duck.is_some(), "DuckDB", duck.map(|p| p.display().to_string()).unwrap_or_else(not_on_path));
    let browser = plots::browser_argv("URL");
    let shown = match &browser {
        Some(argv) => {
            let mut parts = vec![Path::new(&argv[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()];
            parts.extend(argv[1..].iter().cloned());
            parts.join(" ")
        }
        None => tr("only shows the URL (CORRAL_BROWSER)", "solo muestra la URL (CORRAL_BROWSER)").into(),
    };
    check(browser.is_some(), tr("plot window", "ventana de gráficos"), shown);
    let info = plots::mirador_info();
    let up = plots::mirador_alive(&info, false);
    check(true, "Mirador", if up { info.get("url").str().unwrap_or("").to_string() } else {
        tr("stopped; it starts with the first plot or with `corral mirador`", "detenido; se inicia con el primer gráfico o con `corral mirador`").into()
    });

    let width = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    for (ok, name, detail) in rows {
        let pad = width - name.chars().count();
        println!("{} {name}{}  {detail}", if ok { "✓" } else { "✗" }, " ".repeat(pad));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_compares_the_sha256() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(checked("x", b"abc", abc).is_ok());
        assert!(checked("x", b"abc", &abc.to_uppercase()).is_ok());
        assert!(checked("x", b"abd", abc).is_err());
        assert!(checked("x", b"abc", "").is_err());
        assert!(ARK_SHA256.len() == 64 && ARK_SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
