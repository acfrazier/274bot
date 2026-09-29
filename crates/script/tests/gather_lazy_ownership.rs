//! Own test binary with one test: the prepared gathering catalog is process-wide, so who holds it cannot share a
//! process with other Load isolates of the same revision. Startup must not make an isolate a family consumer;
//! only a script that really touches gathering facts acquires the family, and a `GAS_ROCK_IDS` consumer lets it
//! go as soon as its own set is filled.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

const IDLE: &str = "export const apiVersion = 2;\nexport function tick(api) {}\n";

const GAS_CONSUMER: &str = r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = GAS_ROCK_IDS.has(2125);
    }
}
"#;

fn start(
    src: &str,
    shape: LoadShape,
    data: &std::sync::Arc<api::game_data::SelectedGameData>,
) -> LoadIsolate {
    let iso = LoadIsolate::spawn_with_game_data(src.into(), shape, vec![], data.clone()).unwrap();
    // The startup barrier: module evaluation has finished once the isolate answers.
    assert_eq!(iso.probe("1").unwrap(), serde_json::json!(1));
    iso
}

#[test]
fn idle_isolates_never_acquire_the_family_and_consumers_release_it() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    assert!(data.try_gathering().is_none());

    let idle: Vec<LoadIsolate> = (0..4)
        .map(|_| start(IDLE, LoadShape::NativeTick, &data))
        .collect();
    assert!(
        data.try_gathering().is_none(),
        "starting scripts that never touch gathering facts decodes no family"
    );

    let consumers: Vec<LoadIsolate> = (0..3)
        .map(|_| start(GAS_CONSUMER, LoadShape::CompatClass, &data))
        .collect();
    assert!(
        data.try_gathering().is_none(),
        "importing the mining facts is not consuming them"
    );
    for (tick, iso) in (1u64..).zip(&consumers) {
        iso.on_game_tick(tick);
        assert_eq!(
            iso.probe("globalThis.__probe").unwrap(),
            serde_json::json!(true)
        );
    }
    // Each consumer acquired the family for its first touch, copied the ids into its own set and let go: no
    // consumer, live or joined, is left holding the catalog while the idle isolates are still running.
    assert!(
        data.try_gathering().is_none(),
        "a materialized gas set holds no reference to the family"
    );
    for (tick, iso) in (10u64..).zip(&consumers) {
        iso.on_game_tick(tick);
        assert_eq!(
            iso.probe("globalThis.__probe").unwrap(),
            serde_json::json!(true)
        );
    }
    assert!(
        data.try_gathering().is_none(),
        "later checks are served by the isolate's own set and never reacquire the family"
    );
    for iso in consumers {
        iso.join();
    }
    for iso in idle {
        iso.join();
    }
}
