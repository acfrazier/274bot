//! Case selection: frozen `quick`/`full`/`smart`/`--only` semantics over the manifest.
//!
//! The semantics come from the frozen reference `manifestQuery.ts`/`runner.ts`:
//!   * `quick` — non-manual, non-broken, `vetted` cases only (the default level);
//!   * `full` — every non-manual, non-broken case (`vetted`/`documented`/`unvetted`);
//!   * `smart` — runnable cases reachable from the changed paths;
//!   * `--only` — replaces level selection and reaches every case (manual and broken
//!     included) whose id, harness or mapped reference id contains any comma-separated
//!     substring. Multiple matches are intentional.
//!
//! Selection is *metadata selection*: it never grants execution. A selected case with
//! no native adapter, or one declared unavailable, is recorded with its explicit
//! reason and is not substituted by anything else.

use super::manifest::{CaseEntry, CaseKind, CaseStatus, SuiteManifest};
use super::SuiteResult;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Quick,
    Full,
    Smart,
}

impl Level {
    pub fn parse(value: &str) -> SuiteResult<Self> {
        match value {
            "quick" => Ok(Level::Quick),
            "full" => Ok(Level::Full),
            "smart" => Ok(Level::Smart),
            other => Err(format!(
                "unknown level {other:?}: expected quick, full or smart"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Quick => "quick",
            Level::Full => "full",
            Level::Smart => "smart",
        }
    }
}

/// How the changed-path set for `smart` was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangedSource {
    /// Paths supplied on the command line (deterministic, no repository needed).
    Supplied,
    /// Discovered from `git diff --name-only` against the merge base plus the worktree.
    Git,
    /// No repository and no supplied paths: `smart` selects nothing.
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub level: Option<Level>,
    pub only: Vec<String>,
    pub cases: Vec<String>,
    pub why: String,
    pub changed: Vec<String>,
    pub changed_source: ChangedSource,
    /// Selected ids that cannot execute, with the reason (no substitution, no silently
    /// dropped case).
    pub unavailable: Vec<(String, String)>,
}

impl Selection {
    pub fn runnable<'a>(&self, manifest: &'a SuiteManifest) -> Vec<&'a CaseEntry> {
        self.cases
            .iter()
            .filter_map(|id| manifest.case(id))
            .filter(|case| case.is_runnable())
            .collect()
    }
}

pub struct SelectRequest<'a> {
    pub level: Level,
    pub only: &'a [String],
    /// `Some(paths)` uses exactly these; `None` discovers them from git.
    pub changed_paths: Option<&'a [String]>,
    pub repo_root: Option<std::path::PathBuf>,
}

