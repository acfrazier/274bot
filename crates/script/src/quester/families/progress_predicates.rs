use super::super::compile::{CompileContext, CompileError, PredicateContext, PredicatePlan};
use api::quest_progress::QuestProgress;
use api::selected::{FactKey, Knowledge, Truth};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageInArgs {
    quest: String,
    any: Vec<String>,
}

pub fn compile_stage_in(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let args: StageInArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    cx.quests
        .quest(&args.quest)
        .map_err(|_| CompileError::code("unresolved-quest"))?;
    if args.any.is_empty() || args.any.iter().any(|stage| stage.is_empty()) {
        return Err(CompileError::code("invalid-args"));
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
#[serde(deny_unknown_fields)]
struct FlagArgs {
    quest: String,
    flag: String,
    #[serde(default)]
    is: Option<bool>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    at_least: Option<u32>,
}

pub fn compile_flag(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let args: FlagArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    cx.quests
        .quest(&args.quest)
        .map_err(|_| CompileError::code("unresolved-quest"))?;
    if args.flag.is_empty() || (args.count.is_some() && args.at_least.is_some()) {
        return Err(CompileError::code("invalid-args"));
    }
    Ok(Arc::new(Flag {
        quest: FactKey::new(&args.quest),
        flag: FactKey::new(&args.flag),
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
