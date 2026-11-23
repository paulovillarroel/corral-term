//! A small JSON value with a parser and a writer: enough for herdr's socket
//! and Corral's state files, without pulling in a dependency.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    /// Keys keep their order, so files read and written back look the same.
    Obj(Vec<(String, Value)>),
}

pub static NULL: Value = Value::Null;

impl Value {
    pub fn obj() -> Value {
        Value::Obj(Vec::new())
    }

    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    /// Set a key on an object (turns anything else into an object first).
    pub fn set(&mut self, key: &str, value: Value) {
        if !matches!(self, Value::Obj(_)) {
            *self = Value::obj();
        }
        if let Value::Obj(items) = self {
            match items.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => items.push((key.to_string(), value)),
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Value::Obj(items) = self {
            items.retain(|(k, _)| k != key);
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn arr(&self) -> &[Value] {
        match self {
            Value::Arr(items) => items,
            _ => &[],
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out);
        out
    }

    /// Indented, like Python's `json.dumps(indent=...)`, for files people read.
    pub fn to_json_pretty(&self, indent: usize) -> String {
        let mut out = String::new();
        write_pretty(self, &mut out, indent, 0);
        out.push('\n');
        out
    }
}

fn write_pretty(v: &Value, out: &mut String, indent: usize, level: usize) {
    let pad = |n: usize| " ".repeat(indent * n);
    match v {
        Value::Arr(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(level + 1));
                write_pretty(item, out, indent, level + 1);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(level));
            out.push(']');
        }
        Value::Obj(items) if !items.is_empty() => {
            out.push_str("{\n");
            for (i, (k, item)) in items.iter().enumerate() {
                out.push_str(&pad(level + 1));
                write_str(k, out);
                out.push_str(": ");
                write_pretty(item, out, indent, level + 1);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(level));
            out.push('}');
        }
        other => write_value(other, out),
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}

/// Build an object from pairs: `obj(&[("pane_id", pane.into())])`.
pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                let _ = write!(out, "{}", *n as i64);
            } else if n.is_finite() {
                let _ = write!(out, "{}", n);
            } else {
                out.push_str("null");
            }
        }
        Value::Str(s) => write_str(s, out),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Obj(items) => {
            out.push('{');
            for (i, (k, item)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// Parse errors are in English only: the language setting is read from
// config.json through this parser, so asking for the language here (tr/trf!)
// would re-enter i18n::es() while it reads the setting, and hang on its lock
// when config.json is broken.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("JSON: extra text at position {}", p.i));
    }
    Ok(v)
}

