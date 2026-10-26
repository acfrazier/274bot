use api::selected::RunKey;
use script::native::request::NativeKind;
use script::native::{PreparedConfig, RetainedMemory, StartError};
use script::CompiledId;
use std::sync::Arc;

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

    let wrong_type = PreparedConfig::new(CompiledId("Sherlock"), 1, 1, Arc::default(), ());
    assert!(matches!(
        (script_js::sherlock::CARD.create)(
            RunKey {
                slot: 10,
                run: 2,
                session: 0
            },
            wrong_type,
            &mut RetainedMemory::default()
        ),
        Err(StartError::Config(_))
    ));
}
