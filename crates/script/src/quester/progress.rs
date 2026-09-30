//! Typed Path progress compiler and journal/colour resolver.

use super::compile::CompiledPath;
use super::path::{PathDocument, PathRoleDocument, ProgressDocument};
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead, ProgressFlag, QuestProgress};
use api::selected::{FactKey, Gap, Knowledge, SelectedPin, Truth};
use api::snapshot::{QuestListStatus, SnapshotView};
use std::sync::Arc;

const MAX_NORMALIZED_JOURNAL_BYTES: usize = 64 * 1024;
const DEFAULT_COUNT_DIGITS: usize = 9;
const MAX_COUNT_DIGITS: usize = 9;

/// The compiled, immutable progress program owned by a [`CompiledPath`].
#[derive(Debug, Clone)]
pub struct CompiledProgress {
    pub binding: FactKey,
    pub role: Option<FactKey>,
    pub colour_not_started: FactKey,
    pub colour_in_progress: FactKey,
    pub colour_complete: FactKey,
    pub rules: Arc<[CompiledProgressRule]>,
    pub flags: Arc<[CompiledProgressFlagRule]>,
    pub monotonic: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledProgressRule {
    pub stage: FactKey,
    pub all: Arc<[Arc<str>]>,
    pub any: Arc<[Arc<str>]>,
    pub not: Arc<[Arc<str>]>,
    pub varp: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct CompiledProgressFlagRule {
    pub flag: FactKey,
    pub all: Arc<[Arc<str>]>,
    pub any: Arc<[Arc<str>]>,
    pub count: Option<CountCapture>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountCapture {
    pub max_digits: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressRuleDiagnostic {
    pub index: usize,
    pub stage: FactKey,
    pub varp: Option<i32>,
}
impl CompiledProgress {
    /// Return the authored rule for a resolved stage, preserving authored
    /// newest-first order.
    pub fn rule_for_stage(&self, stage: &FactKey) -> Option<&CompiledProgressRule> {
        self.rules.iter().find(|rule| &rule.stage == stage)
    }

    /// The fixture/status diagnostic attached to a stage rule. This is never
    /// consulted while resolving predicates or selecting a stage.
    pub fn rule_diagnostic(&self, stage: &FactKey) -> Option<ProgressRuleDiagnostic> {
        self.rules
            .iter()
            .enumerate()
            .find(|(_, rule)| &rule.stage == stage)
            .map(|(index, rule)| ProgressRuleDiagnostic {
                index,
                stage: rule.stage.clone(),
                varp: rule.varp,
            })
    }

    pub fn varp_hint(&self, stage: &FactKey) -> Option<i32> {
        self.rule_for_stage(stage).and_then(|rule| rule.varp)
    }
}

/// Compile and validate the authored progress program once with the Path.
pub(crate) fn compile_progress(
    document: &PathDocument,
    role: &PathRoleDocument,
    progress: &ProgressDocument,
    quests: &QuestCatalog,
) -> Result<CompiledProgress, super::compile::CompileError> {
    if role.progress_binding.0.is_empty() {
        return Err(super::compile::CompileError::code(
            "invalid-progress-binding",
        ));
    }

    // Identity catalogs know the expected binding. Keep the compiler useful
    // for synthetic/cache fixtures whose deliberately changed path id is not
    // present in the identity roster.
    if let Ok(facts) = quests.quest(document.id.0.as_ref()) {
        if let Knowledge::Known(expected) = &facts.progress_binding {
            if expected != &role.progress_binding {
                return Err(super::compile::CompileError::code("progress-binding"));
            }
        }
    }

    let stages: std::collections::HashSet<&str> = role
        .sequences
        .iter()
        .map(|sequence| sequence.stage.0.as_ref())
        .collect();
    for stage in [
        &progress.colour.not_started,
        &progress.colour.in_progress,
        &progress.colour.complete,
    ] {
        if stage.0.is_empty() || !stages.contains(stage.0.as_ref()) {
            return Err(super::compile::CompileError::code("unbound-progress-stage"));
        }
    }

    let mut rules = Vec::with_capacity(progress.rules.len());
    for rule in &progress.rules {
        if rule.stage.0.is_empty() || !stages.contains(rule.stage.0.as_ref()) {
            return Err(super::compile::CompileError::code("unbound-progress-stage"));
        }
        rules.push(CompiledProgressRule {
            stage: rule.stage.clone(),
            all: compile_needles(&rule.all)?,
            any: compile_needles(&rule.any)?,
            not: compile_needles(&rule.not)?,
            varp: rule.varp,
        });
    }

    let mut flags = Vec::with_capacity(progress.flags.len());
    for flag in &progress.flags {
        if flag.flag.0.is_empty() {
            return Err(super::compile::CompileError::code("invalid-progress-flag"));
        }
        flags.push(CompiledProgressFlagRule {
            flag: flag.flag.clone(),
            all: compile_needles(&flag.all)?,
            any: compile_needles(&flag.any)?,
            count: flag.count.as_deref().map(compile_count).transpose()?,
        });
    }

    Ok(CompiledProgress {
        binding: role.progress_binding.clone(),
        role: role.role.clone(),
        colour_not_started: progress.colour.not_started.clone(),
        colour_in_progress: progress.colour.in_progress.clone(),
        colour_complete: progress.colour.complete.clone(),
        rules: Arc::from(rules),
        flags: Arc::from(flags),
        monotonic: progress.monotonic,
    })
}

fn compile_needles(needles: &[String]) -> Result<Arc<[Arc<str>]>, super::compile::CompileError> {
    let mut compiled = Vec::with_capacity(needles.len());
    for needle in needles {
        if needle.is_empty()
            || needle.trim().is_empty()
            || needle.chars().any(char::is_uppercase)
            || needle.chars().any(char::is_control)
        {
            return Err(super::compile::CompileError::code(
                "invalid-progress-needle",
            ));
        }
        let normalized = normalize_text(needle);
        if normalized.is_empty() {
            return Err(super::compile::CompileError::code(
                "invalid-progress-needle",
            ));
        }
        compiled.push(Arc::<str>::from(normalized));
    }
    Ok(Arc::from(compiled))
}

fn compile_count(pattern: &str) -> Result<CountCapture, super::compile::CompileError> {
    // The count field is intentionally not a regex engine. It is only the
    // bounded decimal capture used by authored journal flags.
    let inner = pattern
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(pattern);
    let max_digits = if inner == r"\d+" {
        DEFAULT_COUNT_DIGITS
    } else if let Some(digits) = inner
        .strip_prefix(r"\d{1,")
        .and_then(|value| value.strip_suffix('}'))
    {
        digits
            .parse::<usize>()
            .ok()
            .filter(|digits| (1..=MAX_COUNT_DIGITS).contains(digits))
            .ok_or_else(|| super::compile::CompileError::code("invalid-progress-count"))?
    } else {
        return Err(super::compile::CompileError::code("invalid-progress-count"));
    };
    Ok(CountCapture {
        max_digits: max_digits as u8,
    })
}

/// Normalize the journal representation used by the authored substring rules.
///
/// Colour tags are exactly the client form `@xxx@`; pipes are line breaks and
/// the remaining text is lowercased. The bounded output keeps a malformed or
/// unexpectedly large journal from growing a per-read allocation without
/// limit.
pub fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_NORMALIZED_JOURNAL_BYTES));
    normalize_into(text, &mut out);
    out
}

pub fn normalize_journal(lines: &[Arc<str>]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            push_bounded(&mut out, '\n');
        }
        normalize_into(line, &mut out);
        if out.len() >= MAX_NORMALIZED_JOURNAL_BYTES {
            break;
        }
    }
    out
}

