//! `corral ark` (Ark's status, or install a version) and `corral doctor`.

use crate::console;
use crate::env;
use crate::herdr;
use crate::plots;
use crate::theme;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Ark version Corral was tested with.
pub const ARK_VERSION: &str = "0.1.252";

/// Download `url` (with curl, which the install line needs anyway).
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl").args(["-fsSL", "--retry", "2", "--connect-timeout", "20", url]).output()
        .map_err(|e| format!("no pude ejecutar curl: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("no pude descargar {url}: {}", if why.is_empty() { "curl falló".into() } else { why }));
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

/// `corral ark [--instalar [VERSIÓN]]`.
pub fn cmd_ark(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {}
        Some("--instalar") => {
            let version = args.get(1).map(String::as_str).unwrap_or(ARK_VERSION);
            println!("descargando Ark {version} desde las versiones publicadas de posit-dev/ark");
            let target = install_ark(version)?;
            println!("Ark {version} instalado en {}", target.display());
        }
        Some(other) => return Err(format!("opción desconocida {other}\nuso: corral ark [--instalar [VERSIÓN]]")),
    }
    let cfg = env::config();
    let ark = console::ark_binary(&cfg);
    let jupyter = env::which("jupyter-console");
    println!("Ark:             {}", ark.as_ref().map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("no instalado (corral ark --instalar {ARK_VERSION})")));
    println!("jupyter-console: {}", jupyter.map(|p| p.display().to_string())
        .unwrap_or_else(|| "no instalado (opcional: corral config ark_frontend jupyter)".into()));
    println!("consola de R:    {}  (corral config r_console ark|r)", console::r_flavor(&cfg));
    println!("frente de Ark:   {}", if console::jupyter_frontend(&cfg) { "jupyter-console" } else { "consola de Corral (Rust)" });
    let spec = env::data_dir().join("jupyter/kernels/ark/kernel.json");
    if spec.is_file() {
        println!("kernel:          {}", spec.display());
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

    let herdr_bin = env::which("herdr");
    check(herdr_bin.is_some(), "herdr", herdr_bin.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "no está en el PATH".into()));
    let inside = env::in_herdr();
    check(inside, "dentro de herdr", if inside { "sí".into() } else { "no (corral funciona igual, sin paneles)".into() });
    if herdr_bin.is_some() {
        match herdr::cli(&["plugin", "list", "--json"]) {
            Ok(list) => {
                let linked = list.get("plugins").arr().iter()
                    .any(|p| p.get("plugin_id").str() == Some("corral") || p.get("id").str() == Some("corral"));
                check(linked, "plugin enlazado", if linked { "sí".into() } else { format!("herdr plugin link {}", env::repo().join("herdr").display()) });
            }
            Err(e) => check(false, "plugin enlazado", e),
        }
    }
    let uv = env::which("uv");
    check(uv.is_some(), "uv (consola de Python)", uv.map(|p| p.display().to_string()).unwrap_or_else(|| "no está en el PATH; https://docs.astral.sh/uv/".into()));
    let cfg = env::config();
    let ark = console::ark_binary(&cfg);
    check(ark.is_some(), "Ark (consola de R)", ark.map(|p| p.display().to_string())
        .unwrap_or_else(|| "corral ark --instalar  (sin Ark, la consola es R normal)".into()));
    if console::jupyter_frontend(&cfg) {
        let j = env::which("jupyter-console");
        check(j.is_some(), "jupyter-console (para Ark)", j.map(|p| p.display().to_string()).unwrap_or_else(|| "uv tool install jupyter-console".into()));
    }
    check(true, "consola de R", if console::r_flavor(&cfg) == "ark" { "Ark".into() } else { "R normal (corral config r_console)".into() });
    let rscript = env::which("Rscript");
    check(rscript.is_some(), "R", rscript.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "no está en el PATH".into()));
    if let Some(r) = &rscript {
        let has = r_packages(r, &["httpgd", "jsonlite", "arrow", "nanoparquet"]);
        check(has[0], "R: httpgd", "gráficos".into());
        check(has[1], "R: jsonlite", "variables en Mirador".into());
        let parquet = if has[2] { Some("arrow") } else if has[3] { Some("nanoparquet") } else { None };
        check(parquet.is_some(), "R: parquet", parquet.map(String::from).unwrap_or_else(|| "instala nanoparquet o arrow; mientras tanto View() usa CSV".into()));
    }
    let wanted = theme::palette().get("name").str().unwrap_or("").to_string();
    let applied = env::read_json(&theme::palette_file());
    let same = applied.get("name").str() == Some(wanted.as_str());
    let source = if theme::own_theme().is_some() { "tema de Corral" } else { "desde herdr" };
    check(same, "tema", format!("{wanted} {source}{}", if same { "" } else { "; corre `corral theme`" }));
    let (mode, why) = plots::image_mode();
    check(mode == "kitty", "gráficos en herdr", format!("{}; {why}", if mode == "kitty" { "nítidos (Kitty)" } else { "con caracteres" }));
    let vd = env::which("vd");
    check(vd.is_some(), "VisiData", vd.map(|p| p.display().to_string()).unwrap_or_else(|| "opcional; las tablas se ven con el visor de Corral".into()));
    let duck = env::which("duckdb");
    check(duck.is_some(), "DuckDB", duck.map(|p| p.display().to_string()).unwrap_or_else(|| "no está en el PATH".into()));
    let browser = plots::browser_argv("URL");
    let shown = match &browser {
        Some(argv) => {
            let mut parts = vec![Path::new(&argv[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()];
            parts.extend(argv[1..].iter().cloned());
            parts.join(" ")
        }
        None => "solo muestra la URL (CORRAL_BROWSER)".into(),
    };
    check(browser.is_some(), "ventana de gráficos", shown);
    let info = plots::mirador_info();
    let up = plots::mirador_alive(&info, false);
    check(true, "Mirador", if up { info.get("url").str().unwrap_or("").to_string() } else {
        "detenido; se inicia con el primer gráfico o con `corral mirador`".into()
    });

    let width = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    for (ok, name, detail) in rows {
        let pad = width - name.chars().count();
        println!("{} {name}{}  {detail}", if ok { "✓" } else { "✗" }, " ".repeat(pad));
    }
    Ok(())
}
