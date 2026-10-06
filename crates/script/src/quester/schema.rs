//! Generated JSON Schema for Path documents and typed handler arguments.

#[cfg(feature = "path-schema")]
pub type ArgsSchema = fn(&mut schemars::SchemaGenerator) -> schemars::Schema;

#[cfg(not(feature = "path-schema"))]
pub type ArgsSchema = ();

#[cfg(feature = "path-schema")]
fn args_schema_for<T: schemars::JsonSchema>(
    generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    generator.subschema_for::<T>()
}

#[cfg(feature = "path-schema")]
pub const fn args_schema<T: schemars::JsonSchema>() -> ArgsSchema {
    args_schema_for::<T>
}

#[cfg(not(feature = "path-schema"))]
pub const fn args_schema<T>() -> ArgsSchema {}

#[cfg(feature = "path-schema")]
const _: fn(super::path::StepDocument) = |super::path::StepDocument {
                                              id: _,
                                              kind: _,
                                              version: _,
                                              args: _,
                                              comment: _,
                                              advances: _,
                                              skip_if: _,
                                              settle: _,
                                          }| {};

#[cfg(feature = "path-schema")]
const _: fn(&super::path::PredicateDocument) = |predicate| match predicate {
    super::path::PredicateDocument::All(_) => (),
    super::path::PredicateDocument::Any(_) => (),
    super::path::PredicateDocument::Not(_) => (),
    super::path::PredicateDocument::Fact {
        kind,
        version,
        args,
    } => {
        let _ = (kind, version, args);
    }
};

#[cfg(feature = "path-schema")]
impl schemars::JsonSchema for super::path::StepDocument {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "StepDocument".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use super::compile::{step_handlers, AdvanceClass};

        let mut branches = Vec::new();
        for handler in step_handlers() {
            let name = format!("step.{}.v{}", handler.kind, handler.version);
            let mut required = vec!["id", "kind", "version", "args", "skip_if", "settle"];
            if handler.advance == AdvanceClass::Explicit {
                required.push("advances");
            }
            let branch = schemars::json_schema!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "kind": { "const": handler.kind },
                    "version": { "const": handler.version },
                    "args": (handler.args_schema)(generator),
                    "comment": generator.subschema_for::<Option<String>>(),
                    "advances": { "type": "boolean" },
                    "skip_if": generator.subschema_for::<super::path::PredicateDocument>(),
                    "settle": generator.subschema_for::<super::path::PredicateDocument>()
                },
                "required": required,
                "additionalProperties": false
            });
            generator
                .definitions_mut()
                .insert(name.clone(), branch.to_value());
            branches.push(schemars::json_schema!({
                "$ref": format!("#/$defs/{name}")
            }));
        }
        schemars::json_schema!({ "oneOf": branches })
    }
}

#[cfg(feature = "path-schema")]
impl schemars::JsonSchema for super::path::PredicateDocument {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "PredicateDocument".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut fact_branches = Vec::new();
        for handler in super::compile::predicate_handlers() {
            let name = format!("fact.{}.v{}", handler.kind, handler.version);
            let branch = schemars::json_schema!({
                "type": "object",
                "properties": {
                    "kind": { "const": handler.kind },
                    "version": { "const": handler.version },
                    "args": (handler.args_schema)(generator)
                },
                "required": ["kind", "version", "args"],
                "additionalProperties": false
            });
            generator
                .definitions_mut()
                .insert(name.clone(), branch.to_value());
            fact_branches.push(schemars::json_schema!({
                "$ref": format!("#/$defs/{name}")
            }));
        }

        let predicate = generator.subschema_for::<super::path::PredicateDocument>();
        schemars::json_schema!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "All": { "type": "array", "items": predicate.clone() }
                    },
                    "required": ["All"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "Any": { "type": "array", "items": predicate.clone() }
                    },
                    "required": ["Any"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "Not": predicate.clone()
                    },
                    "required": ["Not"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "Fact": { "oneOf": fact_branches }
                    },
                    "required": ["Fact"],
                    "additionalProperties": false
                }
            ]
        })
    }
}

// Schemars emits Rust-specific numeric formats (for example, "uint8" and "float")
// that strict JSON Schema validators need not recognize. The numeric type and bounds suffice.
#[cfg(feature = "path-schema")]
fn strip_numeric_formats(schema: &mut serde_json::Value) {
    use serde_json::Value;

    match schema {
        Value::Array(items) => {
            for item in items {
                strip_numeric_formats(item);
            }
        }
        Value::Object(object) => {
            let numeric = match object.get("type") {
                Some(Value::String(kind)) => matches!(kind.as_str(), "integer" | "number"),
                Some(Value::Array(kinds)) => kinds.iter().any(|kind| {
                    kind.as_str()
                        .is_some_and(|kind| matches!(kind, "integer" | "number"))
                }),
                _ => false,
            };
            if numeric {
                object.remove("format");
            }
            for value in object.values_mut() {
                strip_numeric_formats(value);
            }
        }
        _ => {}
    }
}

#[cfg(feature = "path-schema")]
pub fn render() -> String {
    let mut schema = schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<super::path::PathDocument>()
        .to_value();
    strip_numeric_formats(&mut schema);
    if let Some(root) = schema.as_object_mut() {
        root.insert(
            "title".to_owned(),
            serde_json::Value::String("274bot Quester Path (schema 3)".to_owned()),
        );
    }
    let mut rendered = serde_json::to_string_pretty(&schema)
        .expect("serializing a generated Path schema cannot fail");
    rendered.push('\n');
    rendered
}

#[cfg(feature = "path-schema")]
pub fn path_schema_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("paths")
        .join("path.schema.json")
}
