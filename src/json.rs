//! Small JSON reader/writer for Lua plugins (kwick.json_decode/encode),
//! converting straight between JSON text and Lua values.

use mlua::{Lua, Table, Value};

pub fn decode(lua: &Lua, text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        lua,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("余分な文字があります"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    lua: &'a Lua,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        format!("JSON: {msg} (位置 {})", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 200 {
            return Err(self.err("入れ子が深すぎます"));
        }
        let lua_err = |e: mlua::Error| e.to_string();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let t = self.lua.create_table().map_err(lua_err)?;
                self.ws();
                if self.eat("}") {
                    return Ok(Value::Table(t));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("キーが必要です"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err(self.err("':' が必要です"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    t.set(k, v).map_err(lua_err)?;
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Value::Table(t));
                    }
                    return Err(self.err("',' か '}' が必要です"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let t = self.lua.create_table().map_err(lua_err)?;
                self.ws();
                if self.eat("]") {
                    return Ok(Value::Table(t));
                }
                let mut n = 1;
                loop {
                    self.ws();
                    let v = self.value(depth + 1)?;
                    t.raw_set(n, v).map_err(lua_err)?;
                    n += 1;
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Value::Table(t));
                    }
                    return Err(self.err("',' か ']' が必要です"));
                }
            }
            Some(b'"') => {
                let s = self.string()?;
                Ok(Value::String(self.lua.create_string(&s).map_err(lua_err)?))
            }
            Some(b't') if self.eat("true") => Ok(Value::Boolean(true)),
            Some(b'f') if self.eat("false") => Ok(Value::Boolean(false)),
            Some(b'n') if self.eat("null") => Ok(Value::Nil),
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                if let Ok(n) = text.parse::<i64>() {
                    return Ok(Value::Integer(n));
                }
                text.parse::<f64>()
                    .map(Value::Number)
                    .map_err(|_| self.err("数値が不正です"))
            }
            _ => Err(self.err("値が必要です")),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return Err(self.err("文字列が閉じていません"));
            };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("UTF-8 ではありません")),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return Err(self.err("エスケープが途中です"));
                    };
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.eat("\\u") {
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("不明なエスケープです")),
                    }
                }
                _ => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| self.err("\\u の後に 16 進 4 桁が必要です"))?;
        self.i += 4;
        Ok(h)
    }
}

pub fn encode(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    write(value, &mut out, 0)?;
    Ok(out)
}

fn write(value: &Value, out: &mut String, depth: usize) -> Result<(), String> {
    if depth > 200 {
        return Err("JSON: 入れ子が深すぎます (循環参照?)".into());
    }
    match value {
        Value::Nil => out.push_str("null"),
        Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Integer(n) => out.push_str(&n.to_string()),
        Value::Number(n) if n.is_finite() => out.push_str(&n.to_string()),
        Value::Number(_) => out.push_str("null"),
        Value::String(s) => write_str(&s.to_str().map_err(|e| e.to_string())?, out),
        Value::Table(t) => write_table(t, out, depth)?,
        other => return Err(format!("JSON にできない値です: {}", other.type_name())),
    }
    Ok(())
}

fn write_table(t: &Table, out: &mut String, depth: usize) -> Result<(), String> {
    let len = t.raw_len();
    let pairs: Vec<(Value, Value)> = t.clone().pairs().flatten().collect();
    // A sequence (1..n, nothing else) is an array; an empty table is {}.
    if len > 0 && pairs.len() == len {
        out.push('[');
        for i in 1..=len {
            if i > 1 {
                out.push(',');
            }
            let v: Value = t.raw_get(i).map_err(|e| e.to_string())?;
            write(&v, out, depth + 1)?;
        }
        out.push(']');
        return Ok(());
    }
    out.push('{');
    for (n, (k, v)) in pairs.iter().enumerate() {
        if n > 0 {
            out.push(',');
        }
        let key = match k {
            Value::String(s) => s.to_str().map_err(|e| e.to_string())?.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(f) => f.to_string(),
            _ => return Err("JSON のキーは文字列か数値にしてください".into()),
        };
        write_str(&key, out);
        out.push(':');
        write(v, out, depth + 1)?;
    }
    out.push('}');
    Ok(())
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
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let lua = Lua::new();
        let v = decode(
            &lua,
            r#" {"a": [1, 2.5, "x\né😀"], "b": {"c": true, "d": null}, "e": -3} "#,
        )
        .unwrap();
        let t = match &v {
            Value::Table(t) => t.clone(),
            _ => panic!(),
        };
        let a: Table = t.get("a").unwrap();
        assert_eq!(a.get::<i64>(1).unwrap(), 1);
        assert_eq!(a.get::<f64>(2).unwrap(), 2.5);
        assert_eq!(a.get::<String>(3).unwrap(), "x\né😀");
        assert_eq!(t.get::<i64>("e").unwrap(), -3);

        let back = encode(&Value::Table(a)).unwrap();
        assert_eq!(back, r#"[1,2.5,"x\né😀"]"#);
        assert!(decode(&lua, "{\"a\":}").is_err());
        assert!(decode(&lua, "[1,2] x").is_err());
        let empty = lua.create_table().unwrap();
        assert_eq!(encode(&Value::Table(empty)).unwrap(), "{}");
    }
}
