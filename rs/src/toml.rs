//! A small TOML reader for herdr's config.toml: tables, arrays of tables,
//! dotted and quoted keys, strings (basic and literal, multi-line too),
//! numbers, booleans, and arrays and inline tables of those. It reads into the
//! same Value as the JSON module. Writing goes through herdr's own validator
//! (`herdr config check`), so Corral never has to judge a file it changed.

use crate::i18n::tr;
use crate::json::Value;

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = P { s: text.as_bytes(), i: 0 };
    let mut root = Value::obj();
    // The table that key = value lines go to: a path of keys, the last one
    // pointing into an array when the header was [[...]].
    let mut current: Vec<(String, bool)> = Vec::new();
    loop {
        p.skip_ws_comments_newlines();
        if p.eof() {
            return Ok(root);
        }
        if p.peek() == b'[' {
            let array = p.s.get(p.i + 1) == Some(&b'[');
            p.i += if array { 2 } else { 1 };
            p.skip_ws();
            let path = p.key_path()?;
            p.skip_ws();
            p.expect(b']')?;
            if array {
                p.expect(b']')?;
            }
            p.end_of_line()?;
            let target = table_at(&mut root, &path[..path.len() - 1])?;
            let last = path.last().cloned().unwrap_or_default();
            if array {
                let mut items = match target.get(&last) {
                    Value::Arr(a) => a.clone(),
                    Value::Null => Vec::new(),
                    _ => return Err(trf!("{last} isn't an array of tables", "{last} no es una lista de tablas")),
                };
                items.push(Value::obj());
                target.set(&last, Value::Arr(items));
            } else if target.get(&last).is_null() {
                target.set(&last, Value::obj());
            }
            current = path.iter().enumerate().map(|(n, k)| (k.clone(), array && n == path.len() - 1)).collect();
        } else {
            let path = p.key_path()?;
            p.skip_ws();
            p.expect(b'=')?;
            p.skip_ws();
            let value = p.value()?;
            p.end_of_line()?;
            let table = table_for(&mut root, &current)?;
            let target = table_at(table, &path[..path.len() - 1])?;
            target.set(path.last().map(String::as_str).unwrap_or(""), value);
        }
    }
}

/// The table at a header path, following the last element of arrays of tables.
fn table_for<'v>(root: &'v mut Value, path: &[(String, bool)]) -> Result<&'v mut Value, String> {
    let mut node = root;
    for (key, _) in path {
        node = descend(node, key)?;
    }
    Ok(node)
}

/// Walk (and create) plain tables along `path`.
fn table_at<'v>(root: &'v mut Value, path: &[String]) -> Result<&'v mut Value, String> {
    let mut node = root;
    for key in path {
        if node.get(key).is_null() {
            node.set(key, Value::obj());
        }
        node = descend(node, key)?;
    }
    Ok(node)
}

fn descend<'v>(node: &'v mut Value, key: &str) -> Result<&'v mut Value, String> {
    let Value::Obj(items) = node else { return Err(trf!("{key}: not a table", "{key}: no es una tabla")) };
    let slot = items.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v).ok_or_else(|| trf!("missing table {key}", "falta la tabla {key}"))?;
    match slot {
        Value::Arr(a) => a.last_mut().ok_or_else(|| trf!("{key}: empty array", "{key}: lista vacía")),
        Value::Obj(_) => Ok(slot),
        _ => Err(trf!("{key}: not a table", "{key}: no es una tabla")),
    }
}

