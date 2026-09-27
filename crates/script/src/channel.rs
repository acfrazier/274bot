//! Bounded structured-clone subset used by the in-Play BroadcastChannel broker.
//!
//! Values cross the isolate boundary as bytes in the existing FlatBuffer wire.
//! The codec deliberately accepts only the JSON-shaped values JiveKQ sends;
//! functions, typed arrays, non-finite numbers and oversized graphs fail closed.

use serde_json::{Map, Number, Value};

pub const MAX_CHANNEL_BYTES: usize = 16 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_NODES: usize = 512;
const MAX_STRING_BYTES: usize = 1024;
const MAX_COLLECTION: usize = 128;

const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const NUMBER: u8 = 3;
const STRING: u8 = 4;
const ARRAY: u8 = 5;
const OBJECT: u8 = 6;

pub fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(256);
    let mut nodes = 0;
    encode_value(value, 0, &mut nodes, &mut out)?;
    if out.len() > MAX_CHANNEL_BYTES {
        return Err("BroadcastChannel message exceeds 16384 bytes".into());
    }
    Ok(out)
}

fn encode_value(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("BroadcastChannel message nesting exceeds 8".into());
    }
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err("BroadcastChannel message has too many values".into());
    }
    match value {
        Value::Null => out.push(NULL),
        Value::Bool(false) => out.push(FALSE),
        Value::Bool(true) => out.push(TRUE),
        Value::Number(number) => {
            let number = number
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or("BroadcastChannel number is not finite")?;
            out.push(NUMBER);
            out.extend_from_slice(&number.to_le_bytes());
        }
        Value::String(text) => {
            out.push(STRING);
            put_text(text, out)?;
        }
        Value::Array(values) => {
            if values.len() > MAX_COLLECTION {
                return Err("BroadcastChannel array exceeds 128 entries".into());
            }
            out.push(ARRAY);
            put_len(values.len(), out);
            for value in values {
                encode_value(value, depth + 1, nodes, out)?;
            }
        }
        Value::Object(fields) => {
            if fields.len() > MAX_COLLECTION {
                return Err("BroadcastChannel object exceeds 128 fields".into());
            }
            out.push(OBJECT);
            put_len(fields.len(), out);
            for (key, value) in fields {
                put_text(key, out)?;
                encode_value(value, depth + 1, nodes, out)?;
            }
        }
    }
    if out.len() > MAX_CHANNEL_BYTES {
        return Err("BroadcastChannel message exceeds 16384 bytes".into());
    }
    Ok(())
}

fn put_len(len: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&(len as u16).to_le_bytes());
}

fn put_text(text: &str, out: &mut Vec<u8>) -> Result<(), String> {
    if text.len() > MAX_STRING_BYTES {
        return Err("BroadcastChannel string exceeds 1024 bytes".into());
    }
    put_len(text.len(), out);
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_CHANNEL_BYTES {
        return Err("BroadcastChannel message exceeds 16384 bytes".into());
    }
    let mut cursor = Cursor { bytes, pos: 0 };
    let mut nodes = 0;
    let value = cursor.value(0, &mut nodes)?;
    if cursor.pos != bytes.len() {
        return Err("BroadcastChannel message has trailing bytes".into());
    }
    Ok(value)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn value(&mut self, depth: usize, nodes: &mut usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err("BroadcastChannel message nesting exceeds 8".into());
        }
        *nodes += 1;
        if *nodes > MAX_NODES {
            return Err("BroadcastChannel message has too many values".into());
        }
        match self.byte()? {
            NULL => Ok(Value::Null),
            FALSE => Ok(Value::Bool(false)),
            TRUE => Ok(Value::Bool(true)),
            NUMBER => {
                let bytes: [u8; 8] = self.take(8)?.try_into().expect("bounded slice");
                let number = f64::from_le_bytes(bytes);
                if !number.is_finite() {
                    return Err("BroadcastChannel number is not finite".into());
                }
                Number::from_f64(number)
                    .map(Value::Number)
                    .ok_or_else(|| "BroadcastChannel number is invalid".into())
            }
            STRING => Ok(Value::String(self.text()?)),
            ARRAY => {
                let len = self.len()?;
                if len > MAX_COLLECTION {
                    return Err("BroadcastChannel array exceeds 128 entries".into());
                }
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.value(depth + 1, nodes)?);
                }
                Ok(Value::Array(values))
            }
            OBJECT => {
                let len = self.len()?;
                if len > MAX_COLLECTION {
                    return Err("BroadcastChannel object exceeds 128 fields".into());
                }
                let mut fields = Map::new();
                for _ in 0..len {
                    let key = self.text()?;
                    if fields.contains_key(&key) {
                        return Err("BroadcastChannel object has duplicate field".into());
                    }
                    fields.insert(key, self.value(depth + 1, nodes)?);
                }
                Ok(Value::Object(fields))
            }
            _ => Err("BroadcastChannel message has unknown value tag".into()),
        }
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(*self.take(1)?.first().expect("one-byte slice"))
    }

    fn len(&mut self) -> Result<usize, String> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("bounded slice");
        Ok(u16::from_le_bytes(bytes) as usize)
    }

    fn text(&mut self) -> Result<String, String> {
        let len = self.len()?;
        if len > MAX_STRING_BYTES {
            return Err("BroadcastChannel string exceeds 1024 bytes".into());
        }
        std::str::from_utf8(self.take(len)?)
            .map(str::to_owned)
            .map_err(|_| "BroadcastChannel string is not UTF-8".into())
    }

    fn take(&mut self, len: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("BroadcastChannel message is truncated")?;
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jive_member_and_release_round_trip() {
        for value in [
            json!({"member":{"name":"a","session":"uuid","trip":1.0,"stage":"fight","tile":{"x":3500.0,"z":9500.0,"level":0.0},"ready":true,"stats":{"hp":70.0,"prayer":37.0,"food":12.0,"damage":2.5,"dps":0.0,"kills":0.0}}}),
            json!({"release":{"stage":"upper","trip":2.0,"sessions":["a","b","c","d"],"at":1234.0},"sender":"leader"}),
        ] {
            let bytes = encode(&value).expect("encode");
            assert_eq!(decode(&bytes).expect("decode"), value);
        }
    }

    #[test]
    fn limits_fail_closed() {
        assert!(encode(&serde_json::Value::Array(vec![
            serde_json::Value::Null;
            129
        ]))
        .is_err());
        assert!(decode(&[ARRAY, 1]).is_err());
        assert!(encode(&json!("x".repeat(1025))).is_err());
    }
}
