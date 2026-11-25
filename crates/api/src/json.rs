//! JSON helpers for saved documents that must not choose between repeated fields.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;

use serde::de::{self, DeserializeSeed};
use serde_json::{map::Entry, Map, Value};

/// One field repeated in a complete JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateJsonField {
    /// The object the refusal names, such as `the top level` or `the object "nav[0]"`.
    pub object: String,
    /// The field name repeated in that object.
    pub field: String,
}

impl fmt::Display for DuplicateJsonField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "field {:?} is named twice in {}; edit the file to remove one of them",
            self.field, self.object
        )
    }
}

/// Parses complete JSON and retains the first repeated object field, if any.
///
/// JSON errors take precedence over duplicates. The duplicate is recorded
/// while parsing continues because a deserializer can return only its own
/// error. The returned `Value` uses the last value for a repeated field, so
/// callers must refuse the document when the second result is `Some`.
pub fn value_and_duplicate_fields(
    raw: &str,
) -> Result<(Value, Option<DuplicateJsonField>), serde_json::Error> {
    let found = RefCell::new(None);
    let mut parser = serde_json::Deserializer::from_str(raw);
    let document = NoDuplicateFields {
        object: None,
        found: &found,
    }
    .deserialize(&mut parser)?;
    parser.end()?;
    Ok((document, found.into_inner()))
}

/// One segment of a value's position in the document: a reference to the
/// member's field name as the parsing frame holds it (no copy of the name is
/// made for the path), or an array element's index.
enum Segment<'a, 'de> {
    Field(&'a Cow<'de, str>),
    Index(usize),
}

/// Where a value sits in the document. The chain lives on the parsing
/// frames' stacks, so a deep document costs no path text while it parses; a
/// refusal builds the text once, for its message.
struct Position<'a, 'de> {
    parent: Option<&'a Position<'a, 'de>>,
    segment: Segment<'a, 'de>,
}

impl Position<'_, '_> {
    fn path(&self) -> String {
        let mut frames = Vec::new();
        let mut cursor = Some(self);
        while let Some(position) = cursor {
            frames.push(position);
            cursor = position.parent;
        }
        let mut path = String::new();
        for frame in frames.iter().rev() {
            match &frame.segment {
                Segment::Field(name) => {
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(name);
                }
                Segment::Index(index) => {
                    path.push('[');
                    path.push_str(&index.to_string());
                    path.push(']');
                }
            }
        }
        path
    }
}

fn object_phrase(position: Option<&Position>) -> String {
    match position {
        None => "the top level".to_owned(),
        Some(position) => format!("the object {:?}", position.path()),
    }
}

struct NoDuplicateFields<'a, 'de> {
    object: Option<&'a Position<'a, 'de>>,
    found: &'a RefCell<Option<DuplicateJsonField>>,
}

impl<'de> DeserializeSeed<'de> for NoDuplicateFields<'_, 'de> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> de::Visitor<'de> for NoDuplicateFields<'_, 'de> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E>
    where
        E: de::Error,
    {
        // JSON text cannot contain a non-finite float; match serde_json's fallback.
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_unit<E>(self) -> Result<Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        let mut elements = Vec::new();
        loop {
            let position = Position {
                parent: self.object,
                segment: Segment::Index(elements.len()),
            };
            let element = match access.next_element_seed(NoDuplicateFields {
                object: Some(&position),
                found: self.found,
            })? {
                Some(element) => element,
                None => break,
            };
            elements.push(element);
        }
        Ok(Value::Array(elements))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut fields = Map::new();
        while let Some(field) = access.next_key::<Cow<'de, str>>()? {
            let value = access.next_value_seed(NoDuplicateFields {
                object: Some(&Position {
                    parent: self.object,
                    segment: Segment::Field(&field),
                }),
                found: self.found,
            })?;
            match fields.entry(field.into_owned()) {
                Entry::Occupied(mut entry) => {
                    if self.found.borrow().is_none() {
                        *self.found.borrow_mut() = Some(DuplicateJsonField {
                            object: object_phrase(self.object),
                            field: entry.key().clone(),
                        });
                    }
                    entry.insert(value);
                }
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }
        Ok(Value::Object(fields))
    }
}
