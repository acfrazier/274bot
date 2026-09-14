//! Stable script identity, distinct from content version and runtime generation.
//! Display name is never an identity or a same-name fallback.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use vault::{assignment_key, ScriptAssignment};

use crate::registry::CompiledId;
use crate::rs2b0t_registry::ScriptSource;

pub const NOTHING_CHANGED_RELOAD: &str = "Nothing changed; nothing to reload";
pub const NOTHING_CHANGED_CATALOG: &str = "Nothing changed.";

/// Kind string stored in vault assignments.
pub fn source_kind(source: ScriptSource) -> &'static str {
    match source {
        ScriptSource::Catalog => "catalog",
        ScriptSource::File => "file",
        ScriptSource::Builtin => "builtin",
    }
}

pub fn parse_source_kind(kind: &str) -> Option<ScriptSource> {
    match kind {
        "catalog" => Some(ScriptSource::Catalog),
        "file" => Some(ScriptSource::File),
        "builtin" => Some(ScriptSource::Builtin),
        _ => None,
    }
}

/// Stored file identity: canonical path when the file exists, otherwise the
/// given path as stored. Missing files keep their identity.
pub fn file_identity(path: &Path) -> String {
    match path.canonicalize() {
        Ok(canon) => canon.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

pub fn card_identity_id(source: ScriptSource, path: &Path, name: &str) -> String {
    match source {
        ScriptSource::File => file_identity(path),
        ScriptSource::Catalog | ScriptSource::Builtin => name.to_string(),
    }
}

pub fn card_identity_key(source: ScriptSource, path: &Path, name: &str) -> String {
    assignment_key(source_kind(source), &card_identity_id(source, path, name))
}

pub fn compiled_identity_key(id: CompiledId) -> String {
    assignment_key("compiled", id.0)
}

pub fn card_assignment(source: ScriptSource, path: &Path, name: &str) -> ScriptAssignment {
    ScriptAssignment {
        source_kind: source_kind(source).into(),
        identity: card_identity_id(source, path, name),
        display_name: name.to_string(),
        unavailable: None,
    }
}

pub fn compiled_assignment(id: CompiledId) -> ScriptAssignment {
    ScriptAssignment {
        source_kind: "compiled".into(),
        identity: id.0.to_string(),
        display_name: id.0.to_string(),
        unavailable: None,
    }
}

pub fn missing_file_assignment(
    path: &Path,
    display_name: &str,
    reason: impl Into<String>,
) -> ScriptAssignment {
    ScriptAssignment {
        source_kind: "file".into(),
        identity: file_identity(path),
        display_name: display_name.to_string(),
        unavailable: Some(reason.into()),
    }
}

pub fn paths_match(stored: &str, other: &Path) -> bool {
    if stored == other.to_string_lossy() {
        return true;
    }
    let stored_path = PathBuf::from(stored);
    if stored_path == other {
        return true;
    }
    match (stored_path.canonicalize(), other.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// SHA-256 of raw origin bytes (no transpile).
pub fn raw_sha(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Combined content version of an entry plus supported sibling raw hashes.
pub fn combine_fingerprints(entry_sha: &str, siblings: &[(String, String)]) -> String {
    let mut buf = String::from(entry_sha);
    let mut sibs = siblings.to_vec();
    sibs.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, sha) in sibs {
        buf.push('\n');
        buf.push_str(&rel);
        buf.push('=');
        buf.push_str(&sha);
    }
    raw_sha(buf.as_bytes())
}

/// SmithingBot persisted bar labels: Adamant/Rune → Adamantite/Runite.
/// Unknown values stay visible. Other cards are untouched.
pub fn migrate_legacy_setting_value(card_name: &str, id: &str, value: &Value) -> Value {
    let smithing = card_name == "SmithingBot" || card_name == "Smithing";
    if !smithing || id != "bar" {
        return value.clone();
    }
    match value.as_str() {
        Some("Adamant") => Value::String("Adamantite".into()),
        Some("Rune") => Value::String("Runite".into()),
        _ => value.clone(),
    }
}

pub fn migrate_overrides(card_name: &str, overrides: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in overrides {
        out.insert(k.clone(), migrate_legacy_setting_value(card_name, k, v));
    }
    out
}

/// Copy legacy per-card overrides into a profile map once. Idempotent: an
/// existing profile key is left alone so later edits stay isolated.
pub fn claim_legacy_overrides(
    profile: &mut vault::ProfileSettings,
    key: &str,
    card_name: &str,
    legacy: &Map<String, Value>,
) {
    if profile.script_settings.contains_key(key) {
        return;
    }
    profile
        .script_settings
        .insert(key.to_string(), migrate_overrides(card_name, legacy));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_stem_paths_are_distinct_identities() {
        let a = PathBuf::from("/tmp/one/bot.ts");
        let b = PathBuf::from("/tmp/two/bot.ts");
        assert_ne!(file_identity(&a), file_identity(&b));
        assert_eq!(
            assignment_key("file", &file_identity(&a)),
            "file:/tmp/one/bot.ts"
        );
    }

    #[test]
    fn missing_file_keeps_stored_path() {
        let path = PathBuf::from("/definitely/missing/274bot-identity.ts");
        assert_eq!(file_identity(&path), path.to_string_lossy());
        let asg = missing_file_assignment(&path, "identity", "missing file");
        assert_eq!(asg.identity, path.to_string_lossy());
        assert!(asg.unavailable.is_some());
        assert_eq!(asg.display_name, "identity");
    }

    #[test]
    fn smithing_migrates_only_bar_labels() {
        let mut bag = Map::new();
        bag.insert("bar".into(), Value::String("Adamant".into()));
        bag.insert("product".into(), Value::String("Rune".into()));
        bag.insert("other".into(), Value::String("Adamant".into()));
        let migrated = migrate_overrides("SmithingBot", &bag);
        assert_eq!(
            migrated.get("bar"),
            Some(&Value::String("Adamantite".into()))
        );
        assert_eq!(migrated.get("product"), Some(&Value::String("Rune".into())));
        let rune =
            migrate_legacy_setting_value("SmithingBot", "bar", &Value::String("Rune".into()));
        assert_eq!(rune, Value::String("Runite".into()));
        let unknown =
            migrate_legacy_setting_value("SmithingBot", "bar", &Value::String("Mithril".into()));
        assert_eq!(unknown, Value::String("Mithril".into()));
        let other_card =
            migrate_legacy_setting_value("Alcher", "bar", &Value::String("Adamant".into()));
        assert_eq!(other_card, Value::String("Adamant".into()));
    }

    #[test]
    fn claim_legacy_is_idempotent_and_isolated() {
        let mut profile = vault::ProfileSettings::default();
        let mut legacy = Map::new();
        legacy.insert("bar".into(), Value::String("Adamant".into()));
        claim_legacy_overrides(&mut profile, "catalog:SmithingBot", "SmithingBot", &legacy);
        claim_legacy_overrides(
            &mut profile,
            "catalog:SmithingBot",
            "SmithingBot",
            &Map::new(),
        );
        assert_eq!(
            profile
                .script_settings
                .get("catalog:SmithingBot")
                .and_then(|m| m.get("bar")),
            Some(&Value::String("Adamantite".into()))
        );
        profile
            .script_settings
            .get_mut("catalog:SmithingBot")
            .unwrap()
            .insert("bar".into(), Value::String("Iron".into()));
        let mut later = Map::new();
        later.insert("bar".into(), Value::String("Steel".into()));
        claim_legacy_overrides(&mut profile, "catalog:SmithingBot", "SmithingBot", &later);
        assert_eq!(
            profile
                .script_settings
                .get("catalog:SmithingBot")
                .and_then(|m| m.get("bar")),
            Some(&Value::String("Iron".into()))
        );
    }

    #[test]
    fn whitespace_changes_fingerprint() {
        let a = combine_fingerprints(&raw_sha(b"export default class A {}"), &[]);
        let b = combine_fingerprints(&raw_sha(b"export default class A {}\n"), &[]);
        assert_ne!(a, b);
        let same = combine_fingerprints(&raw_sha(b"export default class A {}"), &[]);
        assert_eq!(a, same);
    }
}
