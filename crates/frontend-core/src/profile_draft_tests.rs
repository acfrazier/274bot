//! The guardian vocabulary both frontends read: exactly two choices, one
//! unavailable notice, and a 0.2.9 `ignore` value reaching a draft as Flee.

use vault::ProfileSettings;

use super::{GuardianPolicy, SettingsDraft, GUARDIAN_FIGHT_UNAVAILABLE};

#[test]
fn guardian_policy_options_are_exactly_flee_and_fight() {
    assert_eq!(
        GuardianPolicy::OPTIONS,
        [GuardianPolicy::Flee, GuardianPolicy::Fight],
        "Ignore is not a choice"
    );
    assert_eq!(
        GuardianPolicy::OPTIONS.map(GuardianPolicy::label),
        ["Flee", "Fight (not available yet)"]
    );
    assert!(GuardianPolicy::Flee.available());
    assert!(
        !GuardianPolicy::Fight.available(),
        "Fight is refused until its own slice"
    );
    assert!(
        GUARDIAN_FIGHT_UNAVAILABLE.contains("saved Fight"),
        "the shared notice explains a saved Fight: {GUARDIAN_FIGHT_UNAVAILABLE}"
    );
    assert!(
        !GUARDIAN_FIGHT_UNAVAILABLE.contains("Ignore"),
        "the notice no longer names a removed choice: {GUARDIAN_FIGHT_UNAVAILABLE}"
    );
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
