//! The checked-in Luau declarations and the reference page against the runtime.
//!
//! `docs/api/luau-api-v2.d.luau` and `docs/api/luau-api-v2.md` are written by
//! hand. These tests read them as text and compare them with what the runtime
//! registers (`script_luau::api_members`) and with what the manifest reader
//! accepts, so a member, a signature, a call class or a manifest field cannot
//! change in one place only. They need no game data.

use std::collections::{BTreeMap, BTreeSet};

use script_luau::{api_members, read_manifest, Kind};

const DECLARATIONS: &str = include_str!("../../../docs/api/luau-api-v2.d.luau");
const REFERENCE: &str = include_str!("../../../docs/api/luau-api-v2.md");

/// Member path to signature, read from the `declare api: { ... }` block. A
/// nested table is a namespace. Every member sits on one line, and `---`
/// lines are documentation.
fn declared_members(text: &str) -> BTreeMap<String, String> {
    let mut members = BTreeMap::new();
    let mut namespaces: Vec<&str> = Vec::new();
    let mut inside = false;
    let mut closed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !inside {
            inside = trimmed == "declare api: {";
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        if trimmed == "}" {
            assert!(namespaces.is_empty(), "unbalanced `declare api` block");
            closed = true;
            break;
        }
        if trimmed == "}," {
            namespaces
                .pop()
                .expect("a closing brace has an opening one");
            continue;
        }
        if let Some(name) = trimmed.strip_suffix(": {") {
            namespaces.push(name);
            continue;
        }
        let (name, signature) = trimmed
            .split_once(": ")
            .unwrap_or_else(|| panic!("not a member line: {trimmed}"));
        let signature = signature
            .strip_suffix(',')
            .unwrap_or_else(|| panic!("a member line ends with a comma: {trimmed}"));
        let path = namespaces
            .iter()
            .copied()
            .chain([name])
            .collect::<Vec<_>>()
            .join(".");
        let first = members.insert(path.clone(), signature.to_owned());
        assert!(first.is_none(), "{path} is declared twice");
    }
    assert!(
        inside && closed,
        "the file has one `declare api: {{ ... }}` block"
    );
    members
}

/// The names of the `export type NAME = ...` declarations.
fn declared_types(text: &str) -> BTreeSet<&str> {
    text.lines()
        .filter_map(|line| line.strip_prefix("export type "))
        .filter_map(|rest| rest.split([' ', '<', '=']).next())
        .collect()
}

/// The lines of `export type NAME = {` up to its closing `}`.
fn type_body<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let opening = format!("export type {name} = {{");
    let mut lines = text.lines().skip_while(|line| *line != opening);
    assert!(lines.next().is_some(), "{name} is declared as a table type");
    lines.take_while(|line| *line != "}").collect()
}

/// The field names of a table type, one field to a line.
fn type_fields<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    type_body(text, name)
        .into_iter()
        .filter_map(|line| line.trim().split_once(':'))
        .map(|(field, _)| field.trim())
        .collect()
}

#[test]
fn declarations_list_exactly_the_registered_members_with_their_signatures() {
    let declared = declared_members(DECLARATIONS);
    let registered: BTreeMap<_, _> = api_members()
        .map(|member| (member.path.to_owned(), member.signature.to_owned()))
        .collect();

    let missing: Vec<_> = registered
        .keys()
        .filter(|path| !declared.contains_key(*path))
        .collect();
    assert!(
        missing.is_empty(),
        "registered but not declared in luau-api-v2.d.luau: {missing:?}"
    );
    let extra: Vec<_> = declared
        .keys()
        .filter(|path| !registered.contains_key(*path))
        .collect();
    assert!(
        extra.is_empty(),
        "declared in luau-api-v2.d.luau but not registered: {extra:?}"
    );
    for (path, signature) in &registered {
        assert_eq!(
            &declared[path], signature,
            "the declared signature of api.{path} differs from the runtime's"
        );
    }
}

#[test]
fn every_type_that_a_signature_names_is_declared() {
    let declared = declared_types(DECLARATIONS);
    for member in api_members() {
        for name in member
            .signature
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|word| word.starts_with(|ch: char| ch.is_ascii_uppercase()))
        {
            assert!(
                declared.contains(name),
                "api.{} names the type {name}, which luau-api-v2.d.luau does not declare",
                member.path
            );
        }
    }
    for name in ["ScriptManifest", "ScriptEntry", "PaintContext", "Api"] {
        assert!(declared.contains(name), "{name} is declared");
    }
}

