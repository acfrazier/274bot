//! Content-derived debugproc and engine-cheat metadata.

use client::io::ClientRevision;
use serde::Deserialize;

const MAX_COMMAND_BYTES: usize = 80;

/// An error while loading or decoding the opt-in debug catalog.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DebugCatalogError {
    /// This build embeds no local debug catalog.
    #[error("Debug catalog local data is unavailable")]
    LocalDataUnavailable,
    /// The catalog bytes failed to decode as JSON.
    #[error("{message}")]
    Decode {
        /// The revision whose catalog failed.
        revision: ClientRevision,
        /// The exact decoder message.
        message: std::sync::Arc<str>,
    },
    /// The decoded catalog declares an unsupported schema version.
    #[error("Debug catalog schema mismatch")]
    SchemaMismatch,
}

const DEBUG_ARGUMENT_KINDS: &[&str] = &[
    "int",
    "string",
    "obj",
    "npc",
    "loc",
    "seq",
    "spotanim",
    "interface",
    "stat",
    "varp",
    "inv",
    "idkit",
    "namedobj",
    "coord",
];

/// One positional value accepted by a debug command.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugArgument {
    pub name: String,
    pub kind: String,
    pub optional: bool,
}

/// One content debugproc or engine cheat exposed by the DebugPanel.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugCommand {
    /// The exact wire token. Content debugprocs retain their `~` prefix.
    pub name: String,
    pub category: String,
    pub description: String,
    pub args: Vec<DebugArgument>,
    pub destructive: bool,
    pub production_only: bool,
}

/// One selected-content name usable by a typed debug argument picker.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugName {
    pub id: i32,
    pub alias: String,
    pub name: String,
}

/// Separately loaded catalog; the panel owns its lifetime.
#[derive(Debug)]
pub struct DebugCatalog {
    revision: ClientRevision,
    debug_commands: Vec<DebugCommand>,
    debug_names: std::collections::HashMap<String, Vec<DebugName>>,
    items: Option<std::sync::Arc<crate::game_data::SelectedGameData>>,
}

impl DebugCatalog {
    /// Decode only on explicit Local Debug-tab demand.
    #[cfg(all(
        feature = "debug-catalog",
        feature = "embedded-game-data",
        not(no_local_game_data)
    ))]
    pub fn load(
        revision: ClientRevision,
        items: std::sync::Arc<crate::game_data::SelectedGameData>,
    ) -> Result<Self, DebugCatalogError> {
        let bytes: &[u8] = match revision {
            ClientRevision::R274 => include_bytes!("../data/game-data/274/debug.json"),
            ClientRevision::R289 => include_bytes!("../data/game-data/289/debug.json"),
            ClientRevision::R377 => return Err(DebugCatalogError::LocalDataUnavailable),
        };
        Self::decode(bytes, revision, items)
    }

    #[cfg(all(
        feature = "debug-catalog",
        not(all(feature = "embedded-game-data", not(no_local_game_data)))
    ))]
    pub fn load(
        _revision: ClientRevision,
        _items: std::sync::Arc<crate::game_data::SelectedGameData>,
    ) -> Result<Self, DebugCatalogError> {
        Err(DebugCatalogError::LocalDataUnavailable)
    }

    pub fn decode(
        bytes: &[u8],
        revision: ClientRevision,
        items: std::sync::Arc<crate::game_data::SelectedGameData>,
    ) -> Result<Self, DebugCatalogError> {
        #[derive(Deserialize)]
        struct WireCatalog {
            schema_version: u16,
            #[serde(rename = "revision")]
            _revision: i32,
            debug_commands: Vec<DebugCommand>,
            debug_names: std::collections::HashMap<String, Vec<DebugName>>,
        }

        let catalog: WireCatalog =
            serde_json::from_slice(bytes).map_err(|error| DebugCatalogError::Decode {
                revision,
                message: std::sync::Arc::from(error.to_string()),
            })?;
        if catalog.schema_version != 1 {
            return Err(DebugCatalogError::SchemaMismatch);
        }
        Ok(Self {
            revision,
            debug_commands: catalog.debug_commands,
            debug_names: catalog.debug_names,
            items: Some(items),
        })
    }

    /// The caller-selected client revision used for this catalog.
    pub fn revision(&self) -> ClientRevision {
        self.revision
    }

    pub fn commands(&self) -> &[DebugCommand] {
        &self.debug_commands
    }

    /// Exact identity first, then display names, prefixes and substrings.
    /// Each pass is bounded; only returned rows are copied.
    pub fn search_names(&self, kind: &str, query: &str, limit: usize) -> Vec<DebugName> {
        let kind = kind.trim().to_ascii_lowercase();
        if kind == "obj" || kind == "namedobj" {
            return self.items.as_ref().map_or_else(Vec::new, |items| {
                search_names(
                    items.items().iter().filter_map(|item| {
                        let alias = item.alias.as_deref()?;
                        Some((item.id, alias, item.name.as_deref().unwrap_or(alias)))
                    }),
                    query,
                    limit,
                )
            });
        }
        let family = if kind == "varbit" { "varp" } else { &kind };
        let Some(rows) = self.debug_names.get(family) else {
            return Vec::new();
        };
        search_names(
            rows.iter()
                .map(|row| (row.id, row.alias.as_str(), row.name.as_str())),
            query,
            limit,
        )
    }
}