fn normalize_into(text: &str, out: &mut String) {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '@' {
            let mut probe = chars.clone();
            if probe.next().is_some()
                && probe.next().is_some()
                && probe.next().is_some()
                && probe.next() == Some('@')
            {
                for _ in 0..4 {
                    chars.next();
                }
                continue;
            }
        }
        let ch = match ch {
            '|' => '\n',
            value => value,
        };
        for lower in ch.to_lowercase() {
            if out.len() >= MAX_NORMALIZED_JOURNAL_BYTES {
                break;
            }
            out.push(lower);
        }
        if out.len() >= MAX_NORMALIZED_JOURNAL_BYTES {
            break;
        }
    }
}

fn push_bounded(out: &mut String, ch: char) {
    if out.len() < MAX_NORMALIZED_JOURNAL_BYTES {
        out.push(ch);
    }
}

pub fn colour_stage(
    path: &CompiledPath,
    quests: &QuestCatalog,
    snapshot: SnapshotView<'_>,
) -> Option<FactKey> {
    match quest_colour(path, quests, snapshot)? {
        QuestListStatus::NotStarted => Some(path.colour_not_started.clone()),
        QuestListStatus::Complete => Some(path.colour_complete.clone()),
        QuestListStatus::InProgress => Some(path.colour_in_progress.clone()),
        QuestListStatus::Unknown => None,
    }
}

