//! A small TOML reader for herdr's config.toml: tables, arrays of tables,
//! dotted and quoted keys, strings (basic and literal, multi-line too),
//! numbers, booleans, and arrays and inline tables of those. It reads into the
//! same Value as the JSON module. Writing goes through herdr's own validator
//! (`herdr config check`), so Corral never has to judge a file it changed.

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
                    _ => return Err(format!("{last} no es una lista de tablas")),
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
    let Value::Obj(items) = node else { return Err(format!("{key}: no es una tabla")) };
    let slot = items.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v).ok_or_else(|| format!("falta la tabla {key}"))?;
    match slot {
        Value::Arr(a) => a.last_mut().ok_or_else(|| format!("{key}: lista vacía")),
        Value::Obj(_) => Ok(slot),
        _ => Err(format!("{key}: no es una tabla")),
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
        format!("TOML, línea {line}: {what}")
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == b {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("se esperaba '{}'", b as char)))
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
            _ => Err(self.err("sobra texto al final de la línea")),
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
                    return Err(self.err("se esperaba un nombre"));
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
                        _ => return Err(self.err("se esperaba ',' o ']'")),
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
                        _ => return Err(self.err("se esperaba ',' o '}'")),
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
                        .or_else(|_| if w.is_empty() { Err(self.err("falta el valor")) } else { Ok(Value::Str(w.to_string())) }),
                }
            }
        }
    }

    fn basic_string(&mut self) -> Result<String, String> {
        let multi = self.s[self.i..].starts_with(b"\"\"\"");
        self.i += if multi { 3 } else { 1 };
        if multi && self.peek() == b'\n' {
            self.i += 1;
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            if self.eof() {
                return Err(self.err("texto sin cerrar"));
            }
            if multi && self.s[self.i..].starts_with(b"\"\"\"") {
                self.i += 3;
                break;
            }
            let b = self.peek();
            self.i += 1;
            match b {
                b'"' if !multi => break,
                b'\n' if !multi => return Err(self.err("texto sin cerrar")),
                b'\\' => {
                    let e = self.peek();
                    self.i += 1;
                    match e {
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
                        b'\n' if multi => {
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

    fn literal_string(&mut self) -> Result<String, String> {
        let multi = self.s[self.i..].starts_with(b"'''");
        self.i += if multi { 3 } else { 1 };
        if multi && self.peek() == b'\n' {
            self.i += 1;
        }
        let start = self.i;
        loop {
            if self.eof() || (!multi && self.peek() == b'\n') {
                return Err(self.err("texto sin cerrar"));
            }
            if multi && self.s[self.i..].starts_with(b"'''") {
                let text = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += 3;
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
