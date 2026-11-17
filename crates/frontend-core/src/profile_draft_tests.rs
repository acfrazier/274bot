//! The guardian vocabulary both frontends read: both choices selectable and
//! saved for every event, an explanation per event that cannot disagree with
//! what the runtime admits, and a 0.2.9 `ignore` value reaching a draft as
//! Flee.

use vault::ProfileSettings;

use super::{
    guardian_event_help, guardian_fight_applies, GuardianPolicy, SettingsDraft, GUARDIAN_EVENTS,
};

/// Fight is a real, saved choice for all eight events (the two excluded ones
/// keep it saved too), Flee stays the default, and the two labels tell the
/// operator two different things without calling either unavailable.
#[test]
fn both_choices_are_selectable_and_saved_for_every_event() {
    let mut draft = SettingsDraft::from_settings(ProfileSettings::default());
    for (key, _) in GUARDIAN_EVENTS {
        assert_eq!(
            draft.random_event_policy(key),
            GuardianPolicy::Flee,
            "{key} defaults to Flee"
        );
        for option in GuardianPolicy::OPTIONS {
            draft.set_random_event_policy(key, option);
            assert_eq!(
                draft.random_event_policy(key),
                option,
                "{key} keeps {option:?}"
            );
        }
    }
    let [flee, fight] = GuardianPolicy::OPTIONS.map(GuardianPolicy::label);
    assert_ne!(flee, fight);
    for label in [flee, fight] {
        assert!(
            !label.to_ascii_lowercase().contains("available"),
            "no choice reads as unavailable: {label}"
        );
    }
}

/// The explanation the frontends show for an event says it is fought exactly
/// when the runtime admits an actor of that event, and otherwise says Fight
/// does not apply. A new fightable event cannot ship with the old words.
#[test]
fn the_explanation_agrees_with_what_the_runtime_admits() {
    for (key, _) in GUARDIAN_EVENTS {
        let admitted =
            (0..=500).any(|npc_type| api::random::fight_admission(key, Some(npc_type)).is_some());
        assert_eq!(
            guardian_fight_applies(key),
            admitted,
            "{key}: the frontend rule is the runtime's"
        );
        let help = guardian_event_help(key);
        assert!(!help.is_empty(), "{key} has an explanation");
        assert_eq!(
            help.starts_with("Fight the attacking"),
            admitted,
            "{key} is fought only if the runtime admits it: {help}"
        );
        assert_eq!(
            help.starts_with("Fight does not apply."),
            !admitted,
            "{key} says Fight does not apply only if the runtime refuses it: {help}"
        );
    }
    assert_eq!(
        guardian_event_help("drunken dwarf"),
        "",
        "no policy row, no explanation"
    );
    assert!(!guardian_fight_applies("drunken dwarf"));
}

#[test]
fn a_legacy_ignore_value_reaches_the_draft_as_flee() {
    let settings: ProfileSettings = serde_json::from_str(
        r#"{"lowmem":false,"random_event_policies":{"river troll":"ignore","swarm":"fight"}}"#,
    )
    .unwrap();

    let draft = SettingsDraft::from_settings(settings);
    assert_eq!(
        draft.random_event_policy("river troll"),
        GuardianPolicy::Flee,
        "the vault converted ignore before the draft was built"
    );
    assert_eq!(
        draft.random_event_policy("swarm"),
        GuardianPolicy::Fight,
        "a saved Fight stays Fight"
    );
    assert_eq!(
        draft.random_event_policy("tree spirit"),
        GuardianPolicy::Flee,
        "a missing entry keeps the default"
    );
}