pub fn quest_colour(
    path: &CompiledPath,
    quests: &QuestCatalog,
    snapshot: SnapshotView<'_>,
) -> Option<QuestListStatus> {
    let facts = quests.quest(path.id.0.as_ref()).ok()?;
    let rows = snapshot.quest_statuses()?;
    rows.value
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(facts.display.as_ref()))
        .map(|row| row.status())
}

pub fn resolve_colour(
    path: &CompiledPath,
    colour: QuestListStatus,
    evidence: EvidenceStamp,
    pin: Arc<SelectedPin>,
) -> QuestProgress {
    let (stage, complete) = match colour {
        QuestListStatus::NotStarted => (
            Knowledge::Known(path.colour_not_started.clone()),
            Truth::False,
        ),
        QuestListStatus::InProgress => (
            Knowledge::Known(path.colour_in_progress.clone()),
            Truth::False,
        ),
        QuestListStatus::Complete => (Knowledge::Known(path.colour_complete.clone()), Truth::True),
        QuestListStatus::Unknown => (unknown("quest-colour"), Truth::Unknown),
    };
    let rule = stage.clone();
    QuestProgress {
        quest: path.id.clone(),
        stage,
        complete,
        signals: Arc::from(Vec::<api::selected::SignalRange>::new()),
        flags: Arc::from(Vec::<ProgressFlag>::new()),
        evidence,
        binding: path.progress.binding.clone(),
        role: path.progress.role.clone(),
        rule,
        pin,
    }
}

pub fn resolve_journal(
    path: &CompiledPath,
    read: &JournalRead,
    previous: Option<&QuestProgress>,
) -> QuestProgress {
    if read.quest != path.id {
        return unknown_journal_progress(path, read);
    }
    let text = normalize_journal(&read.lines);
    let evidence = read.closed;
    let hit = path.progress.rules.iter().enumerate().find(|(_, rule)| {
        rule_matches(
            rule.all.as_ref(),
            rule.any.as_ref(),
            rule.not.as_ref(),
            &text,
        )
    });

    let mut stage = hit
        .map(|(_, rule)| Knowledge::Known(rule.stage.clone()))
        .unwrap_or_else(|| unknown("journal-no-match"));
    let mut complete = hit
        .map(|(_, rule)| {
            if rule.stage == path.progress.colour_complete {
                Truth::True
            } else {
                Truth::False
            }
        })
        .unwrap_or(Truth::Unknown);
    let mut rule = hit
        .map(|(_, matched)| Knowledge::Known(matched.stage.clone()))
        .unwrap_or_else(|| unknown("journal-no-match"));

    if path.progress.monotonic {
        if let (Some((current_index, _)), Some(previous)) = (hit, previous) {
            if previous.quest == path.id
                && previous.evidence.run == evidence.run
                && matches!(&previous.stage, Knowledge::Known(_))
            {
                if let Some(previous_index) =
                    path.progress
                        .rules
                        .iter()
                        .position(|candidate| match &previous.stage {
                            Knowledge::Known(stage) => &candidate.stage == stage,
                            _ => false,
                        })
                {
                    if current_index > previous_index {
                        stage = previous.stage.clone();
                        complete = previous.complete;
                        rule = previous.rule.clone();
                    }
                }
            }
        }
    }

    QuestProgress {
        quest: path.id.clone(),
        stage,
        complete,
        signals: Arc::from(Vec::<api::selected::SignalRange>::new()),
        flags: resolve_flags(&path.progress.flags, &text),
        evidence,
        binding: path.progress.binding.clone(),
        role: path.progress.role.clone(),
        rule,
        pin: Arc::clone(&read.pin),
    }
}

fn unknown_journal_progress(path: &CompiledPath, read: &JournalRead) -> QuestProgress {
    QuestProgress {
        quest: path.id.clone(),
        stage: unknown("journal-quest-mismatch"),
        complete: Truth::Unknown,
        signals: Arc::from(Vec::<api::selected::SignalRange>::new()),
        flags: Arc::from(Vec::<ProgressFlag>::new()),
        evidence: read.closed,
        binding: path.progress.binding.clone(),
        role: path.progress.role.clone(),
        rule: unknown("journal-quest-mismatch"),
        pin: Arc::clone(&read.pin),
    }
}

