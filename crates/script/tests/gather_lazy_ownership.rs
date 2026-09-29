//! Own test binary with one test: the prepared gathering catalog is process-wide, so who holds it cannot share a
//! process with other Load isolates of the same revision. Startup must not make an isolate a family consumer, and
//! `GAS_ROCK_IDS` is built from the selected core, so neither importing nor using it ever acquires the family.
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
        "importing the mining facts decodes no family"
    );
    for tick in 1u64..=2 {
        for iso in &consumers {
            iso.on_game_tick(tick);
            assert_eq!(
                iso.probe("globalThis.__probe").unwrap(),
                serde_json::json!(true)
            );
            assert!(
                data.try_gathering().is_none(),
                "gas-rock checks are answered by the isolate's own set, never by the family"
            );
        }
    }
    for iso in consumers {
        iso.join();
    }
    for iso in idle {
        iso.join();
    }
}
