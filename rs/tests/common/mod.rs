//! Helpers shared by the integration tests.

/// The value after `"key":` in a JSON text (a number or a string), without a
/// JSON library: the tests only read flat fields Corral wrote.
pub fn field(json: &str, key: &str) -> Option<String> {
    let rest = &json[json.find(&format!("\"{key}\":"))? + key.len() + 3..];
    let rest = rest.trim_start();
    Some(match rest.strip_prefix('"') {
        Some(s) => s[..s.find('"')?].to_string(),
        None => rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect(),
    })
}
