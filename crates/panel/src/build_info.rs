//! Deploy fingerprint: static release label plus rs2b0t git stamp.
//!
//! Visible line is `alpha 1 · e978193` (same ` · ` as the old 4.5c line).
//! Hover is the crate version (`0.1.0`), then full commit + `builtAt`.
//! Bump [`RELEASE`] by hand when the public name changes.

/// Public name on the dim line. Not derived from git or Cargo.toml.
pub const RELEASE: &str = "alpha 1";
/// Crate version (`Cargo.toml`), shown on hover.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Full 40-char SHA when known, else `"unknown"`.
pub const COMMIT: &str = env!("GIT_COMMIT");
/// First 7 chars of the SHA (or `"unknown"`).
pub const SHORT: &str = env!("GIT_COMMIT_SHORT");
/// Tree was dirty at `cargo build`.
pub const DIRTY: bool = matches!(env!("GIT_DIRTY").as_bytes(), b"1");
/// ISO-8601 UTC timestamp of the build, possibly empty.
pub const BUILT_AT: &str = env!("BUILD_TIME");

/// Git stamp, e.g. `e978193` or `e978193-dirty`.
pub fn git_stamp(short: &str, dirty: bool) -> String {
    if dirty {
        format!("{short}-dirty")
    } else {
        short.to_string()
    }
}

/// `alpha 1 · e978193`
pub fn line(release: &str, stamp: &str) -> String {
    format!("{release} · {stamp}")
}

/// Panel dim-line text.
pub fn build_line() -> String {
    line(RELEASE, &git_stamp(SHORT, DIRTY))
}

/// Hover: crate version, then rs2b0t commit / built.
pub fn tooltip(version: &str, commit: &str, dirty: bool, built_at: &str) -> String {
    let dirty_note = if dirty { " (dirty tree)" } else { "" };
    let when = if built_at.is_empty() { "—" } else { built_at };
    format!("{version}\ncommit {commit}{dirty_note}\nbuilt {when}")
}

pub fn build_tooltip() -> String {
    tooltip(VERSION, COMMIT, DIRTY, BUILT_AT)
}

#[cfg(test)]
mod tests {
    use super::{git_stamp, line, tooltip, RELEASE, SHORT, VERSION};

    #[test]
    fn git_stamp_is_short_or_short_dirty() {
        assert_eq!(git_stamp("e978193", false), "e978193");
        assert_eq!(git_stamp("e978193", true), "e978193-dirty");
        assert_eq!(git_stamp("unknown", false), "unknown");
    }

    #[test]
    fn line_matches_the_old_dot_format() {
        assert_eq!(line("alpha 1", "e978193"), "alpha 1 · e978193");
        assert_eq!(line(RELEASE, "e978193-dirty"), "alpha 1 · e978193-dirty");
    }

    #[test]
    fn tooltip_leads_with_crate_version() {
        assert_eq!(
            tooltip("0.1.0", "abcdef0123456789", false, "2026-08-29T12:00:00Z"),
            "0.1.0\ncommit abcdef0123456789\nbuilt 2026-08-29T12:00:00Z"
        );
        assert_eq!(
            tooltip("0.1.0", "abcdef0123456789", true, ""),
            "0.1.0\ncommit abcdef0123456789 (dirty tree)\nbuilt —"
        );
        assert_eq!(VERSION, "0.1.0");
        assert_eq!(RELEASE, "alpha 1");
    }

    #[test]
    fn baked_short_is_seven_or_unknown() {
        assert!(
            SHORT == "unknown" || SHORT.chars().count() == 7,
            "short must be 7 chars or unknown, got {SHORT:?}"
        );
    }
}
