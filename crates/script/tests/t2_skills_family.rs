use script::load::{LoadIsolate, LoadShape};

#[path = "common/mod.rs"]
mod common;

fn skills_isolate() -> LoadIsolate {
    let source = r#"
import { Skills as DirectSkills } from '../../api/skills/Skills.js';
import { Skills as RootSkills } from '@rs2b0t/api';
export default class SkillsFixture extends LoopingBot {
    loop() {
        globalThis.__skillsDirect = DirectSkills;
        globalThis.__skillsRoot = RootSkills;
    }
}
"#;
    let isolate = LoadIsolate::spawn(source.to_string(), LoadShape::CompatClass, vec![])
        .expect("LoadIsolate serves the Skills module and root barrel");
    let snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    assert_eq!(
        isolate.probe("globalThis.__skillsDirect === globalThis.__skillsRoot").unwrap(),
        true,
        "the root barrel and served Skills module share one exported object"
    );
    isolate
}

#[test]
fn posted_stats_cover_every_skill_and_all_five_methods() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const names = [
                    'attack', 'defence', 'strength', 'hitpoints', 'ranged', 'prayer', 'magic',
                    'cooking', 'woodcutting', 'fletching', 'fishing', 'firemaking', 'crafting',
                    'smithing', 'mining', 'herblore', 'agility', 'thieving', 'runecraft',
                ];
                const errorMessage = (read) => {
                    try { read(); return null; }
                    catch (error) { return error.message; }
                };
                const symbol = Symbol('no-such-skill');
                const ownSymbol = Symbol('owned');
                skills[ownSymbol] = 23;
                return {
                    indexes: names.map((name) => skills.index(name.toUpperCase())),
                    unknownIndex: skills.index('no-such-skill'),
                    symbolIndex: skills.index(symbol),
                    xp: names.map((name) => skills.xp(name)),
                    levels: names.map((name) => skills.level(name)),
                    effective: names.map((name) => skills.effective(name)),
                    hpFraction: skills.hpFraction(),
                    missing: {
                        xp: errorMessage(() => skills.xp('no-such-skill')),
                        level: errorMessage(() => skills.level('no-such-skill')),
                        effective: errorMessage(() => skills.effective('no-such-skill')),
                    },
                    inheritedString: skills.toString === Object.prototype.toString,
                    inheritedConstructor: skills.constructor === Object,
                    symbolValue: skills[ownSymbol],
                    absentSymbol: skills[Symbol('absent')] === undefined,
                    unknownMember: errorMessage(() => skills.noSuchSkillMember),
                };
            })()"#,
        )
        .unwrap();

    assert_eq!(
        result,
        serde_json::json!({
            "indexes": (0..19).collect::<Vec<_>>(),
            "unknownIndex": -1,
            "symbolIndex": -1,
            "xp": [0, 0, 0, 1154, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            "levels": [1, 1, 1, 10, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            "effective": [1, 1, 1, 10, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            "hpFraction": 1,
            "missing": {
                "xp": "not impl: Skills.xp",
                "level": "not impl: Skills.level",
                "effective": "not impl: Skills.effective"
            },
            "inheritedString": true,
            "inheritedConstructor": true,
            "symbolValue": 23,
            "absentSymbol": true,
            "unknownMember": "not impl: Skills.noSuchSkillMember"
        }),
        "all Skills methods read the actual posted frame and retain proxy behavior"
    );
    isolate.join();
}

#[test]
fn index_coerces_before_reading_stats_and_returns_first_row_position() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const snapshot = globalThis.__rs2b0t_host.snapshot;
                const events = [];
                const first = {
                    index: 81,
                    get name() { events.push('first-name'); return 'MiXeD'; },
                };
                const duplicate = { index: 82, name: 'mixed' };
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    get() { events.push('stats'); return [null, first, duplicate]; },
                });
                const query = { toString() { events.push('coerce'); return 'mIxEd'; } };
                const position = skills.index(query);
                return { position, events };
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "position": 1,
            "events": ["coerce", "stats", "first-name", "first-name"]
        }),
        "String(name) runs before the stats getter; name is read twice; first matching row position wins"
    );
    isolate.join();
}