pub fn select(manifest: &SuiteManifest, request: &SelectRequest<'_>) -> SuiteResult<Selection> {
    let only: Vec<String> = request
        .only
        .iter()
        .map(|needle| needle.trim().to_string())
        .filter(|needle| !needle.is_empty())
        .collect();

    let (cases, why) = if !only.is_empty() {
        // `--only` replaces level selection entirely, and reaches manual/broken rows.
        let picked: Vec<&CaseEntry> = manifest
            .cases
            .iter()
            .filter(|case| only.iter().any(|needle| case.matches_only(needle)))
            .collect();
        let missed: Vec<&String> = only
            .iter()
            .filter(|needle| !manifest.cases.iter().any(|case| case.matches_only(needle)))
            .collect();
        let why = if picked.is_empty() {
            format!("--only {} matched no case in the manifest", only.join(","))
        } else if missed.is_empty() {
            format!("--only {} matched {} case(s)", only.join(","), picked.len())
        } else {
            format!(
                "--only {} matched {} case(s); no case matched {}",
                only.join(","),
                picked.len(),
                missed
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        (picked, why)
    } else {
        match request.level {
            Level::Quick | Level::Full => {
                let picked: Vec<&CaseEntry> = manifest
                    .cases
                    .iter()
                    .filter(|case| selected_by_level(case, request.level))
                    .collect();
                let why = format!(
                    "{}: {} case(s) from the manifest",
                    request.level.as_str(),
                    picked.len()
                );
                (picked, why)
            }
            Level::Smart => {
                let (changed, source) = changed_paths(manifest, request)?;
                let (picked, why) = select_by_changes(manifest, &changed, &source);
                return Ok(finish(
                    picked,
                    why,
                    Some(Level::Smart),
                    only,
                    changed,
                    source,
                ));
            }
        }
    };

    let changed = Vec::new();
    let source = ChangedSource::Supplied;
    Ok(finish(
        cases,
        why,
        Some(request.level),
        only,
        changed,
        source,
    ))
}

fn finish(
    cases: Vec<&CaseEntry>,
    why: String,
    level: Option<Level>,
    only: Vec<String>,
    changed: Vec<String>,
    changed_source: ChangedSource,
) -> Selection {
    let unavailable: Vec<(String, String)> = cases
        .iter()
        .filter(|case| !case.is_runnable())
        .map(|case| {
            let reason = case
                .unavailable
                .as_ref()
                .map(|u| format!("{}: {}", u.code, u.reason))
                .unwrap_or_else(|| "no native adapter".to_string());
            (case.id.clone(), reason)
        })
        .collect();
    Selection {
        level,
        only,
        cases: cases.iter().map(|case| case.id.clone()).collect(),
        why,
        changed,
        changed_source,
        unavailable,
    }
}

fn selected_by_level(case: &CaseEntry, level: Level) -> bool {
    if case.manual {
        return false;
    }
    match level {
        Level::Quick => case.selection_status() == CaseStatus::Vetted,
        Level::Full => case.selection_status() != CaseStatus::Broken,
        Level::Smart => true,
    }
}

/// Discover the changed-path set for `smart`: supplied paths win; otherwise git.
fn changed_paths(
    _manifest: &SuiteManifest,
    request: &SelectRequest<'_>,
) -> SuiteResult<(Vec<String>, ChangedSource)> {
    if let Some(paths) = request.changed_paths {
        return Ok((normalize_paths(paths), ChangedSource::Supplied));
    }
    let root = request.repo_root.clone().or_else(manifest_repo_root);
    let Some(root) = root else {
        return Ok((
            Vec::new(),
            ChangedSource::Unavailable("no repository root and no --changed-path supplied".into()),
        ));
    };
    match git_changed_paths(&root) {
        Ok(paths) => Ok((normalize_paths(&paths), ChangedSource::Git)),
        Err(error) => Ok((Vec::new(), ChangedSource::Unavailable(error))),
    }
}

fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for path in paths {
        let trimmed = path.trim().trim_start_matches("./");
        if !trimmed.is_empty() {
            seen.insert(trimmed.to_string());
        }
    }
    seen.into_iter().collect()
}

fn manifest_repo_root() -> Option<std::path::PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn git_changed_paths(root: &std::path::Path) -> Result<Vec<String>, String> {
    let base = String::from("origin/main");
    let mut out = Vec::new();
    for args in [
        vec!["diff", "--name-only", &base],
        vec!["diff", "--name-only"],
    ] {
        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
        if !output.status.success() && args.contains(&base.as_str()) {
            // No `origin/main` in this checkout: fall back to the worktree diff only.
            continue;
        }
        out.extend(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty()),
        );
    }
    Ok(out)
}

