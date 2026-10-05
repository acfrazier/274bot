//! Compiled card `Quester`: Start snapshots the shared Path registry.
#[cfg(test)]
use super::compile::INDEX_JSON;
use super::queue::{Queue, QueueSettings};
use super::registry::{self, BUNDLED_INDEX};
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
    per_account_settings: &["partner_account", "gang", "crest_gauntlets"],
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
    #[serde(default)]
    crest_gauntlets: super::choices::CrestGauntlets,
    #[serde(default = "crate::native::death::default_max_deaths")]
    max_deaths: u8,
    #[serde(default, rename = "allow_teleports")]
    _allow_teleports: bool,
    #[serde(default, rename = "allow_wilderness")]
    _allow_wilderness: bool,
    #[serde(default, rename = "allow_danger_zones")]
    _allow_danger_zones: bool,
}

fn decode_settings(bag: &SettingsBag) -> Result<QuesterSettings, ConfigError> {
    QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
        bag.iter().map(|(key, value)| (key.as_str(), value)),
    ))
    .map_err(|error| ConfigError::new("", "invalid-settings", error.to_string()))
}

pub(crate) fn requires_pairs(bag: &SettingsBag) -> Result<bool, ConfigError> {
    let settings = decode_settings(bag)?;
    Ok(BUNDLED_INDEX.paths.iter().any(|entry| {
        super::pair::PairQuest::from_path(&entry.id).is_some()
            && !settings.skip.contains(&entry.id)
            && (settings.quests.is_empty() || settings.quests.contains(&entry.id))
    }))
}

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
                "Irreversible gang choice for an account not yet joined; must match its owned journal.",
                &["phoenix", "blackarm"],
            ),
            setting(
                "crest_gauntlets",
                "string",
                "chaos",
                "Family Crest gauntlets",
                "Gauntlet enchantment for this account. Defaults to chaos.",
                &["chaos", "cooking", "goldsmith"],
            ),
            max_deaths,
            walk_permission_setting(
                "allow_teleports",
                "Allow teleports",
                "Allow this script to use teleports when the global setting is off.",
            ),
            walk_permission_setting(
                "allow_wilderness",
                "Allow wilderness",
                "Allow this script to route through wilderness when the global setting is off.",
            ),
            walk_permission_setting(
                "allow_danger_zones",
                "Allow danger zones",
                "Allow this script to route through danger zones when the global setting is off.",
            ),
        ]
    });
    &SETTINGS
}

/// Quest picker rows in release order: released Paths with their display
/// names, plus unavailable rows labelled once with their reason, which the
/// picker shows as non-selectable.
fn released_setting_paths() -> &'static [(String, String)] {
    static PATHS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
        registry::PathRegistry::Bundled
            .rows()
            .iter()
            .map(|row| (row.id.clone(), row.label.clone()))
            .collect()
    });
    PATHS.as_slice()
}

/// End-user reason a release-roster quest can't run on this server, if any.
/// Pickers show such quests as non-selectable; the queue keeps them blocked.
pub fn unavailable_quest(id: &str) -> Option<&'static str> {
    BUNDLED_INDEX
        .paths
        .iter()
        .find(|entry| entry.id == id)?
        .unavailable
        .as_deref()
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

fn walk_permission_setting(id: &str, label: &str, help: &str) -> SettingDef {
    let mut definition = setting(id, "boolean", "false", label, help, &[]);
    definition.group = Some("Walk permissions".into());
    definition
}

