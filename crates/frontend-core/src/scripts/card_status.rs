use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use script_js::{JsCard, JsLibrary};

const UNVERIFIED: &str = "unverified on 274bot";
static VERIFIED: LazyLock<VerifiedCatalog<'static>> =
    LazyLock::new(|| VerifiedCatalog::parse(option_env!("BOT_VERIFIED_CATALOG").unwrap_or("")));

struct VerifiedCatalog<'a>(HashSet<(&'a str, &'a str)>);

impl<'a> VerifiedCatalog<'a> {
    fn parse(rows: &'a str) -> Self {
        Self(
            rows.lines()
                .filter_map(|row| row.split_once('\t'))
                .filter(|(sha, name)| {
                    sha.len() == 64
                        && sha
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        && !name.is_empty()
                        && !name.contains(['\t', '\r'])
                })
                .collect(),
        )
    }

    fn contains(&self, name: &str, sha: &str) -> bool {
        self.0.contains(&(sha, name))
    }
}

/// The shared status line for a script card. Verification never gates Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardStatus {
    line: Cow<'static, str>,
    detail: Option<String>,
    blocks_start: bool,
    error: bool,
}

impl CardStatus {
    pub(super) fn for_loaded(
        card: &JsCard,
        library: &JsLibrary,
        include_details: bool,
    ) -> Option<Self> {
        let unverified = card.source == script::ScriptSource::Catalog
            && !VERIFIED.contains(&card.name, &card.sha256);
        let failure = if library.load_failure_count() != 0
            && (include_details || card.unloadable.is_none())
        {
            library.load_failure(&card.identity_key())
        } else {
            None
        };
        let detail = failure
            .filter(|_| include_details)
            .map(script_js::LoadFailure::named_line);
        if let Some(reason) = &card.unloadable {
            let line = if unverified {
                format!("{UNVERIFIED} — {reason}")
            } else {
                reason.line()
            };
            return Some(Self {
                line: Cow::Owned(line),
                detail,
                blocks_start: true,
                error: true,
            });
        }
        if let Some(failure) = failure {
            let stage = failure.stage.as_str();
            let diagnostic = failure
                .diagnostic
                .lines()
                .next()
                .unwrap_or(&failure.diagnostic);
            let line = if unverified {
                format!("{UNVERIFIED} — failed {stage}: {diagnostic}")
            } else {
                format!("failed {stage}: {diagnostic}")
            };
            return Some(Self {
                line: Cow::Owned(line),
                detail,
                blocks_start: false,
                error: true,
            });
        }
        unverified.then_some(Self {
            line: Cow::Borrowed(UNVERIFIED),
            detail: None,
            blocks_start: false,
            error: false,
        })
    }

    /// User-facing text for the card's status line.
    pub fn line(&self) -> &str {
        &self.line
    }

    /// The retained location and full diagnostic, if requested.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The script has an unresolved import.
    pub fn blocks_start(&self) -> bool {
        self.blocks_start
    }

    /// The status includes a load failure, not just a verification notice.
    pub fn is_error(&self) -> bool {
        self.error
    }
}

#[cfg(test)]
mod tests {
    use super::VerifiedCatalog;

    #[test]
    fn proof_is_for_the_exact_picker_name_and_source() {
        const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        const OTHER_SHA: &str = "f123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let rows = format!("{SHA}\tDuel Arena Combat Trainer\ninvalid\tWoodcutter\n");
        let list = VerifiedCatalog::parse(&rows);
        assert!(list.contains("Duel Arena Combat Trainer", SHA));
        assert!(!list.contains("Duel Arena Combat Trainer", OTHER_SHA));
        assert!(!list.contains("Other Card", SHA));
        assert!(!list.contains("Woodcutter", "invalid"));
        assert!(!VerifiedCatalog::parse("").contains("Duel Arena Combat Trainer", SHA));
    }
}