/// How deep arrays and objects may nest. Parsing recurses, so without a limit
/// a long run of `[` would overflow the stack and abort the process.
const MAX_DEPTH: usize = 512;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at position {}", self.i))
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(&b) if b == b'{' || b == b'[' => {
                if self.depth == MAX_DEPTH {
                    return self.err("nested too deep");
                }
                self.depth += 1;
                let v = if b == b'{' { self.object() } else { self.array() };
                self.depth -= 1;
                v
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(_) => self.number(),
        }
    }

    fn literal(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("unknown word")
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        text.parse::<f64>().map(Value::Num).or_else(|_| self.err("invalid number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.i..self.i + 4).ok_or_else(|| "JSON: incomplete \\u".to_string())?;
        let text = std::str::from_utf8(digits).map_err(|e| e.to_string())?;
        self.i += 4;
        u32::from_str_radix(text, 16).map_err(|_| "JSON: invalid \\u".to_string())
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?);
            match self.s.get(self.i) {
                None => return self.err("unclosed string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(_) => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or_else(|| "JSON: incomplete escape".to_string())?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.s[self.i..].starts_with(b"\\u") {
                                let after_high = self.i;
                                self.i += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                } else {
                                    self.i = after_high; // not a pair: that \u is read on its own
                                }
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return self.err("unknown escape"),
                    }
                }
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return self.err("missing , or ]"),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Obj(items));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return self.err("expected a key");
            }
            let key = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return self.err("missing :");
            }
            self.i += 1;
            let v = self.value()?;
            items.push((key, v));
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Obj(items));
                }
                _ => return self.err("missing , or }"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = r#"{"a":[1,2.5,-3,true,null],"b":"x\"y\\z\ná😀","c":{}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.get("b").str().unwrap(), "x\"y\\z\ná😀");
        assert_eq!(v.get("a").arr()[1].num(), Some(2.5));
        let again = parse(&v.to_json()).unwrap();
        assert_eq!(v, again);
    }

    #[test]
    fn numbers_and_set() {
        let mut v = parse(r#"{"ratio":0.57,"n":3}"#).unwrap();
        v.set("n", 4.0.into());
        v.set("new", "s".into());
        v.remove("ratio");
        assert_eq!(v.to_json(), r#"{"n":4,"new":"s"}"#);
    }

    #[test]
    fn unicode_escapes() {
        let s = |text: &str| parse(text).unwrap().str().unwrap().to_string();
        assert_eq!(s(r#""éé""#), "éé");
        assert_eq!(s(r#""😀""#), "😀");
        // Halves of a pair on their own: U+FFFD, and the next escape still counts.
        assert_eq!(s(r#""\ud83dx""#), "\u{FFFD}x");
        assert_eq!(s(r#""\ud83dA""#), "\u{FFFD}A");
        assert_eq!(s(r#""\ude00""#), "\u{FFFD}");
        assert_eq!(s(r#""\/\b\f""#), "/\u{8}\u{c}");
        assert!(parse(r#""\u12""#).is_err());
        assert!(parse(r#""\x41""#).is_err());
    }

    #[test]
    fn control_characters_round_trip() {
        let all: String = (0u32..0x20).chain([0x7f, 0x2028, 0x1F600]).filter_map(char::from_u32).collect();
        let text = Value::Str(all.clone()).to_json();
        assert!(!text.chars().any(|c| (c as u32) < 0x20), "{text:?}");
        assert_eq!(parse(&text).unwrap().str(), Some(all.as_str()));
    }

    #[test]
    fn numbers() {
        for (text, n) in [("0", 0.0), ("-12", -12.0), ("-0.5", -0.5), ("1e3", 1000.0), ("2.5E-2", 0.025),
                          ("1.5e+2", 150.0), ("123456789012", 123456789012.0)] {
            assert_eq!(parse(text).unwrap().num(), Some(n), "{text}");
        }
        for (n, text) in [(3.0, "3"), (-0.0, "0"), (0.1, "0.1"), (-2.5, "-2.5"), (1e15, "1000000000000000"),
                          (f64::NAN, "null"), (f64::INFINITY, "null")] {
            assert_eq!(Value::Num(n).to_json(), text);
        }
        for n in [1e20, 1e-7, 123.456, -9.75e-300, f64::MAX] {
            assert_eq!(parse(&Value::Num(n).to_json()).unwrap().num(), Some(n), "{n}");
        }
        for bad in ["-", "--1", "1e", "+", "."] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn invalid_input_is_an_error() {
        for bad in ["", " ", "{", "[", "[1,]", "[1 2]", "{\"a\" 1}", "{a:1}", "{\"a\":1,}", "\"open", "tru", "nul",
                    "1 2", "{}x", "[\"a\",]", "{\"a\":}", "}", "\"\\"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse(" \n\t{ \"a\" : [ 1 , 2 ] } \r\n").unwrap().get("a").arr().len(), 2);
    }

    #[test]
    fn nesting_has_a_limit_instead_of_a_crash() {
        assert!(parse(&"[".repeat(100_000)).unwrap_err().contains("too deep"));
        assert!(parse(&"{\"a\":".repeat(100_000)).is_err());
        let fine = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&fine).is_ok());
        let one_more = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(parse(&one_more).is_err());
        // The limit is on depth, not on how many there are.
        let wide = format!("[{}]", vec!["[[1]]"; 10_000].join(","));
        assert_eq!(parse(&wide).unwrap().arr().len(), 10_000);
    }

    #[test]
    fn pretty_output() {
        let v = parse(r#"{"a":[1,{"b":[]}],"c":{},"d":"x"}"#).unwrap();
        let pretty = v.to_json_pretty(2);
        assert_eq!(pretty, "{\n  \"a\": [\n    1,\n    {\n      \"b\": []\n    }\n  ],\n  \"c\": {},\n  \"d\": \"x\"\n}\n");
        assert_eq!(parse(&pretty).unwrap(), v);
    }

    #[test]
    fn get_set_remove_on_any_value() {
        let mut v = Value::Null;
        assert!(v.get("x").is_null());
        v.set("a", 1.0.into()); // anything else becomes an object first
        v.set("b", true.into());
        v.set("a", "again".into()); // keeps its place
        assert_eq!(v.to_json(), r#"{"a":"again","b":true}"#);
        v.remove("missing");
        v.remove("a");
        assert_eq!(v.to_json(), r#"{"b":true}"#);
        let arr = Value::Arr(vec![1.0.into()]);
        assert!(arr.get("0").is_null());
        assert!(Value::Str("s".into()).arr().is_empty());
        assert_eq!(obj(vec![("k", Value::Null)]).to_json(), r#"{"k":null}"#);
    }
}