fn resolve_flags(rules: &[CompiledProgressFlagRule], text: &str) -> Arc<[ProgressFlag]> {
    let mut flags = Vec::with_capacity(rules.len());
    for rule in rules {
        if flags
            .iter()
            .any(|flag: &ProgressFlag| flag.flag == rule.flag)
        {
            continue;
        }
        let matched = rules.iter().find(|candidate| {
            candidate.flag == rule.flag
                && rule_matches(candidate.all.as_ref(), candidate.any.as_ref(), &[], text)
        });
        let (truth, count) = match matched {
            None => (Truth::False, None),
            Some(rule) => match rule.count {
                None => (Truth::True, None),
                Some(capture) => match capture_number(rule, text, capture) {
                    Some(value) => (Truth::True, Some(value)),
                    None => (Truth::Unknown, None),
                },
            },
        };
        flags.push(ProgressFlag {
            flag: rule.flag.clone(),
            truth,
            count,
        });
    }
    Arc::from(flags)
}

fn capture_number(
    rule: &CompiledProgressFlagRule,
    text: &str,
    capture: CountCapture,
) -> Option<u32> {
    let mut count = None;
    // A flag's capture belongs to its matching journal line, not to the
    // first unrelated number anywhere in the quest. Ambiguity stays unknown.
    for line in text
        .lines()
        .filter(|line| rule_matches(&rule.all, &rule.any, &[], line))
    {
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_digit() {
                index += 1;
                continue;
            }
            let begin = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let digits = &line[begin..index];
            if digits.len() > capture.max_digits as usize || count.is_some() {
                return None;
            }
            count = Some(digits.parse::<u32>().ok()?);
        }
    }
    count
}

fn rule_matches(all: &[Arc<str>], any: &[Arc<str>], not: &[Arc<str>], text: &str) -> bool {
    all.iter().all(|needle| text.contains(needle.as_ref()))
        && (any.is_empty() || any.iter().any(|needle| text.contains(needle.as_ref())))
        && not.iter().all(|needle| !text.contains(needle.as_ref()))
}

