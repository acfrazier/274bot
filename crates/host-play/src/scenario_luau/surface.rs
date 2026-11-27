//! Generated Luau authoring surface for File scenarios (LD2 §7.1).
//!
//! The text is built from the same static registries that admit calls:
//! `FILE_CHECK_SIGNATURES` (checks), `SEED_ENTRIES` plus their argument table
//! (seeds) and the role's core function list. No hand-written signature list
//! controls dispatch; `docs/dev/scenario.d.luau` is this output.

use std::fmt::Write as _;

use scenario::file_scenario::FILE_CHECK_SIGNATURES;

use super::fixtures::{BuiltinSeed, SEED_ENTRIES};

/// Luau type for one registry field description.
fn luau_type(description: &str) -> &'static str {
    match description {
        "integer" | "positive integer" => "number",
        "string" | "skill" | "signal name" | "exactness" | "hint kind" | "packet family"
        | "packet form" => "string",
        "boolean" => "boolean",
        "integer list" => "{ number }",
        "string list" | "dialogue lines" | "dialogue options" => "{ string }",
        "tile" => "Tile",
        "amount" => "Amount",
        "container" => "Container",
        "to mark or label" => "number | string",
        _ => "any",
    }
}

/// Typed arguments of each built-in seed (LD2 §8.2), checked by
/// `FixtureDefinitions::prepare_seed`.
fn seed_fields(seed: BuiltinSeed) -> &'static [(&'static str, &'static str)] {
    match seed {
        BuiltinSeed::Teleport => &[("x", "number"), ("z", "number"), ("level", "number")],
        BuiltinSeed::Give | BuiltinSeed::GiveBank => &[("item", "string"), ("count", "number")],
        BuiltinSeed::ClearInventory | BuiltinSeed::ClearBank | BuiltinSeed::EnergyFullRunOn => &[],
        BuiltinSeed::SetStat | BuiltinSeed::ResetAndAdvanceStat => {
            &[("skill", "string"), ("level", "number")]
        }
        BuiltinSeed::SetVar => &[
            ("name", "string"),
            ("value", "number"),
            ("confirmation", "(\"state\" | \"operation\")?"),
        ],
    }
}

/// The complete `scenario.d.luau` text.
pub fn authoring_definitions() -> String {
    let mut out = String::new();
    out.push_str(
        "-- Generated from the File scenario registries. Do not edit by hand.\n\
         -- Regenerate: UPDATE_SCENARIO_DEFINITIONS=1 cargo test -p host-play \\\n\
         --   --no-default-features --features scenario-luau,test-support --lib scenario_definitions\n\n\
         export type Mark = number\n\
         export type Handle = number\n\
         export type Tile = { x: number, z: number, level: number }\n\
         export type Amount = number | { eq: number?, min: number?, max: number? }\n\
         export type Container = \"inventory\" | \"worn\" | \"bank\" | number\n\n",
    );
    for signature in FILE_CHECK_SIGNATURES {
        let _ = writeln!(out, "export type Check_{} = {{", signature.name);
        for field in signature.fields {
            let _ = writeln!(out, "    {}: {}?,", field.name, luau_type(field.ty));
        }
        out.push_str("    label: string?,\n    within: number?,\n}\n");
    }
    out.push_str("\ndeclare scenario: {\n");
    out.push_str("    case: string,\n    role: string,\n");
    out.push_str("    step: (label: string, why: string) -> (),\n");
    out.push_str("    phase: (name: \"fixture\" | \"product\") -> (),\n");
    out.push_str("    mark: (label: string?) -> Mark,\n");
    out.push_str("    note: (key: string, value: string) -> (),\n");
    out.push_str("    signal: (name: string) -> (),\n");
    out.push_str("    seal: () -> (),\n");
    out.push_str("    finish: (options: { logout: boolean? }?) -> never,\n");
    out.push_str("    relog: (options: { why: string }) -> (),\n");
    out.push_str("    shot: (options: { label: string, tab: string?, settle: \"frame\"?, again_after_s: number? }) -> (),\n");
    for (scope, result) in [("expect", "()"), ("watch", "Handle"), ("probe", "boolean")] {
        let _ = writeln!(out, "    {scope}: {{");
        for signature in FILE_CHECK_SIGNATURES {
            let _ = writeln!(
                out,
                "        {}: (args: Check_{}) -> {result},",
                signature.name, signature.name
            );
        }
        let _ = writeln!(
            out,
            "        signal: (args: {{ name: string, label: string?, within: number? }}) -> {result},"
        );
        if scope == "expect" {
            out.push_str(
                "        server_var: (args: { name: string, value: number | string, label: string?, within: number? }) -> (),\n",
            );
        }
        out.push_str("    },\n");
    }
    out.push_str("    seed: {\n");
    for &(name, seed) in SEED_ENTRIES {
        let short = name.trim_start_matches("seed.");
        let _ = write!(out, "        {short}: (args: {{ why: string");
        for (field, ty) in seed_fields(seed) {
            let _ = write!(out, ", {field}: {ty}");
        }
        out.push_str(" }) -> (),\n");
    }
    out.push_str("        ext: (args: { op: string, why: string, [string]: any }) -> (),\n");
    out.push_str("    },\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKED_IN: &str = include_str!("../../../../docs/dev/scenario.d.luau");

    /// The checked-in reference is the generator's output; a registry change
    /// without regeneration fails here.
    #[test]
    fn scenario_definitions_match_the_registries() {
        let generated = authoring_definitions();
        if std::env::var_os("UPDATE_SCENARIO_DEFINITIONS").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/dev/scenario.d.luau"
            );
            std::fs::write(path, &generated).expect("write scenario.d.luau");
            return;
        }
        assert_eq!(CHECKED_IN, generated, "regenerate docs/dev/scenario.d.luau");
    }

    /// Paths declared in the generated text, e.g. `expect.received`.
    fn declared_paths(text: &str) -> std::collections::BTreeSet<String> {
        let mut paths = std::collections::BTreeSet::new();
        let mut table: Option<&str> = None;
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with("declare scenario") {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            let trimmed = line.trim_start();
            let depth = line.len() - trimmed.len();
            let Some((name, rest)) = trimmed.split_once(':') else {
                continue;
            };
            match depth {
                4 if rest.trim() == "{" => table = Some(name),
                4 => {
                    table = None;
                    paths.insert(name.to_owned());
                }
                8 => {
                    if let Some(table) = table {
                        paths.insert(format!("{table}.{name}"));
                    }
                }
                _ => {}
            }
        }
        paths
    }

    /// The authoring surface and the runtime admission registry name exactly
    /// the same callable paths (constants `case`/`role` are not calls).
    #[test]
    fn declared_surface_equals_the_admission_registry() {
        let declared: std::collections::BTreeSet<String> = declared_paths(&authoring_definitions())
            .into_iter()
            .filter(|path| path != "case" && path != "role")
            .collect();
        let admitted: std::collections::BTreeSet<String> = super::super::role::registered_paths()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(declared, admitted);
    }
}