fn search_names<'a>(
    rows: impl Iterator<Item = (i32, &'a str, &'a str)> + Clone,
    query: &str,
    limit: usize,
) -> Vec<DebugName> {
    let wanted = query.trim();
    if wanted.is_empty() || limit == 0 {
        return Vec::new();
    }
    let needle = wanted.as_bytes();
    let numeric = wanted.parse::<i32>().ok();
    let mut hits = Vec::new();
    for rank in 0..4 {
        for (id, alias, name) in rows.clone() {
            let starts = |text: &[u8]| {
                text.get(..needle.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(needle))
            };
            let contains = |text: &[u8]| {
                text.windows(needle.len())
                    .any(|s| s.eq_ignore_ascii_case(needle))
            };
            let exact = alias.eq_ignore_ascii_case(wanted) || numeric == Some(id);
            let named = name.eq_ignore_ascii_case(wanted);
            let matches = match rank {
                0 => exact,
                1 => !exact && named,
                2 => !exact && !named && (starts(alias.as_bytes()) || starts(name.as_bytes())),
                _ => {
                    !exact
                        && !named
                        && !starts(alias.as_bytes())
                        && !starts(name.as_bytes())
                        && (contains(alias.as_bytes()) || contains(name.as_bytes()))
                }
            };
            if matches {
                hits.push(DebugName {
                    id,
                    alias: alias.into(),
                    name: name.into(),
                });
                if hits.len() == limit {
                    return hits;
                }
            }
        }
    }
    hits
}

impl DebugCommand {
    /// Compose the exact cheat body and reject malformed shapes before it can
    /// reach the single existing CLIENT_CHEAT path.
    pub fn format_command(&self, values: &[String]) -> Result<String, String> {
        if self.name.is_empty() {
            return Err("debug command has no wire name".to_string());
        }
        if !self.name.is_ascii()
            || self
                .name
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(format!("{} has an invalid wire name", self.name));
        }
        for argument in &self.args {
            if !DEBUG_ARGUMENT_KINDS.contains(&argument.kind.as_str()) {
                return Err(format!(
                    "{} has unsupported argument kind {}",
                    self.name, argument.kind
                ));
            }
        }
        if values.len() > self.args.len() {
            return Err(format!(
                "{} expects at most {} argument(s), got {}",
                self.name,
                self.args.len(),
                values.len()
            ));
        }
        let mut value_count = values.len();
        while value_count > 0
            && values[value_count - 1].is_empty()
            && self.args[value_count - 1].optional
        {
            value_count -= 1;
        }
        for (index, argument) in self.args.iter().enumerate() {
            if index >= value_count {
                if !argument.optional {
                    return Err(format!(
                        "{} argument {} is required at position {}",
                        self.name,
                        argument.name,
                        index + 1
                    ));
                }
                continue;
            }
            let value = &values[index];
            if value.is_empty() {
                return Err(format!("{} argument {} is empty", self.name, argument.name));
            }
            if !value.is_ascii()
                || value
                    .chars()
                    .any(|character| character.is_control() || character.is_whitespace())
            {
                return Err(format!(
                    "{} argument {} is not a single ASCII token",
                    self.name, argument.name
                ));
            }
            if argument.kind == "int" && value.parse::<i32>().is_err() {
                return Err(format!(
                    "{} argument {} is not a signed 32-bit integer",
                    self.name, argument.name
                ));
            }
        }
        let mut command = self.name.clone();
        for value in values.iter().take(value_count) {
            command.push(' ');
            command.push_str(value);
        }
        if command.len() > MAX_COMMAND_BYTES {
            return Err(format!(
                "{} is {} bytes; debug cheats are limited to {} bytes",
                self.name,
                command.len(),
                MAX_COMMAND_BYTES
            ));
        }
        Ok(command)
    }
}
#[cfg(test)]
mod tests {
    use super::{DebugArgument, DebugCommand};

