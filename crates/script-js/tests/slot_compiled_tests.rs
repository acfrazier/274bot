use script::native::request::NativeKind;
use script::native::{PreparedConfig, RetainedMemory, SettingsBag, StartError};
use script::CompiledId;

#[test]
fn sherlock_factory_rejects_wrong_prepared_type() {
    script_js::install_cards().unwrap();
    let actual: Vec<_> = script::compiled_cards()
        .iter()
        .enumerate()
        .map(|(index, card)| (index, card.id.0, card.native_kind))
        .collect();
    assert_eq!(
        actual,
        [
            (0, "Gatherer", NativeKind::Gather),
            (1, "Grinch", NativeKind::Gather),
            (2, "Quester", NativeKind::Quester),
            (3, "Sherlock", NativeKind::Clue),
        ]
    );

    let sherlock = script_js::CARDS
        .iter()
        .find(|card| card.id == CompiledId("Sherlock"))
        .expect("the real Sherlock card is registered");
    let prepared = PreparedConfig::for_test_card(
        sherlock.id,
        sherlock.schema_version,
        1,
        std::sync::Arc::new(SettingsBag::new()),
        (),
    );
    let mut retained = RetainedMemory::default();
    let result = (sherlock.create)(
        api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 0,
        },
        prepared,
        &mut retained,
    );
    assert!(matches!(
        result,
        Err(StartError::Config(error)) if error.code.as_ref() == "config-identity"
    ));
}