fn unknown<T>(code: &'static str) -> Knowledge<T> {
    Knowledge::Unknown(Gap {
        code: Arc::from(code),
        sources: Arc::from([]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, RunKey};

    fn pin() -> Arc<SelectedPin> {
        Arc::new(SelectedPin {
            revision: ClientRevision::R289,
            engine_commit: Arc::from("engine"),
            content_commit: Arc::from("content"),
            cache_id: Arc::from("cache"),
            content_id: Arc::from("content-id"),
            nav_sha256: [0; 32],
            flags_sha256: [0; 32],
            schema: 1,
            manifest_sha256: [0; 32],
        })
    }

    fn stamp(tick: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick,
            sequence: tick,
        }
    }

    fn needles(values: &[&str]) -> Arc<[Arc<str>]> {
        Arc::from(
            values
                .iter()
                .map(|value| Arc::<str>::from(*value))
                .collect::<Vec<_>>(),
        )
    }

    fn rule(stage: &str, all: &[&str], any: &[&str], not: &[&str]) -> CompiledProgressRule {
        CompiledProgressRule {
            stage: FactKey::new(stage),
            all: needles(all),
            any: needles(any),
            not: needles(not),
            varp: None,
        }
    }

    fn path(
        rules: Vec<CompiledProgressRule>,
        flags: Vec<CompiledProgressFlagRule>,
        monotonic: bool,
    ) -> CompiledPath {
        CompiledPath {
            id: FactKey::new("synthetic"),
            display_name: Arc::from("Synthetic"),
            digest: [0; 32],
            colour_not_started: FactKey::new("synthetic:0"),
            colour_in_progress: FactKey::new("synthetic:1"),
            colour_complete: FactKey::new("synthetic:2"),
            progress: CompiledProgress {
                binding: FactKey::new("journal:synthetic"),
                role: None,
                colour_not_started: FactKey::new("synthetic:0"),
                colour_in_progress: FactKey::new("synthetic:1"),
                colour_complete: FactKey::new("synthetic:2"),
                rules: Arc::from(rules),
                flags: Arc::from(flags),
                monotonic,
            },
            prelude: Vec::new(),
            sequences: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn read(path: &CompiledPath, tick: u64, text: &str) -> JournalRead {
        let pin = pin();
        JournalRead {
            quest: path.id.clone(),
            root: 0,
            lines: Arc::from(vec![Arc::<str>::from(text)]),
            colour: None,
            acquired: stamp(tick),
            closed: stamp(tick),
            pin,
        }
    }

    #[test]
    fn normalization_and_no_match_are_explicit() {
        assert_eq!(normalize_text("@red@HELLO|World"), "hello\nworld");
        let path = path(
            vec![rule("synthetic:1", &["hello\nworld"], &[], &[])],
            Vec::new(),
            false,
        );
        let matched = resolve_journal(&path, &read(&path, 1, "@red@HELLO|World"), None);
        assert!(matches!(
            &matched.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:1"
        ));
        let missing = resolve_journal(&path, &read(&path, 2, "unrelated"), None);
        assert!(matches!(&missing.stage, Knowledge::Unknown(_)));
        assert!(matches!(&missing.rule, Knowledge::Unknown(_)));
    }

    #[test]
    fn negation_and_authored_first_win_are_preserved() {
        let path = path(
            vec![
                rule("synthetic:2", &["hello"], &[], &["blocked"]),
                rule("synthetic:1", &["hello"], &[], &[]),
            ],
            Vec::new(),
            false,
        );
        let first = resolve_journal(&path, &read(&path, 1, "hello"), None);
        assert!(matches!(
            &first.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:2"
        ));
        let negated = resolve_journal(&path, &read(&path, 2, "hello blocked"), None);
        assert!(matches!(
            &negated.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:1"
        ));
    }

    #[test]
    fn flags_and_bounded_counts_are_retained() {
        let path = path(
            vec![rule("synthetic:1", &["journal"], &[], &[])],
            vec![
                CompiledProgressFlagRule {
                    flag: FactKey::new("feather"),
                    all: needles(&[]),
                    any: needles(&["feather"]),
                    count: None,
                },
                CompiledProgressFlagRule {
                    flag: FactKey::new("crystals"),
                    all: needles(&[]),
                    any: needles(&["crystals"]),
                    count: Some(CountCapture { max_digits: 9 }),
                },
            ],
            false,
        );
        let progress = resolve_journal(
            &path,
            &read(&path, 1, "journal 99|have feather|3 crystals placed"),
            None,
        );
        assert_eq!(progress.flags.len(), 2);
        assert_eq!(progress.flags[0].truth, Truth::True);
        assert_eq!(progress.flags[0].count, None);
        assert_eq!(progress.flags[1].truth, Truth::True);
        assert_eq!(progress.flags[1].count, Some(3));
        assert!(progress.signals.is_empty());
        for body in [
            "journal|3 of 7 crystals placed",
            "journal|1234567890 crystals placed",
            "journal|crystals placed",
        ] {
            let progress = resolve_journal(&path, &read(&path, 2, body), None);
            assert_eq!(progress.flags[1].truth, Truth::Unknown);
            assert_eq!(progress.flags[1].count, None);
        }
    }

    #[test]
    fn monotonic_uses_authored_order_not_stage_key_order() {
        let path = path(
            vec![
                rule("synthetic:10", &["new"], &[], &[]),
                rule("synthetic:2", &["old"], &[], &[]),
            ],
            Vec::new(),
            true,
        );
        let old = resolve_journal(&path, &read(&path, 1, "old"), None);
        let forward = resolve_journal(&path, &read(&path, 2, "new"), Some(&old));
        assert!(matches!(
            &forward.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:10"
        ));
        let backward = resolve_journal(&path, &read(&path, 3, "old"), Some(&forward));
        assert!(matches!(
            &backward.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:10"
        ));
    }

    #[test]
    fn colour_resolution_bypasses_rules_and_has_empty_signals() {
        let path = path(Vec::new(), Vec::new(), true);
        let progress = resolve_colour(&path, QuestListStatus::Complete, stamp(1), pin());
        assert!(matches!(
            &progress.stage,
            Knowledge::Known(stage) if stage.0.as_ref() == "synthetic:2"
        ));
        assert_eq!(progress.complete, Truth::True);
        assert!(progress.flags.is_empty());
        assert!(progress.signals.is_empty());
    }
}
