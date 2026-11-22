//! `corral ark` (Ark's status, or install a version) and `corral doctor`.

use crate::console;
use crate::env;
use crate::herdr;
use crate::i18n::tr;
use crate::plots;
use crate::theme;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Ark version Corral was tested with.
pub const ARK_VERSION: &str = "0.1.252";

/// Download `url` (with curl, which the install line needs anyway).
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl").args(["-fsSL", "--retry", "2", "--connect-timeout", "20", url]).output()
        .map_err(|e| trf!("couldn't run curl: {e}", "no pude ejecutar curl: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let why = if why.is_empty() { tr("curl failed", "curl falló").to_string() } else { why };
        return Err(trf!("couldn't download {url}: {why}", "no pude descargar {url}: {why}"));
    }
    Ok(out.stdout)
}

/// Install Ark `version` under ~/.local/share/corral/ark/<version>.
pub fn install_ark(version: &str) -> Result<PathBuf, String> {
    let url = format!("https://github.com/posit-dev/ark/releases/download/{version}/ark-{version}-linux-x64.zip");
    let target = env::data_dir().join("ark").join(version);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let zip = download(&url)?;
    crate::zip::extract(&zip, &target)?;
    let ark = target.join("ark");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ark, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", ark.display()))?;
    Ok(target)
}

/// `corral ark [--install [VERSION]]`.
pub fn cmd_ark(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {}
        Some("--install") => {
            let version = args.get(1).map(String::as_str).unwrap_or(ARK_VERSION);
            println!("{}", trf!("downloading Ark {version} from posit-dev/ark's published releases",
                                "descargando Ark {version} desde las versiones publicadas de posit-dev/ark"));
            let target = install_ark(version)?;
            println!("{}", trf!("Ark {version} installed in {}", "Ark {version} instalado en {}", target.display()));
        }
        Some(other) => return Err(trf!("unknown option {other}\nusage: corral ark [--install [VERSION]]",
                                       "opción desconocida {other}\nuso: corral ark [--install [VERSIÓN]]")),
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
pub fn cmd_doctor(_args: &[String]) -> Result<(), String> {
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
    let rscript = env::which("Rscript");
    check(rscript.is_some(), "R", rscript.as_ref().map(|p| p.display().to_string()).unwrap_or_else(not_on_path));
    if let Some(r) = &rscript {
        let has = r_packages(r, &["httpgd", "jsonlite", "arrow", "nanoparquet"]);
        check(has[0], "R: httpgd", tr("plots", "gráficos").into());
        check(has[1], "R: jsonlite", tr("variables in Mirador", "variables en Mirador").into());
        let parquet = if has[2] { Some("arrow") } else if has[3] { Some("nanoparquet") } else { None };
        check(parquet.is_some(), "R: parquet", parquet.map(String::from).unwrap_or_else(|| {
            tr("install nanoparquet or arrow; until then View() uses CSV", "instala nanoparquet o arrow; mientras tanto View() usa CSV").into()
        }));
    }
    let wanted = theme::palette().get("name").str().unwrap_or("").to_string();
    let applied = env::read_json(&theme::palette_file());
    let same = applied.get("name").str() == Some(wanted.as_str());
    let source = if theme::own_theme().is_some() { tr("Corral theme", "tema de Corral") } else { tr("from herdr", "desde herdr") };
    let fix = if same { "" } else { tr("; run `corral theme`", "; corre `corral theme`") };
    check(same, tr("theme", "tema"), format!("{wanted} {source}{fix}"));
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