#[test]
fn stat_lookup_recoerces_per_eligible_row_then_reads_a_fresh_snapshot_and_field() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const snapshot = globalThis.__rs2b0t_host.snapshot;
                const events = [];
                let conversions = 0;
                let statsReads = 0;
                const query = {
                    [Symbol.toPrimitive](hint) {
                        events.push('coerce:' + hint);
                        conversions += 1;
                        return conversions === 1 ? 'not-yet' : 'WANTED';
                    },
                };
                const ineligible = {
                    get name() { events.push('ineligible-name'); return 7; },
                };
                const first = {
                    get name() { events.push('first-name'); return 'other'; },
                };
                const second = {
                    get name() { events.push('second-name'); return 'wanted'; },
                };
                const resultRow = {
                    get xp() {
                        events.push('xp:' + (events.filter((item) => item.startsWith('xp:')).length + 1));
                        return events.filter((item) => item.startsWith('xp:')).length === 1
                            ? 41
                            : 'second-read';
                    },
                };
                const fresh = new Array(3);
                fresh[2] = resultRow;
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    get() {
                        statsReads += 1;
                        events.push('stats:' + statsReads);
                        return statsReads === 1 ? [ineligible, first, second] : fresh;
                    },
                });
                const value = skills.xp(query);
                return { value, statsReads, conversions, events };
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "value": "second-read",
            "statsReads": 2,
            "conversions": 2,
            "events": [
                "stats:1",
                "ineligible-name",
                "first-name", "first-name", "coerce:string",
                "second-name", "second-name", "coerce:string",
                "stats:2", "xp:1", "xp:2"
            ]
        }),
        "lookup must preserve callback order, per-eligible-row String conversion, fresh stats(), and repeated field reads"
    );
    isolate.join();
}

#[test]
fn sparse_duplicate_missing_and_non_numeric_rows_keep_exact_results_and_errors() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const snapshot = globalThis.__rs2b0t_host.snapshot;
                const errorMessage = (read) => {
                    try { read(); return null; }
                    catch (error) { return error.message; }
                };
                const sparse = new Array(4);
                sparse[2] = { index: 101, name: 'Alpha', xp: 7, base: 8, effective: 9 };
                sparse[3] = { index: 102, name: 'alpha', xp: 99, base: 99, effective: 99 };
                snapshot.stats = sparse;
                const first = {
                    index: skills.index('ALPHA'),
                    xp: skills.xp('alpha'),
                    level: skills.level('alpha'),
                    effective: skills.effective('alpha'),
                };
                snapshot.stats = [{ name: 'bad', xp: '7', base: null, effective: false }];
                const bad = {
                    xp: errorMessage(() => skills.xp('bad')),
                    level: errorMessage(() => skills.level('bad')),
                    effective: errorMessage(() => skills.effective('bad')),
                };
                let reads = 0;
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    get() { reads += 1; return reads === 1 ? [{ name: 'vanishes' }] : []; },
                });
                const freshMissing = errorMessage(() => skills.level('vanishes'));
                const readsForMissing = reads;
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    writable: true,
                    value: 0,
                });
                const falsyStats = {
                    index: skills.index('absent'),
                    xp: errorMessage(() => skills.xp('absent')),
                };
                return { first, bad, freshMissing, readsForMissing, falsyStats };
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "first": {"index": 2, "xp": 7, "level": 8, "effective": 9},
            "bad": {
                "xp": "not impl: Skills.xp",
                "level": "not impl: Skills.level",
                "effective": "not impl: Skills.effective"
            },
            "freshMissing": "not impl: Skills.level",
            "readsForMissing": 2,
            "falsyStats": {"index": -1, "xp": "not impl: Skills.xp"}
        }),
        "holes are visited as missing rows, duplicate names select the first match, and stat reads validate numeric types"
    );
    isolate.join();
}

#[test]
fn getters_and_name_coercion_rethrow_the_original_value() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const snapshot = globalThis.__rs2b0t_host.snapshot;
                const nameError = { tag: 'name getter' };
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    get() { return [{ get name() { throw nameError; } }]; },
                });
                let nameGetterIdentity = false;
                try { skills.index('x'); } catch (error) { nameGetterIdentity = error === nameError; }

                const coercionError = { tag: 'name coercion' };
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    writable: true,
                    value: [{ name: 'x', xp: 1 }],
                });
                const query = { [Symbol.toPrimitive]() { throw coercionError; } };
                let coercionIdentity = false;
                try { skills.xp(query); } catch (error) { coercionIdentity = error === coercionError; }

                const fieldError = { tag: 'field getter' };
                Object.defineProperty(snapshot, 'stats', {
                    configurable: true,
                    writable: true,
                    value: [{ name: 'x', get xp() { throw fieldError; } }],
                });
                let fieldIdentity = false;
                try { skills.xp('x'); } catch (error) { fieldIdentity = error === fieldError; }
                return { nameGetterIdentity, coercionIdentity, fieldIdentity };
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "nameGetterIdentity": true,
            "coercionIdentity": true,
            "fieldIdentity": true
        }),
        "native lookup propagates the exact thrown object from getters and coercions"
    );
    isolate.join();
}