    #[test]
    fn decoded_catalog_uses_the_caller_selected_revision() {
        use client::io::ClientRevision;

        let bytes = br#"{"schema_version":1,"revision":289,"debug_commands":[],"debug_names":{}}"#;
        let items = std::sync::Arc::new(
            serde_json::from_slice::<crate::game_data::SelectedGameData>(
                br#"{"schema_version":5,"revision":274,"items":[],"consumption":[],"pickpocket":[]}"#,
            )
            .unwrap(),
        );
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let catalog = super::DebugCatalog::decode(bytes, revision, items.clone())
                .expect("debug facts decode under the caller-selected revision");
            assert_eq!(catalog.revision(), revision);

        }
    }

    #[cfg(feature = "debug-catalog")]
    #[test]
    fn debug_catalog_load_refuses_revision_377() {
        let Some(items) = crate::game_data::test_data(ClientRevision::R289) else {
            return;
        };
        let error = super::DebugCatalog::load(ClientRevision::R377, items).unwrap_err();
        assert_eq!(error, super::DebugCatalogError::LocalDataUnavailable);
    }

    fn command(args: Vec<DebugArgument>) -> DebugCommand {
        DebugCommand {
            name: "~demo".to_string(),
            category: "Account".to_string(),
            description: String::new(),
            args,
            destructive: false,
            production_only: false,
        }
    }

    #[test]
    fn formats_required_and_optional_values_in_wire_order() {
        let debug = command(vec![
            DebugArgument {
                name: "stat".into(),
                kind: "stat".into(),
                optional: false,
            },
            DebugArgument {
                name: "level".into(),
                kind: "int".into(),
                optional: true,
            },
        ]);
        assert_eq!(
            debug.format_command(&["attack".into()]).unwrap(),
            "~demo attack"
        );
        assert_eq!(
            debug
                .format_command(&["attack".into(), "42".into()])
                .unwrap(),
            "~demo attack 42"
        );
        assert_eq!(
            debug
                .format_command(&["attack".into(), String::new()])
                .unwrap(),
            "~demo attack"
        );
        assert!(debug.format_command(&[]).is_err());
        assert!(debug
            .format_command(&["attack".into(), "42".into(), "extra".into()])
            .is_err());
    }

    #[test]
    fn rejects_unknown_kind_empty_values_and_line_breaks() {
        let unknown = command(vec![DebugArgument {
            name: "value".into(),
            kind: "future".into(),
            optional: true,
        }]);
        assert!(unknown.format_command(&[]).is_err());
        let debug = command(vec![DebugArgument {
            name: "value".into(),
            kind: "string".into(),
            optional: false,
        }]);
        assert!(debug.format_command(&[" ".into()]).is_err());
        assert!(debug.format_command(&["line\nbreak".into()]).is_err());
    }

    #[test]
    fn rejects_invalid_integer_tokens_and_optional_gaps() {
        let integer = command(vec![DebugArgument {
            name: "value".into(),
            kind: "int".into(),
            optional: false,
        }]);
        assert!(integer.format_command(&["not-int".into()]).is_err());
        assert!(integer.format_command(&["1 2".into()]).is_err());
        assert!(integer.format_command(&["é".into()]).is_err());
        let gap = command(vec![
            DebugArgument {
                name: "obj".into(),
                kind: "obj".into(),
                optional: false,
            },
            DebugArgument {
                name: "count".into(),
                kind: "int".into(),
                optional: true,
            },
            DebugArgument {
                name: "tag".into(),
                kind: "string".into(),
                optional: true,
            },
        ]);
        assert!(gap
            .format_command(&["coins".into(), String::new(), "x".into()])
            .is_err());
    }

    #[test]
    fn enforces_eighty_byte_wire_limit_not_character_count() {
        let debug = command(vec![DebugArgument {
            name: "value".into(),
            kind: "string".into(),
            optional: false,
        }]);
        let short = "a".repeat(74);
        assert!(debug.format_command(&[short]).is_ok());
        let multibyte = "é".repeat(40);
        assert!(debug.format_command(&[multibyte]).is_err());
    }
}

#[cfg(all(
    test,
    feature = "debug-catalog",
    not(all(feature = "embedded-game-data", not(no_local_game_data)))
))]
mod missing_data_tests {
    use super::*;

    #[test]
    fn unavailable_build_reports_missing_local_catalog_data() {
        let items = std::sync::Arc::new(
            serde_json::from_slice::<crate::game_data::SelectedGameData>(
                br#"{"schema_version":5,"revision":274,"items":[],"consumption":[],"pickpocket":[]}"#,
            )
            .unwrap(),
        );
        assert!(matches!(
            DebugCatalog::load(ClientRevision::R289, items),
            Err(DebugCatalogError::LocalDataUnavailable)
        ));
    }
}