/// Native `smart` coverage map. It is explicit data (see `smart` in the manifest) and is
/// intentionally *not* a translation of the foreign `src/bot/**` paths:
///   * shared paths  — adapter/runtime/api seams: every runnable case is reachable;
///   * subsystem paths — select the cases declaring that subsystem;
///   * per-case paths — select that case;
///   * ignored paths — recorded, select nothing (the reference ignores its own harness
///     tree the same way).
fn select_by_changes<'a>(
    manifest: &'a SuiteManifest,
    changed: &[String],
    source: &ChangedSource,
) -> (Vec<&'a CaseEntry>, String) {
    let runnable: Vec<&CaseEntry> = manifest
        .cases
        .iter()
        .filter(|case| {
            !case.manual
                && case.selection_status() != CaseStatus::Broken
                && case.kind == CaseKind::Native
        })
        .collect();

    if changed.is_empty() {
        let why = match source {
            ChangedSource::Unavailable(reason) => format!("smart: no changed paths ({reason})"),
            ChangedSource::Git => "smart: no changes against origin/main or the worktree".into(),
            ChangedSource::Supplied => "smart: no changed paths supplied".into(),
        };
        return (Vec::new(), why);
    }

    if let Some(path) = changed
        .iter()
        .find(|path| starts_with_any(path, &manifest.smart.shared_paths))
    {
        let why = format!(
            "{} (matched {path})",
            manifest.smart.shared_reason.trim_end_matches('.')
        );
        return (runnable, why);
    }

    let mut subsystems: BTreeSet<&str> = BTreeSet::new();
    let mut ignored = Vec::new();
    for path in changed {
        if starts_with_any(path, &manifest.smart.ignored_paths) {
            ignored.push(path.clone());
            continue;
        }
        if let Some((_, subsystem)) = manifest
            .smart
            .subsystem_paths
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix.as_str()))
        {
            subsystems.insert(subsystem.as_str());
        }
    }

    let picked: Vec<&CaseEntry> = runnable
        .into_iter()
        .filter(|case| {
            case.covers
                .subsystems
                .iter()
                .any(|subsystem| subsystems.contains(subsystem.as_str()))
                || case
                    .covers
                    .paths
                    .iter()
                    .any(|prefix| changed.iter().any(|path| path.starts_with(prefix.as_str())))
        })
        .collect();

    let why = format!(
        "{} changed file(s) matched {} case(s){}",
        changed.len() - ignored.len(),
        picked.len(),
        if ignored.is_empty() {
            String::new()
        } else {
            format!(" ({} ignored)", ignored.len())
        }
    );
    (picked, why)
}

