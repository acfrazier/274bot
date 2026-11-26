use std::sync::Arc;

use script::surface::extension::ExtensionValue;
use script_luau::{read_scenario_header, scenario_source};

#[test]
fn header_is_static_generic_data_and_source_erasure_preserves_positions() {
    let source: Arc<str> = Arc::from(
        "\u{feff}-- file comment\r\nscenario_header {\r\n  name = \"cell-α\",\r\n  cases = { \"default\", \"probe\" },\r\n  custom = { enabled = true, values = { 3, 4.5 } },\r\n}\r\nreturn { tick = function() end }\r\n",
    );
    let parsed = scenario_source(Arc::clone(&source)).expect("scenario source reads");
    let erased = parsed.erased.erased_text();

    assert!(Arc::ptr_eq(&parsed.source, &source));
    assert_eq!(parsed.source.len(), erased.len());
    assert_eq!(
        parsed.source.bytes().filter(|byte| *byte == b'\n').count(),
        erased.bytes().filter(|byte| *byte == b'\n').count()
    );
    assert!(erased.contains("return { tick = function() end }"));
    assert!(!erased.contains("scenario_header"));
    assert_eq!(
        parsed
            .header
            .clone()
            .into_json()
            .expect("header maps to JSON")["name"],
        "cell-α"
    );
    assert_eq!(
        read_scenario_header(&source)
            .expect("reader does not start a VM")
            .into_json()
            .expect("header maps to JSON"),
        parsed.header.into_json().expect("header maps to JSON")
    );
}

#[test]
fn reader_accepts_positional_and_named_literal_tables_without_running_code() {
    let source = r#"
        scenario_header {
            name = "fixture",
            values = { true, false, 12, 2.5, "text" },
            keyed = { alpha = "a", ["beta"] = "b" },
        }
        return {}
    "#;
    let header = read_scenario_header(source).expect("closed literal grammar reads");
    let value = header.into_json().expect("header maps to JSON");
    assert_eq!(value["name"], "fixture");
    assert_eq!(value["values"][0], true);
    assert_eq!(value["values"][2], 12);
    assert_eq!(value["keyed"]["beta"], "b");
}

#[test]
fn parser_rejects_expressions_duplicates_mixed_tables_nil_and_long_strings() {
    let invalid = [
        ("scenario_header { name = make() }", "expression"),
        (
            "scenario_header { name = \"first\", name = \"second\" }",
            "duplicate",
        ),
        ("scenario_header { name = \"x\", true }", "mixed table"),
        ("scenario_header { name = nil }", "nil"),
        ("scenario_header { name = [[long string]] }", "long string"),
        (
            "local before = true\nscenario_header { name = \"late\" }",
            "not first statement",
        ),
        (
            "manifest { name = \"wrong kind\" }",
            "manifest is not a scenario header",
        ),
    ];
    for (source, label) in invalid {
        assert!(read_scenario_header(source).is_err(), "accepted {label}");
    }
}

#[test]
fn parsed_header_uses_plain_extension_values_only() {
    let source = r#"scenario_header { name = "plain", data = { n = 8, ok = true, xs = { "a" } } }
return {}"#;
    let header = read_scenario_header(source).expect("header reads");
    assert!(matches!(header, ExtensionValue::Named(_)));
    let json = header.into_json().expect("header maps to JSON");
    assert_eq!(json["data"]["n"], 8);
    assert_eq!(json["data"]["xs"][0], "a");
}

#[test]
fn scenario_reader_enforces_the_one_mebibyte_file_bound() {
    let mut source = String::from("scenario_header { name = \"large\" }\n");
    source.push_str(&" ".repeat(1024 * 1024));
    assert!(read_scenario_header(&source).is_err());
}
