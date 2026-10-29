use crate::native::{
    CompiledCard, ConfigError, NativeTick, PrepareContext, PreparedConfig, RetainedMemory, Script,
    ScriptFailure, ScriptFlow, SettingsBag, StartError,
};
use crate::quester::prereq::walk::PrereqWalk;
use crate::{CompiledId, SettingDef};
use api::selected::RunKey;
use std::sync::Arc;
use std::task::Poll;

pub(crate) const CARD: CompiledCard = CompiledCard {
    id: CompiledId("WalkTo"),
    native_kind: crate::native::request::NativeKind::Walk,
    name: "WalkTo",
    description: "Internal continuation for a host-owned manual walk.",
    category: "Internal",
    schema_version: 1,
    schema,
    per_account_settings: &[],
    prepare,
    create,
    lifecycle: None,
};

fn schema() -> &'static [SettingDef] {
    &[]
}

fn prepare(
    _cx: &mut PrepareContext<'_>,
    revision: u64,
    settings: Arc<SettingsBag>,
) -> Result<Arc<PreparedConfig>, StartError> {
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        settings,
        (),
    ))
}

fn create(
    run: RunKey,
    config: Arc<PreparedConfig>,
    retained: &mut RetainedMemory,
) -> Result<Box<dyn Script>, StartError> {
    if config.get::<()>().is_none() {
        return Err(StartError::Config(ConfigError::new(
            "",
            "config-identity",
            "not a prepared internal WalkTo card",
        )));
    }
    let handover = retained.take_walk_handover(run).ok_or_else(|| {
        StartError::Unavailable("manual WalkTo handover is missing for this run".into())
    })?;
    Ok(Box::new(WalkTo {
        walk: PrereqWalk::from_handover(handover),
    }))
}

struct WalkTo {
    walk: PrereqWalk,
}

impl Script for WalkTo {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        match self.walk.poll(tick.actions, &mut tick.cx, tick.state) {
            Poll::Pending => Ok(ScriptFlow::Continue),
            Poll::Ready(Ok(receipt)) if receipt.end == crate::native::WalkEnd::Arrived => {
                Ok(ScriptFlow::Complete)
            }
            Poll::Ready(Ok(receipt)) => {
                let message = match receipt.refusal.as_ref() {
                    Some(crate::native::WalkRefusal::Prerequisite(refusal)) => {
                        format!("manual walk refused: {refusal:?}")
                    }
                    _ => receipt
                        .failure_detail()
                        .map(|detail| format!("manual walk failed: {detail}"))
                        .unwrap_or_else(|| format!("manual walk ended as {:?}", receipt.end)),
                };
                Ok(ScriptFlow::Blocked(ScriptFailure {
                    code: Arc::from("manual-walk-failed"),
                    message: message.into(),
                }))
            }
            Poll::Ready(Err(error)) => Ok(ScriptFlow::Blocked(ScriptFailure {
                code: Arc::from("manual-walk-failed"),
                message: format!("manual walk failed: {error:?}").into(),
            })),
        }
    }
}
