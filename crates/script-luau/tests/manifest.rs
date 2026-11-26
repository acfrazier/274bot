//! The manifest reader of a Luau file (ADR 0023 item 18, slice L5).
//!
//! The reader gives the neutral manifest of the `script` crate from the first
//! statement of the file, in Rust, with no VM. The run compiles the file with
//! that statement erased in place.

pub mod support;

use script::card::SettingDef;
use script::manifest::{DeclaredHostCapabilities, ManifestText, ScriptManifest};
use script::surface::{LentOutcome, SurfaceError, TickOutcome};
use script_luau::manifest::{ExpressionKind, KeyKind, SOURCE_LIMIT_BYTES};
use script_luau::{read_manifest, ManifestError, ManifestReason};
use support::{snapshot, spawn_with, tick};

const CARD: &str = "binding-test";

// ---------------------------------------------------------------------------
// Sources

const FIELDS: [(&str, &str); 9] = [
    ("apiVersion", "2"),
    ("name", "\"Miner\""),
    ("version", "\"1.0\""),
    ("author", "\"Alice\""),
    ("description", "\"Mine ore\""),
    ("category", "\"Mining\""),
    ("tags", "{ \"ore\" }"),
    ("settings", "{}"),
    ("capabilities", "{}"),
];

/// A valid manifest statement. `Some` replaces a field, `None` removes it, and
/// a name outside the nine fields is added.
fn manifest_with(overrides: &[(&str, Option<&str>)]) -> String {
    let mut lines = Vec::new();
    for (key, default) in FIELDS {
        match overrides.iter().find(|(name, _)| *name == key) {
            Some((_, Some(value))) => lines.push(format!("    {key} = {value},")),
            Some((_, None)) => {}
            None => lines.push(format!("    {key} = {default},")),
        }
    }
    for (key, value) in overrides {
        if !FIELDS.iter().any(|(name, _)| name == key) {
            if let Some(value) = value {
                lines.push(format!("    {key} = {value},"));
            }
        }
    }
    format!("manifest {{\n{}\n}}\n", lines.join("\n"))
}

fn field(key: &str, value: &str) -> String {
    manifest_with(&[(key, Some(value))])
}

fn read(source: &str) -> ManifestError {
    match read_manifest(source) {
        Ok(read) => panic!("the reader accepted {source:?} as {:?}", read.manifest()),
        Err(error) => error,
    }
}

fn setting_def(id: &str, ty: &str, default: &str) -> SettingDef {
    SettingDef {
        id: id.into(),
        ty: ty.into(),
        default: Some(default.into()),
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// The neutral model

/// Every field of the neutral model, every setting type and every optional
/// setting field, with the comment forms, both quote styles, escapes, the
/// separators and the key forms of the grammar. The strings hold `--`, `}` and
/// `manifest {` to show that the reader scans tokens, not text.
const FULL: &str = r#"--!strict
-- A comment before the statement.
--[==[ A block comment
that spans lines ]] and has a false end. ]==]
manifest {
    apiVersion = 2.0, -- a trailing comment
    name = "Bone Burier",
    version = '1.0.0',
    author = "Example \"author\"",
    description = "Bury bones -- not a comment; } manifest { \\ done",
    category = "Prayer",
    tags = { "prayer", 'banking', "m\u{f6}nch" },
    settings = {
        boneName = {
            type = "string",
            default = "Bones",
            label = "Bone name",
            help = "The item to bury",
            group = "Items",
            options = { "Bones", "Big bones" },
            optionLabels = { ["Big bones"] = "Big" },
        },
        mode = { type = "string", default = "a", options = { "a", "b" }; },
        count = { type = "number", default = 5, min = 1, max = 10.5 },
        ["offset"] = { type = "number", default = -3, min = -10, max = 1e1 },
        shout = { type = "boolean", default = true, showIf = { key = "mode", anyOf = { "b" } } },
        foods = { type = "string[]", default = { "x", "y" }, optionsFrom = "loadouts" },
        banks = { type = "string[]", default = {}, optionsFrom = 'named-banks' },
        home = { type = "tile", default = { x = 3200, z = -3200, level = 0 } },
    },
    capabilities = {},
}

local function tick(api)
    return "manifest {"
end

return { tick = tick }
"#;