#[test]
fn the_reference_lists_exactly_the_registered_members_with_their_call_classes() {
    let registered: BTreeMap<_, _> = api_members()
        .map(|member| (member.path, member.kind))
        .collect();
    let mut listed = BTreeSet::new();
    for line in REFERENCE.lines().filter(|line| line.starts_with("| `api.")) {
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        let member = cells[0]
            .strip_prefix("`api.")
            .and_then(|rest| rest.split(['(', '`']).next())
            .unwrap_or_else(|| panic!("not a member row: {line}"));
        let kind = *registered
            .get(member)
            .unwrap_or_else(|| panic!("the reference lists api.{member}, which does not exist"));
        assert!(listed.insert(member), "api.{member} is listed twice");
        // The status table has no class column: its rows all read.
        let class = match cells.get(1).copied() {
            Some("Read") => Some(Kind::Read),
            Some("Request") => Some(Kind::Request),
            Some("Await") => Some(Kind::Await),
            Some("Output") => Some(Kind::Output),
            _ => None,
        };
        match class {
            Some(class) => assert_eq!(class, kind, "the call class of api.{member}"),
            None => assert_eq!(kind, Kind::Read, "api.{member} has no class column"),
        }
    }
    let missing: Vec<_> = registered
        .keys()
        .filter(|path| !listed.contains(*path))
        .collect();
    assert!(
        missing.is_empty(),
        "registered but not listed in luau-api-v2.md: {missing:?}"
    );
}

const MANIFEST: &str = "manifest {\n\
    apiVersion = 2,\n\
    name = \"Sample\",\n\
    version = \"1\",\n\
    author = \"Example author\",\n\
    description = \"A sample.\",\n\
    category = \"Examples\",\n\
    tags = {},\n\
    settings = {},\n\
    capabilities = {},\n\
}\n";

fn manifest_error(source: &str) -> &'static str {
    match read_manifest(source) {
        Ok(read) => panic!("the reader accepted {source:?} as {:?}", read.manifest()),
        Err(error) => error.code(),
    }
}

#[test]
fn the_declared_manifest_fields_are_the_fields_that_the_reader_requires() {
    read_manifest(MANIFEST).expect("the base manifest is valid");
    let fields = type_fields(DECLARATIONS, "ScriptManifest");
    assert_eq!(fields.len(), 9, "the manifest has nine fields: {fields:?}");
    for field in &fields {
        let without = MANIFEST
            .lines()
            .filter(|line| !line.trim_start().starts_with(&format!("{field} =")))
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(without, MANIFEST, "the base manifest sets {field}");
        assert_eq!(
            manifest_error(&without),
            "manifest-required-field",
            "the reader requires {field}"
        );
    }
    let with_extra = MANIFEST.replacen("capabilities = {},", "capabilities = {}, extra = 1,", 1);
    assert_eq!(manifest_error(&with_extra), "manifest-unknown-field");
}

#[test]
fn the_declared_setting_types_and_fields_are_the_ones_that_the_reader_accepts() {
    let samples = [
        ("BooleanSetting", "boolean", "true"),
        ("NumberSetting", "number", "1"),
        ("StringSetting", "string", "\"a\""),
        ("StringListSetting", "string[]", "{ \"a\" }"),
        ("TileSetting", "tile", "{ x = 1, z = 2, level = 0 }"),
    ];
    let union = DECLARATIONS
        .lines()
        .find_map(|line| line.strip_prefix("export type SettingDef = "))
        .expect("SettingDef is declared");
    let setting = |fields: &str| {
        MANIFEST.replacen(
            "settings = {}",
            &format!("settings = {{ sample = {{ {fields} }} }}"),
            1,
        )
    };
    let mut every_field = BTreeSet::new();
    for (name, ty, default) in samples {
        assert!(union.contains(name), "SettingDef includes {name}");
        let body = type_body(DECLARATIONS, name);
        assert!(
            body.iter()
                .any(|line| line.trim() == format!("type: \"{ty}\",")),
            "{name} has the type tag {ty:?}"
        );
        let source = setting(&format!("type = \"{ty}\", default = {default}"));
        let read = read_manifest(&source)
            .unwrap_or_else(|error| panic!("the reader refuses a {ty} setting: {error}"));
        assert_eq!(read.manifest().settings_schema()[0].ty, ty);
        every_field.extend(type_fields(DECLARATIONS, name));
    }
    assert_eq!(
        manifest_error(&setting("type = \"vector\", default = 1")),
        "setting-type-unknown"
    );
    every_field.remove("type");
    every_field.remove("default");
    assert!(
        !every_field.is_empty(),
        "the declarations list optional fields"
    );
    for field in every_field {
        // An accepted field may still fail for its value or its type; only an
        // unknown field is the failure that proves the declaration wrong.
        let source = setting(&format!("type = \"string\", default = \"a\", {field} = 1"));
        let outcome = read_manifest(&source).err().map(|error| error.code());
        assert_ne!(
            outcome,
            Some("setting-unknown-field"),
            "the reader does not know the declared setting field {field}"
        );
    }
    assert_eq!(
        manifest_error(&setting("type = \"string\", default = \"a\", bogus = 1")),
        "setting-unknown-field"
    );
}
