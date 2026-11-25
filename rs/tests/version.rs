//! Every place that names Corral's version names this one, so a release can't leave
//! one behind: the version PR bumps them all and CI checks it here.

use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn every_docs_page_names_this_version_in_its_header() {
    let mut pages = 0;
    for entry in std::fs::read_dir(repo("site/docs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "html") {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(&format!(r#"corral <span class="tag">docs · {VERSION}</span>"#)),
                    "{} doesn't say docs · {VERSION} in its header", path.display());
            pages += 1;
        }
    }
    assert!(pages > 10, "found only {pages} docs pages");
}

#[test]
fn the_readme_plugin_install_page_and_changelog_name_this_version() {
    assert!(read("README.md").contains(&format!("Version {VERSION} ·")), "README.md");
    assert!(read("herdr/herdr-plugin.toml").contains(&format!("version = \"{VERSION}\"")), "herdr-plugin.toml");
    let install = read("site/docs/install.html");
    assert!(install.contains(&format!("Downloading Corral {VERSION}...")), "install.html sample output");
    assert!(install.contains(&format!("Corral {VERSION}: what it needs")), "install.html --check output");
    assert!(read("CHANGELOG.md").contains(&format!("\n## {VERSION}\n")), "CHANGELOG.md section");
}