fn starts_with_any(path: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|prefix| !prefix.is_empty() && path.starts_with(prefix.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> SuiteManifest {
        SuiteManifest::parse(super::super::EMBEDDED_MANIFEST.as_bytes(), "embedded").unwrap()
    }

    fn ids(selection: &Selection) -> Vec<&str> {
        selection.cases.iter().map(|id| id.as_str()).collect()
    }

    fn run(level: Level, only: &[String], changed: Option<&[&str]>) -> Selection {
        let manifest = manifest();
        let changed: Option<Vec<String>> =
            changed.map(|paths| paths.iter().map(|p| p.to_string()).collect());
        select(
            &manifest,
            &SelectRequest {
                level,
                only,
                changed_paths: changed.as_deref(),
                repo_root: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn quick_selects_vetted_non_manual_cases_in_manifest_order() {
        let selection = run(Level::Quick, &[], None);
        let manifest = manifest();
        assert!(!selection.cases.is_empty());
        let expected: Vec<String> = manifest
            .cases
            .iter()
            .filter(|c| !c.manual && c.selection_status() == CaseStatus::Vetted)
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(selection.cases, expected, "stable manifest order");
        for id in ids(&selection) {
            let case = manifest.case(id).unwrap();
            assert!(!case.manual);
            assert_eq!(case.selection_status(), CaseStatus::Vetted);
        }
        // The vetted native cases with adapters are the runnable subset.
        assert!(selection.cases.contains(&"ardy_fighter".to_string()));
        assert!(selection.cases.contains(&"nature_crafter_air".to_string()));
    }

    #[test]
    fn full_adds_documented_and_unvetted_but_never_manual_or_broken() {
        let full = run(Level::Full, &[], None);
        let quick = run(Level::Quick, &[], None);
        assert!(full.cases.len() > quick.cases.len());
        let manifest = manifest();
        for id in ids(&full) {
            let case = manifest.case(id).unwrap();
            assert!(
                !case.manual,
                "{id} is manual and must not be level-selected"
            );
            assert_ne!(
                case.selection_status(),
                CaseStatus::Broken,
                "{id} is broken"
            );
        }
        assert!(
            !full.cases.contains(&"flax_aio".to_string()),
            "broken stays out of full"
        );
        assert!(
            !full.cases.contains(&"chicken_killer".to_string()),
            "manual stays out of full"
        );
        assert!(full.cases.contains(&"thiever".to_string()));
    }

    #[test]
    fn only_is_substring_selection_over_ids_harnesses_and_reference_cases() {
        // Manual and broken rows are reachable through --only.
        let selection = run(Level::Quick, &["flax".to_string()], None);
        assert!(selection.cases.contains(&"flax_aio".to_string()));
        assert!(selection.cases.contains(&"flax_aio_spin".to_string()));
        assert!(selection.cases.contains(&"flax_picker".to_string()));

        // Multiple substring matches are intentional, not an ambiguity error.
        let selection = run(
            Level::Quick,
            &["thiever".to_string(), "ardy".to_string()],
            None,
        );
        assert!(selection.cases.contains(&"thiever".to_string()));
        assert!(selection.cases.contains(&"ardy_fighter".to_string()));
        assert!(selection.cases.contains(&"ardy_cakes".to_string()));

        // A mapped reference id selects the native case that adapts it.
        let selection = run(
            Level::Quick,
            &["ardyfighter-restock-loop-live".to_string()],
            None,
        );
        assert_eq!(selection.cases, vec!["ardy_fighter".to_string()]);

        // A broken reference case is reachable by --only and stays marked broken.
        let selection = run(Level::Quick, &["hosted".to_string()], None);
        assert!(selection.cases.contains(&"chicken_killer".to_string()));

        // No match is reported clearly and selects nothing.
        let selection = run(Level::Quick, &["does-not-exist".to_string()], None);
        assert!(selection.cases.is_empty());
        assert!(selection.why.contains("matched no case"));
    }

    #[test]
    fn smart_shared_paths_select_every_runnable_case() {
        let selection = run(Level::Smart, &[], Some(&["crates/host-play/src/lib.rs"]));
        assert!(
            selection.why.contains("shared code changed"),
            "{}",
            selection.why
        );
        assert!(selection.cases.contains(&"thiever".to_string()));
        assert!(selection.cases.contains(&"ardy_fighter".to_string()));
        // Manual and broken rows are not level-selected by smart either.
        assert!(!selection.cases.contains(&"chicken_killer".to_string()));
        assert!(!selection.cases.contains(&"flax_aio".to_string()));
    }

    #[test]
    fn smart_maps_rust_paths_explicitly_and_ignores_its_own_tree() {
        let ignored = run(Level::Smart, &[], Some(&["crates/e2e/src/suite/mod.rs"]));
        assert!(
            ignored.cases.is_empty(),
            "the suite's own tree selects nothing"
        );
        assert!(ignored.why.contains("ignored"), "{}", ignored.why);

        let docs = run(
            Level::Smart,
            &[],
            Some(&["docs/compat/p3-runner-design.md"]),
        );
        assert!(docs.cases.is_empty());

        let unknown = run(Level::Smart, &[], Some(&["crates/unknown/thing.rs"]));
        assert!(unknown.cases.is_empty());
        assert!(unknown.why.contains("matched 0 case"));

        let no_changes = run(Level::Smart, &[], Some(&[]));
        assert!(no_changes.cases.is_empty());
        assert!(no_changes.why.contains("no changed paths supplied"));
    }

    #[test]
    fn selection_reports_unavailable_rows_instead_of_substituting() {
        let manifest = manifest();
        let selection = run(Level::Full, &[], None);
        let unavailable: Vec<&str> = selection
            .unavailable
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(unavailable.contains(&"banksorter-live"), "{unavailable:?}");
        assert!(unavailable.contains(&"duel_arena"));
        for (id, reason) in &selection.unavailable {
            assert!(!reason.trim().is_empty(), "{id}");
            let case = manifest.case(id).unwrap();
            assert!(!case.is_runnable());
        }
        // An unavailable row is never silently replaced by another case.
        assert_eq!(
            selection.cases.len(),
            selection.runnable(&manifest).len() + selection.unavailable.len()
        );
    }
}
