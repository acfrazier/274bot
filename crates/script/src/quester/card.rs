//! Compiled card `Quester`: prepare validates settings + release index only.
use super::compile::{path_bytes, INDEX_JSON};
use super::queue::{Queue, QueueSettings, ReleaseIndex};
use super::runner::QueuedQuester;
use crate::native::{
    CompiledCard, ConfigError, PrepareContext, PreparedConfig, RetainedMemory, SettingsBag,
    StartError,
};
use crate::{CompiledId, SettingDef};
use api::quest_facts::QuestCatalog;
use api::selected::RunKey;
use serde::Deserialize;
use std::sync::{Arc, LazyLock};

pub const CARD: CompiledCard = CompiledCard {
    id: CompiledId("Quester"),
    name: "Quester",
    description: "Runs authored quest Paths from live observation.",
    category: "Quests",
    schema_version: 3,
    schema: settings_schema,
    per_account_settings: &["partner_account", "gang"],
    prepare,
    create,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuesterSettings {
    #[serde(default)]
    quests: Vec<String>,
    #[serde(default)]
    order_override: Vec<String>,
    #[serde(default)]
    skip: Vec<String>,
    #[serde(default)]
    partner_account: Option<String>,
    #[serde(default)]
    gang: Option<String>,
    #[serde(default = "crate::native::death::default_max_deaths")]
    max_deaths: u8,
}

static RELEASE_INDEX: LazyLock<ReleaseIndex> =
    LazyLock::new(|| serde_json::from_str(INDEX_JSON).expect("released Path index"));

fn settings_schema() -> &'static [SettingDef] {
    static SETTINGS: LazyLock<Vec<SettingDef>> = LazyLock::new(|| {
        let mut max_deaths = setting(
            "max_deaths",
            "number",
            "2",
            "Maximum deaths",
            "Block on a death after this many deaths across the queue.",
            &[],
        );
        max_deaths.min = Some("0".into());
        max_deaths.max = Some("255".into());
        max_deaths.step = Some("1".into());

        vec![
            path_setting(
                "quests",
                "[]",
                "Quests",
                "Empty selects all released quests.",
            ),
            path_setting(
                "order_override",
                "[]",
                "Order override",
                "Prioritize these selected quest ids; remaining quests retain release order.",
            ),
            path_setting("skip", "[]", "Skip", "Do not run these released quest ids."),
            setting(
                "partner_account",
                "string",
                "",
                "Partner account",
                "Configured account profile used by partner quests.",
                &[],
            ),
            setting(
                "gang",
                "string",
                "",
                "Gang",
                "Explicit partner-quest gang for this account.",
                &["phoenix", "blackarm"],
            ),
            max_deaths,
        ]
    });
    &SETTINGS
}

fn released_setting_paths() -> &'static [(String, String)] {
    static PATHS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
        RELEASE_INDEX
            .paths
            .iter()
            .filter_map(|entry| {
                let bytes = released_path(&entry.id)?;
                let document: super::path::PathDocument =
                    serde_json::from_slice(bytes).expect("released Path document");
                Some((entry.id.clone(), document.display_name))
            })
            .collect()
    });
    PATHS.as_slice()
}

fn path_setting(id: &str, default: &str, label: &str, help: &str) -> SettingDef {
    let mut def = setting(id, "string[]", default, label, help, &[]);
    let paths = released_setting_paths();
    def.options = paths.iter().map(|(id, _)| id.clone()).collect();
    def.option_labels = paths.iter().map(|(_, display)| display.clone()).collect();
    def.options_from = Some(
        if id == "order_override" {
            "released-path-order"
        } else {
            "released-paths"
        }
        .into(),
    );
    def
}
fn setting(
    id: &str,
    ty: &str,
    default: &str,
    label: &str,
    help: &str,
    options: &[&str],
) -> SettingDef {
    SettingDef {
        id: id.into(),
        ty: ty.into(),
        default: Some(default.into()),
        label: Some(label.into()),
        min: None,
        max: None,
        step: None,
        options: options.iter().map(|option| (*option).into()).collect(),
        option_labels: options.iter().map(|option| (*option).into()).collect(),
        group: Some("Quester".into()),
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: Some(help.into()),
        item_option_spec: None,
    }
}