impl P<'_> {
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }

    fn peek(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }

    fn err(&self, what: &str) -> String {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        trf!("TOML, line {line}: {what}", "TOML, línea {line}: {what}")
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == b {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&trf!("expected '{}'", "se esperaba '{}'", b as char)))
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), b' ' | b'\t') {
            self.i += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == b'#' {
            while !self.eof() && self.peek() != b'\n' {
                self.i += 1;
            }
        }
    }

    fn skip_ws_comments_newlines(&mut self) {
        loop {
            self.skip_ws();
            self.skip_comment();
            match self.peek() {
                b'\n' | b'\r' => self.i += 1,
                _ => return,
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), String> {
        self.skip_ws();
        self.skip_comment();
        match self.peek() {
            b'\n' | b'\r' | 0 => Ok(()),
            _ => Err(self.err(tr("extra text at the end of the line", "sobra texto al final de la línea"))),
        }
    }

    fn key_path(&mut self) -> Result<Vec<String>, String> {
        let mut path = vec![self.key()?];
        loop {
            self.skip_ws();
            if self.peek() != b'.' {
                return Ok(path);
            }
            self.i += 1;
            self.skip_ws();
            path.push(self.key()?);
        }
    }

    fn key(&mut self) -> Result<String, String> {
        match self.peek() {
            b'"' => self.basic_string(),
            b'\'' => self.literal_string(),
            _ => {
                let start = self.i;
                while matches!(self.peek(), b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
                    self.i += 1;
                }
                if start == self.i {
                    return Err(self.err(tr("expected a name", "se esperaba un nombre")));
                }
                Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            b'"' => self.basic_string().map(Value::Str),
            b'\'' => self.literal_string().map(Value::Str),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws_comments_newlines();
                    if self.peek() == b']' {
                        self.i += 1;
                        return Ok(Value::Arr(items));
                    }
                    items.push(self.value()?);
                    self.skip_ws_comments_newlines();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {}
                        _ => return Err(self.err(tr("expected ',' or ']'", "se esperaba ',' o ']'"))),
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut table = Value::obj();
                loop {
                    self.skip_ws();
                    if self.peek() == b'}' {
                        self.i += 1;
                        return Ok(table);
                    }
                    let path = self.key_path()?;
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    let v = self.value()?;
                    let target = table_at(&mut table, &path[..path.len() - 1])?;
                    target.set(path.last().map(String::as_str).unwrap_or(""), v);
                    self.skip_ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {}
                        _ => return Err(self.err(tr("expected ',' or '}'", "se esperaba ',' o '}'"))),
                    }
                }
            }
            _ => {
                let start = self.i;
                while !matches!(self.peek(), b',' | b']' | b'}' | b'\n' | b'\r' | b'#' | 0) {
                    self.i += 1;
                }
                let word = String::from_utf8_lossy(&self.s[start..self.i]).trim().to_string();
                self.i = start + word.len();
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    w => w.replace('_', "").parse::<f64>().map(Value::Num)
                        // Dates and times stay as text: Corral never needs them.
                        .or_else(|_| if w.is_empty() { Err(self.err(tr("missing value", "falta el valor"))) } else { Ok(Value::Str(w.to_string())) }),
                }
            }
        }
    }

    fn basic_string(&mut self) -> Result<String, String> {
        let multi = self.s[self.i..].starts_with(b"\"\"\"");
        self.i += if multi { 3 } else { 1 };
        if multi {
            self.skip_first_newline();
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            if self.eof() {
                return Err(self.err(tr("unclosed string", "texto sin cerrar")));
            }
            if multi && self.s[self.i..].starts_with(b"\"\"\"") {
                // Up to two quotes right before the closing ones belong to the text.
                let extra = self.closing_run(b'"');
                out.resize(out.len() + extra, b'"');
                self.i += 3 + extra;
                break;
            }
            let b = self.peek();
            self.i += 1;
            match b {
                b'"' if !multi => break,
                b'\n' if !multi => {
                    self.i -= 1; // the error is on this line, not the next
                    return Err(self.err(tr("unclosed string", "texto sin cerrar")));
                }
                b'\\' => {
                    let e = self.peek();
                    self.i += 1;
                    match e {
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'u' | b'U' => {
                            let n = if e == b'u' { 4 } else { 8 };
                            let hex = String::from_utf8_lossy(&self.s[self.i..(self.i + n).min(self.s.len())]).into_owned();
                            self.i += n;
                            let c = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}');
                            out.extend(c.to_string().as_bytes());
                        }
                        // A backslash ending the line (spaces and CRLF too) joins it to the next.
                        b' ' | b'\t' | b'\r' | b'\n' if multi => {
                            while matches!(self.peek(), b' ' | b'\t' | b'\n' | b'\r') {
                                self.i += 1;
                            }
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// A multi-line string's first newline (LF or CRLF) is trimmed.
    fn skip_first_newline(&mut self) {
        if self.s[self.i..].starts_with(b"\r\n") {
            self.i += 2;
        } else if self.peek() == b'\n' {
            self.i += 1;
        }
    }

    /// At a run of 3 to 5 quotes that closes a multi-line string: how many of
    /// them (0 to 2) are text before the closing three.
    fn closing_run(&self, quote: u8) -> usize {
        self.s[self.i..].iter().take(5).take_while(|&&b| b == quote).count().saturating_sub(3)
    }

    fn literal_string(&mut self) -> Result<String, String> {
        let multi = self.s[self.i..].starts_with(b"'''");
        self.i += if multi { 3 } else { 1 };
        if multi {
            self.skip_first_newline();
        }
        let start = self.i;
        loop {
            if self.eof() || (!multi && self.peek() == b'\n') {
                return Err(self.err(tr("unclosed string", "texto sin cerrar")));
            }
            if multi && self.s[self.i..].starts_with(b"'''") {
                let extra = self.closing_run(b'\'');
                let text = String::from_utf8_lossy(&self.s[start..self.i + extra]).into_owned();
                self.i += 3 + extra;
                return Ok(text);
            }
            if !multi && self.peek() == b'\'' {
                let text = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += 1;
                return Ok(text);
            }
            self.i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &Value) -> Vec<&str> {
        match v {
            Value::Obj(items) => items.iter().map(|(k, _)| k.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    /// A config.toml shaped like the ones Corral meets: comments everywhere,
    /// plain and dotted tables, quoted keys, arrays of tables, inline values.
    const CONFIG: &str = r##"# herdr settings
onboarding = false   # a comment after a value

[theme]
name = "catppuccin"

[theme.custom]
accent = "#ffc799"          # the '#' inside the string is not a comment
"panel.border" = '#2a2a2a'

[keys]
prefix = "ctrl+b"

[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "corral.toggle"

[[keys.command]]
key = "prefix+shift+s"
command = "corral.layout-code"
description = "Corral: code first"

[ui]
sidebar.width = 1_000
ratio = 0.516
negative = -3.5e-1
on = true
started = 2026-11-23T10:00:00Z
dirs = [
  "~/work",   # trailing comment
  'C:\Users\me',
]
inline = { a = 1, b.c = "x" }
"##;

    #[test]
    fn reads_a_herdr_config() {
        let v = parse(CONFIG).unwrap();
        assert_eq!(v.get("onboarding"), &Value::Bool(false));
        assert_eq!(v.get("theme").get("name").str(), Some("catppuccin"));
        assert_eq!(v.get("theme").get("custom").get("accent").str(), Some("#ffc799"));
        assert_eq!(v.get("theme").get("custom").get("panel.border").str(), Some("#2a2a2a"));
        assert_eq!(v.get("keys").get("prefix").str(), Some("ctrl+b"));
        let cmds = v.get("keys").get("command").arr();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].get("command").str(), Some("corral.toggle"));
        assert_eq!(keys(&cmds[1]), ["key", "command", "description"]);
        let ui = v.get("ui");
        assert_eq!(ui.get("sidebar").get("width").num(), Some(1000.0));
        assert_eq!(ui.get("ratio").num(), Some(0.516));
        assert_eq!(ui.get("negative").num(), Some(-0.35));
        assert_eq!(ui.get("on"), &Value::Bool(true));
        assert_eq!(ui.get("started").str(), Some("2026-11-23T10:00:00Z")); // dates stay text
        let dirs: Vec<&str> = ui.get("dirs").arr().iter().filter_map(|d| d.str()).collect();
        assert_eq!(dirs, ["~/work", r"C:\Users\me"]);
        assert_eq!(ui.get("inline").get("a").num(), Some(1.0));
        assert_eq!(ui.get("inline").get("b").get("c").str(), Some("x"));
    }

    #[test]
    fn crlf_reads_the_same() {
        let crlf = CONFIG.replace('\n', "\r\n");
        assert_eq!(parse(&crlf).unwrap(), parse(CONFIG).unwrap());
    }

    #[test]
    fn reads_corrals_own_keys_file() {
        let v = parse(include_str!("../../herdr/keys.example.toml")).unwrap();
        let cmds = v.get("keys").get("command").arr();
        assert!(cmds.len() >= 10, "only {} shortcuts", cmds.len());
        for c in cmds {
            assert!(c.get("key").str().is_some_and(|k| k.starts_with("prefix+")), "{}", c.to_json());
            assert_eq!(c.get("type").str(), Some("plugin_action"));
            assert!(c.get("command").str().is_some_and(|k| k.starts_with("corral.")), "{}", c.to_json());
        }
    }

    #[test]
    fn basic_string_escapes() {
        let v = parse(r#"s = "tab\t nl\n quote\" back\\ e\u00e9 smile\U0001F600 bs\b ff\f""#).unwrap();
        assert_eq!(v.get("s").str(), Some("tab\t nl\n quote\" back\\ e\u{e9} smile\u{1F600} bs\u{8} ff\u{c}"));
    }

    #[test]
    fn multi_line_strings() {
        let text = concat!(
            "a = \"\"\"\nfirst\nsecond\"\"\"\n",
            "b = \"\"\"one \\\n    two \\   \n  three\"\"\"\n",
            "c = \"\"\"\r\nwindows \\\r\n  joined\"\"\"\r\n",
            "d = \"\"\"say \"hi\"\"\"\"\n",
            "e = '''\nC:\\raw\\path\n'''\n",
            "f = '''it's'''\n",
            "g = '''quoted ''''\n",
        );
        let v = parse(text).unwrap();
        assert_eq!(v.get("a").str(), Some("first\nsecond"));
        assert_eq!(v.get("b").str(), Some("one two three"));
        assert_eq!(v.get("c").str(), Some("windows joined"));
        assert_eq!(v.get("d").str(), Some("say \"hi\""));
        assert_eq!(v.get("e").str(), Some("C:\\raw\\path\n"));
        assert_eq!(v.get("f").str(), Some("it's"));
        assert_eq!(v.get("g").str(), Some("quoted '"));
    }

    #[test]
    fn arrays_of_tables_with_sub_tables() {
        let v = parse("[[a]]\nx = 1\n[a.inner]\ny = 2\n[[a]]\nx = 3\n").unwrap();
        let a = v.get("a").arr();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].get("inner").get("y").num(), Some(2.0));
        assert_eq!(a[1].get("x").num(), Some(3.0));
        assert!(a[1].get("inner").is_null());
    }

    #[test]
    fn errors_say_the_line() {
        for (text, line) in [
            ("a = 1\nb = \"open\n", 2),
            ("a = 1\n\nb = 'open\n", 3),
            ("[table\n", 1),
            ("ok = 1\nkey\n", 2),
            ("a =\n", 1),
            ("a = [1, 2\n", 2),
            ("a = \"x\" extra\n", 1),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(&format!(" {line}:")), "{text:?}: {e}");
        }
        assert!(parse("command = 1\n[[command]]\n").is_err());
        assert!(parse("s = \"\"\"never closed\n").is_err());
    }

    #[test]
    fn empty_and_comment_only() {
        assert_eq!(parse("").unwrap(), Value::obj());
        assert_eq!(parse("# nothing\n\n   # here\r\n").unwrap(), Value::obj());
    }
}
