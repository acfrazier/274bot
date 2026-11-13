use super::*;
use rustyscript::RuntimeOptions;

const SHAPE_TEST_NAME: &str =
    "materialize::quest_records_v8::tests::every_record_object_has_fast_properties_from_its_template";
const CHILD_ENV: &str = "SCRIPT_QUEST_RECORDS_SHAPE_TEST_CHILD";

fn setup() -> Runtime {
    crate::runtime::platform::bootstrap_test_v8_platform();
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected game data");
    crate::scene::supply_v2::configure(Some(data));
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    // Twice: the second pass reclaims what the first left to concurrent marking.
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

/// `%HasFastProperties` is a process-wide V8 flag, so the check runs in its own child process.
fn in_child(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output =
            std::process::Command::new(std::env::current_exe().expect("unit-test executable path"))
                .args(["--exact", name, "--nocapture"])
                .env(CHILD_ENV, "1")
                .output()
                .expect("run isolated V8-native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.contains(&format!("test {name} ... ok")) && stdout.contains("1 passed"),
            "isolated test did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    body();
}

#[test]
fn every_record_object_has_fast_properties_from_its_template() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    in_child(SHAPE_TEST_NAME, || {
        let mut runtime = setup();
        // No JavaScript warm-up runs before the first object is built: the check is the first use.
        let ok: bool = runtime
            .eval(
                r#"(() => {
                    const hasFast = new Function('value', 'return %HasFastProperties(value)');
                    const rows = __rs2b0t_quest_records();
                    const need = (condition, message) => { if (!condition) throw new Error(message); };
                    const known = new Set(['id,name,questPoints', 'id,name,questPoints,requirements']);
                    const requirementKeys = ['minQuestPoints', 'quests', 'skills'];
                    need(Array.isArray(rows) && hasFast(rows), 'rows are not a fast array');
                    for (const row of rows) {
                        need(hasFast(row) && Object.getPrototypeOf(row) === Object.prototype, `${row.id}: row`);
                        need(known.has(Reflect.ownKeys(row).join()), `${row.id}: keys ${Reflect.ownKeys(row)}`);
                        const requirements = row.requirements;
                        if (requirements === undefined) continue;
                        need(hasFast(requirements), `${row.id}: requirements`);
                        const keys = Reflect.ownKeys(requirements);
                        need(keys.every(key => requirementKeys.includes(key))
                            && keys.join() === requirementKeys.filter(key => keys.includes(key)).join(),
                            `${row.id}: requirement keys ${keys}`);
                        for (const gate of requirements.skills ?? []) {
                            need(hasFast(gate) && Reflect.ownKeys(gate).join() === 'skill,level', `${row.id}: skill gate`);
                        }
                        need(hasFast(requirements.quests ?? []) && hasFast(requirements.skills ?? []), `${row.id}: arrays`);
                        for (const key of keys) {
                            const descriptor = Object.getOwnPropertyDescriptor(requirements, key);
                            need(descriptor.writable && descriptor.enumerable && descriptor.configurable, `${row.id}.${key}`);
                        }
                    }
                    return true;
                })()"#,
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(ok);
    });
}

#[test]
fn records_carry_the_content_the_catalog_reads() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    let mut runtime = setup();
    let dragon: serde_json::Value = runtime
        .eval("__rs2b0t_quest_records().find(row => row.id === 'dragon')")
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        dragon,
        serde_json::json!({
            "id": "dragon",
            "name": "Dragon Slayer",
            "questPoints": 2,
            "requirements": { "minQuestPoints": 32 },
        })
    );
    let squire: serde_json::Value = runtime
        .eval("__rs2b0t_quest_records().find(row => row.id === 'squire').requirements")
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        squire,
        serde_json::json!({ "skills": [{ "skill": "cooking", "level": 10 }] })
    );
    let count: usize = runtime.eval("__rs2b0t_quest_records().length").unwrap();
    let selected = crate::scene::supply_v2::selected_data().expect("configured");
    assert_eq!(count, compat_quests(&selected).expect("identity").len());
}

#[test]
fn quest_records_bound_the_collected_v8_heap() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    let mut runtime = setup();
    runtime.eval::<()>("globalThis.__held = null;").unwrap();
    let baseline = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("globalThis.__held = __rs2b0t_quest_records();")
        .unwrap();
    let populated = collected_used_heap(&mut runtime);
    // The first 200 builds warm the engine (tier-up allocates code and feedback, which is not retained rows).
    let build_and_drop = "for (let index = 0; index < 200; index++) { __rs2b0t_quest_records(); } globalThis.__held = null;";
    runtime.eval::<()>(build_and_drop).unwrap();
    let warmed = collected_used_heap(&mut runtime);
    runtime.eval::<()>(build_and_drop).unwrap();
    let released = collected_used_heap(&mut runtime);

    let selected = crate::scene::supply_v2::selected_data().expect("configured");
    let quests = compat_quests(&selected).expect("identity");
    // Per record: the row, its requirements, and room for a skill-gate array with its gates and a quest-name array.
    let objects = quests.len() * 6;
    // `cold_native_entity_rows_bound_collected_v8_heap_growth` charges 1 KiB to each retained object.
    let limit = objects * 1024;
    let growth = populated.saturating_sub(baseline);
    println!(
        "Quest records collected V8 heap: records={} objects<={objects} baseline={baseline} populated={populated} warmed={warmed} released={released} growth={growth} limit={limit}",
        quests.len()
    );
    assert!(
        growth <= limit,
        "the records grew collected V8 used heap by {growth} bytes; limit {limit}"
    );
    assert!(
        released.saturating_sub(warmed) <= 64 * 1024,
        "200 more discarded builds grew the collected heap from {warmed} to {released}: a build is retained"
    );
}

#[test]
fn without_selected_content_there_are_no_records() {
    crate::runtime::platform::bootstrap_test_v8_platform();
    crate::scene::supply_v2::configure(None);
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime).unwrap();
    let count: usize = runtime.eval("__rs2b0t_quest_records().length").unwrap();
    assert_eq!(count, 0);
}