fn released(index: &ReleaseIndex, id: &str) -> bool {
    index.schema == 1
        && index.paths.iter().any(|path| {
            path.id == id && path.file.strip_suffix(".json") == Some(id) && path_bytes(id).is_some()
        })
}

/// The released document gate shared by the card and script progress API.
pub fn released_path(id: &str) -> Option<&'static [u8]> {
    released(&RELEASE_INDEX, id)
        .then(|| path_bytes(id))
        .flatten()
}

#[cfg(feature = "load")]
pub fn released_paths() -> &'static [crate::api_progress::QuestPathRow] {
    static ROWS: LazyLock<Vec<crate::api_progress::QuestPathRow>> = LazyLock::new(|| {
        RELEASE_INDEX
            .paths
            .iter()
            .filter_map(|entry| released_path(&entry.id))
            .map(|bytes| {
                let document: super::path::PathDocument =
                    serde_json::from_slice(bytes).expect("released Path document");
                let progress = document.roles[0]
                    .progress
                    .as_ref()
                    .expect("released Path progress");
                let mut stages = [
                    &progress.colour.not_started,
                    &progress.colour.in_progress,
                    &progress.colour.complete,
                ]
                .into_iter()
                .chain(progress.rules.iter().map(|rule| &rule.stage))
                .map(|stage| Arc::clone(&stage.0))
                .collect::<Vec<_>>();
                stages.sort_unstable_by_key(|stage| {
                    stage
                        .rsplit_once(':')
                        .and_then(|(_, ordinal)| ordinal.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });
                stages.dedup();
                crate::api_progress::QuestPathRow {
                    id: Arc::clone(&document.id.0),
                    display: document.display_name.into(),
                    journal: !progress.rules.is_empty(),
                    stages: stages.into(),
                }
            })
            .collect()
    });
    &ROWS
}

