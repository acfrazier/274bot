//! Frozen suite manifest: reference metadata plus the native adapter map.
//!
//! The manifest is *derived, tracked data* (`crates/e2e/fixtures/native-suite/`,
//! regenerate with `derive.py`). It carries the frozen 00d39a17 reference identity,
//! the reference case statuses/budgets/coverage as evidence, and one row per native
//! execution unit with its declared witness, options and gaps.
//!
//! Two row kinds exist:
//!   * `native` — an executable native case (core or pair) or an explicitly
//!     unavailable native case with a reason;
//!   * `reference` — a frozen reference case with no native adapter; retained
//!     visibly with the reason it does not run here.
//!
//! Nothing in this module claims native qualification: reference `vetted`/`provenAt`
//! are historical upstream evidence, and a native row's derived status is selection
//! metadata (see [`CaseEntry::selection_status`]).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SuiteResult;

/// Manifest schema this build understands.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct SuiteManifest {
    pub schema_version: u32,
    pub suite_id: String,
    pub provenance: Provenance,
    pub defaults: Defaults,
    pub smart: SmartRules,
    pub intended_scripts: Vec<String>,
    #[serde(default)]
    pub unavailable_by_operator_decision: Vec<String>,
    pub cases: Vec<CaseEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Provenance {
    pub reference_commit: String,
    pub reference_tree: String,
    pub reference_archive_sha256: String,
    pub reference_upstream: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub derived_by: String,
    #[serde(default)]
    pub reference_statuses_are_evidence: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Defaults {
    pub budget_min: u32,
    pub jobs: u32,
    pub exec: ExecTemplates,
    #[serde(default)]
    pub options: BTreeMap<String, DesiredOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecTemplates {
    pub core: ExecTemplate,
    pub pair: ExecTemplate,
    /// The dedicated external raw TypeScript loader smoke (`panel`'s `external_watch`
    /// example). A distinct entry, never a relabelled `catalog_watch`.
    pub external: ExecTemplate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecTemplate {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A desired run option and whether the current native adapters can select it.
#[derive(Debug, Clone, Deserialize)]
pub struct DesiredOption {
    #[serde(default)]
    pub desired: String,
    pub supported: bool,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub pending: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmartRules {
    pub shared_paths: Vec<String>,
    pub subsystem_paths: BTreeMap<String, String>,
    pub ignored_paths: Vec<String>,
    #[serde(default)]
    pub shared_reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    Native,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Vetted,
    Documented,
    Unvetted,
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    Core,
    Pair,
    /// The dedicated external raw TypeScript loader smoke (`external_watch`). It has no
    /// `CoreCase`/`PairCase` witness identity and is not a scenario catalog row.
    External,
}

impl RunnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunnerKind::Core => "core",
            RunnerKind::Pair => "pair",
            RunnerKind::External => "external",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Covers {
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(default)]
    pub subsystems: Vec<String>,
    /// Explicit Rust path prefixes this case owns for `smart` selection.
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnsupportedOption {
    pub option: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReferenceCase {
    pub id: String,
    pub status: CaseStatus,
    #[serde(default)]
    pub manual: bool,
    pub budget_min: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Unavailable {
    pub code: String,
    pub reason: String,
}

/// A case's capture expectation. `required` means a missing PNG/JSON pair is a
/// shared harness failure, not an ordinary case failure.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CaptureSpec {
    pub label: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseEntry {
    pub id: String,
    pub kind: CaseKind,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub script_key: Option<String>,
    #[serde(default)]
    pub primary: bool,
    pub status: CaseStatus,
    #[serde(default)]
    pub manual: bool,
    pub budget_min: u32,
    #[serde(default)]
    pub runner: Option<RunnerKind>,
    #[serde(default)]
    pub live: Option<String>,
    #[serde(default)]
    pub scenario: Option<String>,
    #[serde(default)]
    pub core_case: Option<String>,
    #[serde(default)]
    pub pair_case: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub unsupported: Vec<UnsupportedOption>,
    #[serde(default)]
    pub covers: Covers,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub reference_cases: Vec<ReferenceCase>,
    #[serde(default)]
    pub unavailable: Option<Unavailable>,
    #[serde(default)]
    pub capture: Option<CaptureSpec>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub proven_at: Option<String>,
    #[serde(default)]
    pub documented_in: Option<String>,
    #[serde(default)]
    pub budget_note: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub reference_args: Vec<String>,
    #[serde(default)]
    pub reference_env: BTreeMap<String, String>,
}

impl CaseEntry {
    /// A case the suite can actually launch: native, with a live scenario, not declared
    /// unavailable.
    pub fn is_runnable(&self) -> bool {
        self.kind == CaseKind::Native && self.unavailable.is_none() && self.live.is_some()
    }

    /// Status used for level selection. Native rows derive it from the reference cases
    /// they adapt (a mapped `vetted`/`broken` reference case is the strongest signal);
    /// it is selection metadata, never a native PASS.
    pub fn selection_status(&self) -> CaseStatus {
        self.status
    }

    pub fn runner(&self) -> RunnerKind {
        self.runner.unwrap_or(RunnerKind::Core)
    }

    /// Whether an `--only` substring names this case. Mirrors the frozen runner: the
    /// substring may hit the case id or its harness (`--live` name/scenario here), and
    /// multiple matches are intentional rather than an error.
    pub fn matches_only(&self, needle: &str) -> bool {
        let mut haystacks: Vec<&str> = vec![self.id.as_str()];
        for extra in [
            &self.live,
            &self.scenario,
            &self.harness,
            &self.script,
            &self.script_key,
        ] {
            if let Some(value) = extra.as_deref() {
                haystacks.push(value);
            }
        }
        for reference in &self.reference_cases {
            haystacks.push(reference.id.as_str());
        }
        haystacks.iter().any(|hay| hay.contains(needle))
    }

    /// Human label used in logs and the ledger: the native case id.
    pub fn label(&self) -> String {
        self.id.clone()
    }

    pub fn budget_min(&self) -> u32 {
        self.budget_min.max(1)
    }
}

impl SuiteManifest {
    pub fn parse(bytes: &[u8], origin: &str) -> SuiteResult<Self> {
        let manifest: SuiteManifest =
            serde_json::from_slice(bytes).map_err(|error| format!("manifest {origin}: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> SuiteResult<Self> {
        let bytes =
            std::fs::read(path).map_err(|error| format!("manifest {}: {error}", path.display()))?;
        Self::parse(&bytes, &path.display().to_string())
    }

    pub fn case(&self, id: &str) -> Option<&CaseEntry> {
        self.cases.iter().find(|case| case.id == id)
    }

    /// Invariants the runner relies on. A broken manifest is a configuration error, not
    /// a case failure: fail closed before any child is launched.
    pub fn validate(&self) -> SuiteResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "manifest schema_version {} is not the supported {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.suite_id.trim().is_empty() {
            return Err("manifest suite_id is empty".into());
        }
        if self.provenance.reference_commit.len() < 7 {
            return Err("manifest provenance carries no reference commit".into());
        }
        if self.defaults.budget_min == 0 {
            return Err("manifest default budget_min must be positive".into());
        }
        if self.defaults.jobs != 1 {
            return Err("manifest default jobs must be 1 (native runs are sequential)".into());
        }
        for (name, template) in [
            ("core", &self.defaults.exec.core),
            ("pair", &self.defaults.exec.pair),
            ("external", &self.defaults.exec.external),
        ] {
            if template.program.trim().is_empty() {
                return Err(format!("manifest {name} exec template has no program"));
            }
        }
        if self.smart.shared_paths.is_empty() {
            return Err("manifest smart.shared_paths is empty".into());
        }
        for case in &self.cases {
            if case.id.trim().is_empty() {
                return Err("manifest case with an empty id".into());
            }
            if case.budget_min == 0 {
                return Err(format!("{}: budget_min must be positive", case.id));
            }
            if let Some(capture) = &case.capture {
                if capture.label.trim().is_empty() {
                    return Err(format!("{}: capture label is empty", case.id));
                }
            }
            match case.kind {
                CaseKind::Native => {
                    if case.is_runnable() && case.runner() == RunnerKind::External {
                        // The dedicated loader smoke has no scenario catalog row and no
                        // CoreCase/PairCase witness: its own contract is validated here.
                        validate_external_case(case)?;
                    } else if case.is_runnable() {
                        let live = case.live.as_deref().unwrap_or_default();
                        if live.trim().is_empty() {
                            return Err(format!("{}: runnable case without a live name", case.id));
                        }
                        let scenario = case.scenario.as_deref().unwrap_or_default();
                        if live != format!("script_{scenario}") {
                            return Err(format!(
                                "{}: --live name {live:?} is not the live name of scenario {scenario:?} \
                                 (the panel's PASS/CATALOG_CORE proof name is the live name, and the \
                                 witness identity is the snake_case host enum wire form)",
                                case.id
                            ));
                        }
                        // The declared witness identity must be the host enum's wire form
                        // (`thiever`, not `Thiever`): the panel serializes the real enum.
                        let declared = match case.runner() {
                            RunnerKind::Pair => case.pair_case.as_deref(),
                            _ => case.core_case.as_deref(),
                        };
                        let declared = declared.unwrap_or_default();
                        if declared.trim().is_empty() {
                            return Err(format!(
                                "{}: {} case without a declared witness identity",
                                case.id,
                                case.runner().as_str()
                            ));
                        }
                        let expected = super::receipt::wire_case(scenario, case.runner()).map_err(
                            |error| {
                                format!("{}: cannot derive the witness identity: {error}", case.id)
                            },
                        )?;
                        if declared != expected {
                            return Err(format!(
                                "{}: declared witness identity {declared:?} is not the host enum wire \
                                 form {expected:?} for scenario {scenario:?}",
                                case.id
                            ));
                        }
                    } else if let Some(unavailable) = &case.unavailable {
                        if unavailable.code.trim().is_empty()
                            || unavailable.reason.trim().is_empty()
                        {
                            return Err(format!(
                                "{}: unavailable row without a code and reason",
                                case.id
                            ));
                        }
                    } else {
                        return Err(format!(
                            "{}: native case is neither runnable nor explicitly unavailable",
                            case.id
                        ));
                    }
                }
                CaseKind::Reference => {
                    let unavailable = case.unavailable.as_ref().ok_or_else(|| {
                        format!("{}: reference row without an explicit reason", case.id)
                    })?;
                    if unavailable.reason.trim().is_empty() {
                        return Err(format!("{}: reference row without a reason", case.id));
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        for case in &self.cases {
            if !seen.insert(case.id.as_str()) {
                return Err(format!("duplicate case id: {}", case.id));
            }
        }
        Ok(())
    }

    pub fn status_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for case in &self.cases {
            let key = match case.status {
                CaseStatus::Vetted => "vetted",
                CaseStatus::Documented => "documented",
                CaseStatus::Unvetted => "unvetted",
                CaseStatus::Broken => "broken",
            };
            *counts.entry(key.to_string()).or_insert(0) += 1;
        }
        counts
    }
}

/// The dedicated external loader smoke's own row contract.
///
/// It is *not* a scenario catalog row and has no `CoreCase`/`PairCase` witness identity: it
/// reuses the panel's `external_watch` proof name, the external loader's live scenario token
/// and the producer's terminal-shot label. Nothing is inferred from the catalog tables, and a
/// row that claims any of them refuses the manifest before a run.
fn validate_external_case(case: &CaseEntry) -> SuiteResult<()> {
    let live = case.live.as_deref().unwrap_or_default();
    if live != host_play::external_loader::LIVE_NAME {
        return Err(format!(
            "{}: external case --live name {live:?} is not the panel's external proof name {:?}",
            case.id,
            host_play::external_loader::LIVE_NAME
        ));
    }
    let scenario = case.scenario.as_deref().unwrap_or_default();
    if scenario != host_play::external_loader::LIVE_SCENARIO {
        return Err(format!(
            "{}: external case scenario {scenario:?} is not the external loader's live scenario \
             {:?}; this row is not a catalog scenario and must not claim one",
            case.id,
            host_play::external_loader::LIVE_SCENARIO
        ));
    }
    if case.core_case.is_some() || case.pair_case.is_some() {
        return Err(format!(
            "{}: external case declares a CoreCase/PairCase witness identity; the external loader \
             smoke has no host-enum witness and must not be presented as a catalog or pair case",
            case.id
        ));
    }
    let spec = case.capture.as_ref().ok_or_else(|| {
        format!(
            "{}: external case contracts no capture; the post-run terminal shot of the owned actor \
             at scene 2 is mandatory and prerequisite captures cannot stand in for it",
            case.id
        )
    })?;
    if spec.label != host_play::external_loader::TERMINAL_SHOT {
        return Err(format!(
            "{}: external case contracts capture {:?}, expected the producer's terminal shot {:?} \
             (the prerequisite shot is a different capture and never satisfies it)",
            case.id,
            spec.label,
            host_play::external_loader::TERMINAL_SHOT
        ));
    }
    if !spec.required {
        return Err(format!(
            "{}: the external terminal capture is mandatory; declare it required",
            case.id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded() -> SuiteManifest {
        SuiteManifest::parse(super::super::EMBEDDED_MANIFEST.as_bytes(), "embedded").unwrap()
    }

    #[test]
    fn embedded_manifest_is_valid_and_covers_the_intended_inventory() {
        let manifest = embedded();
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
        assert_eq!(
            manifest.provenance.reference_commit,
            "00d39a17e056df6c5e461f3f2cfd3598ff9720b6"
        );
        assert_eq!(manifest.intended_scripts.len(), 44);
        for script in ["Duel Arena Combat Trainer", "ClimbingBoots", "RockCrab"] {
            assert!(manifest.intended_scripts.iter().any(|name| name == script));
        }
        let runnable: Vec<&CaseEntry> = manifest.cases.iter().filter(|c| c.is_runnable()).collect();
        assert!(
            runnable.len() > 80,
            "expected the intended inventory, found {}",
            runnable.len()
        );
        // Every enabled script still appears exactly once as a primary native row or as
        // an explicit unavailable row: nothing is silently dropped.
        for script in [
            "BankSorter",
            "ClimbingBoots",
            "Duel Arena Combat Trainer",
            "RockCrab",
        ] {
            assert!(
                manifest
                    .cases
                    .iter()
                    .any(|c| c.script.as_deref() == Some(script)
                        && (c.unavailable.is_some() || c.is_runnable())),
                "{script} must stay visible"
            );
        }
        assert!(manifest
            .cases
            .iter()
            .any(|c| c.id == "banksorter-live" && c.script.as_deref() == Some("BankSorter")));
        // BankSorter is unavailable by operator decision wherever it appears.
        for case in manifest
            .cases
            .iter()
            .filter(|c| c.script.as_deref() == Some("BankSorter"))
        {
            assert_eq!(
                case.unavailable.as_ref().map(|u| u.code.as_str()),
                Some("operator_decision")
            );
        }
    }

    #[test]
    fn runnable_rows_declare_a_witness_and_preserve_reference_metadata() {
        let manifest = embedded();
        for case in manifest.cases.iter().filter(|c| c.is_runnable()) {
            assert!(case.live.is_some(), "{}", case.id);
            match case.runner() {
                RunnerKind::Core => assert!(case.core_case.is_some(), "{}", case.id),
                RunnerKind::Pair => assert!(case.pair_case.is_some(), "{}", case.id),
                // The loader smoke has no catalog witness identity at all; that is asserted
                // positively by `external_row_is_honest_and_not_a_catalog_case`.
                RunnerKind::External => assert!(
                    case.core_case.is_none() && case.pair_case.is_none(),
                    "{} declares a catalog witness identity on an external row",
                    case.id
                ),
            }
            for unsupported in &case.unsupported {
                assert!(!unsupported.reason.trim().is_empty(), "{}", case.id);
            }
        }
        let ardy = manifest
            .case("ardy_fighter")
            .expect("ArdyFighter primary row");
        assert_eq!(
            ardy.budget_min, 5,
            "the 5-minute reference budget is preserved"
        );
        assert!(ardy
            .reference_cases
            .iter()
            .any(|r| r.id == "ardyfighter-restock-loop-live" && r.status == CaseStatus::Vetted));
        let flax = manifest.case("flax_aio").expect("FlaxAIO row");
        assert_eq!(flax.status, CaseStatus::Broken);
        assert_eq!(
            flax.budget_min, 18,
            "the broken 18-minute budget is preserved"
        );
    }

    #[test]
    fn duplicate_ids_and_unknown_schema_are_rejected() {
        let mut value: serde_json::Value =
            serde_json::from_str(super::super::EMBEDDED_MANIFEST).unwrap();
        let cases = value["cases"].as_array_mut().unwrap();
        let clone = cases[0].clone();
        cases.push(clone);
        let error = SuiteManifest::parse(serde_json::to_string(&value).unwrap().as_bytes(), "test")
            .unwrap_err();
        assert!(error.contains("duplicate case id"), "{error}");

        value["cases"].as_array_mut().unwrap().pop();
        value["schema_version"] = serde_json::json!(99);
        let error = SuiteManifest::parse(serde_json::to_string(&value).unwrap().as_bytes(), "test")
            .unwrap_err();
        assert!(error.contains("schema_version"), "{error}");
    }

    #[test]
    fn native_rows_agree_with_the_live_scenario_and_witness_tables() {
        // The manifest must not drift from the code that actually defines the cases.
        let manifest = embedded();
        let names = scenario::names();
        for case in manifest.cases.iter().filter(|c| c.is_runnable()) {
            if case.runner() == RunnerKind::External {
                // The loader smoke is not a scenario row and has no host-enum witness: its
                // own contract is checked by `external_row_is_honest_and_not_a_catalog_case`.
                assert!(case.scenario.as_deref() == Some("external_loader"));
                assert!(
                    !names.contains(&"external_loader"),
                    "the loader smoke must not be a catalog scenario"
                );
                continue;
            }
            let live = case.live.as_deref().unwrap();
            let scenario = case.scenario.as_deref().unwrap();
            assert!(
                names.contains(&scenario),
                "{scenario} is not a live scenario"
            );
            assert_eq!(live, format!("script_{scenario}"));
            assert!(
                scenario::get(scenario).is_some(),
                "{scenario} does not resolve"
            );
            // The declared witness identity is the host enum's serde wire form, the value
            // the panel actually prints in the CATALOG_CORE/PAIRED_CORE receipt.
            match case.runner() {
                RunnerKind::Core => {
                    let identity = case.core_case.as_deref().unwrap();
                    let parsed = host_play::catalog_core::CoreCase::parse(scenario)
                        .unwrap_or_else(|error| panic!("{scenario}: {error}"));
                    assert_eq!(
                        identity,
                        serde_json::to_value(parsed).unwrap().as_str().unwrap(),
                        "{scenario} core identity"
                    );
                }
                RunnerKind::Pair => {
                    let identity = case.pair_case.as_deref().unwrap();
                    let parsed = host_play::paired_core::PairCase::parse(scenario)
                        .unwrap_or_else(|error| panic!("{scenario}: {error}"));
                    assert_eq!(
                        identity,
                        serde_json::to_value(parsed).unwrap().as_str().unwrap(),
                        "{scenario} pair identity"
                    );
                }
                // External rows returned above: they have no host-enum witness identity to
                // compare against.
                RunnerKind::External => {
                    unreachable!("external rows continue before the witness identity check")
                }
            }
        }
    }

    /// The loader smoke is a native row for the external producer's own contract: the
    /// producer's live name, live scenario and terminal-shot label, no catalog witness
    /// identity, and no change to the intended inventory or the catalog `bone_burier` row.
    #[test]
    fn external_row_is_honest_and_not_a_catalog_case() {
        let manifest = embedded();
        let row = manifest
            .case("external_loader")
            .expect("external loader row");
        assert_eq!(row.kind, CaseKind::Native);
        assert_eq!(row.runner(), RunnerKind::External);
        assert_eq!(
            row.live.as_deref(),
            Some(host_play::external_loader::LIVE_NAME)
        );
        assert_eq!(
            row.scenario.as_deref(),
            Some(host_play::external_loader::LIVE_SCENARIO)
        );
        assert!(row.core_case.is_none() && row.pair_case.is_none());
        assert!(!row.primary, "BoneBurier keeps its primary catalog row");
        assert_eq!(
            row.capture
                .as_ref()
                .map(|capture| (capture.label.as_str(), capture.required)),
            Some((host_play::external_loader::TERMINAL_SHOT, true)),
            "the external terminal capture is contracted by the producer's own label"
        );
        assert_eq!(row.selection_status(), CaseStatus::Unvetted);
        assert!(!row.manual);
        assert!(row.is_runnable());
        assert_eq!(
            manifest.defaults.exec.external.program, "cargo",
            "the external template is a distinct entry, not a relabelled core one"
        );
        assert!(manifest
            .defaults
            .exec
            .external
            .args
            .iter()
            .any(|arg| arg == "external_watch"));

        // The catalog core case and the frozen reference row are unchanged.
        let bone = manifest.case("bone_burier").expect("bone_burier row");
        assert_eq!(bone.core_case.as_deref(), Some("bone_burier"));
        assert!(
            bone.reference_cases
                .iter()
                .any(|reference| reference.id == "external-script-test"),
            "the frozen reference row stays recorded where it was"
        );
        assert_eq!(manifest.intended_scripts.len(), 44);
        assert!(manifest
            .intended_scripts
            .iter()
            .any(|name| name == "BoneBurier"));
        // The external row is not a second primary row for any intended script.
        let primaries = manifest
            .cases
            .iter()
            .filter(|case| case.primary && case.kind == CaseKind::Native)
            .count();
        assert_eq!(
            primaries, 44,
            "one primary row per intended script, the loader smoke is not one"
        );
    }

    /// An external row that claims a catalog identity, a catalog live name or another
    /// capture label refuses the manifest instead of being launched as something else.
    #[test]
    fn external_rows_that_borrow_a_catalog_identity_or_capture_are_refused() {
        fn embedded_value() -> serde_json::Value {
            serde_json::from_str(super::super::EMBEDDED_MANIFEST).unwrap()
        }
        fn external_index(value: &serde_json::Value) -> usize {
            value["cases"]
                .as_array()
                .unwrap()
                .iter()
                .position(|case| case["id"] == "external_loader")
                .expect("external row")
        }
        fn parse(value: &serde_json::Value) -> SuiteResult<SuiteManifest> {
            SuiteManifest::parse(serde_json::to_string(value).unwrap().as_bytes(), "test")
        }

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["core_case"] = serde_json::json!("bone_burier");
        let error = parse(&value).unwrap_err();
        assert!(
            error.contains("CoreCase/PairCase witness identity"),
            "{error}"
        );

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["live"] = serde_json::json!("script_bone_burier");
        let error = parse(&value).unwrap_err();
        assert!(error.contains("external proof name"), "{error}");

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["scenario"] = serde_json::json!("bone_burier");
        let error = parse(&value).unwrap_err();
        assert!(error.contains("live scenario"), "{error}");

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["capture"]["label"] = serde_json::json!("external_loader prereq");
        let error = parse(&value).unwrap_err();
        assert!(
            error.contains("expected the producer's terminal shot"),
            "{error}"
        );

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["capture"]["required"] = serde_json::json!(false);
        let error = parse(&value).unwrap_err();
        assert!(error.contains("mandatory"), "{error}");

        let mut value = embedded_value();
        let index = external_index(&value);
        value["cases"][index]["capture"] = serde_json::Value::Null;
        let error = parse(&value).unwrap_err();
        assert!(error.contains("contracts no capture"), "{error}");
    }

    #[test]
    fn targeted_or_excluded_reference_rows_keep_their_reason() {
        let manifest = embedded();
        let excluded = manifest
            .cases
            .iter()
            .filter(|c| c.kind == CaseKind::Reference)
            .filter(|c| c.unavailable.is_none())
            .count();
        assert_eq!(excluded, 0, "every reference row carries its reason");
        let quest = manifest
            .case("aio-quest-test")
            .expect("quest case retained");
        assert_eq!(quest.status, CaseStatus::Documented);
        assert_eq!(quest.budget_min, 120);
        assert_eq!(quest.unavailable.as_ref().unwrap().code, "excluded_script");
        // Excluded scope is never relabelled: it is not a native case.
        assert_eq!(quest.kind, CaseKind::Reference);
    }
}
