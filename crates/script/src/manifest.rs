//! Language-neutral metadata for native script declarations.

use crate::card::{ItemOptionCandidate, ItemOptionSpec, SettingDef};
use std::iter::FromIterator;
use std::mem::size_of;

/// A host capability that a script can declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostCapability {
    /// Permission to use the host packet-protocol API.
    HostPacketProtocol,
}

/// Compact immutable declarations of host capabilities.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredHostCapabilities(u8);

impl DeclaredHostCapabilities {
    /// An empty set of declared host capabilities.
    pub const EMPTY: Self = Self(0);

    /// Returns true when `capability` is present in this set.
    pub const fn contains(self, capability: HostCapability) -> bool {
        let bit = match capability {
            HostCapability::HostPacketProtocol => 1,
        };
        self.0 & bit != 0
    }

    fn insert(&mut self, capability: HostCapability) {
        self.0 |= match capability {
            HostCapability::HostPacketProtocol => 1,
        };
    }
}

impl FromIterator<HostCapability> for DeclaredHostCapabilities {
    fn from_iter<T: IntoIterator<Item = HostCapability>>(iter: T) -> Self {
        let mut capabilities = Self::EMPTY;
        for capability in iter {
            capabilities.insert(capability);
        }
        capabilities
    }
}

/// Descriptive string fields of a [`ScriptManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestText {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub category: String,
}

/// Validated, language-neutral metadata extracted from a native script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptManifest {
    api_version: u32,
    name: String,
    version: String,
    author: String,
    description: String,
    category: String,
    tags: Vec<String>,
    settings_schema: Vec<SettingDef>,
    capabilities: DeclaredHostCapabilities,
}

impl ScriptManifest {
    /// Creates a manifest from values already validated by a language frontend.
    pub fn new(
        api_version: u32,
        text: ManifestText,
        tags: Vec<String>,
        settings_schema: Vec<SettingDef>,
        capabilities: DeclaredHostCapabilities,
    ) -> Self {
        let ManifestText {
            name,
            version,
            author,
            description,
            category,
        } = text;
        Self {
            api_version,
            name,
            version,
            author,
            description,
            category,
            tags,
            settings_schema,
            capabilities,
        }
    }

    /// Returns the declared script API version.
    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    /// Returns the picker display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the author's release label.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the author attribution.
    pub fn author(&self) -> &str {
        &self.author
    }

    /// Returns the plain-text script summary.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the open category label.
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns ordered, distinct search tags.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Returns the shared settings schema in declaration order.
    pub fn settings_schema(&self) -> &[SettingDef] {
        &self.settings_schema
    }

    /// Returns the compact declared capability set.
    pub fn capabilities(&self) -> DeclaredHostCapabilities {
        self.capabilities
    }

    /// Counts retained heap storage in this metadata graph.
    ///
    /// The result includes container capacities and string capacities. It does
    /// not include the allocator's private bookkeeping or the source text.
    pub fn retained_size_bytes(&self) -> usize {
        let mut bytes = size_of::<Self>();
        for value in [
            &self.name,
            &self.version,
            &self.author,
            &self.description,
            &self.category,
        ] {
            add(&mut bytes, value.capacity());
        }
        add_vec::<String>(&mut bytes, self.tags.capacity());
        for tag in &self.tags {
            add(&mut bytes, tag.capacity());
        }
        add_vec::<SettingDef>(&mut bytes, self.settings_schema.capacity());
        for setting in &self.settings_schema {
            add_setting_size(&mut bytes, setting);
        }
        bytes
    }
}

fn add_setting_size(bytes: &mut usize, setting: &SettingDef) {
    for value in [
        Some(&setting.id),
        Some(&setting.ty),
        setting.default.as_ref(),
        setting.label.as_ref(),
        setting.min.as_ref(),
        setting.max.as_ref(),
        setting.step.as_ref(),
        setting.group.as_ref(),
        setting.show_if.as_ref(),
        setting.options_from.as_ref(),
        setting.csv_toggle.as_ref(),
        setting.help.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        add(bytes, value.capacity());
    }
    add_vec::<String>(bytes, setting.options.capacity());
    for option in &setting.options {
        add(bytes, option.capacity());
    }
    add_vec::<String>(bytes, setting.option_labels.capacity());
    for label in &setting.option_labels {
        add(bytes, label.capacity());
    }
    if let Some(spec) = &setting.item_option_spec {
        add(bytes, size_of::<ItemOptionSpec>());
        add_vec::<String>(bytes, spec.prefix.capacity());
        for prefix in &spec.prefix {
            add(bytes, prefix.capacity());
        }
        add_vec::<ItemOptionCandidate>(bytes, spec.candidates.capacity());
        for candidate in &spec.candidates {
            add(bytes, candidate.key.capacity());
            if let Some(label) = &candidate.label {
                add(bytes, label.capacity());
            }
        }
    }
}

fn add_vec<T>(bytes: &mut usize, capacity: usize) {
    add(bytes, capacity.saturating_mul(size_of::<T>()));
}

fn add(bytes: &mut usize, value: usize) {
    *bytes = bytes.saturating_add(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(id: &str, default: &str) -> SettingDef {
        SettingDef {
            id: id.into(),
            ty: "string".into(),
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

    fn manifest(settings_schema: Vec<SettingDef>) -> ScriptManifest {
        ScriptManifest::new(
            2,
            ManifestText {
                name: "Miner".into(),
                version: "1".into(),
                author: "Alice".into(),
                description: "Mine ore".into(),
                category: "Mining".into(),
            },
            vec!["ore".into()],
            settings_schema,
            [HostCapability::HostPacketProtocol].into_iter().collect(),
        )
    }

    #[test]
    fn capability_set_is_compact_and_queryable() {
        let empty = DeclaredHostCapabilities::EMPTY;
        let declared: DeclaredHostCapabilities =
            [HostCapability::HostPacketProtocol].into_iter().collect();
        assert!(!empty.contains(HostCapability::HostPacketProtocol));
        assert!(declared.contains(HostCapability::HostPacketProtocol));
        assert_eq!(size_of::<DeclaredHostCapabilities>(), size_of::<u8>());
    }

    #[test]
    fn retained_size_counts_settings_graph_storage_not_just_metadata_text() {
        let minimal = manifest(Vec::new());
        let expanded = manifest(vec![setting("oreName", "iron ore")]);
        assert!(expanded.retained_size_bytes() > minimal.retained_size_bytes());
        assert!(expanded.retained_size_bytes() > expanded.name().len());
    }
}
