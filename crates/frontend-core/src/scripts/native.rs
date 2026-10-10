//! Native schema, per-account settings, presentation and fenced controls.
use super::{LiveSettings, Scripts};
use crate::operations::OperationId;
use crate::session::{ArmMirror, OperatorSession};
use api::selected::RunKey;
use script::native::ScriptStatus;
use script::shim::ScriptPaint;
use script::{RunState, ScriptLifecycleReceipt, SettingDef};
use serde_json::{Map, Value};
use std::sync::Arc;

pub enum SchemaView<'a> {
    Unavailable(&'a str),
    Ready {
        version: u16,
        fields: &'a [SettingDef],
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCommand {
    Pause,
    Resume,
    Stop,
}
#[derive(Debug, Clone)]
pub struct NativeTarget {
    pub profile: String,
    pub run: RunKey,
}
pub struct NativeDetail {
    pub lifecycle: RunState,
    pub status: Option<Arc<ScriptStatus>>,
    pub paint: Option<Arc<ScriptPaint>>,
    pub terminal: Option<ScriptLifecycleReceipt>,
}

impl Scripts {
    /// A known empty schema is Ready. Unknown cards, malformed envelopes and
    /// unsupported saved versions remain visibly unavailable.
    pub fn compiled_schema<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: Option<&str>,
        id: script::CompiledId,
    ) -> SchemaView<'static> {
        let Some(card) = script::compiled_card(id) else {
            return SchemaView::Unavailable("compiled card unavailable");
        };
        if let Some(entry) = profile
            .and_then(|profile| core.settings_for_edit(profile))
            .and_then(|settings| {
                settings
                    .script_settings
                    .get(&script::compiled_identity_key(id))
            })
        {
            match vault::CompiledSettingsRecord::view(entry) {
                Err(reason) => return SchemaView::Unavailable(reason),
                Ok((version, _)) if version != card.schema_version => {
                    return SchemaView::Unavailable(
                        "saved compiled settings schema is unsupported",
                    );
                }
                _ => {}
            }
        }
        SchemaView::Ready {
            version: card.schema_version,
            fields: (card.schema)(),
        }
    }

    pub fn compiled_overrides<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        id: script::CompiledId,
    ) -> Result<Map<String, Value>, String> {
        let settings = core
            .settings_for_edit(profile)
            .ok_or_else(|| format!("no profile: {profile}"))?;
        compiled_values(settings, id)
    }

    pub fn compiled_edit_bag<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: Option<&str>,
        id: script::CompiledId,
    ) -> Result<Map<String, Value>, String> {
        let card = script::compiled_card(id).ok_or("compiled card unavailable")?;
        let (overrides, partner) = match profile {
            Some(profile) => {
                let settings = core
                    .settings_for_edit(profile)
                    .ok_or("profile unavailable")?;
                (
                    compiled_values(settings, id)?,
                    settings.clue_duel_partner.as_str(),
                )
            }
            None => (Map::new(), ""),
        };
        Ok(self.run_bag_with((card.schema)(), &overrides, partner))
    }

    /// Start consumes durable settings. A still-preparing or failed save cannot
    /// sneak into a newly started run via the editor's draft projection.
    pub fn compiled_bag<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        id: script::CompiledId,
    ) -> Result<Map<String, Value>, String> {
        let card = script::compiled_card(id)
            .ok_or_else(|| format!("compiled card unavailable: {}", id.0))?;
        let row = core
            .durable_profile(profile)
            .ok_or_else(|| format!("no durable profile: {profile}"))?;
        let overrides = compiled_values(&row.settings, id)?;
        Ok(self.run_bag_with((card.schema)(), &overrides, &row.settings.clue_duel_partner))
    }

    pub fn set_compiled_setting<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        id: script::CompiledId,
        field: &str,
        value: Value,
    ) -> Result<OperationId, String> {
        let card = script::compiled_card(id).ok_or("compiled card unavailable")?;
        if !(card.schema)().iter().any(|setting| setting.id == field) {
            return Err(format!("unknown compiled setting: {field}"));
        }
        let mut values = self.compiled_overrides(core, profile, id)?;
        values.insert(field.into(), value);
        self.set_compiled_overrides(core, profile, id, values)
    }

    /// Bulk and single-account edits use the same prepare → durable → configure
    /// transaction; the session reports a SettingsWrite for every accepted edit.
    pub fn set_compiled_overrides<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        id: script::CompiledId,
        values: Map<String, Value>,
    ) -> Result<OperationId, String> {
        self.set_compiled_overrides_assigning(core, profile, id, values, None)
    }

    /// Like [`Self::set_compiled_overrides`], optionally assigning the card in
    /// the same profile write. The copy is validated before that write, so a
    /// rejected envelope keeps the old assignment.
    pub(crate) fn set_compiled_overrides_assigning<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        id: script::CompiledId,
        values: Map<String, Value>,
        assignment: Option<vault::ScriptAssignment>,
    ) -> Result<OperationId, String> {
        let card = script::compiled_card(id).ok_or("compiled card unavailable")?;
        // Do not overwrite an unknown saved schema with current defaults.
        self.compiled_overrides(core, profile, id)?;
        let key = script::compiled_identity_key(id);
        let schema = (card.schema)();
        let mut values = values;
        for setting in schema {
            if let Some(value) = values.get_mut(&setting.id) {
                *value = script::coerce_setting_value(&setting.ty, value);
            }
        }
        let bag = Arc::new(self.run_bag(core, profile, schema, &values));
        let live = super::live_fence(core, profile, &key).and_then(|(identity, generation)| {
            let run = core.play()?.script_native_run(profile)?;
            Some(LiveSettings {
                identity,
                generation,
                run: Some(run),
                bag: Arc::clone(&bag),
                prepared: None,
            })
        });
        let log_action = if assignment.is_some() {
            "native sync assignment"
        } else {
            "native parameters"
        };
        let result = self.upsert_profile_settings(
            core,
            profile,
            ArmMirror::NativeSettings {
                id,
                bag,
                live,
                assign: assignment.is_some(),
            },
            log_action,
            |settings| {
                if let Some(assignment) = assignment {
                    settings.script_assignment = Some(assignment);
                }
                settings.script_settings.insert(
                    key.clone(),
                    vault::CompiledSettingsRecord {
                        schema_version: card.schema_version,
                        values,
                    }
                    .into_entry(),
                );
            },
        );
        if let Ok(op) = result.as_ref() {
            self.track_settings_card(*op, script::ScriptSel::Compiled(id), card.name);
            self.sync.source_edited(profile, &key);
            self.clear_notice();
        }
        result
    }

    pub fn native_target<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
    ) -> Option<NativeTarget> {
        Some(NativeTarget {
            profile: profile.into(),
            run: core.play()?.script_native_run(profile)?,
        })
    }

    pub fn native_detail<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
    ) -> Option<NativeDetail> {
        let play = core.play()?;
        let status = play.script_native_status(profile);
        let lifecycle = play.script_state(profile);
        let compiled = play
            .script_source_identity(profile)
            .is_some_and(|identity| identity.starts_with("compiled:"));
        let blocked_stop = lifecycle == RunState::Idle
            && status.as_ref().is_some_and(|status| {
                status.phase == script::native::NativePhase::Blocked && status.failure.is_some()
            });
        if !compiled && !blocked_stop {
            return None;
        }
        Some(NativeDetail {
            lifecycle,
            status,
            paint: play.script_paint(profile),
            terminal: play.script_lifecycle_receipt(profile),
        })
    }

    pub fn native_command<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        target: &NativeTarget,
        command: NativeCommand,
    ) -> Result<(), String> {
        let play = core.play().ok_or("no play")?;
        let accepted = match command {
            NativeCommand::Pause => play.script_native_pause(&target.profile, target.run, true),
            NativeCommand::Resume => play.script_native_pause(&target.profile, target.run, false),
            NativeCommand::Stop => play.script_native_stop(&target.profile, target.run),
        };
        if accepted {
            Ok(())
        } else {
            Err("stale native run".into())
        }
    }
}

fn compiled_values(
    settings: &vault::ProfileSettings,
    id: script::CompiledId,
) -> Result<Map<String, Value>, String> {
    let card = script::compiled_card(id).ok_or("compiled card unavailable")?;
    let Some(entry) = settings
        .script_settings
        .get(&script::compiled_identity_key(id))
    else {
        return Ok(Map::new());
    };
    let (version, values) = vault::CompiledSettingsRecord::view(entry)?;
    if version != card.schema_version {
        return Err(format!(
            "compiled schema {version} unavailable for {} (supported {})",
            id.0, card.schema_version
        ));
    }
    Ok(values.clone())
}
