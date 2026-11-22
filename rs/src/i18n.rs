//! Corral's language: English by default, Spanish on request
//! (`corral config language es`, or "Corral: idioma español" in herdr's palette).
//!
//! Every text a person reads is written in both languages, next to each other:
//! `tr("Close", "Cerrar")` for plain text and `trf!("{n} rows", "{n} filas")` for
//! formatted text. CORRAL_LANG (en | es) overrides the setting for one process.
//! Long-lived processes (the variables pane, the table viewer, Mirador's server)
//! check the setting again every couple of seconds, so a switch shows up live.

use std::cell::Cell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

thread_local! {
    // True while this thread reads the setting: a message produced on the way
    // (a broken config.json, say) comes out in English instead of asking again.
    static RESOLVING: Cell<bool> = const { Cell::new(false) };
}

/// The languages Corral speaks, as `corral config language` takes them.
pub const LANGUAGES: [&str; 2] = ["en", "es"];

/// "en" or "es": CORRAL_LANG, else `language` in config.json, else English.
pub fn resolve() -> &'static str {
    if let Ok(l) = std::env::var("CORRAL_LANG") {
        if let Some(found) = normalize(&l) {
            return found;
        }
    }
    crate::env::config().get("language").str().and_then(normalize).unwrap_or("en")
}

/// "en", "english", "es", "español", "spanish"… to "en" or "es".
pub fn normalize(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        "en" | "english" | "inglés" | "ingles" => Some("en"),
        "es" | "spanish" | "español" | "espanol" => Some("es"),
        _ => None,
    }
}

/// True when Corral speaks Spanish.
pub fn es() -> bool {
    if RESOLVING.with(Cell::get) {
        return false;
    }
    if let Some((when, value)) = *CACHE.lock().unwrap_or_else(|p| p.into_inner()) {
        if when.elapsed() < Duration::from_secs(2) {
            return value;
        }
    }
    // Read the setting without holding the lock, so nothing it prints can wait on it.
    RESOLVING.with(|r| r.set(true));
    let value = resolve() == "es";
    RESOLVING.with(|r| r.set(false));
    *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), value));
    value
}

/// "en" or "es", for pages and sessions that translate on their side.
pub fn lang() -> &'static str {
    if es() { "es" } else { "en" }
}

/// The text in the user's language.
pub fn tr<'a>(en: &'a str, es_text: &'a str) -> &'a str {
    if es() { es_text } else { en }
}

/// A whole number with thousands separators: 10,000,000 in English, 10.000.000 in Spanish.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let sep = if es() { '.' } else { ',' };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// `format!` in the user's language: `trf!("{n} rows", "{n} filas")`.
#[macro_export]
macro_rules! trf {
    ($en:literal, $es:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::es() { format!($es $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(normalize("ES"), Some("es"));
        assert_eq!(normalize("Español"), Some("es"));
        assert_eq!(normalize("english"), Some("en"));
        assert_eq!(normalize("fr"), None);
    }

    #[test]
    fn formats_with_captured_names() {
        std::env::set_var("CORRAL_LANG", "en");
        let n = 3;
        assert_eq!(trf!("{n} rows", "{n} filas"), "3 rows");
        assert_eq!(trf!("{} of {}", "{} de {}", 1, 2), "1 of 2");
        assert_eq!(tr("Close", "Cerrar"), "Close");
        assert_eq!(thousands(10_000_000), "10,000,000");
        assert_eq!(thousands(999), "999");
    }
}
