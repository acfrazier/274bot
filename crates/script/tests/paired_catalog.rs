use std::path::PathBuf;

use script::load::{JsLibrary, LoadIsolate};
use std::thread;
use std::time::{Duration, Instant};

fn wait_ready(isolate: &LoadIsolate) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match isolate.poll_ready() {
            script::Ready::Ready => return,
            script::Ready::Failed(error) => panic!("isolate setup failed: {error}"),
            script::Ready::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2));
            }
            script::Ready::Pending => panic!("isolate setup timed out"),
        }
    }
}

use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

mod common;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-paired-catalog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "requires the frozen RS2B0T catalog"]
fn paired_catalog_cards_transpile_and_start_with_selected_289_data() {
    let root = script::rs2b0t_root().expect("RS2B0T catalog root configured");
    let dir = scratch();
    let mut library = JsLibrary::with_cache(dir.join("scripts.json"), dir.join("cache"));
    library
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("register frozen catalog");
    let cache = JsCache::new(dir.join("sibling-cache"));
    let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    for name in ["ClueSolver", "Duel Arena Combat Trainer", "JiveKQ"] {
        assert!(
            !script::is_catalog_dim(name),
            "{name} must be Start-enabled"
        );
        library
            .ensure_js(ScriptSource::Catalog, name)
            .unwrap_or_else(|error| panic!("{name} transpile failed: {error}"));
        let card = library
            .get(ScriptSource::Catalog, name)
            .unwrap_or_else(|| panic!("{name} missing from catalog"))
            .clone();
        assert_eq!(card.unloadable, None, "{name} must not be dim");
        let siblings = script::resolve_sibling_modules(
            &card.path,
            &card.origin,
            &cache,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        )
        .unwrap_or_else(|error| panic!("{name} sibling resolution failed: {error}"));
        let isolate = LoadIsolate::spawn_with_game_data(
            card.js.clone(),
            card.shape,
            siblings,
            selected.clone(),
        )
        .unwrap_or_else(|error| panic!("{name} isolate spawn failed: {error}"));
        wait_ready(&isolate);

        let mut overrides = serde_json::Map::new();
        if name == "Duel Arena Combat Trainer" {
            overrides.insert("mode".into(), "Clue helper".into());
            overrides.insert("partner".into(), "b".into());
        } else if name == "JiveKQ" {
            overrides.insert("team".into(), "a,b,c,d".into());
        }
        let bag = script::merge_bag(&card.settings_schema, &overrides, None);
        isolate.post_settings_bag(&bag);
        let mut snapshot = common::ingame_snapshot();
        snapshot.my_name = Some("a");
        snapshot.stats = &common::FRESH_STATS;
        isolate.post_snapshot(script::isolate_fb::encode_snapshot(&snapshot));
        isolate.on_game_tick(1);
        let _ = isolate.probe("true");
        let bad: Vec<_> = isolate
            .drain_logs()
            .into_iter()
            .filter(|line| {
                line.contains("not impl")
                    || line.contains("TypeError")
                    || line.contains("ReferenceError")
                    || line.contains("SyntaxError")
            })
            .collect();
        assert!(bad.is_empty(), "{name} start diagnostics: {bad:?}");
        isolate.join();
    }
}

#[test]
#[ignore = "requires the frozen RS2B0T catalog"]
fn frozen_jive_party_releases_only_a_complete_fresh_roster() {
    let root = script::rs2b0t_root().expect("RS2B0T catalog root configured");
    let dir = scratch();
    let mut library =
        JsLibrary::with_cache(dir.join("party-scripts.json"), dir.join("party-cache"));
    library
        .register_rs2b0t(&root, &dir.join("party-rs2b0t-path"))
        .expect("register frozen catalog");
    library
        .ensure_js(ScriptSource::Catalog, "JiveKQ")
        .expect("transpile JiveKQ");
    let card = library
        .get(ScriptSource::Catalog, "JiveKQ")
        .expect("JiveKQ card");
    let source = r#"
import { Party } from './party.js';
import { BANK } from './policy.js';
let done = false;
export function tick() {
    if (done) return;
    done = true;
    const roster = ['a', 'b', 'c', 'd'];
    const now = 10_000;
    const leader = new Party(roster, 'a', 'sa');
    const follower = new Party(roster, 'b', 'sb');
    const sessions = ['sa', 'sb', 'sc', 'sd'];
    for (let i = 0; i < roster.length; i++) {
        const member = {
            name: roster[i], session: sessions[i], trip: 0, stage: 'bank',
            tile: { ...BANK }, ready: true,
        };
        leader.receive(member, now);
        follower.receive(member, now);
    }
    const release = leader.release('bank', 1, now);
    follower.accept(release, now);
    const departure = follower.departure(0, now);
    follower.receive({
        name: 'c', session: 'sc', trip: 1, stage: 'retreat',
        tile: { ...BANK }, ready: false, reason: 'paused',
    }, now + 1);
    globalThis.__probe = {
        release,
        departure,
        unsafeAfterPause: follower.unsafe(1, now + 1),
    };
}
"#;
    let cache = JsCache::new(dir.join("party-sibling-cache"));
    let siblings = script::resolve_sibling_modules(
        &card.path,
        source,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
            shape: Some("NativeTick".into()),
            api_family: None,
        },
    )
    .expect("resolve frozen Party graph");
    let runtime_source = source
        .replace("'./party.js'", "'/rs2b0t/bot/scripts/bot/party.js'")
        .replace("'./policy.js'", "'/rs2b0t/bot/scripts/bot/policy.js'");
    let source = script::load::transpile_ts(&runtime_source).expect("transpile party probe");
    let isolate = LoadIsolate::spawn_with_game_data(
        source,
        script::LoadShape::NativeTick,
        siblings,
        api::game_data::for_revision(client::io::ClientRevision::R289).unwrap(),
    )
    .expect("spawn Party probe");
    wait_ready(&isolate);
    isolate.on_game_tick(1);
    let value = isolate.probe("globalThis.__probe").unwrap();
    assert_eq!(value["release"]["stage"], "bank");
    assert_eq!(value["release"]["trip"], 1);
    assert_eq!(
        value["release"]["sessions"],
        serde_json::json!(["sa", "sb", "sc", "sd"])
    );
    assert_eq!(value["departure"], 1);
    assert_eq!(value["unsafeAfterPause"], true);
    isolate.join();
}