#[cfg(feature = "load")]
pub fn released_paths() -> &'static [crate::api_progress::QuestPathRow] {
    static ROWS: LazyLock<Vec<crate::api_progress::QuestPathRow>> = LazyLock::new(|| {
        BUNDLED_INDEX
            .paths
            .iter()
            .filter_map(|entry| registry::bundled_path(&entry.id))
            .map(|bytes| {
                let document: super::path::PathDocument =
                    serde_json::from_slice(bytes).expect("released Path document");
                let mut stages = document
                    .roles
                    .iter()
                    .flat_map(|role| {
                        let progress = role.progress.as_ref().expect("released Path progress");
                        [
                            &progress.colour.not_started,
                            &progress.colour.in_progress,
                            &progress.colour.complete,
                        ]
                        .into_iter()
                        .chain(progress.rules.iter().map(|rule| &rule.stage))
                        .map(|stage| Arc::clone(&stage.0))
                    })
                    .collect::<Vec<_>>();
                let ordinal = |stage: &Arc<str>| {
                    stage
                        .rsplit_once(':')
                        .and_then(|(_, ordinal)| ordinal.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                };
                stages.sort_unstable_by(|left, right| {
                    ordinal(left)
                        .cmp(&ordinal(right))
                        .then_with(|| left.cmp(right))
                });
                stages.dedup();
                crate::api_progress::QuestPathRow {
                    id: Arc::clone(&document.id.0),
                    display: document.display_name.into(),
                    journal: document.kind == super::path::PathKind::Quest
                        && document.roles.iter().any(|role| {
                            role.progress
                                .as_ref()
                                .is_some_and(|progress| !progress.rules.is_empty())
                        }),
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
    queue: Queue,
    max_deaths: u8,
    choices: super::choices::QuestChoices,
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
    let settings = decode_settings(&bag).map_err(StartError::Config)?;
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
    for entry in &BUNDLED_INDEX.paths {
        if entry.unavailable.is_none() && registry::bundled_path(&entry.id).is_none() {
            return Err(StartError::Unavailable(Arc::from(format!(
                "release index Path is unavailable: {} ({})",
                entry.id,
                entry.file.as_deref().unwrap_or_default()
            ))));
        }
    }
    let quests =
        QuestCatalog::from_identity(cx.selected.quest_identity()).map_err(StartError::Facts)?;
    let registry = registry::reload_with_catalog(&cx.selected, Some(&quests)).map_err(|error| {
        StartError::Unavailable(
            format!(
                "{}: {}",
                error.code,
                error.detail.as_deref().unwrap_or("Path reload failed")
            )
            .into(),
        )
    })?;
    let queue = Queue::from_registry(registry, queue_settings).map_err(|error| {
        StartError::Config(ConfigError::new(
            error.field,
            "invalid-queue",
            error.message,
        ))
    })?;
    let prepared = Prepared {
        selected: Arc::clone(&cx.selected),
        quests: Arc::new(quests),
        banks: Arc::clone(&cx.banks),
        queue,
        max_deaths: settings.max_deaths,
        choices: super::choices::QuestChoices {
            crest_gauntlets: settings.crest_gauntlets,
        },
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
    script.set_choices(prepared.choices);
    script.restore(retained.quester());
    Ok(Box::new(script))
}

#[cfg(test)]
mod tests {
    use super::super::queue::ReleaseIndex;
    use super::*;
    use api::selected::{ClientRevision, FamilyPreparation};

    fn released(index_json: &str, id: &str) -> bool {
        serde_json::from_str::<ReleaseIndex>(index_json)
            .is_ok_and(|index| registry::bundled_in(&index, id))
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
            assert!(
                registry::bundled_path(id).is_some(),
                "{id} body is not embedded"
            );
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
                "crest_gauntlets",
                "max_deaths",
                "allow_teleports",
                "allow_wilderness",
                "allow_danger_zones",
            ]
        );
        assert_eq!(
            CARD.per_account_settings,
            ["partner_account", "gang", "crest_gauntlets"]
        );
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
        assert!(!defaults._allow_teleports);
        assert!(!defaults._allow_wilderness);
        assert!(!defaults._allow_danger_zones);
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
    #[test]
    fn crest_reward_is_a_persisted_account_choice_with_a_chaos_default() {
        use super::super::choices::CrestGauntlets;
        let decode = |value: serde_json::Value| serde_json::from_value::<QuesterSettings>(value);
        assert_eq!(
            decode(serde_json::json!({})).unwrap().crest_gauntlets,
            CrestGauntlets::Chaos
        );
        for (value, expected) in [
            ("chaos", CrestGauntlets::Chaos),
            ("cooking", CrestGauntlets::Cooking),
            ("goldsmith", CrestGauntlets::Goldsmith),
        ] {
            assert_eq!(
                decode(serde_json::json!({"crest_gauntlets": value}))
                    .unwrap()
                    .crest_gauntlets,
                expected
            );
        }
        assert!(decode(serde_json::json!({"crest_gauntlets": "random"})).is_err());
        assert!(CARD.per_account_settings.contains(&"crest_gauntlets"));
        let setting = settings_schema()
            .iter()
            .find(|setting| setting.id == "crest_gauntlets")
            .unwrap();
        assert_eq!(setting.default.as_deref(), Some("chaos"));
        assert_eq!(setting.options, ["chaos", "cooking", "goldsmith"]);
    }

    #[test]
    fn unavailable_rows_are_picker_rows_never_released_and_refuse_an_only_pick() {
        let reason = unavailable_quest("hauntedmine").expect("Haunted Mine row");
        assert!(!reason.is_empty());
        assert!(registry::bundled_path("hauntedmine").is_none());
        assert!(unavailable_quest("cook").is_none());
        let label = format!("Haunted Mine — {reason}");
        assert!(released_setting_paths()
            .iter()
            .any(|(id, name)| id == "hauntedmine" && *name == label));
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let expected = format!("Haunted Mine: {reason}");
        FamilyPreparation::run(move |families| {
            let pin = selected.selected_pin().unwrap();
            let mut cx = PrepareContext {
                selected,
                pin,
                banks: Arc::new(api::named_banks::NamedBankFacts::empty()),
                families,
            };
            let mut bag = SettingsBag::new();
            bag.insert("quests".into(), serde_json::json!(["hauntedmine"]));
            match prepare(&mut cx, 1, Arc::new(bag)) {
                Err(StartError::Config(error)) => {
                    assert_eq!(error.field.as_ref(), "quests");
                    assert_eq!(error.message.as_ref(), expected);
                }
                other => panic!("an unavailable-only pick must refuse Start, got {other:?}"),
            }
        })
        .unwrap()
        .join()
        .unwrap();
    }
}
#[cfg(test)]
mod walk_permission_settings_tests {
    use super::*;

    #[test]
    fn walk_permission_settings_roundtrip_with_legacy_defaults_and_schema_v3() {
        assert_eq!(CARD.schema_version, 3);
        let ids = ["allow_teleports", "allow_wilderness", "allow_danger_zones"];
        for id in ids {
            let definition = settings_schema()
                .iter()
                .find(|setting| setting.id == id)
                .unwrap();
            assert_eq!(definition.ty, "boolean");
            assert_eq!(definition.default.as_deref(), Some("false"));
            assert_eq!(definition.group.as_deref(), Some("Walk permissions"));
        }

        let mut bag = SettingsBag::new();
        bag.insert("allow_teleports".into(), serde_json::json!(true));
        bag.insert("allow_wilderness".into(), serde_json::json!(false));
        bag.insert("allow_danger_zones".into(), serde_json::json!(true));
        let restored: SettingsBag =
            serde_json::from_value(serde_json::to_value(&bag).unwrap()).unwrap();
        assert_eq!(restored, bag);
        let settings = QuesterSettings::deserialize(serde::de::value::MapDeserializer::new(
            restored.iter().map(|(key, value)| (key.as_str(), value)),
        ))
        .unwrap();
        assert!(settings._allow_teleports);
        assert!(!settings._allow_wilderness);
        assert!(settings._allow_danger_zones);
    }
}
