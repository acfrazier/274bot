//! COMPAT-FINISH-029 entities family: the uncalled 274bot-only `Players.all`
//! extension is removed (census row `compat-only:008`); the supported
//! `Players.query` sibling stays.

use script_js::{LoadIsolate, LoadShape};

const INDEX_D_TS: &str = include_str!("../compat-js/index.d.ts");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_entities__players_all__removed() {
    let src = r#"
import { Players } from '../../api/players/Players.js';
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__done) return;
        globalThis.__done = true;
        const trap = (key) => { try { Players[key]; return 'read'; } catch (e) { return e.message; } };
        globalThis.__facts = {
            own: Object.prototype.hasOwnProperty.call(Players, 'all'),
            keys: Object.keys(Players).includes('all'),
            removed: trap('all'),
            unknown: trap('__noSuchPlayersMember'),
            sibling: typeof Players.query === 'function'
                && typeof Players.query().results === 'function',
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let facts = iso.probe("__facts").unwrap();
    iso.join();
    assert_eq!(facts["own"], false, "Players.all is not an own key");
    assert_eq!(facts["keys"], false, "Players.all is not enumerated");
    assert_eq!(
        facts["removed"]
            .as_str()
            .map(|message| message.replace("Players.all", "Players.<key>")),
        facts["unknown"]
            .as_str()
            .map(|message| message.replace("Players.__noSuchPlayersMember", "Players.<key>")),
        "a removed member uses the generic unknown-member trap"
    );
    assert_eq!(facts["sibling"], true, "Players.query stays");
    let start = INDEX_D_TS
        .find("declare module '*api/players/Players.js' {")
        .expect("index.d.ts declares the Players module");
    let block = &INDEX_D_TS[start..];
    let block = &block[..block
        .find("\n}\n")
        .expect("the Players module block closes")];
    assert!(
        !block.contains(" all("),
        "index.d.ts still declares Players.all"
    );
    assert!(
        !ABI_CONTRACT.contains("Players.all"),
        "abi-contract.json still promises Players.all"
    );
}
