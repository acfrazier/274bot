//! Emit a reproducible JSON inventory from the real catalog loader.
//!
//! Usage: cargo run -p script --example catalog_inventory -- <rs2b0t-root>

use std::path::{Path, PathBuf};

use script::{
    is_catalog_dim, is_reserved, parse_registry, scan_import_specifiers, script_file_path,
    settings_schema_from_source, JsCache, JsLibrary, SettingDef,
};
use serde_json::{json, Value};

fn main() {
    if let Err(error) = run() {
        eprintln!("catalog_inventory: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| "usage: catalog_inventory <rs2b0t-root>".to_string())?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("root {}: {error}", root.display()))?;
    let scratch =
        std::env::temp_dir().join(format!("274bot-catalog-inventory-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)
        .map_err(|error| format!("scratch {}: {error}", scratch.display()))?;

    let mut library =
        JsLibrary::with_cache(scratch.join("js-scripts.json"), scratch.join("js-cache"));
    let registered = library.register_rs2b0t(&root, &scratch.join("rs2b0t-path"))?;

    let index = std::fs::read_to_string(root.join("src/bot/scripts/index.ts"))
        .map_err(|error| format!("registry: {error}"))?;
    let declarations = parse_registry(&index)?;

    let mut enabled = 0usize;
    let mut dim = 0usize;
    let mut import_blocked = 0usize;
    let mut registry_omitted = 0usize;
    let mut reserved_native = 0usize;
    let cards: Vec<Value> = library
        .cards()
        .iter()
        .map(|card| {
            let status = if is_catalog_dim(&card.name)
                || card
                    .unloadable
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("dim:"))
            {
                dim += 1;
                "dim"
            } else if card.unloadable.is_some() {
                import_blocked += 1;
                "import-blocked"
            } else {
                enabled += 1;
                "enabled"
            };
            let imports = import_bindings(&card.origin);
            let imported_api_members: Vec<Value> = imports
                .iter()
                .filter(|(specifier, _)| specifier.contains("/api/") || specifier == "@rs2b0t/api")
                .flat_map(|(specifier, members)| {
                    members.iter().map(move |member| {
                        let (exported_member, local_binding) = binding_names(member);
                        json!({
                            "specifier": specifier,
                            "exported_member": exported_member,
                            "local_binding": local_binding,
                            "used_members": used_members(&card.origin, local_binding),
                        })
                    })
                })
                .collect();
            let foreign_helpers: Vec<Value> = imports
                .iter()
                .filter(|(specifier, _)| !specifier.contains("/api/") && specifier != "@rs2b0t/api")
                .map(|(specifier, members)| json!({"specifier": specifier, "members": members}))
                .collect();
            json!({
                "display_name": card.name,
                "source_path": relative_to(&root, &card.path),
                "source_sha256": card.sha256,
                "category": card.category,
                "tags": card.tags,
                "shape": format!("{:?}", card.shape),
                "settings": card.settings_schema.iter().map(setting_json).collect::<Vec<_>>(),
                "import_specifiers": scan_import_specifiers(&card.origin),
                "imported_api_members": imported_api_members,
                "foreign_helpers": foreign_helpers,
                "status": status,
                "block_reason": card.unloadable,
            })
        })
        .collect();

    let mut omitted = Vec::new();
    for declaration in &declarations {
        if library
            .cards()
            .iter()
            .any(|card| card.name == declaration.name)
        {
            continue;
        }
        let path = script_file_path(&root, &declaration.rel_path);
        let origin = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let status = if is_reserved(&declaration.name) {
            reserved_native += 1;
            "reserved-native"
        } else if is_catalog_dim(&declaration.name) {
            dim += 1;
            "dim"
        } else {
            registry_omitted += 1;
            "registry-omitted"
        };
        omitted.push(json!({
            "display_name": declaration.name,
            "source_path": path.as_ref().map(|path| relative_to(&root, path)),
            "source_sha256": origin.as_ref().map(|origin| JsCache::origin_sha(origin.as_bytes())),
            "settings": origin.as_ref().map(|origin| {
                settings_schema_from_source(origin).iter().map(setting_json).collect::<Vec<_>>()
            }).unwrap_or_default(),
            "status": status,
            "reason": if status == "reserved-native" {
                "reserved by JsLibrary; 274bot native navigation owns WalkTo"
            } else if status == "dim" {
                "dim registry declaration rejected by loader shape; remains visibly deferred"
            } else {
                "registry declaration did not produce a JsLibrary card"
            },
        }));
    }

    let output = json!({
        "schema_version": 1,
        "catalog_root": root,
        "registry_path": "src/bot/scripts/index.ts",
        "registry_declaration_count": declarations.len(),
        "registered_count": registered,
        "counts": {
            "enabled": enabled,
            "dim": dim,
            "import_blocked": import_blocked,
            "registry_omitted": registry_omitted,
            "reserved_native": reserved_native,
        },
        "reserved_native": [{
            "display_name": "WalkTo",
            "owner": "274bot native navigation",
            "catalog_registration": "reserved and intentionally omitted by JsLibrary",
        }],
        "omitted_catalog_entries": omitted,
        "cards": cards,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn setting_json(setting: &SettingDef) -> Value {
    json!({
        "id": setting.id,
        "type": setting.ty,
        "default": setting.default,
        "label": setting.label,
        "min": setting.min,
        "max": setting.max,
        "step": setting.step,
        "options": setting.options,
        "option_labels": setting.option_labels,
        "group": setting.group,
        "show_if": setting.show_if,
        "options_from": setting.options_from,
        "csv_toggle": setting.csv_toggle,
        "help": setting.help,
    })
}

fn import_bindings(source: &str) -> Vec<(String, Vec<String>)> {
    let mut imports = Vec::new();
    for statement in source.split(';') {
        let statement = statement.trim_start();
        let Some(statement) = statement.strip_prefix("import ") else {
            continue;
        };
        let statement = statement.strip_prefix("type ").unwrap_or(statement);
        let Some((clause, quoted)) = statement.rsplit_once(" from ") else {
            continue;
        };
        let Some(specifier) = quoted_specifier(quoted.trim()) else {
            continue;
        };
        let mut members = Vec::new();
        let clause = clause.trim();
        if let Some(open) = clause.find('{') {
            let default = clause[..open].trim().trim_end_matches(',').trim();
            if !default.is_empty() {
                members.push(format!("default as {default}"));
            }
            if let Some(close) = clause.rfind('}') {
                for member in clause[open + 1..close].split(',') {
                    let member = member.trim().strip_prefix("type ").unwrap_or(member.trim());
                    if !member.is_empty() {
                        members.push(member.to_string());
                    }
                }
            }
        } else if clause.starts_with("* as ") {
            members.push(clause.to_string());
        } else if !clause.is_empty() {
            members.push(format!("default as {clause}"));
        }
        imports.push((specifier, members));
    }
    imports
}

fn quoted_specifier(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn binding_names(member: &str) -> (&str, &str) {
    if let Some(local) = member.strip_prefix("default as ") {
        return ("default", local.trim());
    }
    if let Some((exported, local)) = member.split_once(" as ") {
        return (exported.trim(), local.trim());
    }
    (member, member)
}

fn used_members(source: &str, binding: &str) -> Vec<String> {
    let needle = format!("{binding}.");
    let mut out = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find(&needle) {
        let start = offset + relative;
        let boundary_ok = start == 0
            || !source[..start]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$');
        let member_start = start + needle.len();
        let member: String = source[member_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
            .collect();
        if boundary_ok && !member.is_empty() {
            let used = format!("{binding}.{member}");
            if !out.contains(&used) {
                out.push(used);
            }
        }
        offset = member_start;
    }
    if out.is_empty() {
        out.push(binding.to_string());
    }
    out
}