struct Prepared {
    selected: Arc<api::game_data::SelectedGameData>,
    quests: Arc<QuestCatalog>,
    banks: Arc<api::named_banks::NamedBankFacts>,
    queue: Queue<'static>,
    max_deaths: u8,
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
    let gang = match settings
        .gang
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("") => None,
        Some("phoenix") => Some(super::pair::Gang::Phoenix),
        Some("blackarm") => Some(super::pair::Gang::BlackArm),
        Some(_) => {
            return Err(StartError::Config(ConfigError::new(
                "gang",
                "invalid-value",
                "choose phoenix or blackarm",
            )))
        }
    };
    let queue_settings = QueueSettings {
        quests: settings.quests,
        order_override: settings.order_override,
        skip: settings.skip,
        partner_account: settings.partner_account.and_then(|account| {
            let account = account.trim();
            (!account.is_empty()).then(|| super::pair::AccountKey(Arc::from(account)))
        }),
        gang,
    };
    for entry in &RELEASE_INDEX.paths {
        if released_path(&entry.id).is_none() {
            return Err(StartError::Unavailable(Arc::from(format!(
                "release index Path is unavailable: {} ({})",
                entry.id, entry.file
            ))));
        }
    }
    let queue = Queue::from_index(&RELEASE_INDEX, queue_settings).map_err(|error| {
        StartError::Config(ConfigError::new(
            error.field,
            "invalid-queue",
            error.message,
        ))
    })?;
    let quests =
        QuestCatalog::from_identity(cx.selected.quest_identity()).map_err(StartError::Facts)?;
    let prepared = Prepared {
        selected: Arc::clone(&cx.selected),
        quests: Arc::new(quests),
        banks: Arc::clone(&cx.banks),
        queue,
        max_deaths: settings.max_deaths,
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
    retained: &mut RetainedMemory,
) -> Result<Box<dyn crate::native::Script>, StartError> {
    let prepared = config.get::<Prepared>().ok_or_else(|| {
        StartError::Config(ConfigError::new("", "config-identity", "not Quester"))
    })?;
    let mut script = QueuedQuester::new_with_max_deaths(
        run,
        Arc::clone(&prepared.selected),
        Arc::clone(&prepared.quests),
        Arc::clone(&prepared.banks),
        prepared.queue.clone(),
        prepared.max_deaths,
    );
    script.restore(retained.quester());
    Ok(Box::new(script))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, FamilyPreparation};

    fn released(index_json: &str, id: &str) -> bool {
        serde_json::from_str::<ReleaseIndex>(index_json)
            .is_ok_and(|index| super::released(&index, id))
    }

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
    fn release_index_is_authoritative_and_includes_imp_when_embedded() {
        for id in ["cook", "sheep", "runemysteries", "romeojuliet", "imp"] {
            assert!(released(INDEX_JSON, id), "{id} missing from release index");
            assert!(released_path(id).is_some(), "{id} body is not embedded");
        }
        assert!(released(
            r#"{"schema":1,"paths":[{"id":"other","file":"other.json"},{"id":"cook","file":"cook.json"}]}"#,
            "cook"
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
            assert!(!released(rejected, "cook"), "accepted {rejected}");
        }
    }

    #[test]
    fn settings_replace_the_single_quest_setting_and_keep_partner_per_account() {
        let ids = settings_schema()
            .iter()
            .map(|setting| setting.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "quests",
                "order_override",
                "skip",
                "partner_account",
                "gang",
                "max_deaths",
            ]
        );
        assert_eq!(CARD.per_account_settings, ["partner_account", "gang"]);
        let paths = released_setting_paths();
        assert!(!paths.is_empty());
        let quests = settings_schema()
            .iter()
            .find(|setting| setting.id == "quests")
            .unwrap();
        let expected_ids = paths.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
        let expected_labels = paths
            .iter()
            .map(|(_, display)| display.as_str())
            .collect::<Vec<_>>();
        assert_eq!(quests.ty, "string[]");
        assert_eq!(
            quests
                .options
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(
            quests
                .option_labels
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected_labels
        );
        assert_eq!(quests.options_from.as_deref(), Some("released-paths"));
        let skip = settings_schema()
            .iter()
            .find(|setting| setting.id == "skip")
            .unwrap();
        assert_eq!(skip.ty, "string[]");
        assert_eq!(
            skip.options.iter().map(String::as_str).collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(
            skip.option_labels
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected_labels
        );
        assert_eq!(skip.options_from.as_deref(), Some("released-paths"));
        let order = settings_schema()
            .iter()
            .find(|setting| setting.id == "order_override")
            .unwrap();
        assert_eq!(order.options_from.as_deref(), Some("released-path-order"));

        let mut bag = SettingsBag::new();
        bag.insert("quest".into(), serde_json::Value::String("cook".into()));
        let settings = QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
            bag.iter().map(|(key, value)| (key.as_str(), value)),
        ));
        assert!(
            settings.is_err(),
            "obsolete single-quest setting must be rejected"
        );
    }
    #[test]
    fn max_deaths_defaults_to_two_and_matches_the_gatherer_bounds() {
        let empty = SettingsBag::new();
        let defaults = QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
            empty.iter().map(|(key, value)| (key.as_str(), value)),
        ))
        .unwrap();
        assert_eq!(defaults.max_deaths, 2);
        assert_eq!(CARD.schema_version, 3);

        let max_deaths = settings_schema()
            .iter()
            .find(|setting| setting.id == "max_deaths")
            .unwrap();
        assert_eq!(max_deaths.ty, "number");
        assert_eq!(max_deaths.default.as_deref(), Some("2"));
        assert_eq!(max_deaths.min.as_deref(), Some("0"));
        assert_eq!(max_deaths.max.as_deref(), Some("255"));
        assert_eq!(max_deaths.step.as_deref(), Some("1"));

        for cap in [0, 255] {
            let mut bag = SettingsBag::new();
            bag.insert("max_deaths".into(), serde_json::json!(cap));
            let settings = QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
                bag.iter().map(|(key, value)| (key.as_str(), value)),
            ))
            .unwrap();
            assert_eq!(settings.max_deaths, cap);
        }
        for cap in [-1, 256] {
            let mut bag = SettingsBag::new();
            bag.insert("max_deaths".into(), serde_json::json!(cap));
            assert!(
                QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
                    bag.iter().map(|(key, value)| (key.as_str(), value)),
                ))
                .is_err(),
                "out-of-range max_deaths {cap} must be rejected"
            );
        }
    }
}