fn expected_full() -> ScriptManifest {
    let mut bone_name = setting_def("boneName", "string", "Bones");
    bone_name.label = Some("Bone name".into());
    bone_name.help = Some("The item to bury".into());
    bone_name.group = Some("Items".into());
    bone_name.options = strings(&["Bones", "Big bones"]);
    bone_name.option_labels = strings(&["Bones", "Big"]);

    let mut mode = setting_def("mode", "string", "a");
    mode.options = strings(&["a", "b"]);
    mode.option_labels = Vec::new();

    let mut count = setting_def("count", "number", "5");
    count.min = Some("1".into());
    count.max = Some("10.5".into());

    let mut offset = setting_def("offset", "number", "-3");
    offset.min = Some("-10".into());
    offset.max = Some("10".into());

    let mut shout = setting_def("shout", "boolean", "true");
    shout.show_if = Some(serde_json::json!({ "key": "mode", "anyOf": ["b"] }).to_string());

    let mut foods = setting_def("foods", "string[]", r#"["x","y"]"#);
    foods.options_from = Some("loadouts".into());

    let mut banks = setting_def("banks", "string[]", "[]");
    banks.options_from = Some("named-banks".into());

    let home = setting_def(
        "home",
        "tile",
        &serde_json::json!({ "x": 3200, "z": -3200, "level": 0 }).to_string(),
    );

    ScriptManifest::new(
        2,
        ManifestText {
            name: "Bone Burier".into(),
            version: "1.0.0".into(),
            author: "Example \"author\"".into(),
            description: "Bury bones -- not a comment; } manifest { \\ done".into(),
            category: "Prayer".into(),
        },
        strings(&["prayer", "banking", "mönch"]),
        vec![bone_name, mode, count, offset, shout, foods, banks, home],
        DeclaredHostCapabilities::EMPTY,
    )
}

#[test]
fn a_full_manifest_equals_the_neutral_model_field_by_field() {
    let read = read_manifest(FULL).expect("the full manifest reads");
    let got = read.manifest();
    let want = expected_full();

    assert_eq!(got.api_version(), want.api_version());
    assert_eq!(got.name(), "Bone Burier");
    assert_eq!(got.version(), "1.0.0");
    assert_eq!(got.author(), "Example \"author\"");
    assert_eq!(
        got.description(),
        "Bury bones -- not a comment; } manifest { \\ done"
    );
    assert_eq!(got.category(), "Prayer");
    assert_eq!(got.tags(), want.tags());
    assert_eq!(got.capabilities(), DeclaredHostCapabilities::EMPTY);
    assert_eq!(got.settings_schema().len(), want.settings_schema().len());
    for (got, want) in got.settings_schema().iter().zip(want.settings_schema()) {
        assert_eq!(got, want, "setting {}", want.id);
    }
    assert_eq!(got, &want);

    // The statement is exactly the text from `manifest` to its closing brace.
    let statement = &FULL[read.statement()];
    assert!(statement.starts_with("manifest {\n    apiVersion"));
    assert!(statement.ends_with("capabilities = {},\n}"));
}

#[test]
fn the_compact_form_gives_the_same_manifest() {
    let compact = concat!(
        "\u{feff}manifest{[\"apiVersion\"]=2;name=\"Miner\";version=\"1.0\";author=\"Alice\";",
        "description=\"Mine ore\";category=\"Mining\";tags={\"ore\"};settings={};capabilities={}};\n",
        "return {tick=function() end}"
    );
    let spaced = manifest_with(&[]);
    assert_eq!(
        read_manifest(compact).expect("compact reads").manifest(),
        read_manifest(&spaced).expect("spaced reads").manifest()
    );
    let read = read_manifest(compact).expect("compact reads");
    // The statement takes its closing `;`, and a byte-order mark stays outside it.
    assert!(compact[read.statement()].ends_with("capabilities={}};"));
    assert_eq!(read.statement().start, '\u{feff}'.len_utf8());
}

#[test]
fn a_file_of_exactly_the_limit_reads_and_one_byte_more_is_refused() {
    let mut source = manifest_with(&[]);
    source.push_str("--");
    let padding = SOURCE_LIMIT_BYTES - source.len();
    source.push_str(&"x".repeat(padding));
    assert_eq!(source.len(), SOURCE_LIMIT_BYTES);
    read_manifest(&source).expect("a file at the limit reads");

    source.push('x');
    let error = read(&source);
    assert_eq!(error.code(), "luau-manifest-file-too-large");
    assert_eq!(
        error.reason,
        ManifestReason::FileTooLarge {
            bytes: SOURCE_LIMIT_BYTES + 1
        }
    );
}

// ---------------------------------------------------------------------------
// No VM

/// A body that no compiler accepts, and one that never returns when it runs.
const BROKEN_BODY: &str = "local = = =\nwhile true do end\nerror(\n";

#[test]
fn reading_a_manifest_runs_no_luau_code_and_needs_no_valid_body() {
    let source = format!("{}{BROKEN_BODY}", manifest_with(&[]));
    // A VM would stop at the syntax error, or run `while true`. The reader
    // returns the manifest, because it reads the statement and nothing else.
    let read = read_manifest(&source).expect("the manifest reads");
    assert_eq!(read.manifest().name(), "Miner");
    // The erased body still holds the broken text: nothing parsed or ran it.
    assert!(read.erased_source(&source).ends_with(BROKEN_BODY));
}

#[test]
fn the_reader_module_does_not_name_the_vm() {
    let text = include_str!("../src/manifest.rs");
    for name in ["mlua", "Lua::", "StdLib", "Compiler"] {
        assert!(
            !text.contains(name),
            "the manifest reader names `{name}`, a VM type"
        );
    }
}

#[test]
fn a_refused_manifest_stops_the_start_before_the_compile() {
    let source = format!("{}{BROKEN_BODY}", field("extra", "1"));
    let message = spawn_error(&source);
    assert!(
        message.starts_with("luau-manifest: manifest-unknown-field"),
        "{message}"
    );
    // The body is invalid Luau. The compile never saw it.
    assert!(!message.contains("luau-compile"), "{message}");
}

fn spawn_error(source: &str) -> String {
    match spawn_with(source, None) {
        Ok(_) => panic!("the start accepted {source:?}"),
        Err(SurfaceError::Spawn(message)) => message,
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Refusals

/// The first statement is `manifest { ... }` and nothing else.
#[test]
fn a_missing_or_non_leading_manifest_is_refused() {
    let valid = manifest_with(&[]);
    for (label, source) in [
        ("empty file", String::new()),
        ("comments only", "-- nothing\n--[[ here ]]\n".to_owned()),
        (
            "plain script",
            "return { tick = function() end }".to_owned(),
        ),
        ("statement first", format!("local x = 1\n{valid}")),
        ("return first", format!("return {{}}\n{valid}")),
        ("other name", valid.replacen("manifest", "manifests", 1)),
        (
            "local name",
            valid.replacen("manifest", "local manifest =", 1),
        ),
    ] {
        let error = read(&source);
        assert_eq!(error.code(), "luau-manifest-required", "{label}");
        assert_eq!(error.reason, ManifestReason::MissingManifest, "{label}");
    }
}

#[test]
fn the_name_must_be_followed_by_one_table() {
    for source in [
        "manifest",
        "manifest\n",
        "manifest = {}",
        "manifest(\"x\")",
        "manifest \"text\"",
    ] {
        assert_eq!(
            read(source).code(),
            "luau-manifest-expected-table",
            "{source}"
        );
    }
    let parenthesized = format!(
        "manifest({})",
        manifest_with(&[]).trim_start_matches("manifest ")
    );
    assert_eq!(read(&parenthesized).code(), "luau-manifest-expected-table");
}

#[test]
fn code_after_the_table_in_the_same_statement_is_refused() {
    let table = manifest_with(&[]);
    for suffix in [
        "(\"x\")",
        ".field",
        ":method()",
        "[1]",
        "{}",
        "\"text\"",
        "'text'",
        "`text`",
        " + 1\nreturn { tick = function() end }",
        " return { tick = function() end }",
        " .. \"x\"",
        " == nil",
        "; return { tick = function() end }",
        " --[[ same line ]] return { tick = function() end }",
        " é",
    ] {
        let source = format!("{}{suffix}", table.trim_end());
        assert_eq!(
            read(&source).code(),
            "luau-manifest-trailing-expression",
            "{suffix}"
        );
    }
    // A new statement, a `;`, a comment and the end of the file all end it.
    for suffix in [
        "",
        "\nlocal x = 1",
        ";",
        " ; -- end",
        "  \t",
        ";\r\nlocal x = 1",
        "-- end",
        " --[[ a ]] -- b",
        " --[[ spans\nlines ]] local x = 1",
        "\n\n",
    ] {
        read_manifest(&format!("{}{suffix}", table.trim_end())).expect(suffix);
    }
}

#[test]
fn a_manifest_holds_literals_only() {
    use ExpressionKind::{
        Call, Function, Identifier, InterpolatedString, LongString, Nil, NonDecimalNumber, Operator,
    };
    for (value, kind) in [
        ("NAME", Identifier),
        ("string.rep", Identifier),
        ("not_a_value", Identifier),
        ("tostring(1)", Call),
        ("string.rep(\"a\", 3)", Identifier),
        ("f{}", Call),
        ("f\"text\"", Call),
        ("f [[text]]", Call),
        ("(\"a\")", Call),
        ("function() return \"x\" end", Function),
        ("nil", Nil),
        ("`interpolated`", InterpolatedString),
        ("[[long]]", LongString),
        ("[==[long]==]", LongString),
        ("\"a\" .. \"b\"", Operator),
        ("\"a\":upper()", Call),
        ("#\"abc\"", Operator),
        ("not true", Operator),
        ("1 + 1", Operator),
        ("2 * 3", Operator),
        ("0x10", NonDecimalNumber),
        ("0b101", NonDecimalNumber),
        ("-0xff", NonDecimalNumber),
    ] {
        let error = read(&field("name", value));
        assert_eq!(error.code(), "luau-manifest-nonliteral", "{value}");
        assert_eq!(error.reason, ManifestReason::Expression(kind), "{value}");
        assert_eq!(error.line(), 3, "{value}: the name is on the third line");
    }
    // The same in a nested position.
    let error = read(&field(
        "settings",
        "{ s = { type = \"string\", default = NAME } }",
    ));
    assert_eq!(error.reason, ManifestReason::Expression(Identifier));
}

#[test]
fn malformed_numbers_and_strings_are_refused() {
    for (value, code) in [
        ("1e999", "luau-manifest-number-nonfinite"),
        ("-1e999", "luau-manifest-number-nonfinite"),
        ("2.", "luau-manifest-number-malformed"),
        ("1__0", "luau-manifest-number-malformed"),
        ("1_0", "luau-manifest-number-malformed"),
        ("3abc", "luau-manifest-number-malformed"),
        ("1e", "luau-manifest-number-malformed"),
        ("1e+", "luau-manifest-number-malformed"),
        ("-x", "luau-manifest-number-malformed"),
        ("- 2", "luau-manifest-number-malformed"),
        ("\"open", "luau-manifest-unterminated-string"),
        ("\"line\nbreak\"", "luau-manifest-unterminated-string"),
        ("\"\\x41\"", "luau-manifest-escape-unsupported"),
        ("\"\\z\"", "luau-manifest-escape-unsupported"),
        ("\"\\65\"", "luau-manifest-escape-unsupported"),
        ("\"\\\nx\"", "luau-manifest-escape-unsupported"),
        ("\"\\u{D800}\"", "luau-manifest-escape-unicode"),
        ("\"\\u{110000}\"", "luau-manifest-escape-unicode"),
        ("\"\\u{}\"", "luau-manifest-escape-unicode"),
        ("\"\\u0041\"", "luau-manifest-escape-unicode"),
        ("\"\\u{000000041}\"", "luau-manifest-escape-unicode"),
        ("\"\\u{41\"", "luau-manifest-escape-unicode"),
    ] {
        assert_eq!(read(&field("name", value)).code(), code, "{value:?}");
    }
    // The accepted escapes decode. Control characters are legal in a setting
    // default, which is a value and not display text.
    let read = read_manifest(&field(
        "settings",
        r#"{ s = { type = "string", default = "a\nb\rc\td\\e\"f\'g\u{41}\u{1F600}" } }"#,
    ))
    .expect("the escapes read");
    assert_eq!(
        read.manifest().settings_schema()[0].default.as_deref(),
        Some("a\nb\rc\td\\e\"f'gA😀")
    );
}

#[test]
fn unterminated_input_is_refused() {
    let valid = manifest_with(&[]);
    let cut = valid.trim_end().strip_suffix('}').expect("closing brace");
    assert_eq!(read(cut).code(), "luau-manifest-unexpected-end");
    assert_eq!(read("manifest {").code(), "luau-manifest-unexpected-end");
    assert_eq!(
        read(&format!("--[==[ open\n{valid}")).code(),
        "luau-manifest-unterminated-comment"
    );
    assert_eq!(
        read(&format!("{}--[[ open", valid.trim_end())).code(),
        "luau-manifest-unterminated-comment"
    );
    for source in [
        "manifest { apiVersion 2 }",
        "manifest { name = \"a\" \"b\" }",
        "manifest { , }",
        "manifest { name = \"a\",, }",
        "manifest { name = }",
        "manifest { name == \"a\" }",
        "manifest { @ }",
    ] {
        assert!(
            read_manifest(source).is_err(),
            "the reader accepted {source:?}"
        );
    }
}

#[test]
fn keys_follow_the_closed_grammar() {
    use KeyKind::{Computed, Duplicate, Mixed, Reserved};
    for (source, key) in [
        (field("settings", "{ [1] = {} }"), Computed),
        (field("settings", "{ [NAME] = {} }"), Computed),
        (field("settings", "{ [\"a\" .. \"b\"] = {} }"), Computed),
        (field("settings", "{ [true] = {} }"), Computed),
        (field("settings", "{ [\"a\" = {} }"), Computed),
        (
            field("name", "\"a\"").replacen("category", "end", 1),
            Reserved("end".into()),
        ),
        (
            field("name", "\"a\"").replacen("category", "true", 1),
            Reserved("true".into()),
        ),
        (
            field("name", "\"b\"").replacen("category", "name", 1),
            Duplicate("name".into()),
        ),
        (field("tags", "{ \"a\", x = \"b\" }"), Mixed),
        (field("tags", "{ x = \"b\", \"a\" }"), Mixed),
    ] {
        let error = read(&source);
        assert_eq!(error.reason, ManifestReason::Key(key), "{source}");
    }
    // A duplicate in a nested table, and in a spelling with a quoted key.
    let nested = field(
        "settings",
        "{ s = { type = \"string\", default = \"a\", [\"default\"] = \"b\" } }",
    );
    assert_eq!(read(&nested).code(), "luau-manifest-duplicate-key");
    let twice = field(
        "settings",
        "{ s = { type = \"boolean\", default = true }, s = {} }",
    );
    assert_eq!(read(&twice).code(), "luau-manifest-duplicate-key");
    // A `type` key is no reserved word.
    read_manifest(&field(
        "settings",
        "{ s = { type = \"boolean\", default = true } }",
    ))
    .expect("`type` is a name");
}

#[test]
fn deep_tables_are_refused_without_exhausting_the_stack() {
    let depth = 100_000;
    let source = format!("manifest {}", "{".repeat(depth));
    assert!(source.len() < SOURCE_LIMIT_BYTES);
    assert_eq!(read(&source).code(), "luau-manifest-too-deep");
    let nested = field("settings", &format!("{}{}", "{ a = ".repeat(40), "{}"));
    assert_eq!(read(&nested).code(), "luau-manifest-too-deep");
}

#[test]
fn the_nine_fields_are_required_and_unknown_fields_are_refused() {
    for (name, _) in FIELDS {
        let error = read(&manifest_with(&[(name, None)]));
        assert_eq!(error.code(), "manifest-required-field", "{name}");
        assert_eq!(error.field_path, format!("manifest.{name}"));
        let ManifestReason::MissingField(missing) = error.reason else {
            panic!("{name}: {:?}", error.reason);
        };
        assert_eq!(missing, name);
    }
    for unknown in [
        "extra",
        "Name",
        "settingsSchema",
        "hostCapabilities",
        "steps",
    ] {
        let error = read(&manifest_with(&[(unknown, Some("1"))]));
        assert_eq!(error.reason, ManifestReason::UnknownField(unknown.into()));
        assert_eq!(error.field_path, format!("manifest.{unknown}"));
    }
    // An empty table is no manifest; a positional table is not a map.
    assert_eq!(read("manifest {}").code(), "manifest-required-field");
    assert_eq!(read("manifest { \"a\" }").code(), "manifest-field-type");
}

#[test]
fn field_values_follow_the_rules_of_the_neutral_model() {
    for (key, value, code) in [
        ("apiVersion", "3", "api-version-unsupported"),
        ("apiVersion", "1", "api-version-unsupported"),
        ("apiVersion", "2.5", "api-version-malformed"),
        ("apiVersion", "-2", "api-version-malformed"),
        ("apiVersion", "4294967296", "api-version-malformed"),
        ("apiVersion", "\"2\"", "manifest-field-type"),
        ("apiVersion", "true", "manifest-field-type"),
        ("apiVersion", "{}", "manifest-field-type"),
        ("name", "\"\"", "manifest-empty-field"),
        ("name", "\" padded\"", "manifest-field-whitespace"),
        ("name", "\"padded \"", "manifest-field-whitespace"),
        ("name", "\"tab\\t\"", "manifest-field-control"),
        ("name", "\"a\\nb\"", "manifest-field-control"),
        ("name", "\"a\\u{0}b\"", "manifest-field-control"),
        ("name", "\"a\\u{85}b\"", "manifest-field-control"),
        ("name", "\"a\\u{2028}b\"", "manifest-field-control"),
        ("name", "\"a\\u{202e}b\"", "manifest-field-control"),
        ("name", "\"a\\u{2066}b\"", "manifest-field-control"),
        ("name", "5", "manifest-field-type"),
        ("name", "{ \"a\" }", "manifest-field-type"),
        ("version", "\"\"", "manifest-empty-field"),
        ("author", "false", "manifest-field-type"),
        ("description", "\"  \"", "manifest-field-whitespace"),
        ("category", "\"\"", "manifest-empty-field"),
        ("tags", "{ \"a\", \"a\" }", "manifest-duplicate-tag"),
        ("tags", "{ \"\" }", "manifest-empty-field"),
        ("tags", "{ 1 }", "manifest-field-type"),
        ("tags", "{ a = \"x\" }", "manifest-field-type"),
        ("tags", "\"a\"", "manifest-field-type"),
        ("tags", "{ \"a\\u{7}\" }", "manifest-field-control"),
        ("settings", "\"x\"", "manifest-field-type"),
        ("settings", "{ \"a\" }", "manifest-field-type"),
        ("capabilities", "\"x\"", "manifest-field-type"),
        ("capabilities", "{ 1 }", "manifest-field-type"),
        ("capabilities", "{ k = \"v\" }", "manifest-field-type"),
    ] {
        let error = read(&field(key, value));
        assert_eq!(error.code(), code, "{key} = {value}");
        assert_eq!(
            error.field_path,
            format!("manifest.{key}"),
            "{key} = {value}"
        );
    }
    // The tag `ore` and a name with inner space are fine; an empty tag list too.
    let ok = manifest_with(&[("name", Some("\"Bone  Burier\"")), ("tags", Some("{}"))]);
    assert_eq!(
        read_manifest(&ok).expect("reads").manifest().tags().len(),
        0
    );
}

#[test]
fn capabilities_are_refused_because_luau_has_no_packet_api() {
    let error = read(&field("capabilities", "{ \"host-packet-protocol\" }"));
    assert_eq!(
        error.reason,
        ManifestReason::UnsupportedCapability("host-packet-protocol".into())
    );
    assert_eq!(error.code(), "luau-capability-unsupported");
    let error = read(&field("capabilities", "{ \"teleport\" }"));
    assert_eq!(
        error.reason,
        ManifestReason::UnknownCapability("teleport".into())
    );
    assert_eq!(error.code(), "capability-unknown");
}

#[test]
fn settings_follow_the_shared_schema_rules() {
    let one = |setting: &str| format!("{{ s = {setting} }}");
    for (settings, code) in [
        (one("5"), "setting-field-type"),
        (one("\"x\""), "setting-field-type"),
        (one("{}"), "setting-type-missing"),
        (one("{ type = \"string\" }"), "setting-default-missing"),
        (one("{ default = \"a\" }"), "setting-type-missing"),
        (one("{ type = \"list\", default = \"a\" }"), "setting-type-unknown"),
        (one("{ type = 1, default = \"a\" }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", step = 1 }"), "setting-unknown-field"),
        (one("{ type = \"string\", default = \"a\", csvToggle = \"x\" }"), "setting-unknown-field"),
        (one("{ type = \"string\", default = 5 }"), "setting-field-type"),
        (one("{ type = \"number\", default = \"5\" }"), "setting-field-type"),
        (one("{ type = \"boolean\", default = 1 }"), "setting-field-type"),
        (one("{ type = \"string[]\", default = \"a\" }"), "setting-field-type"),
        (one("{ type = \"string[]\", default = { 1 } }"), "setting-field-type"),
        (one("{ type = \"string[]\", default = { a = \"b\" } }"), "setting-field-type"),
        (one("{ type = \"tile\", default = \"a\" }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", label = 1 }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", help = \"x\\ny\" }"), "manifest-field-control"),
        (one("{ type = \"string\", default = \"a\", group = \"x\\u{202e}\" }"), "manifest-field-control"),
        (one("{ type = \"string\", default = \"a\", min = 1 }"), "setting-field-not-allowed"),
        (one("{ type = \"boolean\", default = true, max = 1 }"), "setting-field-not-allowed"),
        (one("{ type = \"boolean\", default = true, options = { \"a\" } }"), "setting-field-not-allowed"),
        (one("{ type = \"number\", default = 1, optionsFrom = \"loadouts\" }"), "setting-field-not-allowed"),
        (one("{ type = \"number\", default = 1, optionLabels = {} }"), "setting-field-not-allowed"),
        (one("{ type = \"number\", default = 1, min = \"0\" }"), "setting-field-type"),
        (one("{ type = \"number\", default = 5, min = 6 }"), "setting-default-outside-range"),
        (one("{ type = \"number\", default = 5, max = 4 }"), "setting-default-outside-range"),
        (one("{ type = \"number\", default = 5, min = 9, max = 2 }"), "setting-range-inverted"),
        (one("{ type = \"string\", default = \"a\", options = {} }"), "setting-options-empty"),
        (one("{ type = \"string\", default = \"a\", options = \"a\" }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", options = { \"a\", \"a\" } }"), "setting-option-duplicate"),
        (one("{ type = \"string\", default = \"c\", options = { \"a\", \"b\" } }"), "setting-default-not-option"),
        (one("{ type = \"string[]\", default = { \"a\", \"c\" }, options = { \"a\", \"b\" } }"), "setting-default-not-option"),
        (one("{ type = \"string\", default = \"a\", options = { \"a\" }, optionLabels = { b = \"B\" } }"), "setting-option-label-unknown"),
        (one("{ type = \"string\", default = \"a\", options = { \"a\" }, optionLabels = { a = 1 } }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", options = { \"a\" }, optionLabels = { \"a\" } }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", optionsFrom = \"banks\" }"), "setting-option-source-unknown"),
        (one("{ type = \"string\", default = \"a\", options = { \"a\" }, optionsFrom = \"loadouts\" }"), "setting-options-competing"),
        (one("{ type = \"tile\", default = { x = 1, z = 2 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = 1, z = 2, level = 4 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = 1, z = 2, level = -1 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = 1.5, z = 2, level = 0 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = 1, z = 2, level = 0, y = 3 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = 1, z = 2, w = 0 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { x = \"1\", z = 2, level = 0 } }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = {} }"), "setting-tile-invalid"),
        (one("{ type = \"tile\", default = { 1, 2, 3 } }"), "setting-tile-invalid"),
        (
            "{ [\" id\"] = { type = \"boolean\", default = true } }".to_owned(),
            "setting-id-invalid",
        ),
        (
            "{ [\"\"] = { type = \"boolean\", default = true } }".to_owned(),
            "setting-id-invalid",
        ),
        (
            "{ [\"a\\nb\"] = { type = \"boolean\", default = true } }".to_owned(),
            "setting-id-invalid",
        ),
        (
            one("{ type = \"boolean\", default = true, showIf = { key = \"zz\", anyOf = { \"a\" } } }"),
            "setting-reference-unknown",
        ),
        (
            one("{ type = \"string\", default = \"a\", showIf = { key = \"s\", anyOf = { \"a\" } } }"),
            "setting-reference-invalid",
        ),
        (
            "{ o = { type = \"boolean\", default = true }, s = { type = \"string\", default = \"a\", showIf = { key = \"o\", anyOf = { \"a\" } } } }".to_owned(),
            "setting-reference-invalid",
        ),
        (one("{ type = \"string\", default = \"a\", showIf = 1 }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", showIf = { key = \"s\" } }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", showIf = { anyOf = { \"a\" } } }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", showIf = { key = \"s\", anyOf = {} } }"), "setting-condition-empty"),
        (one("{ type = \"string\", default = \"a\", showIf = { key = \"s\", anyOf = \"a\" } }"), "setting-field-type"),
        (one("{ type = \"string\", default = \"a\", showIf = { key = \"s\", anyOf = { \"a\", \"a\" } } }"), "setting-condition-duplicate"),
        (one("{ type = \"string\", default = \"a\", showIf = { key = \"s\", anyOf = { \"a\" }, other = 1 } }"), "setting-unknown-field"),
    ] {
        let error = read(&field("settings", &settings));
        assert_eq!(error.code(), code, "{settings}");
        assert!(
            error.field_path.starts_with("manifest.settings"),
            "{settings}: {}",
            error.field_path
        );
    }
}

#[test]
fn a_refusal_names_the_field_and_the_position() {
    let source = format!(
        "--!strict\n\n{}",
        field(
            "settings",
            "{\n        mode = { type = \"list\", default = \"a\" },\n    }"
        )
    );
    let error = read(&source);
    assert_eq!(error.field_path, "manifest.settings.mode.type");
    assert_eq!(&source[error.span.clone()], "\"list\"");
    // `--!strict`, a blank line, `manifest {`, seven fields, then `settings`
    // on line 11 and `mode` on line 12.
    assert_eq!((error.line(), error.column()), (12, 25));
    let text = error.to_string();
    assert!(text.starts_with("setting-type-unknown: "), "{text}");
    assert!(text.contains("manifest.settings.mode.type"), "{text}");
    assert!(text.contains("line 12, column 25"), "{text}");
}

// ---------------------------------------------------------------------------
// Erasure

fn newline_offsets(text: &str) -> Vec<usize> {
    text.bytes()
        .enumerate()
        .filter(|(_, byte)| matches!(byte, b'\n' | b'\r'))
        .map(|(index, _)| index)
        .collect()
}

#[test]
fn erasure_keeps_every_byte_offset_and_line_break() {
    let body = "local x = 1\r\nreturn { tick = function() end }\n";
    for (label, source) in [
        (
            "lf",
            format!(
                "{}{body}",
                FULL.split("local function").next().expect("head")
            ),
        ),
        (
            "crlf",
            format!("{}{body}", manifest_with(&[]).replace('\n', "\r\n")),
        ),
        (
            "cr",
            format!("{}{body}", manifest_with(&[]).replace('\n', "\r")),
        ),
        (
            "bom and multibyte",
            format!(
                "\u{feff}-- {{ ö }}\n{}{body}",
                manifest_with(&[("name", Some("\"Mönch 😀\""))])
            ),
        ),
    ] {
        let read = read_manifest(&source).expect(label);
        let erased = read.erased_source(&source);
        assert_eq!(erased.len(), source.len(), "{label}");
        assert_eq!(
            newline_offsets(&erased),
            newline_offsets(&source),
            "{label}"
        );
        let statement = read.statement();
        assert_eq!(
            &erased[statement.end..],
            &source[statement.end..],
            "{label}"
        );
        assert!(
            erased[statement.clone()]
                .chars()
                .all(|ch| matches!(ch, ' ' | '\n' | '\r')),
            "{label}: the statement is spaces and line breaks"
        );
        let before = if source.starts_with('\u{feff}') {
            // The byte-order mark becomes three spaces: the compiler never sees it.
            assert!(erased.starts_with("   -- { ö }\n"), "{label}");
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        assert_eq!(
            &erased[before..statement.start],
            &source[before..statement.start],
            "{label}: the text before the statement stays"
        );
        assert!(!erased.contains("manifest"), "{label}");
    }
}

// ---------------------------------------------------------------------------
// The run

/// A manifest of nine lines, then code. The 1-based line of `marker` in `source`.
fn line_of(source: &str, marker: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(marker))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("{marker} is not in the source"))
}

/// The message of the one failed tick of a run.
fn tick_error(source: &str) -> String {
    let mut run = spawn_with(source, None).expect("the run starts");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    match run.drain_tick_outcomes().as_slice() {
        [TickOutcome::Error { message, .. }] => message.clone(),
        other => panic!("expected one failed tick, got {other:?}"),
    }
}

/// The traceback entry of a chunk line: `[string "card"]:LINE:`.
fn chunk_line(line: usize) -> String {
    format!("[string \"{CARD}\"]:{line}:")
}

#[test]
fn a_script_that_starts_with_a_manifest_loads_and_ticks() {
    let source = format!(
        "{}\nlocal ticks = 0\nreturn {{\n    tick = function(api)\n        ticks += 1\n        api.log(\"tick \" .. ticks)\n    end,\n}}\n",
        FULL.split("local function").next().expect("head")
    );
    let mut run = spawn_with(&source, None).expect("a script with a manifest starts");
    for expected in 1..=3 {
        assert_eq!(tick(&mut *run, &snapshot(expected)), LentOutcome::Accepted);
        assert_eq!(run.drain_logs(), vec![format!("tick {expected}")]);
    }
}

#[test]
fn the_vm_has_no_manifest_global_and_a_second_statement_fails_at_load() {
    let source = format!(
        "{}return {{ tick = function(api) api.log(tostring(manifest == nil)) end }}\n",
        manifest_with(&[])
    );
    let mut run = spawn_with(&source, None).expect("the run starts");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), vec!["true".to_owned()]);

    let twice = format!(
        "{}{}return {{ tick = function() end }}\n",
        manifest_with(&[]),
        manifest_with(&[])
    );
    let message = spawn_error(&twice);
    // The second statement calls a global that does not exist. It starts on
    // the line after the first statement's last line.
    let second = manifest_with(&[]).lines().count() + 1;
    assert!(message.starts_with("luau-load:"), "{message}");
    assert!(message.contains("attempt to call a nil value"), "{message}");
    assert!(message.contains(&chunk_line(second)), "{message}");
}

#[test]
fn error_lines_of_the_vm_are_the_lines_of_the_file() {
    for (label, newline) in [("lf", "\n"), ("crlf", "\r\n")] {
        let manifest = FULL.split("local function").next().expect("head");
        let source = format!(
            "{manifest}local function helper()\n    return 1\nend\n\nreturn {{\n    tick = function(api)\n        helper()\n        error(\"tick boom\")\n    end,\n}}\n"
        )
        .replace('\n', newline);
        let line = line_of(&source.replace(newline, "\n"), "tick boom");
        assert!(line > 30, "the manifest spans many lines: {line}");
        let message = tick_error(&source);
        assert!(message.contains("tick boom"), "{label}: {message}");
        assert!(
            message.contains(&chunk_line(line)),
            "{label}: line {line} is not in {message}"
        );
    }
}

#[test]
fn a_load_error_after_the_manifest_reports_the_file_line() {
    let source = format!(
        "{}local ok = true\n\nerror(\"load boom\")\nreturn {{ tick = function() end }}\n",
        FULL.split("local function").next().expect("head")
    );
    let line = line_of(&source, "load boom");
    let message = spawn_error(&source);
    assert!(message.starts_with("luau-load:"), "{message}");
    assert!(message.contains(&chunk_line(line)), "{message}");
}

#[test]
fn a_syntax_error_after_the_manifest_reports_the_file_line() {
    let source = format!(
        "{}local ok = true\nlocal = 1\nreturn {{ tick = function() end }}\n",
        manifest_with(&[])
    );
    let line = line_of(&source, "local = 1");
    let message = spawn_error(&source);
    assert!(
        message.starts_with("luau-compile: syntax error: "),
        "{message}"
    );
    assert!(message.contains(&format!("error: {line}:")), "{message}");
}

#[test]
fn a_leading_directive_comment_survives_the_erasure() {
    // `--!nonstrict` is a comment to the reader and a directive to the compiler.
    // The compiler still sees it, because only the statement is erased.
    let source = format!(
        "--!nonstrict\n{}return {{ tick = function(api) api.log(\"ok\") end }}\n",
        manifest_with(&[])
    );
    let mut run = spawn_with(&source, None).expect("the run starts");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), vec!["ok".to_owned()]);
}

#[test]
fn a_file_without_a_manifest_still_starts_and_a_refused_manifest_does_not() {
    let mut run = spawn_with(
        "return { tick = function(api) api.log(\"plain\") end }",
        None,
    )
    .expect("a file with no manifest starts");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), vec!["plain".to_owned()]);

    for (label, source) in [
        ("dynamic value", field("name", "NAME")),
        (
            "capability",
            field("capabilities", "{ \"host-packet-protocol\" }"),
        ),
        ("api version", field("apiVersion", "3")),
    ] {
        let message = spawn_error(&format!("{source}return {{ tick = function() end }}\n"));
        assert!(message.starts_with("luau-manifest: "), "{label}: {message}");
    }
}

#[test]
fn the_source_limit_still_stops_a_start_before_the_manifest_is_read() {
    let mut source = manifest_with(&[]);
    source.push_str("return { tick = function() end }\n--");
    source.push_str(&"x".repeat(SOURCE_LIMIT_BYTES));
    let message = spawn_error(&source);
    assert!(
        message.starts_with("luau-load: source exceeds"),
        "{message}"
    );
}
