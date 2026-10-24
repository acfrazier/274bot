use crate::interact::InteractReq;

#[cfg(feature = "load")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum MaybeInteractReq {
    Req(InteractReq),
    Skip(RejectedRow),
}

/// A queued interact row no [`InteractReq`] variant accepts, named by its
/// `op` so the tick loop can log what it refused instead of dropping it
/// silently. Accepts every value kind, as the `IgnoredAny` it replaced did.
#[cfg(feature = "load")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RejectedRow(pub(crate) String);

#[cfg(feature = "load")]
impl<'de> serde::Deserialize<'de> for RejectedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RejectedRow(match Shape::deserialize(d)? {
            Shape::Str(_) => "a string".into(),
            Shape::Object(Some(op)) => format!("op {op:?}"),
            Shape::Object(None) => "an object without a string op".into(),
            Shape::Other(kind) => kind.into(),
        }))
    }
}

/// One queued row, read on its own: a row serde cannot read at all (a
/// BigInt, say) is refused like any other malformed row instead of failing
/// every row beside it.
#[cfg(feature = "load")]
pub(crate) struct QueuedRow(pub(crate) MaybeInteractReq);

#[cfg(feature = "load")]
impl<'de> serde::Deserialize<'de> for QueuedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(QueuedRow(MaybeInteractReq::deserialize(d).unwrap_or_else(
            |e| MaybeInteractReq::Skip(RejectedRow(format!("unreadable ({e})"))),
        )))
    }
}

/// Any value, as much of it as a log line names: a string's text, an
/// object's string `op`, or the kind of anything else.
#[cfg(feature = "load")]
enum Shape {
    Str(String),
    Object(Option<String>),
    Other(&'static str),
}

#[cfg(feature = "load")]
impl<'de> serde::Deserialize<'de> for Shape {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Shape;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "any JS value")
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Shape, E> {
                Ok(Shape::Other("a boolean"))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Shape, E> {
                Ok(Shape::Str(s.to_string()))
            }
            fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<Shape, E> {
                Ok(Shape::Other("bytes"))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Shape, E> {
                Ok(Shape::Other("null"))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Shape, E> {
                Ok(Shape::Other("null"))
            }
            fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Shape, D::Error> {
                <Shape as serde::Deserialize>::deserialize(d)
            }
            fn visit_newtype_struct<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Shape, D::Error> {
                <Shape as serde::Deserialize>::deserialize(d)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Shape, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Shape::Other("an array"))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Shape, A::Error> {
                let mut op = None;
                while let Some(key) = map.next_key::<Shape>()? {
                    match key {
                        Shape::Str(key) if key == "op" => {
                            op = match map.next_value::<Shape>()? {
                                Shape::Str(op) => Some(op),
                                _ => None,
                            };
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(Shape::Object(op))
            }
        }
        d.deserialize_any(V)
    }
}
