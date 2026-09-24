//! v1 `meleeWeapons.js` / `PartnerTrade.js` name maps reach their Rust
//! helpers from a catalog-shaped card. Expected values are read off the frozen
//! `meleeWeapons.ts` / `PartnerTrade.ts`.

use script::{LoadIsolate, LoadShape};

#[test]
fn melee_weapon_and_partner_trade_exports_answer_from_rust() {
    let src = r#"
import { bestMeleeWeapon, knownMeleeWeapon } from '../../api/combat/meleeWeapons.js';
import {
    DEFAULT_TRADE_RANGE, MULE_MODE_OPTIONS, namesMatch, parsePartnerList, isConfiguredPartner,
    countOfferByName, countOfferMatching, decideReceiverOfferScreen, decideGiverOfferScreen,
    parseMuleMode, muleGathererHandoffActive,
} from '../../api/trade/PartnerTrade.js';

let matching;
try { countOfferMatching([], () => true); } catch (e) { matching = String(e.message); }
globalThis.__probe = {
    stab: bestMeleeWeapon(['Rune scimitar', 'Rune longsword'], { attack: 40, preferStab: true }),
    refused: bestMeleeWeapon(['Rune scimitar', 'Rune sword'], { attack: 40, preferStab: false, unusable: new Set(['Rune scimitar']) }),
    gated: bestMeleeWeapon(['Rune scimitar'], { attack: 1, preferStab: false }),
    known: knownMeleeWeapon(['Lobster', 'rune sword', 'Bronze scimitar']),
    range: DEFAULT_TRADE_RANGE,
    modes: MULE_MODE_OPTIONS,
    match: namesMatch(' Jive Maker', 'jive maker'),
    list: parsePartnerList(' Alice , ,bob'),
    partner: isConfiguredPartner('ALICE', ['Alice']),
    count: countOfferByName([{ name: 'Flax', count: 0 }, { name: 'flax', count: 4 }, { name: null, count: 2 }], 'FLAX'),
    decline: decideReceiverOfferScreen({ partnerHeader: 'Mallory', partners: ['Alice'], myOfferSlots: 0, theirProductCount: 1 }),
    accept: decideReceiverOfferScreen({ partnerHeader: 'alice', partners: ['Alice'], myOfferSlots: 0, theirProductCount: 1 }),
    giver: decideGiverOfferScreen(0),
    mode: parseMuleMode(' Mule '),
    handoff: muleGathererHandoffActive('gatherer', ['Alice'], false),
    matching,
};
export default class T extends LoopingBot { async loop() {} }
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R289).unwrap(),
    )
    .expect("card loads");
    let v = iso.probe("globalThis.__probe").expect("probe");
    assert_eq!(v["stab"], "Rune longsword");
    assert_eq!(v["refused"], "Rune sword");
    assert!(v["gated"].is_null());
    assert_eq!(v["known"], "Bronze scimitar");
    assert_eq!(v["range"], 2);
    assert_eq!(
        v["modes"],
        serde_json::json!(["Off", "Gatherer", "Mule", "Cooker", "Supplier"])
    );
    assert_eq!(v["match"], true);
    assert_eq!(v["list"], serde_json::json!(["Alice", "bob"]));
    assert_eq!(v["partner"], true);
    assert_eq!(v["count"], 5);
    assert_eq!(
        v["decline"],
        serde_json::json!({ "action": "decline", "reason": "not a configured partner (Mallory)" })
    );
    assert_eq!(v["accept"], serde_json::json!({ "action": "accept" }));
    assert_eq!(v["giver"], "offer");
    assert_eq!(v["mode"], "mule");
    assert_eq!(v["handoff"], true);
    assert_eq!(
        v["matching"],
        "not impl: PartnerTrade.countOfferMatching: caller match predicate per offer slot"
    );
    iso.join();
}
