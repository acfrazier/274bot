use super::super::compile::{CompileContext, CompileError, PredicateContext, PredicatePlan};
use api::quest_progress::QuestProgress;
use api::selected::{FactKey, Knowledge, Truth};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct StageInArgs {
    /// Symbolic quest key whose journal progress is read.
    quest: String,
    /// Stage keys accepted as a match; this list must be non-empty.
    any: Vec<String>,
}

pub(super) fn compile_stage_in(
    args: StageInArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    validate_progress_quest(cx, &args.quest)?;
    if args.any.is_empty() || args.any.iter().any(|stage| stage.is_empty()) {
        return Err(CompileError::code("invalid-args"));
    }
    if args.any.iter().any(|stage| {
        !cx.progress
            .stage_keys
            .iter()
            .any(|known| known.0.as_ref() == stage)
    }) {
        return Err(CompileError::code("unresolved-progress-stage"));
    }
    Ok(Arc::new(StageIn {
        quest: FactKey::new(&args.quest),
        any: args
            .any
            .iter()
            .map(|stage| FactKey::new(stage))
            .collect::<Vec<_>>()
            .into(),
    }))
}

struct StageIn {
    quest: FactKey,
    any: Arc<[FactKey]>,
}

impl PredicatePlan for StageIn {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(progress) = matching_progress(cx, &self.quest) else {
            return Truth::Unknown;
        };
        match &progress.stage {
            Knowledge::Known(stage) => truth(self.any.iter().any(|wanted| wanted == stage)),
            Knowledge::Partial { .. } | Knowledge::Unknown(_) => Truth::Unknown,
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct FlagArgs {
    /// Symbolic quest key whose journal progress is read.
    quest: String,
    /// Declared progress flag to query.
    flag: String,
    /// Optional expected Boolean state.
    #[serde(default)]
    is: Option<bool>,
    /// Optional exact count expected on a counted flag.
    #[serde(default)]
    count: Option<u32>,
    /// Optional minimum count expected on a counted flag.
    #[serde(default)]
    at_least: Option<u32>,
}

pub(super) fn compile_flag(
    args: FlagArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    validate_progress_quest(cx, &args.quest)?;
    if args.flag.is_empty() || (args.count.is_some() && args.at_least.is_some()) {
        return Err(CompileError::code("invalid-args"));
    }
    let flag = FactKey::new(&args.flag);
    let Some(rule) = cx.progress.flags.iter().find(|rule| rule.flag == flag) else {
        return Err(CompileError::code("unresolved-progress-flag"));
    };
    if (args.count.is_some() || args.at_least.is_some()) && rule.count.is_none() {
        return Err(CompileError::code("unresolved-progress-count"));
    }
    Ok(Arc::new(Flag {
        quest: FactKey::new(&args.quest),
        flag,
        is: args.is,
        count: args.count,
        at_least: args.at_least,
    }))
}

struct Flag {
    quest: FactKey,
    flag: FactKey,
    is: Option<bool>,
    count: Option<u32>,
    at_least: Option<u32>,
}

impl PredicatePlan for Flag {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(progress) = matching_progress(cx, &self.quest) else {
            return Truth::Unknown;
        };
        if !matches!(&progress.stage, Knowledge::Known(_)) {
            return Truth::Unknown;
        }
        let Some(flag) = progress.flags.iter().find(|flag| flag.flag == self.flag) else {
            return Truth::Unknown;
        };
        if let Some(expected) = self.count {
            return flag.count.map_or(no_count_evidence(flag.truth), |count| {
                truth(count == expected)
            });
        }
        if let Some(minimum) = self.at_least {
            return flag.count.map_or(no_count_evidence(flag.truth), |count| {
                truth(count >= minimum)
            });
        }
        match self.is {
            Some(expected) => {
                if expected {
                    flag.truth
                } else {
                    !flag.truth
                }
            }
            None => flag.truth,
        }
    }
}

fn validate_progress_quest(cx: &CompileContext<'_>, quest: &str) -> Result<(), CompileError> {
    cx.quests
        .quest(quest)
        .map_err(|_| CompileError::code("unresolved-quest"))?;
    if cx.path.0.as_ref() != quest {
        return Err(CompileError::code("foreign-progress-quest"));
    }
    Ok(())
}
fn matching_progress<'a>(
    cx: &'a PredicateContext<'_, '_>,
    quest: &FactKey,
) -> Option<&'a QuestProgress> {
    cx.progress
        .iter()
        .find(|progress| progress.quest == *quest && progress.evidence.run == cx.required_after.run)
}

fn truth(value: bool) -> Truth {
    if value {
        Truth::True
    } else {
        Truth::False
    }
}

fn no_count_evidence(value: Truth) -> Truth {
    match value {
        Truth::Unknown => Truth::Unknown,
        Truth::True | Truth::False => Truth::False,
    }
}
