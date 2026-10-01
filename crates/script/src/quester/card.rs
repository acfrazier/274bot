//! Compiled card `Quester`: prepare validates settings + release index only.
use super::compile::{compile_path, cook_bytes, INDEX_JSON};
use super::runner::Quester;
use crate::native::{
    CompiledCard, ConfigError, PrepareContext, PreparedConfig, RetainedMemory, SettingsBag,
    StartError,
};
use crate::CompiledId;
use api::quest_facts::QuestCatalog;
use api::selected::RunKey;
use serde::Deserialize;
use std::sync::Arc;

pub const CARD: CompiledCard = CompiledCard {
    id: CompiledId("Quester"),
    name: "Quester",
    description: "Runs authored quest Paths from live observation.",
    category: "Quests",
    schema_version: 2,
    schema: || &[],
    per_account_settings: &[],
    prepare,
    create,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuesterSettings {
    #[serde(default)]
    quest: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseIndex {
    schema: u16,
    paths: Vec<ReleasePath>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleasePath {
    id: String,
    file: String,
}
fn cook_released(index_json: &str) -> bool {
    serde_json::from_str::<ReleaseIndex>(index_json).is_ok_and(|index| {
        index.schema == 1
            && index
                .paths
                .iter()
                .any(|path| path.id == "cook" && path.file == "cook.json")
    })
}
static COOK_RELEASED: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| cook_released(INDEX_JSON));

struct Prepared {
    selected: Arc<api::game_data::SelectedGameData>,
    quests: Arc<QuestCatalog>,
    _quest: String,
}

fn prepare(
    cx: &mut PrepareContext<'_>,
    revision: u64,
    bag: Arc<SettingsBag>,
) -> Result<Arc<PreparedConfig>, StartError> {
    if cx
        .selected
        .selected_pin()
        .map_err(StartError::Facts)?
        .as_ref()
        != cx.pin.as_ref()
    {
        return Err(StartError::Facts(api::selected::FactError::PinMismatch));
    }
    if cx.pin.revision != api::selected::ClientRevision::R289 {
        return Err(StartError::Unavailable(
            "Quester S1 supports revision 289 only".into(),
        ));
    }
    let settings = QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
        bag.iter().map(|(key, value)| (key.as_str(), value)),
    ))
    .map_err(|e| StartError::Config(ConfigError::new("", "invalid-settings", e.to_string())))?;
    if !*COOK_RELEASED {
        return Err(StartError::Unavailable("release index missing cook".into()));
    }
    let quest = settings.quest.unwrap_or_else(|| "cook".into());
    if quest != "cook" {
        return Err(StartError::Config(ConfigError::new(
            "quest",
            "unknown-path",
            "S1 ships Cook's Assistant only",
        )));
    }
    let quests =
        QuestCatalog::from_identity(cx.selected.quest_identity()).map_err(StartError::Facts)?;
    let prepared = Prepared {
        selected: Arc::clone(&cx.selected),
        quests: Arc::new(quests),
        _quest: quest,
    };
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        bag,
        prepared,
    ))
}

fn create(
    run: RunKey,
    config: Arc<PreparedConfig>,
    _retained: &mut RetainedMemory,
) -> Result<Box<dyn crate::native::Script>, StartError> {
    let prepared = config.get::<Prepared>().ok_or_else(|| {
        StartError::Config(ConfigError::new("", "config-identity", "not Quester"))
    })?;
    let path = compile_path(cook_bytes(), &prepared.selected, &prepared.quests).map_err(|err| {
        let message = match err.detail.as_deref() {
            Some(detail) => format!("compile: {}: {detail}", err.code),
            None => format!("compile: {}", err.code),
        };
        StartError::Unavailable(Arc::from(message))
    })?;
    Ok(Box::new(Quester::new(
        run,
        path,
        Arc::clone(&prepared.quests),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, FamilyPreparation};

    #[test]
    fn prepare_on_274_reports_289_only_without_creating_a_run() {
        let selected = api::game_data::for_revision(ClientRevision::R274).unwrap();
        FamilyPreparation::run(move |families| {
            let pin = selected.selected_pin().unwrap();
            let mut cx = PrepareContext {
                selected,
                pin,
                banks: Arc::new(api::named_banks::NamedBankFacts::empty()),
                families,
            };
            match prepare(&mut cx, 1, Arc::new(SettingsBag::new())) {
                Err(StartError::Unavailable(reason)) => {
                    assert_eq!(reason.as_ref(), "Quester S1 supports revision 289 only");
                }
                other => panic!("274 must be unavailable, got {other:?}"),
            }
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[test]
    fn release_index_requires_schema_and_exact_cook_identity() {
        assert!(cook_released(INDEX_JSON));
        assert!(cook_released(
            r#"{"schema":1,"paths":[{"id":"other","file":"other.json"},{"id":"cook","file":"cook.json"}]}"#
        ));
        for rejected in [
            r#"{"schema":2,"paths":[{"id":"cook","file":"cook.json"}]}"#,
            r#"{"schema":1,"paths":[]}"#,
            r#"{"schema":1,"paths":[{"id":"other","file":"cook.json"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":"other.json"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":"other.json"},{"id":"other","file":"cook.json"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":"cook.json","unknown":true}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":"cook.json"}],"unknown":true}"#,
            "not json",
        ] {
            assert!(!cook_released(rejected), "accepted {rejected}");
        }
    }
}
