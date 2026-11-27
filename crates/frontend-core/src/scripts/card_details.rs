//! The facts both pickers show for one card beyond its collapsed row:
//! where it came from, its release label, author, API family and declared
//! host capabilities. One model, built here, so the panel and the TUI show
//! the same rows in the same order and neither parses card metadata itself.

use super::{BrowseCard, CardSource};

/// The row labels of [`CardDetails`], in the order the rows are listed.
pub const DETAIL_SOURCE: &str = "Source";
pub const DETAIL_PATH: &str = "Path";
pub const DETAIL_VERSION: &str = "Version";
pub const DETAIL_AUTHOR: &str = "Author";
pub const DETAIL_API: &str = "API";
pub const DETAIL_DECLARES: &str = "Declares";

/// Labelled facts of one card. Declared capabilities are shown as
/// information; no row carries a permission control.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CardDetails {
    rows: Vec<(&'static str, String)>,
}

impl CardDetails {
    pub(super) fn of(card: BrowseCard<'_>) -> Self {
        let mut rows = vec![(
            DETAIL_SOURCE,
            match card.source() {
                CardSource::Catalog => "Catalog",
                CardSource::File => "File",
                CardSource::Builtin => "Built in",
            }
            .to_owned(),
        )];
        if let Some(path) = card.path() {
            rows.push((DETAIL_PATH, path.display().to_string()));
        }
        #[cfg(feature = "js")]
        if let Some(loaded) = card.loaded() {
            if let Some(version) = loaded.version() {
                rows.push((DETAIL_VERSION, version.to_owned()));
            }
            if let Some(author) = loaded.author() {
                rows.push((DETAIL_AUTHOR, author.to_owned()));
            }
            rows.push((
                DETAIL_API,
                match loaded.api_family() {
                    "v2" => "v2 (native)".to_owned(),
                    other => format!("{other} (rs2b0t compat)"),
                },
            ));
            let declared = loaded.declared_capabilities();
            rows.push((
                DETAIL_DECLARES,
                if declared.is_empty() {
                    "no host capabilities".to_owned()
                } else {
                    declared.join(", ")
                },
            ));
        }
        #[cfg(feature = "luau")]
        if let Some(manifest) = card.luau_manifest() {
            if !manifest.version().is_empty() {
                rows.push((DETAIL_VERSION, manifest.version().to_owned()));
            }
            if !manifest.author().is_empty() {
                rows.push((DETAIL_AUTHOR, manifest.author().to_owned()));
            }
            rows.push((DETAIL_API, format!("v{} (native)", manifest.api_version())));
            let declares = if manifest
                .capabilities()
                .contains(script::manifest::HostCapability::HostPacketProtocol)
            {
                "host packet protocol"
            } else {
                "no host capabilities"
            };
            rows.push((DETAIL_DECLARES, declares.to_owned()));
        }
        Self { rows }
    }

    /// The `(label, value)` rows in display order.
    pub fn rows(&self) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        self.rows
            .iter()
            .map(|(label, value)| (*label, value.as_str()))
    }

    /// The value of the row labelled `label`, if the card has one.
    pub fn value(&self, label: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|(row, _)| *row == label)
            .map(|(_, value)| value.as_str())
    }

    /// One `Label: value` line per row.
    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.rows
            .iter()
            .map(|(label, value)| format!("{label}: {value}"))
    }
}

/// Whether every whitespace-separated term of `query` appears, ASCII or
/// Unicode case-insensitively, in at least one of `fields`. An empty query
/// matches everything.
pub(super) fn matches_all_terms<'f>(
    query: &str,
    fields: impl Iterator<Item = &'f str> + Clone,
) -> bool {
    query.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        fields
            .clone()
            .any(|field| field.to_lowercase().contains(&term))
    })
}