#[test]
fn hp_fraction_uses_overridden_methods_in_order_and_preserves_number_semantics() {
    let isolate = skills_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const skills = globalThis.__skillsDirect;
                const originalLevel = skills.level;
                const originalEffective = skills.effective;
                const calls = [];
                const errorMessage = (read) => {
                    try { read(); return null; }
                    catch (error) { return error.message; }
                };
                try {
                    skills.level = function(name) {
                        calls.push(['level', name, this === skills]);
                        return 0;
                    };
                    skills.effective = function(name) {
                        calls.push(['effective', name, this === skills]);
                        return 12;
                    };
                    const zero = errorMessage(() => skills.hpFraction());
                    const zeroCalls = calls.splice(0);

                    skills.level = function(name) {
                        calls.push(['level', name, this === skills]);
                        return -1;
                    };
                    const negative = errorMessage(() => skills.hpFraction());
                    const negativeCalls = calls.splice(0);

                    skills.level = function(name) {
                        calls.push(['level', name, this === skills]);
                        return 8;
                    };
                    skills.effective = function(name) {
                        calls.push(['effective', name, this === skills]);
                        return 12;
                    };
                    const boosted = skills.hpFraction();
                    const boostedCalls = calls.splice(0);

                    skills.level = () => NaN;
                    skills.effective = () => 3;
                    const nanBase = Number.isNaN(skills.hpFraction());
                    skills.level = () => Infinity;
                    skills.effective = () => Infinity;
                    const infiniteRatioIsNan = Number.isNaN(skills.hpFraction());
                    skills.level = () => 1;
                    skills.effective = () => -0;
                    const negativeZero = Object.is(skills.hpFraction(), -0);
                    skills.level = () => -0;
                    const negativeZeroBase = errorMessage(() => skills.hpFraction());

                    const thrown = { tag: 'level override' };
                    skills.level = function() { calls.push(['throwing-level']); throw thrown; };
                    skills.effective = function() { calls.push(['must-not-run']); return 1; };
                    let thrownIdentity = false;
                    try { skills.hpFraction(); } catch (error) { thrownIdentity = error === thrown; }
                    const thrownCalls = calls.splice(0);
                    return {
                        zero, zeroCalls, negative, negativeCalls, boosted, boostedCalls,
                        nanBase, infiniteRatioIsNan, negativeZero, negativeZeroBase,
                        thrownIdentity, thrownCalls,
                    };
                } finally {
                    skills.level = originalLevel;
                    skills.effective = originalEffective;
                }
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "zero": "not impl: Skills.hpFraction",
            "zeroCalls": [["level", "hitpoints", true]],
            "negative": "not impl: Skills.hpFraction",
            "negativeCalls": [["level", "hitpoints", true]],
            "boosted": 1.5,
            "boostedCalls": [["level", "hitpoints", true], ["effective", "hitpoints", true]],
            "nanBase": true,
            "infiniteRatioIsNan": true,
            "negativeZero": true,
            "negativeZeroBase": "not impl: Skills.hpFraction",
            "thrownIdentity": true,
            "thrownCalls": [["throwing-level"]]
        }),
        "HP must call level before effective, honor public overrides, preserve errors, and use JavaScript comparison/division"
    );
    isolate.join();
}

#[test]
fn snapshot_replacement_is_observed_after_a_new_posted_frame() {
    let source = r#"
import { Skills } from '../../api/skills/Skills.js';
export default class SkillsFixture extends LoopingBot {
    loop() {
        globalThis.__frameSkills = {
            index: Skills.index('hitpoints'),
            xp: Skills.xp('hitpoints'),
            level: Skills.level('hitpoints'),
            effective: Skills.effective('hitpoints'),
        };
    }
}
"#;
    let isolate = LoadIsolate::spawn(source.to_string(), LoadShape::CompatClass, vec![])
        .expect("LoadIsolate serves Skills for successive posted frames");
    let first = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &first);
    isolate.on_game_tick(1);
    assert_eq!(
        isolate.probe("globalThis.__frameSkills").unwrap(),
        serde_json::json!({"index": 3, "xp": 1154, "level": 10, "effective": 10})
    );

    let replacement_stats = [script::isolate_fb::StatInput {
        index: 77,
        name: "hitpoints",
        xp: 9876,
        base: 20,
        effective: 15,
    }];
    let mut replacement = common::ingame_snapshot();
    replacement.tick = 2;
    replacement.stats = &replacement_stats;
    common::post_snapshot_input(&isolate, &replacement);
    isolate.on_game_tick(2);
    assert_eq!(
        isolate.probe("globalThis.__frameSkills").unwrap(),
        serde_json::json!({"index": 0, "xp": 9876, "level": 20, "effective": 15})
    );
    isolate.join();
}
