use super::*;
use api::game_data::TrailParam;
use client::io::ClientRevision;
use std::sync::Arc;

const CASKET: i32 = 3531;
const CLUE: i32 = 3554;
/// `trail_clue_easy_map001_casket`: an easy casket, so its pack-full food
/// is the frozen option forms and not the hard `SHARK_ID`.
const EASY_CASKET: i32 = 2714;
/// `trail_clue_hard_sextant028_casket`: the casket of the held 3554 clue.
const SEXTANT_CASKET: i32 = 3555;
/// `trail_clue_easy_simple001`: the selected search membership,
/// `trail_loc=^true` with `trail_coord=1_50_50_9_18`.
const SEARCH: i32 = 2677;
/// `trail_clue_easy_map001`: the coord-only easy map the widened dig
/// membership reads, `trail_coord=0_49_52_41_32` → (3177, 3360, 0) with no
/// `trail_sextant` at all.
const MAP: i32 = 2713;
/// `trail_clue_hard_map001`: a selected clue row with no params at all.
const MAP_EMPTY: i32 = 2722;
/// `trail_clue_medium_riddle001`: the desc-only frozen `keyFrom` riddle the
/// selected `talk_key.keys` family publishes as Black Heather's own step —
/// key `2832`, packed type `202` at the published `(3039, 3700, plane 0)`.
const RIDDLE: i32 = 2831;
/// `trail_clue_medium_sextant001`: the unguarded-dig membership,
/// `trail_coord=0_49_50_24_51` → (3160, 3251, 0).
const UNGUARDED: i32 = 2801;
/// `trail_clue_hard_sextant001`: the guarded sibling of that membership,
/// same `trail_sextant` and a `trail_guardian`.
const GUARDED: i32 = 2723;
/// The item id the frozen `SPADE_NAME` display belongs to. Posted on the
/// test pack pages the way `snapshot.inv` posts it; the machine itself
/// never reads an id for the Dig verb.
const SPADE_ITEM: i32 = 952;
/// `trail_clue_hard_riddle014`: a desc-only hard riddle whose own selected
/// `trail_clue_hard_riddle014_puzzlebox` item is the box its puzzle step
/// joins to.
const PUZZLE_RIDDLE: i32 = 2794;
/// That box item, display `Puzzle box`.
const PUZZLE_BOX: i32 = 2795;
/// `trail_clue_hard_riddle022`: a desc-only hard riddle with **no**
/// `_puzzlebox` sibling at all, which stays idle.
const PUZZLE_RIDDLE_NO_BOX: i32 = 3572;
/// `2831`'s own key: the ground row its keeper drops on the published
/// spawn, and the pack row that ends the hunt.
const KEEPER_KEY: i32 = 2832;
/// The packed type that riddle's keeper names, and the posted display name
/// the page carries beside it. The matcher's script alias (`black_heather`)
/// is never compared to a posted string.
const KEEPER_ID: i32 = 202;
const KEEPER_NAME: &str = "Black Heather";
/// `trail_clue_medium_riddle008`: the second unique-spawn type keeper —
/// Penda, packed id 1087, at the published `(2910, 3539, plane 0)` — and
/// the key `3608` it drops.
const PENDA: i32 = 3607;
const PENDA_KEY: i32 = 3608;
const PENDA_ID: i32 = 1087;
const PENDA_NAME: &str = "Penda";
/// The five `talk_key.keys` rows that publish no unique spawn:
/// `riddle002` and `riddle003` name a packed type the family covered for a
/// non-unique jm2 NPC spawn, `riddle004` and `riddle007` a category and
/// `riddle005` a bare name. None of them hunts anything — idle over any
/// scene — because a packed type is the only identity the posted npc page
/// can be matched by.
const MATCHER_KEEPERS: [i32; 5] = [2833, 2835, 2837, 2839, 3605];

fn selected() -> Arc<SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R274).expect("selected data")
}

/// A held id that is none of the selected families' rows: not a membership
/// row, not one of the six challenge scrolls — holding one of those is the
/// challenge seam's own join onto its parent, not an unrelated page — and
/// not a talk step. The C3 exemplar has to be a page no selected family
/// names, or it would test a path that no longer exists.
fn unrelated(data: &SelectedGameData) -> i32 {
    let trails = data.trails().expect("trails");
    let talk = data.talk_key().expect("talk_key");
    (1..)
        .find(|id| {
            !trails.rows.iter().any(|row| row.id == *id)
                && !trails.challenge_answers.iter().any(|row| row.id == *id)
                && !talk.talk.iter().any(|row| row.id == *id)
        })
        .expect("a held id no selected family names")
}

fn payload(op: &str, token: Option<u64>, held: Value, extra: Value) -> Value {
    let mut call = json!({ "op": op, "generation": 4, "held": held });
    if let Some(token) = token {
        call["token"] = json!(token);
    }
    for (key, value) in extra.as_object().expect("extra") {
        call[key] = value.clone();
    }
    call
}

fn begin(data: &SelectedGameData, held: Value) -> Value {
    dispatch(Some(data), &payload("begin", None, held, json!({})))
}

fn call(data: &SelectedGameData, token: u64, held: Value, extra: Value) -> Value {
    dispatch(Some(data), &payload("next", Some(token), held, extra))
}

fn token_of(step: &Value) -> u64 {
    step["token"].as_u64().expect("token")
}

/// The wrapper's posted `here` tile.
fn here(x: i32, z: i32, level: i32) -> Value {
    json!({ "x": x, "z": z, "level": level })
}

/// One wrapper-marshalled posted loc row.
fn loc(id: i32, x: i32, z: i32, level: i32, actions: &[&str]) -> Value {
    json!({ "id": id, "x": x, "z": z, "level": level, "actions": actions })
}

/// One wrapper-marshalled posted ground row: the same `SceneEntity` shape
/// as a loc, plus the display name the Take rides along with.
fn ground(id: i32, name: &str, x: i32, z: i32, level: i32, actions: &[&str]) -> Value {
    json!({ "id": id, "name": name, "x": x, "z": z, "level": level, "actions": actions })
}

/// One wrapper-marshalled posted pack row.
fn inv(id: i32, name: &str, count: i32) -> Value {
    json!({ "id": id, "name": name, "count": count })
}

/// A full pack page: the posted `inv_size` rows, none of them a listed name
/// and none a selected trail item, so the frozen make-room deposit takes
/// them. The first row is the one a fresh attempt's first deposit names.
fn full_pack() -> Value {
    Value::Array(
        (0..28)
            .map(|slot| {
                let name = if slot == 0 {
                    "Big bones".to_string()
                } else {
                    format!("Loot {slot}")
                };
                inv(526 + slot, &name, 1)
            })
            .collect(),
    )
}

/// That page with its first row gone: the make-room deposit landed.
fn full_minus_one() -> Value {
    Value::Array(full_pack().as_array().expect("rows")[1..].to_vec())
}

/// The tile the casket's overflow lands on, as the wrapper posts it.
fn loot_tile() -> Value {
    here(3222, 3223, 1)
}

/// One Collecting call's pages: the posted `here`, the posted ground page,
/// the posted pack rows and slot count, and the posted main modal.
fn pages(ground_rows: Value, pack: Value, size: Value, main: Value) -> Value {
    json!({
        "here": loot_tile(),
        "ground": ground_rows,
        "inv": pack,
        "inv_size": size,
        "main_modal_id": main,
    })
}

/// A casket step driven to its Open: the landed report, then the `held`
/// step that captures the trail-end `hard` and is the collect's entry
/// permit.
fn opened(data: &SelectedGameData, id: i32) -> u64 {
    let page = json!([[id, 1]]);
    let token = steady(data, id);
    let open = call(data, token, page, json!({}));
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["action"], "Open", "{open}");
    token
}

/// The machine's own collect deadline, armed once on Collecting entry.
fn bound_armed() -> bool {
    RUNTIME.with(|rt| rt.borrow().clock.deadline.is_some())
}

/// Arm the deadline as already due: the only way to reach the frozen bound
/// without a two-second test.
fn force_bound() {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.clock.deadline = Some(rt.clock.now());
    });
}

/// One row of the selected family.
fn row(data: &SelectedGameData, id: i32) -> &TrailMembershipRow {
    data.trails()
        .expect("trails")
        .rows
        .iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("row {id}"))
}

/// The casket row the selected family maps to `clue` through the clue's own
/// `trail_casket` param — the casket a dig produces.
fn casket_of(data: &SelectedGameData, clue: i32) -> i32 {
    let alias = row(data, clue)
        .params
        .iter()
        .find(|param| param.key == "trail_casket")
        .expect("trail_casket")
        .value
        .clone();
    data.trails()
        .expect("trails")
        .rows
        .iter()
        .find(|row| row.alias == alias)
        .unwrap_or_else(|| panic!("casket row {alias}"))
        .id
}

/// Drive a held row to `Steady`: begin, the gate, the log line, then the
/// status line. The search verbs start on the call after this one.
fn steady(data: &SelectedGameData, id: i32) -> u64 {
    let page = json!([[id, 1]]);
    let token = token_of(&begin(data, page.clone()));
    let gate = call(data, token, page.clone(), json!({}));
    assert_eq!(gate["kind"], "callback.enabled", "{gate}");
    let logged = call(data, token, page.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let posted = call(data, token, page, json!({}));
    assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
    token
}

/// One synthetic membership row over the params a test names, so a
/// classify can be read without the selected family.
fn member(params: Vec<TrailParam>, access: Option<&str>) -> TrailMembershipRow {
    TrailMembershipRow {
        alias: "trail_clue_test".into(),
        id: 1,
        role: "clue".into(),
        params,
        access: access.map(str::to_string),
    }
}

/// One synthetic selected param on such a row.
fn param(key: &str, value: &str) -> TrailParam {
    TrailParam {
        key: key.into(),
        value: value.into(),
    }
}

/// The three selected coordinate-tool items the trio acquire chain joins by
/// alias: the ids the posted pack rows carry, and the display names the
/// pages post beside them. The join the machine makes is the id; the names
/// are only corroboration.
const SEXTANT_ITEM: i32 = 2574;
const WATCH_ITEM: i32 = 2575;
const CHART_ITEM: i32 = 2576;
const SEXTANT_NAME: &str = "Sextant";
const WATCH_NAME: &str = "Watch";
const CHART_NAME: &str = "Chart";

/// The three held trio rows as the posted pack posts them: what a sextant
/// row's own acquire chain reads as `hasAllTrio` before it lets either dig
/// arm run.
fn trio_inv() -> Value {
    json!([
        inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
        inv(WATCH_ITEM, WATCH_NAME, 1),
        inv(CHART_ITEM, CHART_NAME, 1),
    ])
}

/// The posted pack a sextant row's non-acquire scene carries: the held trio
/// plus the rows a test names. A test that posts no trio at all is a test
/// of the intercept itself.
fn trio_pack(rows: &[Value]) -> Value {
    let mut pack = trio_inv();
    pack.as_array_mut().expect("rows").extend_from_slice(rows);
    pack
}

/// The tile the selected observatory professor stands on, as the published
/// `trio_givers` row posts it — never a copied frozen coordinate.
fn professor_tile(data: &SelectedGameData) -> Tile {
    giver_tile(data, OBSERVATORY_PROFESSOR)
}

/// The published spawn of one selected giver, as this machine reads it.
fn giver_tile(data: &SelectedGameData, alias: &str) -> Tile {
    let row = data
        .trio_givers()
        .expect("trio_givers")
        .rows
        .iter()
        .find(|row| row.alias == alias)
        .unwrap_or_else(|| panic!("giver {alias}"));
    let spawn = row.spawn.as_ref().expect("published spawn");
    Tile {
        x: spawn.x,
        z: spawn.z,
        level: spawn.plane,
    }
}

/// The decoded tile of the guarded exemplar `2723`: `0_47_60_50_44`.
fn guarded_tile_of(data: &SelectedGameData) -> Tile {
    guarded_tile(row(data, GUARDED)).expect("guarded tile")
}

/// The marshalled scene the guarded encounter walks and Digs over: the
/// posted `here` on (or off) the decoded tile and the pack that carries
/// the Spade beside the held trio the acquire chain already cleared.
fn dig_scene(here_tile: Value, spade: bool) -> Value {
    json!({ "here": here_tile, "inv": dig_inv(spade) })
}

/// The pack a dig or fight scene posts: the held trio, and — when the test
/// says so — the Spade the Dig resolves. The Spade is posted as the
/// marshalled item row the host resolves by display name.
fn dig_inv(spade: bool) -> Value {
    if spade {
        trio_pack(&[inv(SPADE_ITEM, SPADE_NAME, 1)])
    } else {
        trio_pack(&[])
    }
}

/// One selected giver's own packed id and display name, as the published
/// `trio_givers` row carries them — never a copied number and never a
/// frozen table.
fn giver_identity<'a>(data: &'a SelectedGameData, alias: &str) -> (i32, &'a str) {
    let row = data
        .trio_givers()
        .expect("trio_givers")
        .rows
        .iter()
        .find(|row| row.alias == alias)
        .unwrap_or_else(|| panic!("giver {alias}"));
    (row.id, row.name.as_str())
}

/// One posted chat option row: the text the arm folds, and the 1-based
/// posted slot it answers with.
fn option(text: &str, slot: i32) -> Value {
    json!({ "text": text, "option": slot })
}

/// One wrapper-marshalled posted npc row on the decoded `here` tile: the
/// posted index the Attack carries, the posted id, tile and `in_combat`
/// flag the marshalled page always carries, the posted name the family
/// filter matches, the posted health pair and target pair the kill is read
/// through, and the posted distance the frozen radius prefers.
fn npc(index: i32, name: &str, distance: i32, health: i32, max_health: i32) -> Value {
    json!({
        "index": index,
        "id": 100 + index,
        "name": name,
        "x": 3058,
        "z": 3884,
        "level": 0,
        "distance": distance,
        "health": health,
        "max_health": max_health,
        "in_combat": false,
        "actions": [ATTACK],
        "target_kind": 0,
        "target_index": -1,
    })
}

/// The same posted row without a posted distance: the Chebyshev read the
/// spawn filter makes from the row's own posted tile and this call's
/// `here`.
fn tiled(index: i32, name: &str, x: i32, z: i32, level: i32) -> Value {
    let mut row = npc(index, name, 3, 10, 10);
    row.as_object_mut().expect("object").remove("distance");
    row["x"] = json!(x);
    row["z"] = json!(z);
    row["level"] = json!(level);
    row
}

/// One posted field of a marshalled npc row replaced.
fn field(mut row: Value, key: &str, value: Value) -> Value {
    row[key] = value;
    row
}

/// One posted field of a marshalled npc row dropped: what a page that
/// never posted it hands the machine.
fn unfield(mut row: Value, key: &str) -> Value {
    row.as_object_mut().expect("object").remove(key);
    row
}

/// The same posted row with the npc's own target on the player.
fn targeting(mut row: Value, slot: i32) -> Value {
    row["target_kind"] = json!(PLAYER_KIND);
    row["target_index"] = json!(slot);
    row
}

/// One fight call's pages: the posted npc page, the posted local-player
/// slot, the Protect from Magic overlay the gate reads, and the posted
/// effective hitpoints. The overlay is up and the player is healthy unless
/// a test says otherwise, so each test names only the gate it is about.
fn fight_scene(npcs: Value, extra: Value) -> Value {
    let mut scene = json!({
        "here": here(3058, 3884, 0),
        "inv": dig_inv(true),
        "npcs": npcs,
        "self_slot": 0,
        "varp95": 1,
        "hitpoints": 40,
    });
    for (key, value) in extra.as_object().expect("extra") {
        scene[key] = value.clone();
    }
    scene
}

/// Drive the guarded exemplar from `Steady` to its spawn: the walk, then
/// the first Dig. The encounter is live after this and the fight pages
/// begin.
fn spawned(data: &SelectedGameData) -> u64 {
    let page = json!([[GUARDED, 1]]);
    let token = steady(data, GUARDED);
    let tile = guarded_tile_of(data);
    let walk = call(
        data,
        token,
        page.clone(),
        dig_scene(here(3100, 3300, 0), true),
    );
    assert_eq!(walk["kind"], "walk", "{walk}");
    let dig = call(
        data,
        token,
        page,
        dig_scene(here(tile.x, tile.z, tile.level), true),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
    token
}

/// The owned wizard's last-seen aged past the frozen grace: the only way to
/// reach the gone-outside-grace read without a six-second test.
fn age_owned_seen(ms: u64) {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        if let Some(owned) = rt.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
            owned.seen_at -= Duration::from_millis(ms);
        }
    });
}

/// A freeze that outlasted the remaining kill grace, without a six-second
/// test: the clock's own `frozen_at` and the owned wizard's last-seen are
/// both placed at the freeze's start, so the reclaim the thaw makes is
/// exactly the frozen interval — the shape a real long freeze hands the
/// session, where the pause begins with the stamp it found and the thaw
/// lands a whole interval later.
fn froze_across_grace() {
    on_pause();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let frozen_at = Instant::now() - Duration::from_millis(KILL_GRACE_MS + 1);
        rt.clock.frozen_at = Some(frozen_at);
        if let Some(owned) = rt.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
            owned.seen_at = frozen_at;
        }
    });
    on_resume();
}

/// The wizard the frozen cap documents for `2723`'s family alias.
const WIZARD: &str = "Zamorak Wizard";

#[test]
fn a_frozen_clock_emits_wait_and_keeps_the_gate_unanswered() {
    on_reset();
    let data = selected();
    let opened = begin(&data, json!([[CASKET, 1]]));
    assert_eq!(opened["kind"], "token");
    let token = token_of(&opened);
    let gate = call(&data, token, json!([[CASKET, 1]]), json!({}));
    assert_eq!(gate["kind"], "callback.enabled", "{gate}");

    // Paused: no callback, no verb, and the answer is not consumed. The
    // freeze wins over the posted interrupt carried in the same call.
    on_pause();
    let paused = call(
        &data,
        token,
        json!([[CASKET, 1]]),
        json!({ "resume": true, "hold": true }),
    );
    assert_eq!(paused["kind"], "wait", "{paused}");
    assert!(paused.get("message").is_none(), "{paused}");
    on_resume();
    let after_pause = call(&data, token, json!([[CASKET, 1]]), json!({}));
    assert_eq!(
        after_pause["kind"], "callback.enabled",
        "a frozen call burns nothing: {after_pause}"
    );

    // A held clock is the same wait.
    on_hold(true);
    let held_clock = call(
        &data,
        token,
        json!([[CASKET, 1]]),
        json!({ "resume": true }),
    );
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);

    // The posted `hold || ours` signal without a frozen clock: yield, and
    // the token survives it.
    let yielded = call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true }));
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    assert_eq!(token_of(&yielded), token, "{yielded}");

    // The gate was still open, so the enabled answer lands now.
    let enabled = call(
        &data,
        token,
        json!([[CASKET, 1]]),
        json!({ "resume": true }),
    );
    assert_eq!(enabled["kind"], "callback.log", "{enabled}");
}

#[test]
fn a_generation_bump_aborts_the_session_without_a_verb() {
    on_reset();
    let data = selected();
    let opened = begin(&data, json!([[MAP_EMPTY, 1]]));
    let token = token_of(&opened);

    let bumped = dispatch(
        Some(&data),
        &payload(
            "next",
            Some(token),
            json!([[MAP_EMPTY, 1]]),
            json!({ "generation": 5 }),
        ),
    );
    assert_eq!(bumped["kind"], "aborted", "{bumped}");
    assert_eq!(bumped["reason"], "aborted", "{bumped}");
    for kind in [
        "wait",
        "yield",
        "callback.enabled",
        "callback.log",
        "callback.setStatus",
        "grind-ready",
        "done",
        "dead",
        "abandon",
        "supplies-needed",
        "guardian-lost",
    ] {
        assert_ne!(bumped["kind"], kind, "{bumped}");
    }
    // The token died with the bump: the same call on the old generation
    // is stale, not a resumed session.
    let after = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

#[test]
fn enabled_is_re_read_at_every_gate_and_never_captured() {
    on_reset();
    let data = selected();
    // Extra begin keys are ignored, not captured: no `enabled`, `resume`,
    // hold flag or page identity is frozen into the session.
    let opened = dispatch(
        Some(&data),
        &payload(
            "begin",
            None,
            json!([[MAP_EMPTY, 1]]),
            json!({ "enabled": false, "resume": false, "hold": true }),
        ),
    );
    assert_eq!(opened["kind"], "token", "{opened}");
    let token = token_of(&opened);
    assert!(
        opened.get("kind").is_some() && opened.get("reason").is_none(),
        "begin never returns a continue kind and never a reason: {opened}"
    );

    let first = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
    assert_eq!(first["kind"], "callback.enabled", "{first}");
    // False means do not execute: an idle continue with the token live.
    let denied = call(
        &data,
        token,
        json!([[MAP_EMPTY, 1]]),
        json!({ "resume": false }),
    );
    assert_eq!(denied["kind"], "wait", "{denied}");
    assert_eq!(token_of(&denied), token, "{denied}");
    // The next tick asks again rather than replaying the captured answer.
    let again = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
    assert_eq!(again["kind"], "callback.enabled", "{again}");
    // And a later true is honored, because nothing was captured at begin.
    let enabled = call(
        &data,
        token,
        json!([[MAP_EMPTY, 1]]),
        json!({ "resume": true }),
    );
    assert_eq!(enabled["kind"], "callback.log", "{enabled}");
    let message = enabled["message"].as_str().unwrap_or("");
    assert!(message.contains("trail_clue_hard_map001"), "{enabled}");
    assert!(message.contains("2722"), "{enabled}");

    // The paramless 2722 row keeps the token and then idles.
    let steady = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
    assert_eq!(steady["kind"], "callback.setStatus", "{steady}");
    assert!(
        steady["message"]
            .as_str()
            .unwrap_or("")
            .contains("trail_clue_hard_map001"),
        "{steady}"
    );
    for _ in 0..2 {
        let idle = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
        assert_eq!(idle["kind"], "wait", "{idle}");
        assert_eq!(token_of(&idle), token, "{idle}");
    }
}

#[test]
fn a_new_held_step_re_arms_the_gate() {
    on_reset();
    let data = selected();
    let token = token_of(&begin(&data, json!([[CASKET, 1]])));
    assert_eq!(
        call(&data, token, json!([[CASKET, 1]]), json!({}))["kind"],
        "callback.enabled"
    );
    assert_eq!(
        call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true })
        )["kind"],
        "callback.log"
    );
    // Another row replaces the casket: the enabled read is not reused.
    let re_armed = call(
        &data,
        token,
        json!([[MAP_EMPTY, 1]]),
        json!({ "resume": true }),
    );
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    assert_eq!(token_of(&re_armed), token, "{re_armed}");
}

#[test]
fn none_held_is_a_refusal_and_never_a_live_token_or_a_done() {
    on_reset();
    let data = selected();
    let challenge = unrelated(&data);
    for held in [json!([]), json!([[challenge, 1]]), json!([[CASKET, 0]])] {
        let refused = begin(&data, held.clone());
        assert_eq!(refused["kind"], "aborted", "{held} {refused}");
        assert_eq!(refused["reason"], "none-held", "{held} {refused}");
        // No live token: the refusal's own number is not a session.
        let after = call(&data, token_of(&refused), held.clone(), json!({}));
        assert_eq!(after["reason"], "stale", "{held} {after}");
    }

    // A live session that loses its held membership errors `none-held`
    // and aborts — it does not quietly become done.
    let token = token_of(&begin(&data, json!([[CASKET, 1]])));
    let lost = call(&data, token, json!([[challenge, 1]]), json!({}));
    assert_eq!(lost["kind"], "aborted", "{lost}");
    assert_eq!(lost["reason"], "none-held", "{lost}");
    let after = call(&data, token, json!([[CASKET, 1]]), json!({}));
    assert_eq!(after["reason"], "stale", "{after}");
}

#[test]
fn a_missing_selected_pin_is_not_none_held() {
    on_reset();
    let refused = dispatch(
        None,
        &payload("begin", None, json!([[CASKET, 1]]), json!({})),
    );
    assert_eq!(refused["kind"], "aborted", "{refused}");
    assert_eq!(refused["reason"], "missing-selected-data", "{refused}");
    // The live token's view of a vanished pin is the same refusal.
    let data = selected();
    let token = token_of(&begin(&data, json!([[CASKET, 1]])));
    let gone = dispatch(
        None,
        &payload("next", Some(token), json!([[CASKET, 1]]), json!({})),
    );
    assert_eq!(gone["kind"], "aborted", "{gone}");
    assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
}

/// The solved mark is eleven characters, lowercase, one space — the exact
/// string — and it is the machine's own `callback.setStatus`, posted once a
/// collect is over and by no other path on this machine.
#[test]
fn the_exact_clue_solved_string_is_the_finished_collects_alone() {
    on_reset();
    let data = selected();
    // The paramless 2722 row is identified, reported and then idled, and
    // never a verb of any kind — and never the solved mark.
    let token = token_of(&begin(&data, json!([[MAP_EMPTY, 1]])));
    let steps = vec![
        call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
        call(
            &data,
            token,
            json!([[MAP_EMPTY, 1]]),
            json!({ "resume": true }),
        ),
        call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
        call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
        call(
            &data,
            token,
            json!([[MAP_EMPTY, 1]]),
            json!({ "hold": true }),
        ),
    ];
    assert_eq!(
        steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("callback.enabled"),
            json!("callback.log"),
            json!("callback.setStatus"),
            json!("wait"),
            json!("yield"),
        ],
        "{steps:?}"
    );
    for step in &steps {
        let text = step.to_string();
        assert!(!text.contains("clue solved"), "{step}");
        assert!(!text.contains("ownsEquipment"), "{step}");
        assert!(
            step["status"].is_null(),
            "the machine emits kinds, not the public status: {step}"
        );
    }

    // The finished collect is that one seat: the status is the exact
    // string, it rides the live token, and the two completion steps that
    // follow it are the continue and the end of the session.
    let token = opened(&data, EASY_CASKET);
    let scene = pages(json!([]), json!([]), json!(28), json!(-1));
    let waiting = call(&data, token, json!([]), scene.clone());
    assert_eq!(waiting["kind"], "wait", "{waiting}");
    assert!(!waiting.to_string().contains("clue solved"), "{waiting}");
    force_bound();
    let solved = call(&data, token, json!([]), scene.clone());
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    assert_eq!(token_of(&solved), token, "{solved}");
    let ready = call(&data, token, json!([]), scene.clone());
    assert_eq!(ready["kind"], "grind-ready", "{ready}");
    assert_eq!(
        token_of(&ready),
        token,
        "the grind handback is live: {ready}"
    );
    assert!(!ready.to_string().contains("clue solved"), "{ready}");
    let done = call(&data, token, json!([]), scene.clone());
    assert_eq!(done["kind"], "done", "{done}");
    let after = call(&data, token, json!([]), scene);
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

#[test]
fn the_packed_coord_contract_is_copied_and_off_contract_tokens_idle() {
    // The pinned vector: `trail_clue_easy_simple001`'s selected token.
    assert_eq!(
        decode_trail_coord("1_50_50_9_18"),
        Some(Tile {
            x: 3209,
            z: 3218,
            level: 1
        })
    );
    // The landed `nav::canlight` vector, plus both ends of each bound.
    assert_eq!(
        decode_trail_coord("0_50_53_50_24"),
        Some(Tile {
            x: 3250,
            z: 3416,
            level: 0
        })
    );
    assert_eq!(
        decode_trail_coord("3_0_0_0_0"),
        Some(Tile {
            x: 0,
            z: 0,
            level: 3
        })
    );
    assert_eq!(
        decode_trail_coord("0_0_0_63_63"),
        Some(Tile {
            x: 63,
            z: 63,
            level: 0
        })
    );
    for token in [
        "",
        "1_50_50_9",
        "1_50_50_9_18_1",
        "1_50_50_9_x",
        "one_50_50_9_18",
        "4_50_50_9_18",
        "-1_50_50_9_18",
        "1_50_50_64_18",
        "1_50_50_9_64",
        "1_50_50_-1_18",
        "1_50_50_9_-1",
        " 1_50_50_9_18",
        "1_50_50_9_18 ",
        // A map that would overflow the packed widening is not a tile.
        "2147483647_50_50_9_18",
        "1_2147483647_50_9_18",
    ] {
        assert_eq!(decode_trail_coord(token), None, "{token:?}");
    }
}

#[test]
fn membership_is_the_loc_pin_plus_a_decodable_coord_on_the_same_row() {
    let data = selected();
    assert_eq!(
        search_tile(row(&data, SEARCH)),
        Some(Tile {
            x: 3209,
            z: 3218,
            level: 1
        })
    );
    // The coord-only maps, the desc-only frozen `keyFrom` riddles, the
    // constrained 3554 clue and a paramless casket are not search rows.
    for id in [MAP, RIDDLE, CLUE, CASKET] {
        assert_eq!(search_tile(row(&data, id)), None, "{id}");
    }
    // The selected pin is the membership, and every pinned row on this
    // pin carries a decodable coord.
    let facts = data.trails().expect("trails");
    let pinned: Vec<&TrailMembershipRow> = facts
        .rows
        .iter()
        .filter(|row| {
            row.params
                .iter()
                .any(|param| param.key == "trail_loc" && param.value == "^true")
        })
        .collect();
    assert_eq!(pinned.len(), 58, "selected trail_loc=^true rows");
    for row in &pinned {
        assert!(search_tile(row).is_some(), "{}", row.alias);
    }
    // Synthetic rows: the pin without a coord, an off-contract coord, and
    // a bare `true` that is not the `^true` pin all idle.
    let pin = || TrailParam {
        key: "trail_loc".into(),
        value: "^true".into(),
    };
    let coord = |value: &str| TrailParam {
        key: "trail_coord".into(),
        value: value.into(),
    };
    let member = |params: Vec<TrailParam>| TrailMembershipRow {
        alias: "trail_clue_test".into(),
        id: 1,
        role: "clue".into(),
        params,
        access: None,
    };
    assert_eq!(search_tile(&member(vec![pin()])), None);
    assert_eq!(search_tile(&member(vec![coord("1_50_50_9_18")])), None);
    assert_eq!(search_tile(&member(vec![pin(), coord("1_50_50_9")])), None);
    assert_eq!(
        search_tile(&member(vec![
            TrailParam {
                key: "trail_loc".into(),
                value: "true".into(),
            },
            coord("1_50_50_9_18"),
        ])),
        None
    );
    assert_eq!(
        search_tile(&member(vec![pin(), coord("1_50_50_9_18")])),
        Some(Tile {
            x: 3209,
            z: 3218,
            level: 1
        })
    );
}

#[test]
fn a_search_row_walks_to_the_decoded_tile_and_then_picks_the_loc() {
    on_reset();
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let token = steady(&data, SEARCH);

    // Not arrived: the walk is the decoded tile, and it repeats.
    for _ in 0..2 {
        let walk = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3200, 3218, 1) }),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], 3209, "{walk}");
        assert_eq!(walk["z"], 3218, "{walk}");
        assert_eq!(walk["level"], 1, "{walk}");
        assert_eq!(token_of(&walk), token, "{walk}");
    }
    // Another level is not arrival, even standing on the decoded tile,
    // and neither is two tiles away on one axis.
    for far in [here(3209, 3218, 0), here(3211, 3218, 1)] {
        let walk = call(&data, token, page.clone(), json!({ "here": far }));
        assert_eq!(walk["kind"], "walk", "{walk}");
    }

    // Arrived. The nearest row wins over the better rank: the adjacent
    // `Search` loses to the `Open` on the decoded tile.
    let nearest = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(3209, 3218, 1),
            "locs": [
                loc(11, 3210, 3218, 1, &["Search"]),
                loc(12, 3209, 3218, 1, &["Open"]),
            ],
        }),
    );
    assert_eq!(nearest["kind"], "loc", "{nearest}");
    assert_eq!(nearest["action"], "Open", "{nearest}");
    assert_eq!(nearest["id"], 12, "{nearest}");
    assert_eq!(
        (
            nearest["x"].clone(),
            nearest["z"].clone(),
            nearest["level"].clone()
        ),
        (json!(3209), json!(3218), json!(1)),
        "{nearest}"
    );

    // Same distance: the rank decides, not the posted order.
    let ranked = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(3209, 3218, 1),
            "locs": [
                loc(13, 3209, 3218, 1, &["Open"]),
                loc(14, 3209, 3218, 1, &["Search", "Pick"]),
            ],
        }),
    );
    assert_eq!(ranked["kind"], "loc", "{ranked}");
    assert_eq!(ranked["action"], "Search", "{ranked}");
    assert_eq!(ranked["id"], 14, "{ranked}");

    // Same distance and rank: the first posted row wins.
    let first = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(3209, 3218, 1),
            "locs": [
                loc(15, 3209, 3218, 1, &["Search"]),
                loc(16, 3209, 3218, 1, &["search"]),
            ],
        }),
    );
    assert_eq!(first["id"], 15, "{first}");
    assert_eq!(first["action"], "Search", "{first}");

    // Off-contract rows are skipped: another level, distance two, no
    // searchable action, a non-array action list, and no posted id. The
    // survivor matches case-insensitively and keeps its own tile.
    let filtered = call(
        &data,
        token,
        page,
        json!({
            "here": here(3209, 3218, 1),
            "locs": [
                loc(20, 3209, 3218, 2, &["Search"]),
                loc(21, 3211, 3218, 1, &["Search"]),
                loc(22, 3209, 3218, 1, &["Pick", "Use"]),
                json!({ "id": 23, "x": 3209, "z": 3218, "level": 1, "actions": "Search" }),
                json!({ "x": 3209, "z": 3218, "level": 1, "actions": ["Search"] }),
                loc(25, 3210, 3217, 1, &["sEaRcH"]),
            ],
        }),
    );
    assert_eq!(filtered["kind"], "loc", "{filtered}");
    assert_eq!(filtered["action"], "Search", "{filtered}");
    assert_eq!(filtered["id"], 25, "{filtered}");
    assert_eq!(
        (
            filtered["x"].clone(),
            filtered["z"].clone(),
            filtered["level"].clone()
        ),
        (json!(3210), json!(3217), json!(1)),
        "the verb keeps the posted row's own tile: {filtered}"
    );
}

#[test]
fn an_arrived_search_row_waits_without_a_loc_and_never_abandons() {
    on_reset();
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let token = steady(&data, SEARCH);
    for locs in [json!([]), json!([loc(11, 3209, 3218, 1, &["Pick"])])] {
        let idle = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3209, 3218, 1), "locs": locs }),
        );
        assert_eq!(idle["kind"], "wait", "{idle}");
        assert_eq!(token_of(&idle), token, "{idle}");
        let text = idle.to_string();
        for forbidden in ["no-searchable-loc", "abandon", "done", "clue solved"] {
            assert!(!text.contains(forbidden), "{idle}");
        }
    }
    // No posted `here` at all is the same wait: no arrival claim and no
    // blind walk.
    let no_tile = call(&data, token, page.clone(), json!({ "locs": [] }));
    assert_eq!(no_tile["kind"], "wait", "{no_tile}");
    let malformed = call(
        &data,
        token,
        page.clone(),
        json!({ "here": json!({ "x": 1 }) }),
    );
    assert_eq!(malformed["kind"], "wait", "{malformed}");
    // And the session is still live for the loc that appears later.
    let picked = call(
        &data,
        token,
        page,
        json!({
            "here": here(3208, 3218, 1),
            "locs": [loc(42, 3209, 3218, 1, &["Search"])],
        }),
    );
    assert_eq!(picked["kind"], "loc", "{picked}");
    assert_eq!(picked["id"], 42, "{picked}");
}

#[test]
fn freeze_and_yield_beat_the_search_walk() {
    on_reset();
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let token = steady(&data, SEARCH);
    let scene = json!({
        "here": here(3100, 3300, 1),
        "locs": [loc(11, 3209, 3218, 1, &["Search"])],
    });
    // Frozen: wait, and neither verb rides along with it.
    on_pause();
    let paused = call(&data, token, page.clone(), scene.clone());
    assert_eq!(paused["kind"], "wait", "{paused}");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, page.clone(), scene.clone());
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    // The posted `hold || ours` interrupt, unfrozen: yield, still no verb.
    let yielded = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(3100, 3300, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
            "hold": true,
        }),
    );
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    for step in [&paused, &held_clock, &yielded] {
        assert!(step.get("x").is_none(), "{step}");
        assert!(step.get("action").is_none(), "{step}");
        assert!(step.get("id").is_none(), "{step}");
    }
    // The walk is still there once the interrupt clears.
    let walking = call(&data, token, page, scene);
    assert_eq!(walking["kind"], "walk", "{walking}");
}

#[test]
fn non_search_rows_stay_idle_over_a_fully_posted_scene() {
    on_reset();
    let data = selected();
    let scene = json!({
        "here": here(3209, 3218, 1),
        "locs": [loc(11, 3209, 3218, 1, &["Search"])],
    });
    // A casket is a held Open and the packed 3554 clue is the constrained
    // refusal, so neither is in this set: 2722 is a paramless clue row and
    // 2831 is the key-hunt riddle, which walks to its published spawn from
    // this very scene and has its own proof below. The coord-only map is no
    // longer one of them either: the widened dig arm walks and Digs from
    // its own tile. The five matcher-keepers the key family publishes no
    // packed type for are idled here too.
    for id in std::iter::once(MAP_EMPTY).chain(MATCHER_KEEPERS) {
        let page = json!([[id, 1]]);
        let token = steady(&data, id);
        for _ in 0..2 {
            let idle = call(&data, token, page.clone(), scene.clone());
            assert_eq!(idle["kind"], "wait", "{id} {idle}");
            assert_eq!(token_of(&idle), token, "{id} {idle}");
            assert!(idle.get("x").is_none(), "{id} {idle}");
            assert!(idle.get("action").is_none(), "{id} {idle}");
        }
    }
}

/// The packed 3554 `access: "constrained"` clue is not idled: the machine
/// refuses it — `aborted` / `constrained`, no verb and no live token — and
/// a `begin` that identifies it is refused the same way.
#[test]
fn a_held_constrained_row_is_refused_and_never_played() {
    on_reset();
    let data = selected();
    let page = json!([[CLUE, 1]]);
    let refused = begin(&data, page.clone());
    assert_eq!(refused["kind"], "aborted", "{refused}");
    assert_eq!(refused["reason"], "constrained", "{refused}");
    // No live token: the refusal's own number is not a session.
    let after = call(&data, token_of(&refused), page.clone(), json!({}));
    assert_eq!(after["reason"], "stale", "{after}");

    // The same row held by a live session, over a scene the dig arms would
    // walk and Dig from: identified, then refused with no verb at all. The
    // refusal is not the gate: the constrained check runs before any
    // re-arm, so the previous step's identity never gets a callback.
    let scene = json!({
        "here": here(3209, 3218, 1),
        "locs": [loc(11, 3209, 3218, 1, &["Search"])],
        "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
    });
    let token = token_of(&begin(&data, json!([[CASKET, 1]])));
    let denied = call(&data, token, page.clone(), scene.clone());
    assert_eq!(denied["kind"], "aborted", "{denied}");
    assert_eq!(denied["reason"], "constrained", "{denied}");
    for absent in ["name", "action", "x", "z", "level", "message", "id"] {
        assert!(denied.get(absent).is_none(), "{absent} {denied}");
    }
    let after = call(&data, token, page, scene);
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

#[test]
fn a_casket_row_opens_the_held_item_only_after_its_own_report() {
    on_reset();
    let data = selected();
    let page = json!([[CASKET, 1]]);
    let token = token_of(&begin(&data, page.clone()));
    // The landed report comes first: no Open rides along with the gate,
    // the log line or the status line.
    let steps = vec![
        call(&data, token, page.clone(), json!({})),
        call(&data, token, page.clone(), json!({ "resume": true })),
    ];
    assert_eq!(steps[0]["kind"], "callback.enabled", "{steps:?}");
    assert_eq!(steps[1]["kind"], "callback.log", "{steps:?}");
    let posted = call(&data, token, page.clone(), json!({}));
    assert_eq!(posted["kind"], "callback.setStatus", "{posted}");

    // Open replaces `Steady` from here, and repeats while the same casket
    // id stays held — the host refuses a casket it no longer holds.
    for _ in 0..2 {
        let open = call(&data, token, page.clone(), json!({}));
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["name"], "Casket", "{open}");
        assert_eq!(open["action"], "Open", "{open}");
        assert_eq!(token_of(&open), token, "{open}");
        // No bound row identity and no scene verb: the host resolves the
        // first inventory row with this name.
        assert!(open.get("id").is_none(), "{open}");
        assert!(open.get("x").is_none(), "{open}");
        assert!(open.get("z").is_none(), "{open}");
        assert!(open.get("level").is_none(), "{open}");
        assert_ne!(open["kind"], "done", "{open}");
        assert!(!open.to_string().contains("clue solved"), "{open}");
    }

    // A different held row re-arms the gate: the Open was the casket's,
    // and the paramless row behind it is not opened.
    let other = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
    assert_eq!(other["kind"], "callback.enabled", "{other}");
    assert_eq!(token_of(&other), token, "{other}");
}

#[test]
fn the_sextant_casket_is_the_open_and_not_the_packed_clue() {
    on_reset();
    let data = selected();
    // Identify is casket-first: with the constrained 3554 clue and its
    // casket both held, the casket row is the step. The landed report is
    // the casket's own, and the Open is never 3554 play.
    let both = json!([[CLUE, 1], [SEXTANT_CASKET, 1]]);
    let token = token_of(&begin(&data, both.clone()));
    assert_eq!(
        call(&data, token, both.clone(), json!({}))["kind"],
        "callback.enabled"
    );
    let logged = call(&data, token, both.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let message = logged["message"].as_str().unwrap_or("");
    assert!(
        message.contains("trail_clue_hard_sextant028_casket"),
        "{logged}"
    );
    assert!(message.contains("3555"), "{logged}");
    assert!(!message.contains("3554"), "{logged}");
    assert_eq!(
        call(&data, token, both.clone(), json!({}))["kind"],
        "callback.setStatus"
    );
    let open = call(&data, token, both.clone(), json!({}));
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["name"], "Casket", "{open}");
    assert_eq!(open["action"], "Open", "{open}");
    // The clue left on its own is the constrained refusal, never 3554
    // play: casket-first is the precedence that kept it out of the Open.
    let clue_only = call(&data, token, json!([[CLUE, 1]]), json!({}));
    assert_eq!(clue_only["kind"], "aborted", "{clue_only}");
    assert_eq!(clue_only["reason"], "constrained", "{clue_only}");
    let after = call(&data, token, json!([[CLUE, 1]]), json!({}));
    assert_eq!(after["reason"], "stale", "{after}");
}

#[test]
fn freeze_and_yield_beat_the_casket_open() {
    on_reset();
    let data = selected();
    let page = json!([[CASKET, 1]]);
    let token = steady(&data, CASKET);
    // Frozen: wait, and no Open rides along with it.
    on_pause();
    let paused = call(&data, token, page.clone(), json!({ "hold": true }));
    assert_eq!(paused["kind"], "wait", "{paused}");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, page.clone(), json!({}));
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    // The posted `hold || ours` interrupt, unfrozen: yield, still no Open
    // and the token lives.
    let yielded = call(&data, token, page.clone(), json!({ "hold": true }));
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    assert_eq!(token_of(&yielded), token, "{yielded}");
    for step in [&paused, &held_clock, &yielded] {
        assert!(step.get("name").is_none(), "{step}");
        assert!(step.get("action").is_none(), "{step}");
    }
    // Thawed and unheld, the Open is still there.
    let open = call(&data, token, page, json!({}));
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["name"], "Casket", "{open}");
    assert_eq!(open["action"], "Open", "{open}");
}

#[test]
fn every_selected_casket_row_resolves_the_casket_display_name() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).expect("selected data");
        let facts = data.trails().expect("trails");
        let caskets: Vec<&TrailMembershipRow> = facts
            .rows
            .iter()
            .filter(|row| row.role == CASKET_ROLE)
            .collect();
        assert_eq!(caskets.len(), 71, "{revision:?}");
        for casket in &caskets {
            assert!(casket.params.is_empty(), "{revision:?} {}", casket.alias);
            assert_eq!(
                casket_name(Some(&data), casket),
                Some("Casket"),
                "{revision:?} {}",
                casket.alias
            );
        }
        // The clue rows carry a display name too, and are still not
        // caskets: the role decides, not the item table.
        assert_eq!(casket_name(Some(&data), row(&data, CLUE)), None);
        assert_eq!(casket_name(Some(&data), row(&data, SEARCH)), None);
        // No selected pin, and a casket id that joins no selected item,
        // are both no identity to dispatch — never an invented name.
        let orphan = TrailMembershipRow {
            alias: "trail_clue_test_casket".into(),
            id: i32::MAX,
            role: CASKET_ROLE.into(),
            params: Vec::new(),
            access: None,
        };
        assert_eq!(casket_name(None, &orphan), None, "{revision:?}");
        assert_eq!(casket_name(Some(&data), &orphan), None, "{revision:?}");
    }
}

#[test]
fn the_search_row_emits_only_walk_and_loc_and_never_a_completion() {
    on_reset();
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let token = steady(&data, SEARCH);
    let steps = vec![
        call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3200, 3218, 1) }),
        ),
        call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [loc(11, 3209, 3218, 1, &["Search"])],
            }),
        ),
        call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3209, 3218, 1) }),
        ),
        call(&data, token, page, json!({ "hold": true })),
    ];
    for step in &steps {
        let text = step.to_string();
        for forbidden in ["clue solved", "abandon", "supplies-needed", "dead", "done"] {
            assert!(!text.contains(forbidden), "{step}");
        }
        assert!(
            step["status"].is_null(),
            "the machine emits kinds, not the public status: {step}"
        );
        assert!(
            matches!(
                step["kind"].as_str().unwrap_or(""),
                "walk" | "loc" | "wait" | "yield"
            ),
            "the search row emits walk, loc, wait or yield only: {step}"
        );
    }
    assert_eq!(
        steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![json!("walk"), json!("loc"), json!("wait"), json!("yield"),],
        "{steps:?}"
    );
}

#[test]
fn an_unknown_op_is_not_impl() {
    on_reset();
    let data = selected();
    let step = dispatch(Some(&data), &json!({ "op": "solve" }));
    assert_eq!(step["kind"], "notImpl", "{step}");
}

/// `Steady` on a casket whose Open went out survives the identify
/// `none-held` that used to abort: the posted reward interface closes, the
/// casket's own overflow is Taken, the settled Take logs, and the empty
/// page after it finishes with the machine's own three-step completion —
/// the exact `'clue solved'` status, then `grind-ready`, then the `done`
/// the token dies on. This is the sextant casket the packed 3554 clue
/// belongs to, so the collect is never 3554 play.
#[test]
fn collecting_closes_the_posted_reward_then_takes_the_casket_overflow() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    // The Open is not the bound: the window is armed on Collecting entry
    // and nowhere else.
    assert!(!bound_armed(), "the Open arms nothing");

    // The casket left the pack: identify is `none-held` from the step
    // whose Open went out. The posted reward interface is closed first, by
    // the id the page posted — never a 6960 constant.
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([inv(385, "Shark", 5)]),
        json!(28),
        json!(6960),
    );
    let closed = call(&data, token, json!([]), scene.clone());
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    assert_eq!(closed["token"], json!(token), "{closed}");
    assert!(closed.get("reason").is_none(), "{closed}");
    assert!(bound_armed(), "entry arms the frozen reward window");

    // Any other posted open id closes the same way; the verb is the page's
    // own fact and carries nothing else.
    let other = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(42)),
    );
    assert_eq!(other["kind"], "close-modal", "{other}");
    for absent in ["x", "z", "level", "name", "action", "id", "message"] {
        assert!(other.get(absent).is_none(), "{absent} {other}");
    }

    // Closed: the overflow on the posted tile is Taken by posted name.
    let took = call(
        &data,
        token,
        json!([]),
        pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([inv(385, "Shark", 5)]),
            json!(28),
            json!(-1),
        ),
    );
    assert_eq!(took["kind"], "obj", "{took}");
    assert_eq!(took["action"], "Take", "{took}");
    assert_eq!(took["name"], "Rune platebody", "{took}");
    assert_eq!(
        (took["x"].clone(), took["z"].clone(), took["level"].clone()),
        (json!(3222), json!(3223), json!(1)),
        "{took}"
    );
    assert_eq!(took["token"], json!(token), "{took}");

    // The row left the posted ground: the frozen line, one call later.
    let logged = call(
        &data,
        token,
        json!([]),
        pages(
            json!([]),
            json!([inv(900, "Rune platebody", 1)]),
            json!(28),
            json!(-1),
        ),
    );
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    assert_eq!(
        logged["message"], "took 'Rune platebody' from the casket",
        "{logged}"
    );

    // Still within the window an empty page is a wait, not a finish.
    let waiting = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(-1)),
    );
    assert_eq!(waiting["kind"], "wait", "{waiting}");
    assert_eq!(waiting["token"], json!(token), "{waiting}");

    // Past it the collect is over: the machine's own completion, one step
    // per call — the exact status, the live-token continue, then the end.
    force_bound();
    let solved = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(-1)),
    );
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    assert_eq!(solved["token"], json!(token), "{solved}");
    assert!(bound_armed(), "the status is not the end of the session");

    // Latched: the same page runs the completion, not the loot again. The
    // grind handback is a live token and a verbless continue.
    let ready = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(-1)),
    );
    assert_eq!(ready["kind"], "grind-ready", "{ready}");
    assert_eq!(ready["token"], json!(token), "{ready}");
    for absent in ["action", "name", "x", "z", "level", "message", "id"] {
        assert!(ready.get(absent).is_none(), "{absent} {ready}");
    }
    let done = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(-1)),
    );
    assert_eq!(done["kind"], "done", "{done}");
    assert!(!bound_armed(), "the end clears the deadline");
    let after = call(
        &data,
        token,
        json!([]),
        pages(json!([]), json!([]), json!(28), json!(-1)),
    );
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");

    let text = json!([closed, other, took, logged, waiting, solved, ready]).to_string();
    for forbidden in [
        "abandon",
        "ownsEquipment",
        "supplies-needed",
        "trail complete",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {text}");
    }
}

/// Identify is still made on every Collecting call: the next scroll — or a
/// leftover casket — leaves the phase and re-arms the landed gate, so no
/// close and no Take is ever dispatched under a step that is still held.
#[test]
fn collecting_leaves_for_the_next_scroll_and_re_arms_the_gate() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([]),
        json!(28),
        json!(6960),
    );

    // The next scroll came back: the collect is skipped.
    let page = json!([[RIDDLE, 1]]);
    let re_armed = call(&data, token, page.clone(), scene.clone());
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    assert_eq!(re_armed["token"], json!(token), "{re_armed}");
    assert!(!bound_armed(), "leaving the collect clears its window");

    // And the landed report for the new row follows, with the progress
    // line of the row identify returned — never of the casket.
    let mut gate = scene.clone();
    gate["resume"] = json!(true);
    let logged = call(&data, token, page, gate);
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let message = logged["message"].as_str().unwrap_or("");
    assert!(message.contains("2831"), "{logged}");
    assert!(!message.contains("3555"), "{logged}");
}

/// A leftover casket after the Open is the landed Open again, not a
/// second collect: identify wins and the gate re-arms for it.
#[test]
fn a_leftover_casket_opens_instead_of_collecting() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([]),
        json!(28),
        json!(6960),
    );
    let page = json!([[CASKET, 1]]);
    assert_eq!(
        call(&data, token, page.clone(), scene.clone())["kind"],
        "callback.enabled"
    );
    assert_eq!(
        call(
            &data,
            token,
            page.clone(),
            json!({ "resume": true, "here": loot_tile(), "main_modal_id": 6960 })
        )["kind"],
        "callback.log"
    );
    assert_eq!(
        call(&data, token, page.clone(), scene.clone())["kind"],
        "callback.setStatus"
    );
    let open = call(&data, token, page, scene);
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["name"], "Casket", "{open}");
    assert_eq!(open["action"], "Open", "{open}");
}

/// The ground scan: same tile as the posted `here`, a posted `Take`, the
/// frozen `SHARK_ID` skipped, and the row's own posted name riding along.
#[test]
fn the_take_needs_the_posted_tile_a_posted_take_and_a_posted_name() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let pack = json!([inv(385, "Shark", 5)]);

    // The shark is skipped, so a shark-only page has nothing to take: a
    // wait, and the token lives.
    let shark_only = call(
        &data,
        token,
        json!([]),
        pages(
            json!([ground(SHARK_ID, "Shark", 3222, 3223, 1, &["Take"])]),
            pack.clone(),
            json!(28),
            json!(-1),
        ),
    );
    assert_eq!(shark_only["kind"], "wait", "{shark_only}");

    // Off-tile, another level, no posted `Take`, no posted name, and a
    // malformed row are all skipped rather than guessed at.
    let filtered = call(
        &data,
        token,
        json!([]),
        pages(
            json!([
                ground(900, "Rune platebody", 3223, 3223, 1, &["Take"]),
                ground(901, "Rune platebody", 3222, 3223, 0, &["Take"]),
                ground(902, "Rune platebody", 3222, 3223, 1, &["Use", "Examine"]),
                json!({ "id": 903, "name": "Rune platebody", "x": 3222, "z": 3223, "level": 1, "actions": "Take" }),
                json!({ "id": 904, "x": 3222, "z": 3223, "level": 1, "actions": ["Take"] }),
            ]),
            pack.clone(),
            json!(28),
            json!(-1),
        ),
    );
    assert_eq!(filtered["kind"], "wait", "{filtered}");

    // The survivor matches case-insensitively and keeps its own posted
    // name and tile.
    let picked = call(
        &data,
        token,
        json!([]),
        pages(
            json!([ground(905, "Dragon bones", 3222, 3223, 1, &["tAkE"])]),
            pack,
            json!(28),
            json!(-1),
        ),
    );
    assert_eq!(picked["kind"], "obj", "{picked}");
    assert_eq!(picked["name"], "Dragon bones", "{picked}");
    assert_eq!(picked["action"], "Take", "{picked}");
}

/// A Take that has not settled is re-dispatched: the row is still posted,
/// so the next call is the verb again and never the `took` line.
#[test]
fn an_unsettled_take_is_dispatched_again() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([]),
        json!(28),
        json!(-1),
    );
    for _ in 0..2 {
        let took = call(&data, token, json!([]), scene.clone());
        assert_eq!(took["kind"], "obj", "{took}");
        assert_eq!(took["action"], "Take", "{took}");
    }
}

/// A full pack Drops one frozen food row and Takes next call. The easy
/// casket reaches the frozen option forms through the landed helper: the
/// row is a *form* of `Cake`, whose display name is not one of the option
/// keys.
#[test]
fn a_full_pack_drops_one_food_row_then_takes() {
    on_reset();
    let data = selected();
    assert!(
        droppable_forms(Some(&data))
            .iter()
            .any(|form| form == "slice of cake"),
        "the landed helper resolves the cake family"
    );
    let token = opened(&data, EASY_CASKET);
    let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);
    let pack = json!([inv(1895, "Slice of cake", 1), inv(1200, "Rune scimitar", 1)]);

    let dropped = call(
        &data,
        token,
        json!([]),
        pages(loot.clone(), pack.clone(), json!(2), json!(-1)),
    );
    assert_eq!(dropped["kind"], "held", "{dropped}");
    assert_eq!(dropped["name"], "Slice of cake", "{dropped}");
    assert_eq!(dropped["action"], "Drop", "{dropped}");
    assert_eq!(dropped["token"], json!(token), "{dropped}");
    // No tile and no id ride along: the host resolves the name.
    for absent in ["x", "z", "level", "id"] {
        assert!(dropped.get(absent).is_none(), "{absent} {dropped}");
    }

    // The Drop freed the slot: the Take is next call, and the row the Drop
    // put on the floor is not the row that is taken.
    let took = call(
        &data,
        token,
        json!([]),
        pages(
            json!([
                ground(900, "Rune platebody", 3222, 3223, 1, &["Take"]),
                ground(1895, "Slice of cake", 3222, 3223, 1, &["Take"]),
            ]),
            json!([inv(1200, "Rune scimitar", 1)]),
            json!(2),
            json!(-1),
        ),
    );
    assert_eq!(took["kind"], "obj", "{took}");
    assert_eq!(took["name"], "Rune platebody", "{took}");
}

/// The hard casket's own food is the frozen `SHARK_ID` — the same id the
/// Take skip already carries — and never a name match.
#[test]
fn a_full_pack_drops_the_shark_on_a_hard_casket() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);

    // A cake form is droppable on an easy casket and not on this one.
    let caked = call(
        &data,
        token,
        json!([]),
        pages(
            loot.clone(),
            json!([inv(1895, "Slice of cake", 1), inv(1200, "Rune scimitar", 1)]),
            json!(2),
            json!(-1),
        ),
    );
    assert_eq!(caked["kind"], "callback.log", "{caked}");
    assert!(
        caked["message"]
            .as_str()
            .unwrap_or("")
            .ends_with("no Shark to drop"),
        "{caked}"
    );

    // The shark itself is the hard drop: a fresh session, because the
    // WARNING above already ended this collect.
    on_reset();
    let token = opened(&data, SEXTANT_CASKET);
    let dropped = call(
        &data,
        token,
        json!([]),
        pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([inv(1200, "Rune scimitar", 1), inv(SHARK_ID, "Shark", 3)]),
            json!(2),
            json!(-1),
        ),
    );
    assert_eq!(dropped["kind"], "held", "{dropped}");
    assert_eq!(dropped["name"], "Shark", "{dropped}");
    assert_eq!(dropped["action"], "Drop", "{dropped}");
}

/// A full pack with nothing droppable logs the frozen WARNING and then
/// finishes with the machine's own completion: a log line, not an error
/// token — and never the landed `none-held` abort, which is for the paths
/// that are not a finished collect.
#[test]
fn a_full_pack_with_no_food_warns_and_then_finishes_done() {
    on_reset();
    let data = selected();
    let token = opened(&data, EASY_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([
            inv(1200, "Rune scimitar", 1),
            inv(1201, "Rune platebody", 1)
        ]),
        json!(2),
        json!(-1),
    );
    let warned = call(&data, token, json!([]), scene.clone());
    assert_eq!(warned["kind"], "callback.log", "{warned}");
    assert_eq!(
        warned["message"],
        "WARNING: 'Rune platebody' is left on the ground, the pack is full with no food to drop",
        "{warned}"
    );
    assert_eq!(warned["token"], json!(token), "{warned}");
    assert!(!warned.to_string().contains("clue solved"), "{warned}");

    // The WARNING ended the collect: the same page finishes next call
    // rather than Taking, and the token dies with the `done`.
    let solved = call(&data, token, json!([]), scene.clone());
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    let ready = call(&data, token, json!([]), scene.clone());
    assert_eq!(ready["kind"], "grind-ready", "{ready}");
    assert_eq!(ready["token"], json!(token), "{ready}");
    let done = call(&data, token, json!([]), scene.clone());
    assert_eq!(done["kind"], "done", "{done}");
    let after = call(&data, token, json!([]), scene);
    assert_eq!(after["reason"], "stale", "{after}");
}

/// The pack's fullness is the posted `inv_size`: a page that did not post
/// it is a wait, and the client's 28 is never invented for it.
#[test]
fn a_missing_inv_size_waits_and_never_invents_the_default_pack() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);
    // Twenty-eight occupied rows and still no posted slot count.
    let full_looking: Vec<Value> = (0..28)
        .map(|slot| inv(1200 + slot, "Rune scimitar", 1))
        .collect();
    for size in [json!(null), json!("28"), json!(28.5)] {
        let waiting = call(
            &data,
            token,
            json!([]),
            pages(loot.clone(), json!(full_looking), size, json!(-1)),
        );
        assert_eq!(waiting["kind"], "wait", "{waiting}");
        assert_eq!(waiting["token"], json!(token), "{waiting}");
    }
    // The posted count is honoured: one occupied row in a one-slot pack
    // is full, so the missing-food WARNING is what comes next.
    let warned = call(
        &data,
        token,
        json!([]),
        pages(
            loot,
            json!([inv(1200, "Rune scimitar", 1)]),
            json!(1),
            json!(-1),
        ),
    );
    assert_eq!(warned["kind"], "callback.log", "{warned}");
}

/// The main modal is closed only while this call's page posts it open: an
/// omitted slot is not a close, and the posted `-1` is the closed state.
#[test]
fn a_posted_main_is_closed_and_an_omitted_slot_is_not() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);

    // Omitted: the page never posted a modal, so there is nothing to close
    // and the Take is the step.
    let omitted = call(
        &data,
        token,
        json!([]),
        json!({ "here": loot_tile(), "ground": loot.clone(), "inv": [], "inv_size": 28 }),
    );
    assert_eq!(omitted["kind"], "obj", "{omitted}");
    // A malformed id is not a close either.
    let malformed = call(
        &data,
        token,
        json!([]),
        pages(loot.clone(), json!([]), json!(28), json!("6960")),
    );
    assert_eq!(malformed["kind"], "obj", "{malformed}");
    // Posted open: the close is the step, whatever the id is.
    for id in [json!(6960), json!(0), json!(-2)] {
        let closed = call(
            &data,
            token,
            json!([]),
            pages(loot.clone(), json!([]), json!(28), id.clone()),
        );
        assert_eq!(closed["kind"], "close-modal", "{id} {closed}");
    }
}

/// Freeze and yield beat the collect arm exactly as they beat the landed
/// verbs: no close, no Take, no Drop, and the token lives.
#[test]
fn freeze_and_yield_beat_the_collect() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([]),
        json!(28),
        json!(6960),
    );

    // Frozen before the entry call: the first Collecting tick has not run,
    // so the freeze is a plain wait and nothing else.
    on_pause();
    let paused = call(&data, token, json!([]), scene.clone());
    assert_eq!(paused["kind"], "wait", "{paused}");
    assert!(!bound_armed(), "a frozen call enters nothing");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, json!([]), scene.clone());
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    // The posted `hold || ours` interrupt, unfrozen: yield, still no verb.
    let mut interrupted = scene.clone();
    interrupted["hold"] = json!(true);
    let yielded = call(&data, token, json!([]), interrupted);
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    assert_eq!(yielded["token"], json!(token), "{yielded}");
    for step in [&paused, &held_clock, &yielded] {
        for absent in ["action", "name", "x", "z", "level", "message", "id"] {
            assert!(step.get(absent).is_none(), "{absent} {step}");
        }
    }
    // Thawed and unheld, the collect picks the page up where it left it.
    let closed = call(&data, token, json!([]), scene);
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    assert!(bound_armed(), "the entry arms the window once");
}

/// A collect that enters and then loses the selected pin, or the family,
/// is still the landed refusal — the seam is `none-held` alone.
#[test]
fn the_collect_seam_is_none_held_alone() {
    on_reset();
    let data = selected();
    let token = opened(&data, SEXTANT_CASKET);
    let scene = pages(
        json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
        json!([]),
        json!(28),
        json!(-1),
    );
    // The first Collecting call, so the phase and its window are live.
    assert_eq!(
        call(&data, token, json!([]), scene)["kind"],
        "obj",
        "the token is Collecting"
    );
    let gone = dispatch(None, &payload("next", Some(token), json!([]), json!({})));
    assert_eq!(gone["kind"], "aborted", "{gone}");
    assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
    assert!(!bound_armed(), "the refusal clears the window");
    let after = call(&data, token, json!([]), json!({}));
    assert_eq!(after["reason"], "stale", "{after}");
}

/// The whole collect arm's outcome set: the landed verbs, the wait inside
/// the reward window, and then the machine's own completion — the exact
/// `'clue solved'` status, the `grind-ready` handback and the `done` the
/// token dies on. Nothing else in the envelope is reachable from this arm.
#[test]
fn the_collect_arm_outcome_set_is_the_verbs_and_the_completion() {
    on_reset();
    let data = selected();
    let token = opened(&data, EASY_CASKET);
    let steps = vec![
        call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(6960)),
        ),
        call(
            &data,
            token,
            json!([]),
            pages(
                json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
                json!([]),
                json!(28),
                json!(-1),
            ),
        ),
        call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        ),
        call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        ),
    ];
    for step in &steps {
        let text = step.to_string();
        for forbidden in [
            "abandon",
            "supplies-needed",
            "dead",
            "guardian-lost",
            "grind-ready",
            "ownsEquipment",
            "trail complete",
            "the trail is complete",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
        assert!(step["status"].is_null(), "{step}");
        assert!(
            matches!(
                step["kind"].as_str().unwrap_or(""),
                "close-modal" | "obj" | "held" | "callback.log" | "callback.enabled" | "wait"
            ),
            "{step}"
        );
    }
    assert_eq!(
        steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("close-modal"),
            json!("obj"),
            json!("callback.log"),
            json!("wait"),
        ],
        "{steps:?}"
    );

    // Past the reward window the arm finishes, and the finish is the three
    // completion steps above and nothing else.
    force_bound();
    let empty = pages(json!([]), json!([]), json!(28), json!(-1));
    let solved = call(&data, token, json!([]), empty.clone());
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    assert_eq!(solved["token"], json!(token), "{solved}");
    let ready = call(&data, token, json!([]), empty.clone());
    assert_eq!(ready["kind"], "grind-ready", "{ready}");
    assert_eq!(ready["token"], json!(token), "{ready}");
    let done = call(&data, token, json!([]), empty.clone());
    assert_eq!(done["kind"], "done", "{done}");
    let after = call(&data, token, json!([]), empty);
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

/// The dig membership is the selected-param classify, not the frozen
/// `type`: a decodable `trail_coord` on a row with no `trail_loc`, no
/// `trail_guardian` and an `access` that is not `"constrained"`. The
/// `trail_sextant` param is not read here, so the forty rows are the twenty
/// medium sextant rows plus the coord-bearing easy maps, the vague and the
/// riddle-with-coord rows, on both pins — with no swallow of a search row
/// and no leak of a guarded, packed, coord-less or casket row.
#[test]
fn the_unguarded_dig_membership_is_the_selected_param_set() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).expect("selected data");
        let facts = data.trails().expect("trails");
        // The pinned decodes: the medium sextant 2801's selected token is
        // (3160, 3251, 0), and the easy map 2713's is (3177, 3360, 0).
        assert_eq!(
            dig_tile(row(&data, UNGUARDED)),
            Some(Tile {
                x: 3160,
                z: 3251,
                level: 0
            }),
            "{revision:?}"
        );
        assert_eq!(
            dig_tile(row(&data, MAP)),
            Some(Tile {
                x: 3177,
                z: 3360,
                level: 0
            }),
            "{revision:?}"
        );
        let members: Vec<i32> = facts
            .rows
            .iter()
            .filter(|row| dig_tile(row).is_some())
            .map(|row| row.id)
            .collect();
        assert_eq!(
            members,
            vec![
                2713, 2716, 2719, 3510, 3516, 3518, 2827, 2801, 2803, 2805, 2807, 2809, 2811, 2813,
                2815, 2817, 2819, 2821, 2823, 2825, 3582, 3584, 3586, 3588, 3590, 3592, 3594, 3596,
                3599, 3602, 2774, 2776, 2780, 2783, 2786, 2788, 2790, 3520, 3522, 3580,
            ],
            "{revision:?}"
        );
        assert_eq!(members.len(), 40, "{revision:?}");
        // Half the forty pin the sextant and half do not: the param is not
        // the membership, and neither is `trail_casket`.
        let sextant = members
            .iter()
            .filter(|id| {
                row(&data, **id)
                    .params
                    .iter()
                    .any(|param| param.key == "trail_sextant" && param.value == "yes")
            })
            .count();
        assert_eq!(sextant, 20, "{revision:?}");
        // No swallow: the 58 search rows are the other classify.
        let searchable = facts
            .rows
            .iter()
            .filter(|row| search_tile(row).is_some())
            .collect::<Vec<_>>();
        assert_eq!(searchable.len(), 58, "{revision:?}");
        for row in &searchable {
            assert_eq!(dig_tile(row), None, "{revision:?} {}", row.alias);
        }
        assert_eq!(search_tile(row(&data, UNGUARDED)), None, "{revision:?}");
        // No leak: the guarded rows are the same sextant shape plus a
        // guardian, the constrained 3554 clue carries a decodable coord
        // and its own casket, the desc-only 2831 has no coord at all, and
        // the paramless 2722 and every casket stay out.
        for id in [GUARDED, CLUE, RIDDLE, MAP_EMPTY, CASKET, SEXTANT_CASKET] {
            assert_eq!(dig_tile(row(&data, id)), None, "{revision:?} {id}");
        }
        // The guarded 30 are the sextant rows that carry a guardian: the
        // sextant param alone is never the membership.
        let guarded = facts
            .rows
            .iter()
            .filter(|row| row.params.iter().any(|param| param.key == "trail_guardian"))
            .count();
        assert_eq!(guarded, 30, "{revision:?}");
    }
    // Synthetic rows: the coord alone is the membership the sextant pin
    // used to gate, either sextant value rides along, a loc param of any
    // value is not this membership, and an off-contract token is never
    // rounded into an invented coordinate.
    let sextant = || param("trail_sextant", "yes");
    let coord = || param("trail_coord", "0_49_50_24_51");
    let hit = Some(Tile {
        x: 3160,
        z: 3251,
        level: 0,
    });
    assert_eq!(dig_tile(&member(vec![coord()], None)), hit);
    assert_eq!(dig_tile(&member(vec![sextant(), coord()], None)), hit);
    assert_eq!(
        dig_tile(&member(vec![param("trail_sextant", "no"), coord()], None)),
        hit
    );
    assert_eq!(
        dig_tile(&member(
            vec![coord(), param("trail_casket", "trail_clue_test_casket")],
            None
        )),
        hit
    );
    assert_eq!(dig_tile(&member(vec![sextant()], None)), None);
    for blocked in [
        vec![param("trail_loc", "^true"), coord()],
        vec![param("trail_loc", "^false"), sextant(), coord()],
        vec![param("trail_guardian", "trail_hard"), coord()],
        vec![param("trail_guardian", "trail_hard"), sextant(), coord()],
        vec![sextant(), param("trail_coord", "0_49_50_24")],
    ] {
        assert_eq!(dig_tile(&member(blocked, None)), None);
    }
    // `access` is read as the one constrained bound it is: any other value
    // — and a row that was posted with none — is outside the refusal.
    assert_eq!(dig_tile(&member(vec![coord()], Some("constrained"))), None);
    assert_eq!(dig_tile(&member(vec![coord()], Some("open"))), hit);
}

/// The Dig identity is the selected-verified display the host resolves by
/// first name match — not an item id, not the membership alias.
#[test]
fn the_dig_identity_is_the_selected_spade_display() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).expect("selected data");
        let spade = data.item_by_id(SPADE_ITEM).expect("spade item");
        assert_eq!(spade.name.as_deref(), Some(SPADE_NAME), "{revision:?}");
    }
}

/// `Steady` on an unguarded-dig row: walk to the decoded tile until the
/// posted `here` arrives, then Dig with the held Spade — repeating while
/// that same clue stays held, and never collecting.
#[test]
fn an_unguarded_dig_row_walks_to_the_decoded_tile_and_then_digs_the_spade() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    // Not arrived: the walk is the decoded tile, and it repeats. Another
    // level and two tiles along one axis are both not arrival.
    for far in [
        here(3100, 3300, 0),
        here(3160, 3251, 1),
        here(3162, 3251, 0),
        here(3160, 3253, 0),
    ] {
        let walk = call(&data, token, page.clone(), dig_scene(far, false));
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], 3160, "{walk}");
        assert_eq!(walk["z"], 3251, "{walk}");
        assert_eq!(walk["level"], 0, "{walk}");
        assert_eq!(token_of(&walk), token, "{walk}");
    }
    // No posted `here` at all: no arrival claim and no blind walk, even
    // with the Spade posted beside the held trio.
    let no_tile = call(&data, token, page.clone(), json!({ "inv": dig_inv(true) }));
    assert_eq!(no_tile["kind"], "wait", "{no_tile}");
    // Arrived: the held Spade is the Dig, and it repeats while this same
    // clue id stays held.
    for here_tile in [here(3160, 3251, 0), here(3161, 3250, 0)] {
        let dig = call(&data, token, page.clone(), dig_scene(here_tile, true));
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], "Spade", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
        assert_eq!(token_of(&dig), token, "{dig}");
        // The verb is the item identity alone: no bound row id and no tile.
        for absent in ["id", "x", "z", "level", "message"] {
            assert!(dig.get(absent).is_none(), "{absent} {dig}");
        }
    }
    // The clue left: the landed `none-held` abort, and the collect seam is
    // never armed by a Dig — Collecting stays the casket Open's alone.
    let gone = call(&data, token, json!([]), json!({}));
    assert_eq!(gone["kind"], "aborted", "{gone}");
    assert_eq!(gone["reason"], "none-held", "{gone}");
    assert!(!bound_armed(), "a dig row never arms the reward window");
    let after = call(&data, token, json!([]), json!({}));
    assert_eq!(after["reason"], "stale", "{after}");
}

/// Arrived without the Spade on this call's posted pack page is the named
/// `supplies-needed` wait-class: never `abandon`, never a public
/// `no-spade`, and never a fetch. The token stays live for the page that
/// posts it.
#[test]
fn an_unguarded_dig_row_without_the_spade_is_supplies_needed_and_live() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let arrived = here(3160, 3251, 0);
    for pack in [
        // No page at all, an empty page, another item, a zero count, a
        // nameless row and a name that is not the display. Every one of
        // them holds the trio the acquire chain already cleared: the
        // Spade is the only thing missing.
        trio_pack(&[]),
        trio_pack(&[inv(385, "Shark", 5)]),
        trio_pack(&[inv(SPADE_ITEM, SPADE_NAME, 0)]),
        trio_pack(&[inv(SPADE_ITEM, "", 1)]),
        trio_pack(&[inv(SPADE_ITEM, "Spade cert", 1)]),
        trio_pack(&[json!({ "id": SPADE_ITEM, "count": 1 })]),
    ] {
        let idle = call(
            &data,
            token,
            page.clone(),
            json!({ "here": arrived.clone(), "inv": pack }),
        );
        assert_eq!(idle["kind"], "supplies-needed", "{idle}");
        assert_eq!(token_of(&idle), token, "the wait-class is live: {idle}");
        for absent in ["action", "name", "x", "z", "level", "message", "id"] {
            assert!(idle.get(absent).is_none(), "{absent} {idle}");
        }
        let text = idle.to_string();
        for forbidden in ["no-spade", "abandon", "done", "clue solved", "dead"] {
            assert!(!text.contains(forbidden), "{idle}");
        }
    }
    // A malformed `here` is a plain wait, and the whole `inv` slot may be
    // omitted: neither is a verb.
    let malformed = call(
        &data,
        token,
        page.clone(),
        json!({ "here": json!({ "x": 3160 }), "inv": dig_inv(true) }),
    );
    assert_eq!(malformed["kind"], "wait", "{malformed}");
    // The page that carries it: the Dig, and the token was live all along.
    // The identity is the posted display name, matched the way the landed
    // collect matches one, so the cert id and the case both still Dig.
    let dig = call(
        &data,
        token,
        page,
        json!({
            "here": here(3160, 3251, 0),
            "inv": trio_pack(&[inv(385, "Shark", 5), inv(953, "sPaDe", 1)]),
        }),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["name"], "Spade", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
}

/// The rows the dig classify leaves out stay identified then idle even over
/// a scene the dig arm would walk and Dig from — `here` on the row's own
/// selected tile with the Spade posted: the paramless 2722, the five
/// matcher-keepers the key family publishes no packed type for, and the
/// desc-only riddles no key row names. 2831 is no longer one of them: the
/// key-keeper hunt walks to its own published spawn from that same tile.
/// The packed constrained 3554 clue is refused instead of idled, and the
/// guarded row is no longer one of them either: its own encounter walks and
/// Digs from this same scene.
#[test]
fn rows_outside_the_dig_classify_stay_idle_over_a_walkable_dig_scene() {
    on_reset();
    let data = selected();
    for id in std::iter::once(MAP_EMPTY).chain(MATCHER_KEEPERS) {
        let here_tile = row(&data, id)
            .params
            .iter()
            .find(|param| param.key == "trail_coord")
            .and_then(|param| decode_trail_coord(&param.value))
            .unwrap_or(Tile {
                x: 3160,
                z: 3251,
                level: 0,
            });
        let scene = json!({
            "here": here(here_tile.x, here_tile.z, here_tile.level),
            "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
        });
        let page = json!([[id, 1]]);
        let token = steady(&data, id);
        for _ in 0..2 {
            let idle = call(&data, token, page.clone(), scene.clone());
            assert_eq!(idle["kind"], "wait", "{id} {idle}");
            assert_eq!(token_of(&idle), token, "{id} {idle}");
            assert!(idle.get("x").is_none(), "{id} {idle}");
            assert!(idle.get("action").is_none(), "{id} {idle}");
        }
    }

    // The constrained clue is not idled over that scene: it is refused,
    // with no walk, no Dig and no token left.
    let scene = json!({
        "here": here(3160, 3251, 0),
        "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
    });
    let page = json!([[CLUE, 1]]);
    let token = token_of(&begin(&data, json!([[MAP_EMPTY, 1]])));
    let refused = call(&data, token, page.clone(), scene.clone());
    assert_eq!(refused["kind"], "aborted", "{refused}");
    assert_eq!(refused["reason"], "constrained", "{refused}");
    let after = call(&data, token, page, scene);
    assert_eq!(after["reason"], "stale", "{after}");
}

/// `Steady` on a coord-only map row: the widened unguarded dig is the same
/// walk-then-Dig the medium sextant rows keep, so `trail_clue_easy_map001`
/// walks to its own decoded (3177, 3360, 0) and Digs the Spade with no
/// `trail_sextant` on the row at all.
#[test]
fn a_coord_only_map_row_walks_to_its_decoded_tile_and_digs_the_spade() {
    on_reset();
    let data = selected();
    let page = json!([[MAP, 1]]);
    let token = steady(&data, MAP);
    // Not arrived: the walk is the map's own decoded tile, not the
    // sextant sibling's.
    let walk = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(3100, 3300, 0) }),
    );
    assert_eq!(walk["kind"], "walk", "{walk}");
    assert_eq!(walk["x"], 3177, "{walk}");
    assert_eq!(walk["z"], 3360, "{walk}");
    assert_eq!(walk["level"], 0, "{walk}");
    assert_eq!(token_of(&walk), token, "{walk}");
    // Arrived without the Spade: the named wait-class, never a verb.
    let bare = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(3177, 3360, 0) }),
    );
    assert_eq!(bare["kind"], "supplies-needed", "{bare}");
    assert_eq!(token_of(&bare), token, "{bare}");
    // Arrived with it: the same held Spade Dig the sibling dispatches.
    let dig = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(3177, 3360, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["name"], "Spade", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
    assert_eq!(token_of(&dig), token, "{dig}");
    // The clue left: still the landed `none-held` abort, and a Dig never
    // arms the collect seam.
    let gone = call(&data, token, json!([]), json!({}));
    assert_eq!(gone["kind"], "aborted", "{gone}");
    assert_eq!(gone["reason"], "none-held", "{gone}");
    assert!(!bound_armed(), "a dig row never arms the reward window");
}

/// Freeze and yield beat the dig arm the way they beat the landed verbs: no
/// walk and no held ride along, and the token lives for the thaw.
#[test]
fn freeze_and_yield_beat_the_dig() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let scene = dig_scene(here(3160, 3251, 0), true);
    on_pause();
    let paused = call(&data, token, page.clone(), scene.clone());
    assert_eq!(paused["kind"], "wait", "{paused}");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, page.clone(), scene.clone());
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    let mut yield_scene = scene.clone();
    yield_scene["hold"] = json!(true);
    let yielded = call(&data, token, page.clone(), yield_scene);
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    assert_eq!(token_of(&yielded), token, "{yielded}");
    for step in [&paused, &held_clock, &yielded] {
        assert!(step.get("name").is_none(), "{step}");
        assert!(step.get("action").is_none(), "{step}");
        assert!(step.get("x").is_none(), "{step}");
        assert!(step.get("z").is_none(), "{step}");
    }
    // Thawed and unheld, the Dig is still there.
    let dig = call(&data, token, page, scene);
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
}

/// Identify is casket-first, so the casket a dig produced is the step: the
/// dig row re-arms the gate for it, the Open follows, and the clue's return
/// re-arms again — never a second Dig under the casket.
#[test]
fn a_produced_casket_opens_and_the_dig_row_re_arms() {
    on_reset();
    let data = selected();
    let clue_page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let scene = dig_scene(here(3160, 3251, 0), true);
    let dig = call(&data, token, clue_page.clone(), scene.clone());
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");

    // The casket the dig produced is held beside its own clue.
    let casket = casket_of(&data, UNGUARDED);
    let casket_alias = row(&data, casket).alias.clone();
    let both = json!([[UNGUARDED, 1], [casket, 1]]);
    let re_armed = call(&data, token, both.clone(), scene.clone());
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    assert_eq!(token_of(&re_armed), token, "{re_armed}");
    let logged = call(&data, token, both.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let message = logged["message"].as_str().unwrap_or("");
    assert!(message.contains(&casket_alias), "{logged}");
    assert!(!message.contains(" [2801]"), "{logged}");
    assert_eq!(
        call(&data, token, both.clone(), scene.clone())["kind"],
        "callback.setStatus"
    );
    let open = call(&data, token, both.clone(), scene.clone());
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["name"], "Casket", "{open}");
    assert_eq!(open["action"], "Open", "{open}");

    // And the clue alone again: the dig row re-arms rather than Digging
    // under whatever the casket left behind.
    let back = call(&data, token, clue_page, scene);
    assert_eq!(back["kind"], "callback.enabled", "{back}");
}

/// The dig row emits walk, the held Dig, wait, `supplies-needed` or yield
/// only — never a completion kind, and never a `status` field of its own.
#[test]
fn the_dig_row_emits_only_walk_held_wait_supplies_needed_and_yield() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let steps = vec![
        call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3100, 3300, 0) }),
        ),
        call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3160, 3251, 0), "inv": dig_inv(false) }),
        ),
        call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3160, 3251, 0), "inv": dig_inv(true) }),
        ),
        call(
            &data,
            token,
            page,
            json!({
                "here": here(3160, 3251, 0),
                "inv": dig_inv(true),
                "hold": true,
            }),
        ),
    ];
    for step in &steps {
        let text = step.to_string();
        for forbidden in [
            "clue solved",
            "done",
            "abandon",
            "dead",
            "guardian-lost",
            "grind-ready",
            "no-spade",
            "ownsEquipment",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
        assert!(step["status"].is_null(), "{step}");
        assert!(
            matches!(
                step["kind"].as_str().unwrap_or(""),
                "walk" | "held" | "wait" | "yield" | "supplies-needed"
            ),
            "{step}"
        );
    }
    assert_eq!(
        steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("walk"),
            json!("supplies-needed"),
            json!("held"),
            json!("yield")
        ],
        "{steps:?}"
    );
}

/// The guarded membership is the selected param set — a decodable coord,
/// no loc pin, `trail_sextant=yes`, a `trail_guardian` and an access that
/// is not constrained — thirty rows on both pins, the exemplar decode, and
/// no swallow of its unguarded sibling. The family alias is a posted-name
/// filter and never a row field.
#[test]
fn the_guarded_dig_membership_is_the_selected_param_set() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).expect("selected data");
        let facts = data.trails().expect("trails");
        // The pinned decode: 2723's selected token is (3058, 3884, 0).
        assert_eq!(
            guarded_tile(row(&data, GUARDED)),
            Some(Tile {
                x: 3058,
                z: 3884,
                level: 0
            }),
            "{revision:?}"
        );
        let members: Vec<i32> = facts
            .rows
            .iter()
            .filter(|row| guarded_tile(row).is_some())
            .map(|row| row.id)
            .collect();
        assert_eq!(members.len(), 30, "{revision:?} {members:?}");
        for id in &members {
            let row = row(&data, *id);
            assert!(!guardian_names(row).is_empty(), "{revision:?} {id}");
            // No swallow: the search and the unguarded classify are other
            // memberships, and neither is this one.
            assert!(dig_tile(row).is_none(), "{revision:?} {id}");
            assert!(search_tile(row).is_none(), "{revision:?} {id}");
        }
        // Every guarded row carries one of the two family aliases the cap
        // documents, and each maps to its own posted-name filter.
        let mut families = facts
            .rows
            .iter()
            .filter(|row| guarded_tile(row).is_some())
            .filter_map(|row| {
                row.params
                    .iter()
                    .find(|param| param.key == "trail_guardian")
                    .map(|param| param.value.clone())
            })
            .collect::<Vec<_>>();
        families.sort();
        families.dedup();
        assert_eq!(families, vec!["trail_hard", "trail_hard2"], "{revision:?}");
        assert_eq!(
            guardian_names(row(&data, GUARDED)).to_vec(),
            vec!["Zamorak Wizard"],
            "{revision:?}"
        );
        let hard2 = facts
            .rows
            .iter()
            .find(|row| {
                row.params
                    .iter()
                    .any(|param| param.key == "trail_guardian" && param.value == "trail_hard2")
            })
            .expect("trail_hard2 row");
        assert_eq!(
            guardian_names(hard2).to_vec(),
            vec!["Saradomin Wizard"],
            "{revision:?}"
        );
        // No leak: the unguarded sibling, the search rows, the constrained
        // clue, the coord-only map, the riddle, the paramless row and the
        // caskets are not this membership and carry no filter.
        for id in [
            UNGUARDED,
            SEARCH,
            CLUE,
            MAP,
            RIDDLE,
            MAP_EMPTY,
            CASKET,
            SEXTANT_CASKET,
        ] {
            assert_eq!(guarded_tile(row(&data, id)), None, "{revision:?} {id}");
            assert!(
                guardian_names(row(&data, id)).is_empty(),
                "{revision:?} {id}"
            );
        }
    }
    // Synthetic rows: each half of the pin on its own idles, a loc pin of
    // any value is never this membership, and an off-contract token is
    // never rounded into an invented coordinate.
    let sextant = || param("trail_sextant", "yes");
    let coord = || param("trail_coord", "0_47_60_50_44");
    let guardian = || param("trail_guardian", "trail_hard");
    let hit = Some(Tile {
        x: 3058,
        z: 3884,
        level: 0,
    });
    assert_eq!(guarded_tile(&member(vec![coord(), guardian()], None)), None);
    assert_eq!(
        guarded_tile(&member(vec![sextant(), guardian()], None)),
        None
    );
    assert_eq!(guarded_tile(&member(vec![sextant(), coord()], None)), None);
    assert_eq!(
        guarded_tile(&member(vec![sextant(), coord(), guardian()], None)),
        hit
    );
    for blocked in [
        vec![param("trail_loc", "^true"), sextant(), coord(), guardian()],
        vec![param("trail_loc", "^false"), sextant(), coord(), guardian()],
        vec![param("trail_sextant", "no"), coord(), guardian()],
        vec![sextant(), param("trail_coord", "0_47_60_50"), guardian()],
    ] {
        assert_eq!(guarded_tile(&member(blocked, None)), None);
    }
    assert_eq!(
        guarded_tile(&member(
            vec![sextant(), coord(), guardian()],
            Some("constrained")
        )),
        None
    );
    assert_eq!(
        guarded_tile(&member(vec![sextant(), coord(), guardian()], Some("open"))),
        hit
    );
    // The alias is a filter and not a name: an unpinned family has none,
    // and reading one never writes the wizard onto the row.
    assert_eq!(
        guardian_names(&member(vec![guardian()], None)).to_vec(),
        vec!["Zamorak Wizard"]
    );
    assert!(guardian_names(&member(vec![param("trail_guardian", "trail_hard9")], None)).is_empty());
    assert!(guardian_names(&member(vec![param("trail_guardian", "")], None)).is_empty());
    let row = member(vec![sextant(), coord(), guardian()], None);
    let before = row.clone();
    assert_eq!(guardian_names(&row).to_vec(), vec!["Zamorak Wizard"]);
    assert_eq!(row.alias, before.alias);
    assert_eq!(row.params.len(), before.params.len());
}

/// The selected `cap.prayer` row the fight raises: `Protect from Magic`,
/// the component the generic `if-button` carries and the overlay varp the
/// marshalled `varp95` is read from, on both pins — and never a copy of a
/// prayer table in this file.
#[test]
fn the_selected_protect_from_magic_row_is_the_click_and_the_overlay() {
    assert_eq!(PROTECT_FROM_MAGIC, "Protect from Magic");
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).expect("selected data");
        let row = api::prayer::lookup(&data, PROTECT_FROM_MAGIC).expect("prayer row");
        assert_eq!(row.button_com, 5621, "{revision:?}");
        assert_eq!(row.varp, 95, "{revision:?}");
    }
}

/// `Steady` on a guarded row: walk to the decoded tile, then Dig with the
/// held Spade — the landed sibling arrival and the verb that spawns the
/// wizard. The fight is what follows, never a second Dig off the same
/// scene.
#[test]
fn a_guarded_row_walks_then_digs_the_spade_and_that_dig_is_the_spawn() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = steady(&data, GUARDED);
    let tile = guarded_tile_of(&data);
    // Not arrived: the walk is the decoded tile, and it repeats.
    for far in [
        here(3100, 3300, 0),
        here(3058, 3884, 1),
        here(3060, 3884, 0),
    ] {
        let walk = call(&data, token, page.clone(), dig_scene(far, true));
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], 3058, "{walk}");
        assert_eq!(walk["z"], 3884, "{walk}");
        assert_eq!(walk["level"], 0, "{walk}");
        assert_eq!(token_of(&walk), token, "{walk}");
    }
    // No posted `here` at all waits; arrived without the Spade is the
    // named wait-class. Neither starts an encounter.
    let no_tile = call(
        &data,
        token,
        page.clone(),
        json!({ "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
    );
    assert_eq!(no_tile["kind"], "wait", "{no_tile}");
    let no_spade = call(
        &data,
        token,
        page.clone(),
        dig_scene(here(tile.x, tile.z, tile.level), false),
    );
    assert_eq!(no_spade["kind"], "supplies-needed", "{no_spade}");
    assert_eq!(token_of(&no_spade), token, "{no_spade}");
    // Arrived with the Spade: the held Dig, which is the spawn.
    let dig = call(
        &data,
        token,
        page.clone(),
        dig_scene(here(tile.x, tile.z, tile.level), true),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["name"], SPADE_NAME, "{dig}");
    assert_eq!(dig["action"], DIG, "{dig}");
    // The encounter is live: the same row and the same arrived scene are
    // the fight now, so a page with no wizard waits instead of Digging
    // again.
    let idle = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(idle["kind"], "wait", "{idle}");
    assert!(idle.get("action").is_none(), "{idle}");
    // A frozen call in the spawn stage burned nothing: the encounter is
    // this token's own and the spawn Dig was the last verb.
    assert_eq!(token_of(&idle), token, "{idle}");
}

/// The spawn wait: a posted npc page with no wizard of the row family is a
/// wait whatever else it carries. The nearest anything is never Attacked.
#[test]
fn the_spawn_wait_never_attacks_the_nearest_anything() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let mut unposted = fight_scene(json!([]), json!({}));
    unposted.as_object_mut().expect("object").remove("npcs");
    for scene in [
        // No npc page at all, and an empty one.
        unposted,
        fight_scene(json!([]), json!({})),
        // Another name with the Attack action, closest of all.
        fight_scene(json!([npc(7, "Guard", 1, 10, 10)]), json!({})),
        // The right name without the Attack action.
        fight_scene(
            json!([{
                "index": 7, "id": 107, "name": WIZARD, "distance": 3,
                "health": 10, "max_health": 10, "actions": ["Talk-to"],
                "target_kind": 0, "target_index": -1,
            }]),
            json!({}),
        ),
        // The right name one tile outside the frozen radius.
        fight_scene(
            json!([npc(7, WIZARD, GUARDIAN_RADIUS + 1, 10, 10)]),
            json!({}),
        ),
        // The right name with no posted distance, no posted index, and no
        // posted name at all.
        fight_scene(
            json!([{
                "index": 7, "name": WIZARD, "health": 10, "max_health": 10,
                "actions": [ATTACK],
            }]),
            json!({}),
        ),
        fight_scene(
            json!([{
                "name": WIZARD, "distance": 3, "health": 10, "max_health": 10,
                "actions": [ATTACK],
            }]),
            json!({}),
        ),
        fight_scene(json!([npc(7, "", 3, 10, 10)]), json!({})),
        // A name that only shares the prefix.
        fight_scene(
            json!([npc(7, "Zamorak Wizard (hard)", 3, 10, 10)]),
            json!({}),
        ),
    ] {
        let idle = call(&data, token, page.clone(), scene.clone());
        assert_eq!(idle["kind"], "wait", "{scene} {idle}");
        assert!(idle.get("action").is_none(), "{scene} {idle}");
        assert!(idle.get("index").is_none(), "{scene} {idle}");
    }
    // Inside the frozen radius the same row is the Attack.
    let attack = call(
        &data,
        token,
        page,
        fight_scene(json!([npc(7, WIZARD, GUARDIAN_RADIUS, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    assert_eq!(attack["name"], WIZARD, "{attack}");
    assert_eq!(attack["action"], ATTACK, "{attack}");
    assert_eq!(attack["index"], 7, "{attack}");
}

/// The Attack carries the posted name — the filter folds case and never
/// substitutes the frozen spelling — and no row id, tile or health rides
/// along: the posted index is the identity the host matches.
#[test]
fn the_attack_carries_the_posted_name_of_the_posted_row() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        page,
        fight_scene(json!([npc(4, "zamorak wizard", 3, 9, 9)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    assert_eq!(attack["name"], "zamorak wizard", "{attack}");
    assert_eq!(attack["action"], ATTACK, "{attack}");
    assert_eq!(attack["index"], 4, "{attack}");
    for absent in ["id", "x", "z", "level", "health", "max_health", "message"] {
        assert!(attack.get(absent).is_none(), "{absent} {attack}");
    }
}

/// Among matching wizards the row whose own posted target is the player
/// wins; with none of them on the player the nearest posted distance wins,
/// then posted order. A page that posted no local-player slot never reads
/// a `targetsMe`.
#[test]
fn the_attack_prefers_the_posted_target_of_the_player() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let two = json!([
        targeting(npc(3, WIZARD, 9, 30, 30), 0),
        npc(5, WIZARD, 2, 30, 30),
    ]);
    let token = spawned(&data);
    let preferred = call(
        &data,
        token,
        page.clone(),
        fight_scene(two.clone(), json!({})),
    );
    assert_eq!(preferred["kind"], "npc", "{preferred}");
    assert_eq!(preferred["index"], 3, "{preferred}");
    // Neither on the player: the nearer posted distance.
    let token = spawned(&data);
    let nearest = call(
        &data,
        token,
        page.clone(),
        fight_scene(
            json!([npc(6, WIZARD, 7, 30, 30), npc(8, WIZARD, 4, 30, 30)]),
            json!({}),
        ),
    );
    assert_eq!(nearest["kind"], "npc", "{nearest}");
    assert_eq!(nearest["index"], 8, "{nearest}");
    // Neither on the player and both at the same distance: posted order.
    let token = spawned(&data);
    let first = call(
        &data,
        token,
        page.clone(),
        fight_scene(
            json!([npc(2, WIZARD, 4, 30, 30), npc(9, WIZARD, 4, 30, 30)]),
            json!({}),
        ),
    );
    assert_eq!(first["kind"], "npc", "{first}");
    assert_eq!(first["index"], 2, "{first}");
    // No posted slot: the zero slot is not invented for the preference.
    let token = spawned(&data);
    let no_slot = call(
        &data,
        token,
        page,
        fight_scene(two, json!({ "self_slot": null })),
    );
    assert_eq!(no_slot["kind"], "npc", "{no_slot}");
    assert_eq!(no_slot["index"], 5, "{no_slot}");
}

/// The Protect from Magic overlay gates the Attack: an overlay posted off
/// enqueues the generic `if-button` with the selected component id and
/// Attacks nothing, an overlay posted on skips the click and Attacks, and
/// an overlay that was not posted is not a proven on — the click still
/// goes out, no Attack does, and no toggle is waited out.
#[test]
fn the_protect_from_magic_overlay_gates_the_attack() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);
    let token = spawned(&data);
    let off = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 0 })),
    );
    assert_eq!(off["kind"], "if-button", "{off}");
    assert_eq!(off["component_id"], 5621, "{off}");
    for absent in ["name", "action", "index", "message"] {
        assert!(off.get(absent).is_none(), "{absent} {off}");
    }
    // Still off: the same click, still no Attack.
    let again = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 0 })),
    );
    assert_eq!(again["kind"], "if-button", "{again}");
    assert_eq!(again["component_id"], 5621, "{again}");
    // Unobserved: not a proven on, so the click goes out and never an
    // Attack — and no timeout token is invented for it.
    let mut unobserved = fight_scene(wizard.clone(), json!({}));
    unobserved.as_object_mut().expect("object").remove("varp95");
    let unknown = call(&data, token, page.clone(), unobserved);
    assert_eq!(unknown["kind"], "if-button", "{unknown}");
    assert_eq!(unknown["component_id"], 5621, "{unknown}");
    // A posted value that is not the on value is not the on value.
    let other = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 2 })),
    );
    assert_eq!(other["kind"], "if-button", "{other}");
    // Posted on: no click, and the Attack goes out.
    let on = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 1 })),
    );
    assert_eq!(on["kind"], "npc", "{on}");
    assert_eq!(on["index"], 7, "{on}");
    assert!(on.get("component_id").is_none(), "{on}");
    // The gate is read on every fight call rather than latched: an overlay
    // that reads off again mid-fight is clicked again, and only a posted-on
    // overlay leaves the fight to its kill wait.
    let dropped = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 0 })),
    );
    assert_eq!(dropped["kind"], "if-button", "{dropped}");
    assert_eq!(dropped["component_id"], 5621, "{dropped}");
    let settled = call(&data, token, page, fight_scene(wizard, json!({})));
    assert_eq!(settled["kind"], "wait", "{settled}");
}

/// The kill is the owned wizard: zero health beside a posted maximum on the
/// page that still shows this token's fight on it, or the owned index
/// leaving the page inside the frozen grace. Only that kill walks back to
/// the decoded tile and Digs again.
#[test]
fn the_owned_kill_walks_back_and_digs_again() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let tile = guarded_tile_of(&data);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    // Posted and alive: the fight waits, and the Attack is not re-issued.
    let alive = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 9, 10)]), json!({})),
    );
    assert_eq!(alive["kind"], "wait", "{alive}");
    assert!(alive.get("action").is_none(), "{alive}");
    // Killed by health, off the tile: the walk back to the decoded pin.
    let walked_back = call(
        &data,
        token,
        page.clone(),
        fight_scene(
            json!([targeting(npc(7, WIZARD, 3, 0, 10), 0)]),
            json!({ "here": here(3100, 3300, 0) }),
        ),
    );
    assert_eq!(walked_back["kind"], "walk", "{walked_back}");
    assert_eq!(walked_back["x"], tile.x, "{walked_back}");
    // Arrived: the post-kill Dig, repeating while the clue stays held.
    for _ in 0..2 {
        let redig = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(redig["kind"], "held", "{redig}");
        assert_eq!(redig["name"], SPADE_NAME, "{redig}");
        assert_eq!(redig["action"], DIG, "{redig}");
    }
    // The same grace path: the owned index leaving the page.
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    let gone = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(gone["kind"], "held", "{gone}");
    assert_eq!(gone["action"], DIG, "{gone}");
}

/// A fight that has not settled waits: the owned wizard posted and alive,
/// and a death beside another player that is not this token's fight. The
/// owned index gone only after the frozen grace was spent is the other
/// thing entirely: the encounter is lost, so the kind is `guardian-lost`
/// and the token dies with it — never a redig, never `'clue solved'`.
#[test]
fn an_unsettled_fight_waits_and_a_lost_wizard_ends_the_token() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    // Dead beside another player: not this token's kill, and not a loss.
    let stolen = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([targeting(npc(7, WIZARD, 3, 0, 10), 9)]), json!({})),
    );
    assert_eq!(stolen["kind"], "wait", "{stolen}");
    // Owned and still posted: the same wait.
    let alive = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(alive["kind"], "wait", "{alive}");
    for step in [&stolen, &alive] {
        let text = step.to_string();
        for forbidden in [
            "guardian-lost",
            "clue solved",
            "dead",
            "done",
            "abandon",
            "Dig",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
        assert!(step.get("action").is_none(), "{step}");
        assert_eq!(token_of(step), token, "{step}");
    }

    // Gone, and only after the grace was spent: the wizard is lost.
    age_owned_seen(KILL_GRACE_MS + 1);
    let lost = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(lost["kind"], "guardian-lost", "{lost}");
    for absent in [
        "action",
        "index",
        "component_id",
        "name",
        "message",
        "x",
        "z",
        "level",
    ] {
        assert!(lost.get(absent).is_none(), "{absent} {lost}");
    }
    assert!(!lost.to_string().contains("clue solved"), "{lost}");
    // The token died with the encounter: no redig, and no session left.
    let after = call(&data, token, page, fight_scene(json!([]), json!({})));
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

/// A disappearance this token never Attacked for is a wait, not a redig:
/// while a wizard of the row family is posted the overlay gate is the
/// click, and a page that posts none is the spawn wait — the overlay is
/// never raised for a spawn that was not posted, and the wizard coming and
/// going under it never Digs and never walks.
#[test]
fn a_wizard_that_leaves_without_an_attack_is_a_wait_not_a_redig() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let click = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 })),
    );
    assert_eq!(click["kind"], "if-button", "{click}");
    // The wizard left before any Attack: nothing of the family is posted,
    // so the click is not raised either and the fight waits.
    let empty = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({ "varp95": 0 })),
    );
    assert_eq!(empty["kind"], "wait", "{empty}");
    assert!(empty.get("component_id").is_none(), "{empty}");
    // Another wizard of the family posted and still unowned, but dead on
    // the page: the spawn is observed, so the click goes out and no
    // Attack or redig follows from a death this token never fought.
    let posted = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(9, WIZARD, 3, 0, 10)]), json!({ "varp95": 0 })),
    );
    assert_eq!(posted["kind"], "if-button", "{posted}");
    for step in [&empty, &posted] {
        let text = step.to_string();
        for forbidden in ["guardian-lost", "Dig", "walk", "done", "abandon"] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
        assert_eq!(token_of(step), token, "{step}");
    }
}

/// The spawn observation runs before the overlay: the Protect from Magic
/// click is only ever raised behind a posted wizard of the row family, and
/// the Attack only ever follows that click's on read. A page with no such
/// wizard waits with no click at all — whatever the overlay reads, and
/// whatever else it posted — and the owned wizard's own leave is read
/// before the gate too, so a kill never waits on the prayer.
#[test]
fn the_overlay_is_only_raised_behind_a_posted_spawn() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);
    let token = spawned(&data);
    let mut unobserved = fight_scene(json!([npc(7, "Guard", 1, 10, 10)]), json!({}));
    unobserved.as_object_mut().expect("object").remove("varp95");
    for scene in [
        // No spawn posted with the overlay off, on and unobserved: the
        // spawn wait, never a click.
        fight_scene(json!([]), json!({ "varp95": 0 })),
        fight_scene(json!([]), json!({ "varp95": 1 })),
        unobserved,
        // A wizard of the family the frozen radius refuses.
        fight_scene(
            json!([npc(7, WIZARD, GUARDIAN_RADIUS + 1, 10, 10)]),
            json!({ "varp95": 0 }),
        ),
    ] {
        let idle = call(&data, token, page.clone(), scene.clone());
        assert_eq!(idle["kind"], "wait", "{scene} {idle}");
        assert!(idle.get("component_id").is_none(), "{scene} {idle}");
        assert!(idle.get("action").is_none(), "{scene} {idle}");
    }
    // The spawn posted and the overlay off: the click, and still no
    // Attack.
    let click = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 0 })),
    );
    assert_eq!(click["kind"], "if-button", "{click}");
    assert_eq!(click["component_id"], 5621, "{click}");
    assert!(click.get("index").is_none(), "{click}");
    // The posted-on overlay leaves the Attack, for that posted index.
    let attack = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 1 })),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    assert_eq!(attack["index"], 7, "{attack}");
    // Owned and still posted with the overlay off again: the gate is read
    // on every fight call rather than latched, so the click goes out again
    // and the Attack is never re-issued.
    let owned = call(
        &data,
        token,
        page.clone(),
        fight_scene(wizard.clone(), json!({ "varp95": 0 })),
    );
    assert_eq!(owned["kind"], "if-button", "{owned}");
    assert!(owned.get("index").is_none(), "{owned}");
    // The owned wizard gone with nothing of the family posted: the grace
    // kill, read before the overlay and never a click.
    let gone = call(
        &data,
        token,
        page,
        fight_scene(json!([]), json!({ "varp95": 0 })),
    );
    assert_eq!(gone["kind"], "held", "{gone}");
    assert_eq!(gone["name"], SPADE_NAME, "{gone}");
    assert_eq!(gone["action"], DIG, "{gone}");
}

/// The spawn filter reads the posted tile: the posted row must be on this
/// call's posted `here` level, and a row that posted no distance is
/// measured from its own posted tile with the landed Chebyshev read — the
/// nearer of two such rows wins before posted order, and a tile outside
/// the frozen radius is refused exactly like a posted distance outside it.
#[test]
fn the_spawn_pick_reads_the_posted_level_and_the_posted_tile() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    for scene in [
        // Another level, with a posted distance and with only its own
        // tile to measure.
        fight_scene(
            json!([field(npc(7, WIZARD, 3, 10, 10), "level", json!(1))]),
            json!({}),
        ),
        fight_scene(json!([tiled(7, WIZARD, 3058, 3884, 1)]), json!({})),
        // No posted level at all is not the `here` level either, and a
        // row with no posted tile and no posted distance has no measure.
        fight_scene(
            json!([unfield(npc(7, WIZARD, 3, 10, 10), "level")]),
            json!({}),
        ),
        fight_scene(
            json!([unfield(
                unfield(unfield(npc(7, WIZARD, 3, 10, 10), "distance"), "x"),
                "z"
            )]),
            json!({}),
        ),
        // No posted `here`: no level to compare and no base to measure
        // from, whatever the row posted.
        fight_scene(
            json!([tiled(7, WIZARD, 3058, 3884, 0)]),
            json!({ "here": null }),
        ),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "here": null })),
        // The row's own tile one step outside the frozen radius.
        fight_scene(
            json!([tiled(7, WIZARD, 3058 + GUARDIAN_RADIUS + 1, 3884, 0)]),
            json!({}),
        ),
    ] {
        let idle = call(&data, token, page.clone(), scene.clone());
        assert_eq!(idle["kind"], "wait", "{scene} {idle}");
        assert!(idle.get("index").is_none(), "{scene} {idle}");
        assert!(idle.get("component_id").is_none(), "{scene} {idle}");
    }
    // Two rows measured by their own tiles: the nearer one wins before
    // posted order, the way the posted-distance read already does.
    let near = call(
        &data,
        token,
        page.clone(),
        fight_scene(
            json!([
                tiled(4, WIZARD, 3058, 3884 - 9, 0),
                tiled(6, WIZARD, 3058 + 2, 3884, 0),
            ]),
            json!({}),
        ),
    );
    assert_eq!(near["kind"], "npc", "{near}");
    assert_eq!(near["index"], 6, "{near}");
    // Exactly on the frozen radius is inside it, and the tile-measured row
    // is the Attack with no posted distance at all.
    let token = spawned(&data);
    let edge = call(
        &data,
        token,
        page,
        fight_scene(
            json!([tiled(7, WIZARD, 3058, 3884 + GUARDIAN_RADIUS, 0)]),
            json!({}),
        ),
    );
    assert_eq!(edge["kind"], "npc", "{edge}");
    assert_eq!(edge["index"], 7, "{edge}");
    assert_eq!(edge["name"], WIZARD, "{edge}");
}

/// A freeze that outlasted the remaining kill grace: the owned last-seen
/// lives at the freeze's own start and the wizard is gone by the time the
/// session thaws. The thaw reclaims the frozen gap into that stamp the way
/// the landed hunt fight shifts its own, so the disappearance is still
/// this token's kill rather than a grace the pause spent.
#[test]
fn a_freeze_across_the_kill_grace_still_ends_in_the_kill() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    assert_eq!(attack["index"], 7, "{attack}");
    // The freeze: without the reclaim the next call's own `now` is the
    // whole frozen interval ahead of the owned stamp, so the grace would
    // already read as spent and the fight would wait forever.
    froze_across_grace();
    let killed = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(
        killed["kind"], "held",
        "a freeze never spends the grace: {killed}"
    );
    assert_eq!(killed["name"], SPADE_NAME, "{killed}");
    assert_eq!(killed["action"], DIG, "{killed}");
    // Frozen again: nothing is read and nothing is emitted, and the thaw
    // after it leaves the post-kill Dig where it was.
    on_pause();
    let frozen = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(frozen["kind"], "wait", "{frozen}");
    assert!(frozen.get("action").is_none(), "{frozen}");
    on_resume();
    let redig = call(&data, token, page, fight_scene(json!([]), json!({})));
    assert_eq!(redig["kind"], "held", "{redig}");
    assert_eq!(redig["action"], DIG, "{redig}");
}

/// The posted effective hitpoints at or below zero kill the token: the kind
/// is `dead` on any live call, the token dies with the player and nothing
/// posts `'clue solved'`. A page that posted no stat is not a zero, so the
/// fight still Attacks — and after the kill the redig is not gated by a
/// player death that has already been posted.
#[test]
fn a_posted_hitpoints_at_zero_is_dead_and_kills_the_token() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);

    // Missing stat first: the Attack still goes out under a fight that is
    // posted and alive.
    let token = spawned(&data);
    let mut bare = fight_scene(wizard.clone(), json!({}));
    bare.as_object_mut().expect("object").remove("hitpoints");
    let attack = call(&data, token, page.clone(), bare);
    assert_eq!(attack["kind"], "npc", "{attack}");
    assert_eq!(attack["index"], 7, "{attack}");

    // Every posted zero-or-below is the terminal, whatever the page also
    // carries, and the token dies on it.
    for hp in [0, -1, -20] {
        on_reset();
        let token = spawned(&data);
        let downed = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "hitpoints": hp })),
        );
        assert_eq!(downed["kind"], "dead", "{hp} {downed}");
        for absent in ["action", "index", "component_id", "name", "message"] {
            assert!(downed.get(absent).is_none(), "{hp} {absent} {downed}");
        }
        assert!(!downed.to_string().contains("clue solved"), "{hp} {downed}");
        let after = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({})),
        );
        assert_eq!(after["kind"], "aborted", "{hp} {after}");
        assert_eq!(after["reason"], "stale", "{hp} {after}");
    }

    // The kill, then a zero posted on the very next call: the death is the
    // terminal on any live call, the post-kill redig included.
    let token = spawned(&data);
    let mut bare = fight_scene(wizard.clone(), json!({}));
    bare.as_object_mut().expect("object").remove("hitpoints");
    let attack = call(&data, token, page.clone(), bare);
    assert_eq!(attack["kind"], "npc", "{attack}");
    let kill = call(
        &data,
        token,
        page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(kill["kind"], "held", "{kill}");
    assert_eq!(kill["action"], DIG, "{kill}");
    let after = call(
        &data,
        token,
        page,
        fight_scene(json!([]), json!({ "hitpoints": 0 })),
    );
    assert_eq!(after["kind"], "dead", "{after}");
    assert!(!after.to_string().contains("clue solved"), "{after}");
}

/// Freeze and yield beat the guarded encounter the way they beat the landed
/// verbs: no walk, no held, no npc and no if-button ride along, and the
/// token lives for the thaw.
#[test]
fn freeze_and_yield_beat_the_guarded_encounter() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let tile = guarded_tile_of(&data);
    // The ladder over one scene: paused, held, then the posted interrupt.
    let ladder = |token: u64, scene: &Value| {
        let mut out = Vec::new();
        on_pause();
        out.push(call(&data, token, page.clone(), scene.clone()));
        on_resume();
        on_hold(true);
        out.push(call(&data, token, page.clone(), scene.clone()));
        on_hold(false);
        let mut yielded_scene = scene.clone();
        yielded_scene["hold"] = json!(true);
        out.push(call(&data, token, page.clone(), yielded_scene));
        out
    };
    // The spawn stage: the walk never goes out under a frozen clock.
    let token = steady(&data, GUARDED);
    let spawn_scene = dig_scene(here(3100, 3300, 0), true);
    let spawn_steps = ladder(token, &spawn_scene);
    assert_eq!(
        spawn_steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![json!("wait"), json!("wait"), json!("yield")],
        "{spawn_steps:?}"
    );
    for step in &spawn_steps {
        assert!(step.get("x").is_none(), "{step}");
        assert!(step.get("action").is_none(), "{step}");
        assert_eq!(token_of(step), token, "{step}");
    }
    // Nothing burned: the thawed call is still the walk.
    let walk = call(&data, token, page.clone(), spawn_scene);
    assert_eq!(walk["kind"], "walk", "{walk}");
    let dig = call(
        &data,
        token,
        page.clone(),
        dig_scene(here(tile.x, tile.z, tile.level), true),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    // The fight stage: the same ladder, and the overlay-off click is not
    // emitted under it either.
    let fight = fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 }));
    let fight_steps = ladder(token, &fight);
    assert_eq!(
        fight_steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![json!("wait"), json!("wait"), json!("yield")],
        "{fight_steps:?}"
    );
    for step in &fight_steps {
        assert!(step.get("component_id").is_none(), "{step}");
        assert!(step.get("action").is_none(), "{step}");
    }
    // Thawed and unheld, the click is still there.
    let click = call(&data, token, page, fight);
    assert_eq!(click["kind"], "if-button", "{click}");
    assert_eq!(click["component_id"], 5621, "{click}");
}

/// The encounter is session state on the live step: a different held row
/// re-arms the gate and drops it, so coming back to the guarded row walks
/// and Digs its spawn again rather than resuming the fight the old token
/// already owned.
#[test]
fn a_different_held_step_drops_the_guarded_encounter() {
    on_reset();
    let data = selected();
    let guard_page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        guard_page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    // A different membership row is held: the landed gate re-arms for it.
    let other = json!([[RIDDLE, 1]]);
    let re_armed = call(
        &data,
        token,
        other.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    let logged = call(&data, token, other.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let _ = call(&data, token, other.clone(), json!({}));
    // Back to the guarded row: the gate re-arms again, and the next steady
    // call is the spawn again — a walk and a Dig, never the old Attack.
    let back = call(
        &data,
        token,
        guard_page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(back["kind"], "callback.enabled", "{back}");
    let logged = call(&data, token, guard_page.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let _ = call(&data, token, guard_page.clone(), json!({}));
    let reborn = call(
        &data,
        token,
        guard_page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(reborn["kind"], "held", "{reborn}");
    assert_eq!(reborn["action"], DIG, "{reborn}");
}

/// The post-kill Dig is the landed sibling Dig: it repeats while the clue
/// stays held, the casket it produces is the landed casket-first Open that
/// re-arms the gate, and a `none-held` right after a guarded Dig is still
/// the landed abort — the collect is the casket Open's alone.
#[test]
fn the_guarded_redig_repeats_and_its_casket_opens() {
    on_reset();
    let data = selected();
    let clue_page = json!([[GUARDED, 1]]);
    let token = spawned(&data);
    let attack = call(
        &data,
        token,
        clue_page.clone(),
        fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
    );
    assert_eq!(attack["kind"], "npc", "{attack}");
    let redig = call(
        &data,
        token,
        clue_page.clone(),
        fight_scene(json!([]), json!({})),
    );
    assert_eq!(redig["kind"], "held", "{redig}");
    assert_eq!(redig["action"], DIG, "{redig}");
    // The casket the Dig produced is held beside its own clue: identify is
    // casket-first, so the gate re-arms for it and its own Open follows.
    let casket = casket_of(&data, GUARDED);
    let casket_alias = row(&data, casket).alias.clone();
    let both = json!([[GUARDED, 1], [casket, 1]]);
    let scene = fight_scene(json!([]), json!({}));
    let re_armed = call(&data, token, both.clone(), scene.clone());
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    let logged = call(&data, token, both.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    assert!(
        logged["message"]
            .as_str()
            .unwrap_or("")
            .contains(&casket_alias),
        "{logged}"
    );
    assert_eq!(
        call(&data, token, both.clone(), scene.clone())["kind"],
        "callback.setStatus"
    );
    let open = call(&data, token, both.clone(), scene.clone());
    assert_eq!(open["kind"], "held", "{open}");
    assert_eq!(open["action"], OPEN, "{open}");
    // A `none-held` right after a guarded Dig is the landed abort, and the
    // reward window was never armed by it.
    let token = spawned(&data);
    let gone = call(&data, token, json!([]), json!({}));
    assert_eq!(gone["kind"], "aborted", "{gone}");
    assert_eq!(gone["reason"], NONE_HELD, "{gone}");
    assert!(!bound_armed(), "a guarded Dig never arms the reward window");
}

/// The guarded encounter emits walk, held Dig, npc Attack, if-button, wait
/// or yield only — never a completion, never a hunt kind of its own, and
/// never one of the public refusal tokens.
#[test]
fn the_guarded_encounter_emits_only_walk_held_npc_if_button_and_wait() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let tile = guarded_tile_of(&data);
    let token = steady(&data, GUARDED);
    let mut yielded = dig_scene(here(tile.x, tile.z, tile.level), true);
    yielded["hold"] = json!(true);
    let steps = vec![
        // The spawn walk, the spawn Dig, the overlay click, the Attack,
        // the kill wait, the post-kill redig and the interrupt.
        call(
            &data,
            token,
            page.clone(),
            dig_scene(here(3100, 3300, 0), true),
        ),
        call(
            &data,
            token,
            page.clone(),
            dig_scene(here(tile.x, tile.z, tile.level), true),
        ),
        call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 })),
        ),
        call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        ),
        call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        ),
        call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([targeting(npc(7, WIZARD, 3, 0, 10), 0)]), json!({})),
        ),
        call(&data, token, page, yielded),
    ];
    for step in &steps {
        assert!(step["status"].is_null(), "{step}");
        assert!(
            matches!(
                step["kind"].as_str().unwrap_or(""),
                "walk" | "held" | "npc" | "if-button" | "wait" | "yield"
            ),
            "{step}"
        );
        let text = step.to_string();
        for forbidden in [
            "clue solved",
            "done",
            "abandon",
            "supplies-needed",
            "dead",
            "guardian-lost",
            "grind-ready",
            "no-spade",
            "ownsEquipment",
            "sustain",
            "arm-special",
            "safespot",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
    }
    assert_eq!(
        steps
            .iter()
            .map(|step| step["kind"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("walk"),
            json!("held"),
            json!("if-button"),
            json!("npc"),
            json!("wait"),
            json!("held"),
            json!("yield"),
        ],
        "{steps:?}"
    );
}

/// `trail_clue_medium_sextant001`'s own acquire chain, walked end to end
/// over the posted pages: the professor's published tile, his Talk-to, the
/// one posted option the chain answers, the sextant's second giver, the
/// watch's own giver — and then, once the posted pack holds the trio, the
/// landed unguarded Dig.
///
/// Every tile here is the published `trio_givers` row's own spawn, read
/// from the selected family: no frozen `PROFESSOR` / `MURPHY` / `KOJO`
/// table and no first-in-file coordinate is ever the destination.
#[test]
fn a_sextant_row_acquires_the_trio_in_the_frozen_order() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let professor = professor_tile(&data);
    let murphy = giver_tile(&data, MURPHY);
    let kojo = giver_tile(&data, BROTHER_KOJO);

    // Not arrived and no trio held: the walk is the professor's own
    // published tile, not the row's decoded one.
    let walk = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(3160, 3251, 0) }),
    );
    assert_eq!(walk["kind"], "walk", "{walk}");
    assert_eq!(walk["x"], professor.x, "{walk}");
    assert_eq!(walk["z"], professor.z, "{walk}");
    assert_eq!(walk["level"], professor.level, "{walk}");

    // Arrived with the professor posted: the talk arm's unique-spawn rule,
    // over that giver's own packed id and posted display name.
    let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
    let posted = json!([talk_npc(
        9,
        id,
        name,
        Tile {
            x: professor.x,
            z: professor.z,
            level: professor.level,
        },
        1,
        &["Talk-to"],
    )]);
    let arrived = json!({
        "here": here(professor.x, professor.z, professor.level),
        "npcs": posted,
    });
    let talked = call(&data, token, page.clone(), arrived.clone());
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["name"], name, "{talked}");
    assert_eq!(talked["action"], "Talk-to", "{talked}");
    assert_eq!(talked["index"], 9, "{talked}");

    // The chat opens on the closed handler's own option, posted third: the
    // answer carries that posted 1-based slot and nothing else.
    let mut chat = arrived.clone();
    chat["chat_modal_id"] = json!(968);
    chat["chat_options"] = json!([
        option("Can you tell me about Treasure Trails?", 2),
        option("Talk about Treasure Trails.", 3),
    ]);
    let answered = call(&data, token, page.clone(), chat.clone());
    assert_eq!(answered["kind"], "answer", "{answered}");
    assert_eq!(answered["option"], 3, "{answered}");
    assert_eq!(token_of(&answered), token, "{answered}");

    // The same option folds on ASCII case: the literal is matched, never a
    // fragment of it.
    chat["chat_options"] = json!([option("i've lost my navigation chart.", 1)]);
    let folded = call(&data, token, page.clone(), chat);
    assert_eq!(folded["kind"], "answer", "{folded}");
    assert_eq!(folded["option"], 1, "{folded}");

    // That chat closed: the sextant's second stop is Murphy's own tile.
    let closed = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(professor.x, professor.z, professor.level),
            "npcs": posted,
        }),
    );
    assert_eq!(closed["kind"], "walk", "{closed}");
    assert_eq!(closed["x"], murphy.x, "{closed}");
    assert_eq!(closed["z"], murphy.z, "{closed}");

    // Murphy's chat is linear: a posted `chat_continue` is the step.
    let (mid, mname) = giver_identity(&data, MURPHY);
    let continued = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(murphy.x, murphy.z, murphy.level),
            "npcs": [talk_npc(
                11,
                mid,
                mname,
                Tile {
                    x: murphy.x,
                    z: murphy.z,
                    level: murphy.level,
                },
                1,
                &["Talk-to"],
            )],
            "chat_continue": true,
        }),
    );
    assert_eq!(continued["kind"], "continue", "{continued}");
    assert!(continued.get("option").is_none(), "{continued}");

    // The sextant landed: the chain rebases to the watch, which is Kojo's
    // alone, and Kojo's own options are none of this arm's business —
    // never the last one, never another giver's literal.
    let (kid, kname) = giver_identity(&data, BROTHER_KOJO);
    let kojo_posted = json!([talk_npc(
        12,
        kid,
        kname,
        Tile {
            x: kojo.x,
            z: kojo.z,
            level: kojo.level,
        },
        1,
        &["Talk-to"],
    )]);
    let watched = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(murphy.x, murphy.z, murphy.level),
            "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)],
        }),
    );
    assert_eq!(watched["kind"], "walk", "{watched}");
    assert_eq!(watched["x"], kojo.x, "{watched}");
    let foreign = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(kojo.x, kojo.z, kojo.level),
            "npcs": kojo_posted,
            "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)],
            "chat_modal_id": 968,
            "chat_options": [
                option("What is this place?", 1),
                option("Talk about Treasure Trails.", 2),
            ],
        }),
    );
    assert_eq!(foreign["kind"], "wait", "{foreign}");
    assert_eq!(token_of(&foreign), token, "{foreign}");

    // The watch landed: the chart is the professor's again, and the pack
    // that finally holds the whole trio falls through to the landed dig
    // arm — walk to the row's own decoded tile, then the held Dig.
    let charted = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(kojo.x, kojo.z, kojo.level),
            "inv": [
                inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
                inv(WATCH_ITEM, WATCH_NAME, 1),
            ],
        }),
    );
    assert_eq!(charted["kind"], "walk", "{charted}");
    assert_eq!(charted["x"], professor.x, "{charted}");
    let walked = call(
        &data,
        token,
        page.clone(),
        dig_scene(here(3100, 3300, 0), true),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
    assert_eq!(walked["x"], 3160, "{walked}");
    assert_eq!(walked["z"], 3251, "{walked}");
    let dig = call(&data, token, page, dig_scene(here(3160, 3251, 0), true));
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["name"], SPADE_NAME, "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
}

/// The intercept is in front of **both** dig arms: the guarded exemplar
/// walks to the professor, never to its decoded tile, until the posted pack
/// holds the trio — and only then does its own encounter spawn.
#[test]
fn the_trio_intercept_is_in_front_of_the_guarded_dig_too() {
    on_reset();
    let data = selected();
    let page = json!([[GUARDED, 1]]);
    let token = steady(&data, GUARDED);
    let professor = professor_tile(&data);
    let walk = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(3100, 3300, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
    );
    assert_eq!(walk["kind"], "walk", "{walk}");
    assert_eq!(walk["x"], professor.x, "{walk}");
    assert_eq!(walk["z"], professor.z, "{walk}");
    // Held trio and Spade: the encounter's own walk-then-Dig, unchanged.
    let tile = guarded_tile_of(&data);
    let walked = call(
        &data,
        token,
        page.clone(),
        dig_scene(here(3100, 3300, 0), true),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
    assert_eq!(walked["x"], tile.x, "{walked}");
    let dig = call(
        &data,
        token,
        page,
        dig_scene(here(tile.x, tile.z, tile.level), true),
    );
    assert_eq!(dig["kind"], "held", "{dig}");
    assert_eq!(dig["action"], "Dig", "{dig}");
}

/// A posted option list the selected literals do not match waits: never the
/// last option, never a frozen fragment (`Treasure Trails`, `lost`), never
/// the trawler's or the Clock Tower quest's choices, and never a Talk-to
/// behind the open chat.
#[test]
fn the_trio_intercept_waits_on_options_it_may_not_answer() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let professor = professor_tile(&data);
    let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
    let npcs = json!([talk_npc(
        9,
        id,
        name,
        Tile {
            x: professor.x,
            z: professor.z,
            level: professor.level,
        },
        1,
        &["Talk-to"],
    )]);
    let arrived = json!({
        "here": here(professor.x, professor.z, professor.level),
        "npcs": npcs,
    });
    // An open chat is posted open: even the giver's own row is not
    // Talked-to again behind it.
    let bare = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here(professor.x, professor.z, professor.level),
            "npcs": npcs,
            "chat_modal_id": 968,
        }),
    );
    assert_eq!(bare["kind"], "wait", "{bare}");
    for options in [
        // Frozen fragments, another giver's literal, the last option a
        // sequencer would fall back to, and a matched text with no slot.
        json!([
            option("Treasure Trails", 1),
            option("I am lost!!!", 2),
            option("Yes please.", 3),
        ]),
        json!([option("I've lost my navigation chart", 1)]),
        json!([json!({ "text": "Talk about Treasure Trails." })]),
        json!([]),
    ] {
        let mut scene = arrived.clone();
        scene["chat_modal_id"] = json!(968);
        scene["chat_options"] = options.clone();
        let idle = call(&data, token, page.clone(), scene);
        assert_eq!(idle["kind"], "wait", "{options} {idle}");
        assert_eq!(token_of(&idle), token, "{options} {idle}");
        assert!(
            idle.get("option").is_none() && idle.get("index").is_none(),
            "{options} {idle}"
        );
    }
}

/// The picker is the talk arm's unique-spawn rule: the giver's own packed
/// id first, then its posted display name, a posted talk action, and the
/// frozen radius around the published tile. A wanderer is not chased, a
/// same-name lookalike with another packed id is not this giver, and a page
/// with no match waits at the tile.
#[test]
fn the_trio_pick_is_the_unique_spawn_rule_and_never_a_lookalike() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let professor = professor_tile(&data);
    let here_tile = here(professor.x, professor.z, professor.level);
    let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
    let tile = Tile {
        x: professor.x,
        z: professor.z,
        level: professor.level,
    };
    for npcs in [
        // No posted npc page at all, an empty page, and another npc.
        json!([]),
        json!([talk_npc(1, 541, "Zeke", tile, 1, &["Talk-to"])]),
        // The `observatory_professor2` lookalike: the same display name and
        // another packed id, which is not this family's row.
        json!([talk_npc(2, id + 1, name, tile, 1, &["Talk-to"])]),
        // This giver's own id, but no posted talk action.
        json!([talk_npc(3, id, name, tile, 1, &["Attack"])]),
        // Posted, off the published tile and out of the radius.
        json!([talk_npc(
            4,
            id,
            name,
            Tile {
                x: tile.x + 8,
                z: tile.z,
                level: tile.level,
            },
            8,
            &["Talk-to"],
        )]),
        // Posted on another level.
        json!([talk_npc(
            5,
            id,
            name,
            Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level + 1,
            },
            1,
            &["Talk-to"],
        )]),
    ] {
        let idle = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here_tile, "npcs": npcs }),
        );
        assert_eq!(idle["kind"], "wait", "{npcs} {idle}");
        assert_eq!(token_of(&idle), token, "{npcs} {idle}");
    }
    // The posted name alone is enough when the packed id is not posted: the
    // verb then carries that posted name and index.
    let named = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": here_tile,
            "npcs": [unfield(talk_npc(6, id, name, tile, 1, &["Talk-to"]), "id")],
        }),
    );
    assert_eq!(named["kind"], "npc", "{named}");
    assert_eq!(named["name"], name, "{named}");
    assert_eq!(named["index"], 6, "{named}");
}

/// Freeze, hold, the posted interrupt and a posted death all beat the
/// acquire chain exactly as they beat the landed arms, and the chain itself
/// never emits a completion kind.
#[test]
fn freeze_yield_and_death_beat_the_trio_acquire() {
    on_reset();
    let data = selected();
    let page = json!([[UNGUARDED, 1]]);
    let token = steady(&data, UNGUARDED);
    let professor = professor_tile(&data);
    let here_tile = here(professor.x, professor.z, professor.level);
    let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
    let scene = json!({
        "here": here_tile,
        "npcs": [talk_npc(
            9,
            id,
            name,
            Tile {
                x: professor.x,
                z: professor.z,
                level: professor.level,
            },
            1,
            &["Talk-to"],
        )],
    });
    on_pause();
    let paused = call(&data, token, page.clone(), scene.clone());
    assert_eq!(paused["kind"], "wait", "{paused}");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, page.clone(), scene.clone());
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    let mut yielded_scene = scene.clone();
    yielded_scene["hold"] = json!(true);
    let yielded = call(&data, token, page.clone(), yielded_scene);
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    let mut dead_scene = scene.clone();
    dead_scene["hitpoints"] = json!(0);
    let dead = call(&data, token, page.clone(), dead_scene);
    assert_eq!(dead["kind"], "dead", "{dead}");
    for step in [&paused, &held_clock, &yielded, &dead] {
        let text = step.to_string();
        for forbidden in ["clue solved", "grind-ready", "cause", "message", "answer"] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
    }
    assert_eq!(
        call(&data, token, page, scene)["kind"],
        "aborted",
        "the dead token is gone"
    );
}

/// The membership and the plan over the selected pins: the param is the
/// whole of the row's need, the trio is the posted pack's own read, and the
/// order is the frozen `nextCoordTool` one — sextant, then watch, then
/// chart. A pack that already holds the whole trio is not this arm's.
#[test]
fn the_trio_plan_is_the_param_and_the_posted_pack() {
    on_reset();
    let data = selected();
    // The param on the row itself, and nothing else: another value, the
    // sibling params and a paramless row are all not this arm's.
    assert!(needs_trio(row(&data, UNGUARDED)));
    assert!(needs_trio(row(&data, GUARDED)));
    assert!(needs_trio(&member(vec![param(TRAIL_SEXTANT, "yes")], None)));
    assert!(!needs_trio(&member(vec![param(TRAIL_SEXTANT, "no")], None)));
    assert!(!needs_trio(&member(
        vec![param("trail_casket", "trail_clue_medium_sextant001_casket")],
        None
    )));
    assert!(!needs_trio(row(&data, MAP)));
    assert!(!needs_trio(row(&data, RIDDLE)));
    assert!(!needs_trio(&member(vec![], None)));
    // No selected pin and no published givers: no plan at all.
    assert!(trio_plan(None, &json!({})).is_none());
    // The plan is the first tool the posted pack is short of.
    let empty = json!({});
    let (givers, tool) = trio_plan(Some(&data), &empty).expect("plan");
    assert_eq!(tool, Tool::Sextant);
    assert_eq!(stop_of(&givers, tool, 0).row.alias, OBSERVATORY_PROFESSOR);
    assert_eq!(stop_of(&givers, tool, 1).row.alias, MURPHY);
    assert_eq!(last_stop(tool), 1);
    let sextant = json!({ "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)] });
    let (givers, tool) = trio_plan(Some(&data), &sextant).expect("plan");
    assert_eq!(tool, Tool::Watch);
    assert_eq!(stop_of(&givers, tool, 0).row.alias, BROTHER_KOJO);
    assert_eq!(last_stop(tool), 0);
    let watch = json!({
        "inv": [
            inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
            inv(WATCH_ITEM, WATCH_NAME, 1),
        ],
    });
    let (givers, tool) = trio_plan(Some(&data), &watch).expect("plan");
    assert_eq!(tool, Tool::Chart);
    assert_eq!(stop_of(&givers, tool, 0).row.alias, OBSERVATORY_PROFESSOR);
    // A zero count, a missing count and an unknown id are not held: the
    // join is the selected id beside a positive posted count, and the
    // posted display name is never the identity.
    for pack in [
        json!({ "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 0)] }),
        json!({ "inv": [json!({ "id": SEXTANT_ITEM, "name": SEXTANT_NAME })] }),
        json!({ "inv": [inv(999_999, SEXTANT_NAME, 1)] }),
    ] {
        let (_, tool) = trio_plan(Some(&data), &pack).expect("plan");
        assert_eq!(tool, Tool::Sextant, "{pack}");
    }
    // The whole trio held: the intercept's own completion, and the landed
    // dig arms run.
    assert!(trio_plan(Some(&data), &json!({ "inv": trio_inv() })).is_none());
}

/// The rows this arm is not: a search row, a talk step and a key-keeper
/// riddle over the same empty pack walk, Talk-to and idle exactly as they
/// did — the intercept never becomes a second classify of them.
#[test]
fn search_talk_and_key_rows_never_enter_the_trio_acquire() {
    on_reset();
    let data = selected();
    // The search membership: its own decoded walk, not the professor's.
    let token = steady(&data, SEARCH);
    let walk = call(
        &data,
        token,
        json!([[SEARCH, 1]]),
        json!({ "here": here(3100, 3300, 0) }),
    );
    assert_eq!(walk["kind"], "walk", "{walk}");
    assert_eq!(walk["x"], 3209, "{walk}");
    assert_eq!(walk["z"], 3218, "{walk}");
    // The talk step: its own published spawn, not the professor's tile.
    let spawn = data
        .talk_key()
        .expect("talk_key")
        .talk
        .iter()
        .find(|talk| talk.id == TALK)
        .and_then(|talk| talk.spawn.as_ref())
        .expect("the talk step publishes a spawn");
    let token = steady(&data, TALK);
    let talked = call(
        &data,
        token,
        json!([[TALK, 1]]),
        json!({ "here": here(3100, 3300, 0), "npcs": [] }),
    );
    assert_eq!(talked["kind"], "walk", "{talked}");
    assert_eq!(talked["x"], spawn.x, "{talked}");
    assert_eq!(talked["z"], spawn.z, "{talked}");
    // The key-keeper riddle: the sibling hunt's own published spawn, never
    // the professor's tile and never a trio requirement.
    let spawn = data
        .talk_key()
        .expect("talk_key")
        .keys
        .iter()
        .find(|key| key.id == RIDDLE)
        .and_then(|key| key.spawn.as_ref())
        .expect("the keeper publishes a spawn");
    let token = steady(&data, RIDDLE);
    let hunted = call(
        &data,
        token,
        json!([[RIDDLE, 1]]),
        json!({ "here": here(3100, 3300, 0), "npcs": [] }),
    );
    assert_eq!(hunted["kind"], "walk", "{hunted}");
    assert_eq!(hunted["x"], spawn.x, "{hunted}");
    assert_eq!(hunted["z"], spawn.z, "{hunted}");
}

/// The b run's first piece: these tests post boards from that run's own
/// numbering, so a cell's posted piece id is `PIECE_B + target`.
const PIECE_B: i32 = 2749;

/// The component the test boards post.
const BOARD_COMPONENT: i32 = 6600;

/// One wrapper-marshalled posted board page plus the session generation
/// the click rides: a sparse row per filled cell, in slot order, so the
/// machine's own read is what fills the 25.
fn board_page(board: &Board, generation: u64) -> Value {
    let mut rows = Vec::new();
    for (slot, cell) in board.iter().enumerate() {
        if let Some(target) = *cell {
            rows.push(json!({ "slot": slot as i32, "id": PIECE_B + i32::from(target) }));
        }
    }
    json!({
        "puzzle_board": {
            "component_id": BOARD_COMPONENT,
            "size": clue_puzzle::PUZZLE_SIZE as i32,
            "items": rows,
        },
        "puzzle_board_generation": generation,
    })
}

/// The closed board SNAP posts beside a box that was never opened: a
/// present object with no component and no rows.
fn closed_board_page() -> Value {
    json!({
        "puzzle_board": { "component_id": -1, "size": 0, "items": [] },
        "puzzle_board_generation": 0,
    })
}

/// The solved board: every piece on its own slot and the gap on 24.
fn solved_board() -> Board {
    let mut board: Board = [None; clue_puzzle::PUZZLE_SIZE];
    for (slot, cell) in board.iter_mut().enumerate() {
        *cell = (slot != clue_puzzle::PUZZLE_BLANK_SLOT).then_some(slot as u8);
    }
    board
}

/// One slide from solved: the piece belonging on 23 stands on the blank
/// slot, so the frozen plan is the single click on 24.
fn one_move_board() -> Board {
    let mut board = solved_board();
    board[clue_puzzle::PUZZLE_BLANK_SLOT] = Some(23);
    board[23] = None;
    board
}

/// The pack page of a held puzzle step: the desc-only riddle and its own
/// selected box.
fn held_box() -> Value {
    json!([[PUZZLE_RIDDLE, 1], [PUZZLE_BOX, 1]])
}

#[test]
fn the_puzzle_join_is_the_rows_own_selected_box() {
    let data = selected();
    assert_eq!(
        puzzle_box(Some(&data), row(&data, PUZZLE_RIDDLE)),
        Some((PUZZLE_BOX, "Puzzle box"))
    );
    // No box of its own: the desc-only riddle with no such item, and the
    // search row whose alias joins to nothing.
    assert_eq!(
        puzzle_box(Some(&data), row(&data, PUZZLE_RIDDLE_NO_BOX)),
        None
    );
    assert_eq!(puzzle_box(Some(&data), row(&data, SEARCH)), None);
    assert_eq!(puzzle_box(None, row(&data, PUZZLE_RIDDLE)), None);
    // Held means the posted page carries a positive count for that id and
    // nothing else.
    assert!(holds(&json!({ "held": held_box() }), PUZZLE_BOX));
    assert!(!holds(&json!({ "held": [[PUZZLE_BOX, 0]] }), PUZZLE_BOX));
    assert!(!holds(&json!({ "held": [[PUZZLE_RIDDLE, 1]] }), PUZZLE_BOX));
    assert!(!holds(&json!({ "held": [] }), PUZZLE_BOX));
    assert!(!holds(&json!({}), PUZZLE_BOX));
}

#[test]
fn a_held_puzzle_box_opens_by_its_selected_name_and_repeats_while_closed() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    assert_eq!(opened["token"], token, "{opened}");
    assert_eq!(opened["name"], "Puzzle box", "{opened}");
    assert_eq!(opened["action"], "Open", "{opened}");
    // The name is the identity: no row id, no tile and no slot rides along.
    for absent in ["id", "x", "z", "level", "slot"] {
        assert!(opened.get(absent).is_none(), "{absent} {opened}");
    }
    // The first Open armed the frozen open window, and the repeats are
    // page-driven rather than a second arm.
    assert!(bound_armed(), "{opened}");
    let repeated = call(&data, token, held_box(), closed_board_page());
    assert_eq!(repeated["kind"], "held", "{repeated}");
    // The window runs out: the attempt ends with no closer verb, because
    // there is no board to close.
    force_bound();
    let expired = call(&data, token, held_box(), closed_board_page());
    assert_eq!(expired["kind"], "wait", "{expired}");
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
}

#[test]
fn a_readable_board_dispatches_one_puzzle_move_from_the_posted_row() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    assert_eq!(moved["token"], token, "{moved}");
    // The posted row's own identity and this call's board session: the
    // piece standing on the plan's slot, that slot, the posted component
    // and the posted generation.
    assert_eq!(moved["id"], PIECE_B + 23, "{moved}");
    assert_eq!(moved["slot"], 24, "{moved}");
    assert_eq!(moved["component"], BOARD_COMPONENT, "{moved}");
    assert_eq!(moved["generation"], 7, "{moved}");
    assert!(moved.get("name").is_none(), "{moved}");
    assert!(moved.get("action").is_none(), "{moved}");
}

#[test]
fn a_landed_move_closes_the_solved_board_and_never_closes_it_twice() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    // The live board is the one that click was expected to produce, and it
    // is the solved one: the frozen `isPuzzleSolved` read closes it.
    let closed = call(&data, token, held_box(), board_page(&solved_board(), 7));
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    assert_eq!(closed["token"], token, "{closed}");
    // The close landed: the board left the page and the step idles. A
    // solved board posted again is not a second close either.
    let gone = call(&data, token, held_box(), closed_board_page());
    assert_eq!(gone["kind"], "wait", "{gone}");
    let reposted = call(&data, token, held_box(), board_page(&solved_board(), 7));
    assert_eq!(reposted["kind"], "wait", "{reposted}");
    // A different held row re-arms the step, and `clear_step` drops the
    // latch with it: back on the riddle the box opens again.
    let other = call(&data, token, json!([[SEARCH, 1]]), json!({}));
    assert_eq!(other["kind"], "callback.enabled", "{other}");
    let gate = call(&data, token, held_box(), json!({}));
    assert_eq!(gate["kind"], "callback.enabled", "{gate}");
    let logged = call(&data, token, held_box(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let posted = call(&data, token, held_box(), json!({}));
    assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
    let reopened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(reopened["kind"], "held", "{reopened}");
}

#[test]
fn a_solved_board_the_token_never_opened_is_still_closed() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    // The box is held and the board is already solved: the frozen `puzzle
    // already solved` path closes it rather than opening anything.
    let closed = call(&data, token, held_box(), board_page(&solved_board(), 3));
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    let gone = call(&data, token, held_box(), closed_board_page());
    assert_eq!(gone["kind"], "wait", "{gone}");
}

#[test]
fn a_board_that_goes_unreadable_mid_solve_closes_and_latches() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    // The board went away with the click outstanding, and the modal the
    // Open landed on may still be up: the attempt exits through the same
    // one close as a stall or `MAX_MOVES`.
    let closed = call(&data, token, held_box(), closed_board_page());
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    assert_eq!(closed["token"], token, "{closed}");
    // The close is out: neither the closed page it leaves nor a board
    // posted again draws a second close, and the window is what is left of
    // the exit.
    for page in [closed_board_page(), board_page(&one_move_board(), 7)] {
        let waited = call(&data, token, held_box(), page);
        assert_eq!(waited["kind"], "wait", "{waited}");
    }
    force_bound();
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
    // The solved-or-attempted latch holds: a readable board posted again is
    // not a second attempt and not a second close.
    let reposted = call(&data, token, held_box(), board_page(&solved_board(), 7));
    assert_eq!(reposted["kind"], "wait", "{reposted}");
}

#[test]
fn a_still_open_board_the_map_cannot_place_takes_the_same_close() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    // The page still posts a live size-25 board, but one piece the selected
    // map cannot place: the board is up and unreadable, so the same close
    // goes out rather than a bare latch with the modal open.
    let mut unplaceable = board_page(&one_move_board(), 7);
    unplaceable["puzzle_board"]["items"][0]["id"] = json!(PIECE_B + 99);
    let closed = call(&data, token, held_box(), unplaceable);
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    let waited = call(&data, token, held_box(), closed_board_page());
    assert_eq!(waited["kind"], "wait", "{waited}");
    force_bound();
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
}

#[test]
fn a_settle_bound_that_runs_out_unlanded_replans_from_the_live_board() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    // The click is out and the board has not moved: sent is not observed,
    // so this call waits the frozen window out.
    let unsettled = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(unsettled["kind"], "wait", "{unsettled}");
    force_bound();
    let stalled = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(stalled["kind"], "wait", "{stalled}");
    // The refusal is counted and the next call plans again from the board
    // it reads — the same one-move board, so the same click.
    let replanned = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(replanned["kind"], "puzzle-move", "{replanned}");
    assert_eq!(replanned["slot"], 24, "{replanned}");
    assert_eq!(replanned["id"], PIECE_B + 23, "{replanned}");
}

#[test]
fn a_board_no_plan_exists_for_stalls_to_the_close() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    // A mixed picture set: two pieces for one target and none for another
    // is not a valid board, so the frozen solver has no plan for it.
    let mut mixed = one_move_board();
    mixed[6] = Some(5);
    let page = board_page(&mixed, 7);
    for struck in 1..STALL_LIMIT {
        let stalled = call(&data, token, held_box(), page.clone());
        assert_eq!(stalled["kind"], "wait", "{struck} {stalled}");
    }
    let closed = call(&data, token, held_box(), page);
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
}

#[test]
fn a_multi_move_board_clicks_one_plan_slot_per_call_until_it_closes() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    // Two slides from solved: the piece belonging on 18 stands on 23, the
    // one belonging on 23 on the blank slot, and the gap is on 18.
    let mut board = solved_board();
    board[18] = None;
    board[23] = Some(18);
    board[clue_puzzle::PUZZLE_BLANK_SLOT] = Some(23);
    let mut clicked = Vec::new();
    loop {
        let step = call(&data, token, held_box(), board_page(&board, 7));
        if step["kind"] == "close-modal" {
            break;
        }
        assert_eq!(step["kind"], "puzzle-move", "{step}");
        // The plan's own first slot, applied to the live board: the next
        // call is handed the page that click produced. One verb per call,
        // and the click is the posted row standing on that slot.
        let slot = usize::try_from(step["slot"].as_u64().expect("slot")).expect("slot");
        assert_eq!(
            step["id"],
            PIECE_B + i32::from(board[slot].expect("piece")),
            "{step}"
        );
        assert!(clue_puzzle::apply_puzzle_move(&mut board, slot), "{step}");
        clicked.push(step["slot"].as_i64().expect("slot"));
        assert!(clicked.len() < STALL_LIMIT as usize, "{step}");
    }
    assert_eq!(clicked, vec![23, 24], "the frozen plan, one click per call");
    assert!(clue_puzzle::is_puzzle_solved(&board));
    // The close landed: the step idles from here.
    let gone = call(&data, token, held_box(), closed_board_page());
    assert_eq!(gone["kind"], "wait", "{gone}");
}

#[test]
fn freeze_and_yield_beat_the_puzzle_arm() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    // Frozen: no Open, no click and no close, and the open window is not
    // spent by the frozen call.
    on_pause();
    let paused = call(&data, token, held_box(), closed_board_page());
    assert_eq!(paused["kind"], "wait", "{paused}");
    assert!(!bound_armed(), "a frozen call spends nothing: {paused}");
    on_resume();
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    // The posted `hold || ours` interrupt, unfrozen: yield, and no verb
    // rides along with it. The click still goes out on the next call.
    on_hold(true);
    let held_clock = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    let mut yield_page = board_page(&one_move_board(), 7);
    yield_page["hold"] = json!(true);
    let yielded = call(&data, token, held_box(), yield_page);
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
}

#[test]
fn rows_without_their_own_held_box_keep_their_own_arm() {
    on_reset();
    let data = selected();
    // A desc-only row with no box of its own, with another row's box in
    // the pack: identified, then idle.
    let page = json!([[PUZZLE_RIDDLE_NO_BOX, 1], [PUZZLE_BOX, 1]]);
    let token = steady(&data, PUZZLE_RIDDLE_NO_BOX);
    let idle = call(&data, token, page.clone(), board_page(&solved_board(), 7));
    assert_eq!(idle["kind"], "wait", "{idle}");
    // The riddle's own box is not on this page: not a puzzle step either,
    // so the solved board it posts is not closed by this arm.
    let token = steady(&data, PUZZLE_RIDDLE);
    let unheld = call(
        &data,
        token,
        json!([[PUZZLE_RIDDLE, 1]]),
        board_page(&solved_board(), 7),
    );
    assert_eq!(unheld["kind"], "wait", "{unheld}");
    // A search row keeps the search arm with a box sitting in the pack:
    // the join is the row's own alias, so no box steals it.
    let token = steady(&data, SEARCH);
    let walked = call(
        &data,
        token,
        json!([[SEARCH, 1], [PUZZLE_BOX, 1]]),
        json!({ "here": here(3100, 3300, 1) }),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
}

/// `trail_clue_hard_riddle019`: the second puzzle riddle whose own selected
/// `trail_clue_hard_riddle019_puzzlebox` item is held, and one of the five
/// identity-only talk steps — `Examiner`, packed id 618, with no published
/// spawn — so its latched fall-through takes the nearest posted npc of that
/// identity.
const PUZZLE_RIDDLE_IDENTITY: i32 = 3566;
/// That box item, display `Puzzle box`.
const PUZZLE_BOX_IDENTITY: i32 = 3567;
/// The exemplar riddle014's own talk identity: `Oziach`, packed id 747, at
/// the published `(3069, 3517, plane 0)`.
const OZIACH_ID: i32 = 747;
const OZIACH_NAME: &str = "Oziach";
/// riddle019's identity: `Examiner`, packed id 618, no unique spawn.
const EXAMINER_ID: i32 = 618;
const EXAMINER_NAME: &str = "Examiner";

/// The published spawn tile one selected talk step's walk carries, read
/// from the family rather than copied.
fn talk_tile(data: &SelectedGameData, id: i32) -> Tile {
    let spawn = talk_of(data, id)
        .spawn
        .as_ref()
        .expect("a unique jm2 spawn");
    Tile {
        x: spawn.x,
        z: spawn.z,
        level: spawn.plane,
    }
}

/// The latched puzzle riddle re-talks: once the solved-or-attempted latch is
/// set, the `Steady` dispatch skips the box's own arm and the row's own
/// talk step runs the landed walk-then-Talk-to. The latch-arming call is
/// still the puzzle arm's own `wait`; the fall-through is the next live call.
#[test]
fn a_latched_puzzle_riddle_re_talks_over_the_landed_talk_step() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    // The board never opened: the frozen open window runs out and latches
    // the step with no closer verb.
    force_bound();
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
    let step = talk_of(&data, PUZZLE_RIDDLE);
    assert_eq!(step.id, PUZZLE_RIDDLE, "{}", step.alias);
    assert_eq!(
        (step.npc.id, step.npc.name.as_str()),
        (OZIACH_ID, OZIACH_NAME)
    );
    let tile = talk_tile(&data, PUZZLE_RIDDLE);
    // No posted npc page: no invented target and no walk, with the token
    // still live.
    let blind = call(
        &data,
        token,
        held_box(),
        json!({ "here": here(tile.x, tile.z, tile.level) }),
    );
    assert_eq!(blind["kind"], "wait", "{blind}");
    assert_eq!(token_of(&blind), token, "{blind}");
    // Posted far from the published tile: the walk is that tile, with the
    // published `plane` as the verb's `level`.
    let walked = call(
        &data,
        token,
        held_box(),
        talk_scene(here(tile.x - 30, tile.z, tile.level), json!([]), json!({})),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
    assert_eq!(
        (
            walked["x"].as_i64(),
            walked["z"].as_i64(),
            walked["level"].as_i64()
        ),
        (
            Some(i64::from(tile.x)),
            Some(i64::from(tile.z)),
            Some(i64::from(tile.level))
        ),
        "{walked}"
    );
    // Arrived with the step's own npc posted on the tile: the landed
    // Talk-to, carrying the posted name and the posted scene index.
    let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
    let talked = call(
        &data,
        token,
        held_box(),
        talk_scene(here(tile.x, tile.z, tile.level), npcs.clone(), json!({})),
    );
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["name"], OZIACH_NAME, "{talked}");
    assert_eq!(talked["action"], "Talk-to", "{talked}");
    assert_eq!(talked["index"], 3, "{talked}");
    // The still-held solved board is never a second close and the box is
    // never re-Opened: the latched step takes the talk arm, not the board.
    let again = call(
        &data,
        token,
        held_box(),
        talk_scene(
            here(tile.x, tile.z, tile.level),
            npcs,
            board_page(&solved_board(), 7),
        ),
    );
    assert_eq!(again["kind"], "npc", "{again}");
}

/// The fall-through is either-way: a stalled attempt that solved nothing
/// re-talks exactly like a solved board, because the trigger is the latch
/// and never the solved read.
#[test]
fn a_stalled_puzzle_riddle_re_talks_the_same_way() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    // A mixed picture set: the frozen solver has no plan for it, so the
    // consecutive refusals run out and the exit close goes out.
    let mut mixed = one_move_board();
    mixed[6] = Some(5);
    let page = board_page(&mixed, 7);
    for struck in 1..STALL_LIMIT {
        let stalled = call(&data, token, held_box(), page.clone());
        assert_eq!(stalled["kind"], "wait", "{struck} {stalled}");
    }
    let closed = call(&data, token, held_box(), page);
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    force_bound();
    let latched = call(&data, token, held_box(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
    let tile = talk_tile(&data, PUZZLE_RIDDLE);
    let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
    let talked = call(
        &data,
        token,
        held_box(),
        talk_scene(here(tile.x, tile.z, tile.level), npcs, json!({})),
    );
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["index"], 3, "{talked}");
}

/// The frozen close window is not the exclusion: `latch` is unset while the
/// one close is out, so a posted talk scene draws nothing until the window
/// ends. The call that arms the latch still waits inside the puzzle arm,
/// and the next live call talks although `closing` is never cleared.
#[test]
fn a_latched_puzzle_riddle_never_talks_inside_the_close_window() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE);
    let opened = call(&data, token, held_box(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
    assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    let closed = call(&data, token, held_box(), board_page(&solved_board(), 7));
    assert_eq!(closed["kind"], "close-modal", "{closed}");
    let tile = talk_tile(&data, PUZZLE_RIDDLE);
    let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
    let scene = talk_scene(here(tile.x, tile.z, tile.level), npcs, json!({}));
    // The close is out and its window is running: no walk and no Talk-to,
    // whatever the page posts.
    for waited in 0..2 {
        let waiting = call(&data, token, held_box(), scene.clone());
        assert_eq!(waiting["kind"], "wait", "{waited} {waiting}");
    }
    // The window ends: this call arms the latch and waits, and the next
    // live call is the landed talk step.
    force_bound();
    let latched = call(&data, token, held_box(), scene.clone());
    assert_eq!(latched["kind"], "wait", "{latched}");
    let talked = call(&data, token, held_box(), scene);
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["index"], 3, "{talked}");
}

/// The identity-only latched riddle re-talks the same landed arm: no
/// published spawn, so the posted npc of the step's own identity is what is
/// walked to and Talked-to — and a page that posts none of them is a `wait`.
/// No Examiner, tile or target is invented for it.
#[test]
fn a_latched_identity_only_puzzle_riddle_waits_without_its_own_npc() {
    on_reset();
    let data = selected();
    let token = steady(&data, PUZZLE_RIDDLE_IDENTITY);
    let page = json!([[PUZZLE_RIDDLE_IDENTITY, 1], [PUZZLE_BOX_IDENTITY, 1]]);
    let opened = call(&data, token, page.clone(), closed_board_page());
    assert_eq!(opened["kind"], "held", "{opened}");
    force_bound();
    let latched = call(&data, token, page.clone(), closed_board_page());
    assert_eq!(latched["kind"], "wait", "{latched}");
    let step = talk_of(&data, PUZZLE_RIDDLE_IDENTITY);
    assert_eq!(step.id, PUZZLE_RIDDLE_IDENTITY, "{}", step.alias);
    assert!(step.spawn.is_none(), "{}", step.alias);
    assert_eq!(
        (step.npc.id, step.npc.name.as_str()),
        (EXAMINER_ID, EXAMINER_NAME)
    );
    let here_tile = Tile {
        x: 3207,
        z: 3233,
        level: 0,
    };
    // No posted npc page at all, and an empty one: a wait with the token
    // live, never an invented Examiner.
    for scene in [
        json!({}),
        talk_scene(
            here(here_tile.x, here_tile.z, here_tile.level),
            json!([]),
            json!({}),
        ),
    ] {
        let waited = call(&data, token, page.clone(), scene);
        assert_eq!(waited["kind"], "wait", "{waited}");
        assert_eq!(token_of(&waited), token, "{waited}");
    }
    // Another identity posted on the tile is not this step's npc either.
    let other = json!([talk_npc(1, 0, "Hans", here_tile, 0, &["Talk-to"])]);
    let unmatched = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(here_tile.x, here_tile.z, here_tile.level),
            other,
            json!({}),
        ),
    );
    assert_eq!(unmatched["kind"], "wait", "{unmatched}");
    // The identity-only rule: a posted Examiner out of reach is walked to
    // at its own posted tile, which is the only tile this arm has.
    let posted_tile = Tile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let far = json!([talk_npc(
        4,
        EXAMINER_ID,
        EXAMINER_NAME,
        posted_tile,
        5,
        &["Talk-to"]
    )]);
    let walked = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(here_tile.x, here_tile.z, here_tile.level),
            far,
            json!({}),
        ),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
    assert_eq!(
        (walked["x"].as_i64(), walked["z"].as_i64()),
        (
            Some(i64::from(posted_tile.x)),
            Some(i64::from(posted_tile.z))
        ),
        "{walked}"
    );
    // Arrived there with its own identity posted: the landed Talk-to.
    let near = json!([talk_npc(
        4,
        EXAMINER_ID,
        EXAMINER_NAME,
        posted_tile,
        0,
        &["Talk-to"]
    )]);
    let talked = call(
        &data,
        token,
        page,
        talk_scene(
            here(posted_tile.x, posted_tile.z, posted_tile.level),
            near,
            json!({}),
        ),
    );
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["name"], EXAMINER_NAME, "{talked}");
    assert_eq!(talked["index"], 4, "{talked}");
}

/// `trail_clue_easy_simple005`: a talk membership whose jm2 spawn is unique
/// — `hans` at `(3207, 3233, plane 0)`.
const TALK: i32 = 2681;
/// `trail_clue_hard_riddle025`: the talk step that publishes a plane-1
/// spawn — `Heckel Funch` at `(2493, 3488, plane 1)`. The published plane
/// is the walk verb's `level`, and it is the only place a level comes from.
const TALK_PLANE: i32 = 3575;
/// `trail_clue_easy_simple008`: an identity-only talk step. Its jm2 spawn
/// is not unique, so the family publishes no tile and the arm takes the
/// nearest posted npc of its own identity — `Tanner`, packed id 804.
const TALK_IDENTITY: i32 = 2684;
/// `trail_clue_medium_anagram001`: the talk step that is also a challenge
/// parent — `Hazelmere`, whose `2842` scroll answers `"6859"`.
const TALK_CHALLENGE: i32 = 2841;
/// That scroll.
const CHALLENGE: i32 = 2842;
/// `trail_clue_medium_anagram003`: the identity-only challenge parent —
/// Zoo keeper, whose `2846` scroll answers `"40"`.
const TALK_CHALLENGE_IDENTITY: i32 = 2845;
/// That scroll.
const CHALLENGE_IDENTITY: i32 = 2846;

/// One wrapper-marshalled posted npc row as the talk arm reads it: the
/// posted index the Talk-to carries, the packed id and posted display name
/// the identity join compares, the posted tile and distance the arrival is
/// measured by, and the posted action list the talk action is read from.
fn talk_npc(index: i32, id: i32, name: &str, tile: Tile, distance: i32, actions: &[&str]) -> Value {
    json!({
        "index": index,
        "id": id,
        "name": name,
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
        "distance": distance,
        "health": 0,
        "max_health": 0,
        "in_combat": false,
        "actions": actions,
        "target_kind": 0,
        "target_index": -1,
    })
}

/// One talk call's pages: the posted `here` tile and the posted npc page,
/// plus whatever else the call under test needs — the posted chat slots,
/// the posted count dialog, the posted hitpoints.
fn talk_scene(here_tile: Value, npcs: Value, extra: Value) -> Value {
    let mut scene = json!({ "here": here_tile, "npcs": npcs });
    for (key, value) in extra.as_object().expect("extra") {
        scene[key] = value.clone();
    }
    scene
}

/// The selected talk step these tests read.
fn talk_of(data: &SelectedGameData, id: i32) -> &TalkKeyTalkRow {
    talk_step(Some(data), id).unwrap_or_else(|| panic!("talk step {id}"))
}

/// The held page of one talk step.
fn talk_page(id: i32) -> Value {
    json!([[id, 1]])
}

/// The talk membership is the selected `talk_key.talk` family and nothing
/// else, and no talk step is a second classify of the landed arms: the
/// seven key keepers, the desc-only rows the family does not publish and
/// every search, guarded, dig and casket membership stay out.
#[test]
fn the_talk_membership_is_the_selected_talk_family_and_nothing_else() {
    on_reset();
    let data = selected();
    let talk = data.talk_key().expect("talk_key");
    assert_eq!(talk.talk.len(), 47, "the landed family");
    assert_eq!(
        talk.talk.iter().filter(|row| row.spawn.is_some()).count(),
        42,
        "the unique-spawn slice"
    );
    assert_eq!(
        talk.talk.iter().filter(|row| row.spawn.is_none()).count(),
        5,
        "the identity-only slice"
    );
    for step in &talk.talk {
        let row = row(&data, step.id);
        assert_eq!(
            talk_step(Some(&data), step.id).map(|talk| talk.id),
            Some(step.id),
            "{}",
            step.alias
        );
        // The talk arm is the last `Steady` arm, so a talk step is never
        // claimed by the search, guarded, unguarded-dig or casket classify.
        assert_eq!(search_tile(row), None, "{}", step.alias);
        assert_eq!(guarded_tile(row), None, "{}", step.alias);
        assert_eq!(dig_tile(row), None, "{}", step.alias);
        assert_eq!(casket_name(Some(&data), row), None, "{}", step.alias);
        // Every talk membership is a clue row the landed identify returns.
        assert_eq!(row.role, "clue", "{}", step.alias);
    }
    // The key keepers are not talk membership, and neither is any row the
    // family does not publish.
    for key in &talk.keys {
        assert_eq!(
            talk_step(Some(&data), key.id).map(|talk| talk.id),
            None,
            "{}",
            key.alias
        );
    }
    for id in [MAP_EMPTY, RIDDLE, SEARCH, UNGUARDED, GUARDED, CLUE, CASKET] {
        assert_eq!(talk_step(Some(&data), id).map(|talk| talk.id), None, "{id}");
    }
    // No selected pin is no talk step at all.
    assert_eq!(talk_step(None, TALK).map(|talk| talk.id), None);
}

/// The unique-spawn arm: the walk is the published `{x, z, plane}` and the
/// Talk-to is only ever a posted npc of this step's identity standing on
/// that tile. A wanderer, another identity, a wrong level and a row with no
/// talk action all wait at the tile.
#[test]
fn a_unique_spawn_talk_step_walks_to_the_published_tile_and_then_talks() {
    on_reset();
    let data = selected();
    let step = talk_of(&data, TALK);
    assert_eq!(step.npc.id, 0, "{}", step.alias);
    assert_eq!(step.npc.name, "Hans", "{}", step.alias);
    let spawn = step.spawn.as_ref().expect("a unique jm2 spawn");
    assert_eq!((spawn.x, spawn.z, spawn.plane), (3207, 3233, 0));

    let page = talk_page(TALK);
    let token = steady(&data, TALK);

    // No posted `here`: there is no arrival claim to make, so this tick
    // waits rather than walking blind.
    let blind = call(&data, token, page.clone(), json!({ "npcs": [] }));
    assert_eq!(blind["kind"], "wait", "{blind}");
    assert_eq!(token_of(&blind), token, "{blind}");

    // Posted far from the tile: the walk is the published tile, and the
    // `plane` is the verb's own `level`.
    let walked = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3200, 3233, 0), json!([]), json!({})),
    );
    assert_eq!(walked["kind"], "walk", "{walked}");
    assert_eq!(walked["x"], spawn.x, "{walked}");
    assert_eq!(walked["z"], spawn.z, "{walked}");
    assert_eq!(walked["level"], spawn.plane, "{walked}");

    // Arrived, with the step's own npc posted on the tile: the posted name
    // and posted action ride the verb with the posted scene index.
    let on_tile = json!([talk_npc(
        11,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 0
        },
        1,
        &["Talk-to"]
    )]);
    let talked = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), on_tile.clone(), json!({})),
    );
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["name"], "Hans", "{talked}");
    assert_eq!(talked["action"], "Talk-to", "{talked}");
    assert_eq!(talked["index"], 11, "{talked}");
    assert_eq!(token_of(&talked), token, "{talked}");

    // Identity is the packed id first: a posted row that carries it keeps
    // the page's own display name on the verb.
    let by_id = json!([talk_npc(
        12,
        0,
        "Someone Else",
        Tile {
            x: 3208,
            z: 3233,
            level: 0,
        },
        1,
        &["Talk-to"]
    )]);
    let renamed = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), by_id, json!({})),
    );
    assert_eq!(renamed["kind"], "npc", "{renamed}");
    assert_eq!(renamed["name"], "Someone Else", "{renamed}");
    assert_eq!(renamed["index"], 12, "{renamed}");

    // The frozen `talk_op` rule: the first posted action whose first four
    // characters are `talk`, emitted as the page posted it.
    let plain = json!([talk_npc(
        13,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 0,
        },
        1,
        &["Examine", "Talk"]
    )]);
    let action = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), plain, json!({})),
    );
    assert_eq!(action["kind"], "npc", "{action}");
    assert_eq!(action["action"], "Talk", "{action}");

    // A wanderer four tiles off the published tile is not this step's npc:
    // the arm keeps the tile and waits — no walk, no npc and no Clear.
    let wandered = json!([talk_npc(
        14,
        0,
        "Hans",
        Tile {
            x: 3211,
            z: 3233,
            level: 0
        },
        4,
        &["Talk-to"]
    )]);
    let waited = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), wandered, json!({})),
    );
    assert_eq!(waited["kind"], "wait", "{waited}");
    assert!(waited.get("x").is_none(), "{waited}");

    // Another identity on the tile is not this step's either, whatever it
    // is called and however close it stands.
    let other = json!([talk_npc(
        15,
        541,
        "Zeke",
        Tile {
            x: 3207,
            z: 3233,
            level: 0
        },
        1,
        &["Talk-to"]
    )]);
    let stranger = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), other, json!({})),
    );
    assert_eq!(stranger["kind"], "wait", "{stranger}");

    // No talk action on the posted row: not a row this arm dispatches at.
    let silent = json!([talk_npc(
        16,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 0
        },
        1,
        &["Examine"]
    )]);
    let no_action = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), silent, json!({})),
    );
    assert_eq!(no_action["kind"], "wait", "{no_action}");

    // Same level is part of the membership: the same npc posted one level
    // up is not the one this Dig-free spawn owns.
    let above = json!([talk_npc(
        17,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 1
        },
        1,
        &["Talk-to"]
    )]);
    let leveled = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 0), above, json!({})),
    );
    assert_eq!(leveled["kind"], "wait", "{leveled}");

    // Posted on another level entirely: arrival is same-level, so the arm
    // walks again rather than Talking-to anything.
    let off_level = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3207, 3233, 1), on_tile, json!({})),
    );
    assert_eq!(off_level["kind"], "walk", "{off_level}");
    assert_eq!(off_level["level"], 0, "{off_level}");

    // The published plane is the walk's level on the plane-1 sibling too.
    let plane = talk_of(&data, TALK_PLANE);
    let spawn = plane.spawn.as_ref().expect("a unique jm2 spawn");
    assert_eq!(spawn.plane, 1, "{}", plane.alias);
    let token = steady(&data, TALK_PLANE);
    let upstairs = call(
        &data,
        token,
        talk_page(TALK_PLANE),
        talk_scene(here(2485, 3488, 1), json!([]), json!({})),
    );
    assert_eq!(upstairs["kind"], "walk", "{upstairs}");
    assert_eq!(upstairs["level"], 1, "{upstairs}");
    assert_eq!(upstairs["x"], spawn.x, "{upstairs}");
    assert_eq!(upstairs["z"], spawn.z, "{upstairs}");
}

/// The identity-only arm: the nearest posted npc of this step's own
/// identity, walked to and then Talk-to'd, with no alias, no first-in-file
/// row and no frozen tile anywhere in the pick.
#[test]
fn an_identity_only_talk_step_picks_the_nearest_posted_match() {
    on_reset();
    let data = selected();
    let step = talk_of(&data, TALK_IDENTITY);
    assert!(step.spawn.is_none(), "{}", step.alias);
    assert_eq!(step.npc.id, 804, "{}", step.alias);
    assert_eq!(step.npc.name, "Tanner", "{}", step.alias);

    let page = talk_page(TALK_IDENTITY);
    let token = steady(&data, TALK_IDENTITY);

    // No posted match at all: a wait, and the token stays live.
    let none = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3200, 3200, 0), json!([]), json!({})),
    );
    assert_eq!(none["kind"], "wait", "{none}");
    assert_eq!(token_of(&none), token, "{none}");

    // No posted `here`: no distance to measure a posted row by, so the arm
    // waits instead of inventing a base.
    let unmeasured = call(
        &data,
        token,
        page.clone(),
        json!({ "npcs": [talk_npc(21, 804, "Tanner", Tile { x: 3200, z: 3204, level: 0 }, 1, &["Talk-to"])] }),
    );
    assert_eq!(unmeasured["kind"], "wait", "{unmeasured}");

    // Two rows of this identity, farthest posted first: the nearest wins
    // and the walk goes to that row's own tile, never the first in file.
    let near = talk_npc(
        22,
        804,
        "Tanner",
        Tile {
            x: 3200,
            z: 3205,
            level: 0,
        },
        5,
        &["Talk-to"],
    );
    let far = talk_npc(
        23,
        804,
        "Tanner",
        Tile {
            x: 3300,
            z: 3300,
            level: 0,
        },
        30,
        &["Talk-to"],
    );
    let pick = call(
        &data,
        token,
        page.clone(),
        talk_scene(here(3200, 3200, 0), json!([far, near]), json!({})),
    );
    assert_eq!(pick["kind"], "walk", "{pick}");
    assert_eq!(pick["x"], 3200, "{pick}");
    assert_eq!(pick["z"], 3205, "{pick}");
    assert_eq!(pick["level"], 0, "{pick}");

    // Arrived: the nearest posted row is inside the frozen radius, so the
    // verb is the Talk-to with that row's own posted identity.
    let arrived = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3200, 3205, 0),
            json!([
                talk_npc(
                    24,
                    999,
                    "Tanner",
                    Tile {
                        x: 3200,
                        z: 3206,
                        level: 0
                    },
                    3,
                    &["Talk-to"]
                ),
                talk_npc(
                    25,
                    804,
                    "Tanner",
                    Tile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                ),
            ]),
            json!({}),
        ),
    );
    assert_eq!(arrived["kind"], "npc", "{arrived}");
    assert_eq!(arrived["name"], "Tanner", "{arrived}");
    assert_eq!(arrived["action"], "Talk-to", "{arrived}");
    assert_eq!(arrived["index"], 25, "{arrived}");

    // Ties keep posted order: the scan only replaces on a strict
    // improvement.
    let tie = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3200, 3205, 0),
            json!([
                talk_npc(
                    26,
                    804,
                    "Tanner",
                    Tile {
                        x: 3201,
                        z: 3205,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                ),
                talk_npc(
                    27,
                    804,
                    "Tanner",
                    Tile {
                        x: 3200,
                        z: 3206,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                ),
            ]),
            json!({}),
        ),
    );
    assert_eq!(tie["index"], 26, "{tie}");

    // The posted `distance` is what the pick ranks by when the page
    // carries one: a row that reads near is the one that is Talk-to'd.
    let posted_near = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3200, 3205, 0),
            json!([
                talk_npc(
                    28,
                    804,
                    "Tanner",
                    Tile {
                        x: 3290,
                        z: 3290,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                ),
                talk_npc(
                    29,
                    804,
                    "Tanner",
                    Tile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    },
                    4,
                    &["Talk-to"]
                ),
            ]),
            json!({}),
        ),
    );
    assert_eq!(posted_near["index"], 28, "{posted_near}");

    // Another step's identity is not this one's, and neither is a row that
    // carries only this step's script alias — the join is the packed id
    // and the display name, never the alias and never a nearest anything.
    let wrong = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3200, 3205, 0),
            json!([
                talk_npc(
                    30,
                    541,
                    "Zeke",
                    Tile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                ),
                talk_npc(
                    31,
                    999,
                    "tanner",
                    Tile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    },
                    1,
                    &["Examine"]
                ),
            ]),
            json!({}),
        ),
    );
    assert_eq!(wrong["kind"], "wait", "{wrong}");

    // A matching row with no talk action is not a match, and a matching
    // row the page posted no tile or distance for is unmeasured.
    let silent = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3200, 3205, 0),
            json!([talk_npc(
                32,
                804,
                "Tanner",
                Tile {
                    x: 3200,
                    z: 3205,
                    level: 0
                },
                1,
                &["Examine"]
            )]),
            json!({}),
        ),
    );
    assert_eq!(silent["kind"], "wait", "{silent}");
    let mut unmeasured = talk_npc(
        33,
        804,
        "Tanner",
        Tile {
            x: 3200,
            z: 3205,
            level: 0,
        },
        2,
        &["Talk-to"],
    );
    unmeasured.as_object_mut().expect("row").remove("x");
    unmeasured.as_object_mut().expect("row").remove("z");
    unmeasured.as_object_mut().expect("row").remove("distance");
    let blind = call(
        &data,
        token,
        page,
        talk_scene(here(3200, 3205, 0), json!([unmeasured]), json!({})),
    );
    assert_eq!(blind["kind"], "wait", "{blind}");
}

/// The challenge seam: a page that holds only a selected challenge scroll
/// joins that scroll's parent talk step, and the posted count dialog is
/// answered with the selected answer. Empty, zero and unrelated pages still
/// abort `none-held`.
#[test]
fn a_held_challenge_scroll_joins_the_parent_talk_step_and_answers_the_count() {
    on_reset();
    let data = selected();
    let parent = talk_of(&data, TALK_CHALLENGE);
    assert_eq!(parent.npc.name, "Hazelmere", "{}", parent.alias);

    // Begin on the scroll alone: the identify is `none-held`, and the seam
    // hands out the parent's token rather than a refusal.
    let opened = begin(&data, talk_page(CHALLENGE));
    assert_eq!(opened["kind"], "token", "{opened}");
    let token = token_of(&opened);
    let gate = call(&data, token, talk_page(CHALLENGE), json!({}));
    assert_eq!(gate["kind"], "callback.enabled", "{gate}");
    let logged = call(
        &data,
        token,
        talk_page(CHALLENGE),
        json!({ "resume": true }),
    );
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let message = logged["message"].as_str().unwrap_or("");
    assert!(message.contains("trail_clue_medium_anagram001"), "{logged}");
    assert!(message.contains(&TALK_CHALLENGE.to_string()), "{logged}");
    assert!(
        !message.contains(&CHALLENGE.to_string()),
        "the step is the parent, never the scroll: {logged}"
    );
    let posted = call(&data, token, talk_page(CHALLENGE), json!({}));
    assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
    assert_eq!(
        RUNTIME.with(|rt| rt.borrow().step_id),
        TALK_CHALLENGE,
        "the scroll id is never the step"
    );

    // The parent's own talk path: arrived on the published plane-1 tile
    // with the parent's npc posted.
    let spawn = parent.spawn.as_ref().expect("a unique jm2 spawn");
    let hazelmere = json!([talk_npc(
        41,
        parent.npc.id,
        "Hazelmere",
        Tile {
            x: 2678,
            z: 3086,
            level: 1,
        },
        1,
        &["Talk-to"]
    )]);
    let talked = call(
        &data,
        token,
        talk_page(CHALLENGE),
        talk_scene(
            here(spawn.x, spawn.z, spawn.plane),
            hazelmere.clone(),
            json!({}),
        ),
    );
    assert_eq!(talked["kind"], "npc", "{talked}");
    assert_eq!(talked["index"], 41, "{talked}");

    // The posted count dialog is answered with the selected string, and
    // only with it: no Talk-to rides along.
    let answered = call(
        &data,
        token,
        talk_page(CHALLENGE),
        talk_scene(
            here(spawn.x, spawn.z, spawn.plane),
            hazelmere.clone(),
            json!({ "count_dialog_open": true }),
        ),
    );
    assert_eq!(answered["kind"], "answer-count", "{answered}");
    assert_eq!(answered["value"], 6859, "{answered}");
    assert_eq!(token_of(&answered), token, "{answered}");

    // A posted `false` is a closed dialog: the arm goes back to the talk
    // path rather than answering anything.
    let closed = call(
        &data,
        token,
        talk_page(CHALLENGE),
        talk_scene(
            here(spawn.x, spawn.z, spawn.plane),
            hazelmere,
            json!({ "count_dialog_open": false }),
        ),
    );
    assert_eq!(closed["kind"], "npc", "{closed}");

    // A talk step with no challenge of its own has no answer to give: the
    // open count dialog is a wait, never an invented number.
    let other = steady(&data, TALK);
    let no_answer = call(
        &data,
        other,
        talk_page(TALK),
        talk_scene(
            here(3207, 3233, 0),
            json!([]),
            json!({ "count_dialog_open": true }),
        ),
    );
    assert_eq!(no_answer["kind"], "wait", "{no_answer}");
    assert!(no_answer.get("value").is_none(), "{no_answer}");

    // The identity-only challenge parent joins the same way and answers
    // its own selected string.
    let zoo = begin(&data, talk_page(CHALLENGE_IDENTITY));
    assert_eq!(zoo["kind"], "token", "{zoo}");
    let zoo = token_of(&zoo);
    assert_eq!(
        call(&data, zoo, talk_page(CHALLENGE_IDENTITY), json!({}))["kind"],
        "callback.enabled"
    );
    assert_eq!(
        call(
            &data,
            zoo,
            talk_page(CHALLENGE_IDENTITY),
            json!({ "resume": true })
        )["kind"],
        "callback.log"
    );
    assert_eq!(
        call(&data, zoo, talk_page(CHALLENGE_IDENTITY), json!({}))["kind"],
        "callback.setStatus"
    );
    assert_eq!(
        RUNTIME.with(|rt| rt.borrow().step_id),
        TALK_CHALLENGE_IDENTITY
    );
    let zoo_answer = call(
        &data,
        zoo,
        talk_page(CHALLENGE_IDENTITY),
        json!({ "count_dialog_open": true }),
    );
    assert_eq!(zoo_answer["kind"], "answer-count", "{zoo_answer}");
    assert_eq!(zoo_answer["value"], 40, "{zoo_answer}");

    // The first posted selected scroll wins when a page holds two.
    let two = begin(&data, json!([[CHALLENGE, 1], [CHALLENGE_IDENTITY, 1]]));
    assert_eq!(two["kind"], "token", "{two}");
    assert_eq!(
        call(
            &data,
            token_of(&two),
            json!([[CHALLENGE, 1], [CHALLENGE_IDENTITY, 1]]),
            json!({})
        )["kind"],
        "callback.enabled"
    );
    assert_eq!(RUNTIME.with(|rt| rt.borrow().step_id), TALK_CHALLENGE);

    // The seam does not weaken C3: a zero count, an unrelated id and an
    // empty page are all still `none-held` refusals.
    let unrelated = unrelated(&data);
    for held in [
        json!([]),
        json!([[CHALLENGE, 0]]),
        json!([[unrelated, 1]]),
        json!([[CHALLENGE, -1]]),
    ] {
        let refused = begin(&data, held.clone());
        assert_eq!(refused["kind"], "aborted", "{held} {refused}");
        assert_eq!(refused["reason"], "none-held", "{held} {refused}");
    }
    // And the `one-held` sibling: an unrelated id beside the scroll is the
    // seam's page, because the membership rows still win first.
    let mixed = begin(&data, json!([[CASKET, 1], [CHALLENGE, 1]]));
    assert_eq!(mixed["kind"], "token", "{mixed}");
}

/// The parent clue swapping for its scroll keeps the live token: the
/// identify goes `none-held`, the seam returns the same parent step, and
/// `Collecting` still wins the page when the token is collecting.
#[test]
fn the_challenge_seam_keeps_the_live_token_on_the_parent_step() {
    on_reset();
    let data = selected();
    let token = steady(&data, TALK_CHALLENGE);
    // The server takes the parent clue and leaves the scroll: the token
    // survives and the step is still the parent's.
    let swapped = call(&data, token, talk_page(CHALLENGE), json!({ "npcs": [] }));
    assert_eq!(token_of(&swapped), token, "{swapped}");
    assert_ne!(swapped["kind"], "aborted", "{swapped}");
    assert_eq!(RUNTIME.with(|rt| rt.borrow().step_id), TALK_CHALLENGE);
    // The answer is still the parent's own.
    let answered = call(
        &data,
        token,
        talk_page(CHALLENGE),
        json!({ "count_dialog_open": true }),
    );
    assert_eq!(answered["kind"], "answer-count", "{answered}");
    assert_eq!(answered["value"], 6859, "{answered}");

    // Collecting first: a live collect is not a challenge join.
    let collecting = opened(&data, CASKET);
    let still = call(&data, collecting, talk_page(CHALLENGE), json!({}));
    assert_ne!(still["kind"], "aborted", "{still}");
    assert_eq!(token_of(&still), collecting, "{still}");
}

/// An open chat closes the talk arm for the tick: no Talk-to and no walk
/// while the landed `dialog_ready` holds, and no answer behind an
/// unobserved count dialog.
#[test]
fn no_talk_to_while_the_chat_or_the_count_dialog_is_posted_open() {
    on_reset();
    let data = selected();
    let page = talk_page(TALK);
    let token = steady(&data, TALK);
    let on_tile = json!([talk_npc(
        51,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 0
        },
        1,
        &["Talk-to"]
    )]);

    // The posted chat modal without a continue is still not a Talk-to,
    // and it does not walk either: `dialog_ready` waits the tick out.
    {
        let extra = json!({ "chat_modal_id": 968 });
        let open = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), on_tile.clone(), extra.clone()),
        );
        assert_eq!(open["kind"], "wait", "{extra} {open}");
        let far = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3100, 3233, 0), on_tile.clone(), extra.clone()),
        );
        assert_eq!(far["kind"], "wait", "{extra} {far}");
    }

    // A posted continue is the frozen `drainChat`: it goes out before the
    // Talk-to, arrived or far, and is never a wait behind an open chat.
    for extra in [
        json!({ "chat_continue": true }),
        json!({ "chat_modal_id": 968, "chat_continue": true }),
    ] {
        let open = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), on_tile.clone(), extra.clone()),
        );
        assert_eq!(open["kind"], "continue", "{extra} {open}");
        let far = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3100, 3233, 0), on_tile.clone(), extra.clone()),
        );
        assert_eq!(far["kind"], "continue", "{extra} {far}");
    }

    // The posted closed chat is not an open one, and an omitted slot is
    // unobserved rather than open: both leave the Talk-to free.
    for extra in [
        json!({ "chat_modal_id": -1, "chat_continue": false }),
        json!({ "chat_continue": false }),
        json!({}),
    ] {
        let free = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), on_tile.clone(), extra.clone()),
        );
        assert_eq!(free["kind"], "npc", "{extra} {free}");
    }

    // A posted open count dialog blocks the Talk-to the same way — and for
    // a step with no challenge of its own there is nothing to answer.
    let counted = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3207, 3233, 0),
            on_tile.clone(),
            json!({ "count_dialog_open": true }),
        ),
    );
    assert_eq!(counted["kind"], "wait", "{counted}");

    // The landed precedence is untouched by the talk arm: the posted
    // interrupt yields, the frozen clock waits, and a posted hitpoints at
    // or below zero is `dead` — never `done` and never `'clue solved'`.
    let yielded = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3207, 3233, 0),
            on_tile.clone(),
            json!({ "hold": true }),
        ),
    );
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    let dead = call(
        &data,
        token,
        page.clone(),
        talk_scene(
            here(3207, 3233, 0),
            on_tile.clone(),
            json!({ "hitpoints": 0 }),
        ),
    );
    assert_eq!(dead["kind"], "dead", "{dead}");
    assert!(dead["token"].is_number(), "{dead}");
    assert!(!dead.to_string().contains("clue solved"), "{dead}");
    let after = call(
        &data,
        token,
        page,
        talk_scene(here(3207, 3233, 0), on_tile, json!({})),
    );
    assert_eq!(after["reason"], "stale", "{after}");
}

/// The frozen `drainChat` at the head of `Steady`: a posted continue with
/// no option list is sent before the Talk-to, the search walk and the
/// casket Open. A posted option list is not drained, and a count dialog
/// still belongs to the talk arm.
#[test]
fn steady_drains_a_posted_continue_before_the_step_verb() {
    on_reset();
    let data = selected();

    let token = steady(&data, TALK);
    let on_tile = json!([talk_npc(
        51,
        0,
        "Hans",
        Tile {
            x: 3207,
            z: 3233,
            level: 0
        },
        1,
        &["Talk-to"]
    )]);
    let continued = call(
        &data,
        token,
        talk_page(TALK),
        talk_scene(
            here(3207, 3233, 0),
            on_tile.clone(),
            json!({ "chat_continue": true }),
        ),
    );
    assert_eq!(continued["kind"], "continue", "{continued}");

    let token = steady(&data, TALK);
    let listed = call(
        &data,
        token,
        talk_page(TALK),
        talk_scene(
            here(3207, 3233, 0),
            on_tile,
            json!({
                "chat_continue": true,
                "chat_options": [option("I seek a challenge.", 1)],
            }),
        ),
    );
    assert_eq!(listed["kind"], "wait", "{listed}");

    let token = steady(&data, SEARCH);
    let searched = call(
        &data,
        token,
        json!([[SEARCH, 1]]),
        json!({
            "here": { "x": 2000, "z": 2000, "level": 0 },
            "chat_continue": true,
        }),
    );
    assert_eq!(searched["kind"], "continue", "{searched}");

    let token = steady(&data, EASY_CASKET);
    let opened = call(
        &data,
        token,
        json!([[EASY_CASKET, 1]]),
        json!({ "chat_continue": true }),
    );
    assert_eq!(opened["kind"], "continue", "{opened}");
    let token = steady(&data, EASY_CASKET);
    let held = call(&data, token, json!([[EASY_CASKET, 1]]), json!({}));
    assert_eq!(held["kind"], "held", "{held}");
    assert_eq!(held["action"], "Open", "{held}");
}

/// The talk arm's own outcome set: the walk, the Talk-to, the answer and
/// the waits, and never a verb or a completion that belongs to another arm.
#[test]
fn the_talk_arm_emits_only_walk_npc_answer_count_wait_and_yield() {
    on_reset();
    let data = selected();
    let scenes = [
        (TALK, json!({ "npcs": [] }), "no page"),
        (
            TALK,
            talk_scene(here(3100, 3233, 0), json!([]), json!({})),
            "far",
        ),
        (
            TALK,
            talk_scene(
                here(3207, 3233, 0),
                json!([talk_npc(
                    61,
                    0,
                    "Hans",
                    Tile {
                        x: 3207,
                        z: 3233,
                        level: 0
                    },
                    1,
                    &["Talk-to"]
                )]),
                json!({}),
            ),
            "arrived",
        ),
        (
            TALK_IDENTITY,
            talk_scene(here(3200, 3200, 0), json!([]), json!({})),
            "no match",
        ),
        (
            TALK_CHALLENGE,
            talk_scene(
                here(2678, 3086, 1),
                json!([]),
                json!({ "count_dialog_open": true }),
            ),
            "count",
        ),
    ];
    let mut kinds = Vec::new();
    for (id, scene, name) in scenes {
        let token = steady(&data, id);
        let step = call(&data, token, talk_page(id), scene.clone());
        assert_eq!(token_of(&step), token, "{name} {step}");
        kinds.push(step["kind"].as_str().unwrap_or("").to_string());
        let text = step.to_string();
        for forbidden in ["clue solved", "done", "abandon", "supplies-needed"] {
            assert!(!text.contains(forbidden), "{name} {step}");
        }
    }
    assert_eq!(
        kinds,
        vec!["wait", "walk", "npc", "wait", "answer-count"],
        "{kinds:?}"
    );
    // The same step driven through the collect's `'clue solved'` is not
    // this arm's: a talk step never reaches a completion kind.
    for kind in &kinds {
        assert!(
            ![
                "done",
                "grind-ready",
                "held",
                "loc",
                "obj",
                "if-button",
                "close-modal",
                "puzzle-move",
                "dead",
                "guardian-lost",
                "aborted"
            ]
            .contains(&kind.as_str()),
            "{kind}"
        );
    }
    // `status` is still the machine's continue shape on every one of them,
    // and `dead` never posts the solved string.
    let token = steady(&data, TALK);
    let dead = call(
        &data,
        token,
        talk_page(TALK),
        talk_scene(here(3207, 3233, 0), json!([]), json!({ "hitpoints": 0 })),
    );
    assert_eq!(dead["kind"], "dead", "{dead}");
    assert!(!dead.to_string().contains("clue solved"), "{dead}");
}

/// The key-hunt step of one of the two unique-spawn type keepers.
fn key_of(data: &SelectedGameData, id: i32) -> &TalkKeyKeyRow {
    key_step(Some(data), id).unwrap_or_else(|| panic!("key keeper {id}"))
}

/// The published spawn tile a key row's walk carries.
fn spawn_of(key: &TalkKeyKeyRow) -> Tile {
    let spawn = key.spawn.as_ref().expect("a unique jm2 spawn");
    Tile {
        x: spawn.x,
        z: spawn.z,
        level: spawn.plane,
    }
}

/// One key-hunt call's pages: the posted `here` tile, the posted npc page,
/// the posted ground page, the posted pack rows with their slot count, and
/// the posted local-player slot the kill's own `targetsMe` read compares
/// with. Everything the hunt does not need this call is left empty, so each
/// test names only the page it is about.
fn key_scene(here_tile: Value, extra: Value) -> Value {
    let mut scene = json!({
        "here": here_tile,
        "npcs": [],
        "ground": [],
        "inv": [],
        "inv_size": 28,
        "self_slot": 0,
    });
    for (key, value) in extra.as_object().expect("extra") {
        scene[key] = value.clone();
    }
    scene
}

/// One wrapper-marshalled posted npc row as the keeper hunt reads it: the
/// posted index the Attack carries, the packed id and posted display name
/// the identity join compares, the posted tile and distance the published
/// spawn's radius is measured by, the posted health pair the kill is read
/// through, and the posted action list the `Attack` is read from.
fn keeper_npc(
    index: i32,
    id: i32,
    name: &str,
    tile: Tile,
    distance: i32,
    actions: &[&str],
) -> Value {
    json!({
        "index": index,
        "id": id,
        "name": name,
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
        "distance": distance,
        "health": 10,
        "max_health": 10,
        "in_combat": false,
        "actions": actions,
        "target_kind": 0,
        "target_index": -1,
    })
}

/// The owned keeper's last-seen aged past the frozen grace: the only way to
/// reach the gone-outside-grace read without a six-second test.
fn age_keeper_seen(ms: u64) {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        if let Some(owned) = rt.keeper.as_mut().and_then(|keeper| keeper.owned.as_mut()) {
            owned.seen_at -= Duration::from_millis(ms);
        }
    });
}

/// A freeze that outlasted the remaining keeper grace, without a
/// six-second test: the clock's own `frozen_at` and the owned keeper's
/// last-seen are both placed at the freeze's start, so the reclaim the thaw
/// makes is exactly the frozen interval — the shape a real long freeze
/// hands the session.
fn froze_across_the_keeper_grace() {
    on_pause();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let frozen_at = Instant::now() - Duration::from_millis(KILL_GRACE_MS + 1);
        rt.clock.frozen_at = Some(frozen_at);
        if let Some(owned) = rt.keeper.as_mut().and_then(|keeper| keeper.owned.as_mut()) {
            owned.seen_at = frozen_at;
        }
    });
    on_resume();
}

/// The key-hunt membership is the selected `talk_key.keys` family and
/// nothing else: the two unique-spawn type keepers are the hunt's steps,
/// the five matcher-keepers publish no unique spawn and are not, and
/// neither family's step is the other's — a key keeper is never a talk step
/// and a talk step is never hunted.
#[test]
fn the_key_membership_is_the_selected_keys_family_and_nothing_else() {
    on_reset();
    let data = selected();
    let talk = data.talk_key().expect("talk_key");
    assert_eq!(talk.keys.len(), 7, "the landed family");
    assert_eq!(
        talk.keys.iter().filter(|key| key.spawn.is_some()).count(),
        2,
        "the unique-spawn slice"
    );
    for key in &talk.keys {
        let row = row(&data, key.id);
        assert_eq!(row.role, "clue", "{}", key.alias);
        // Every key row is a membership row the landed identify returns,
        // and none is a second classify of the arms ahead of the hunt.
        assert_eq!(
            key_step(Some(&data), key.id).map(|key| key.id),
            Some(key.id),
            "{}",
            key.alias
        );
        assert_eq!(search_tile(row), None, "{}", key.alias);
        assert_eq!(guarded_tile(row), None, "{}", key.alias);
        assert_eq!(dig_tile(row), None, "{}", key.alias);
        assert_eq!(casket_name(Some(&data), row), None, "{}", key.alias);
        assert_eq!(
            talk_step(Some(&data), key.id).map(|talk| talk.id),
            None,
            "{}",
            key.alias
        );
    }
    // The two slice rows: one packed-type keeper each, on a published
    // spawn, with the pinned key the arm Takes.
    for (id, key_id, keeper, name, spawn) in [
        (
            RIDDLE,
            KEEPER_KEY,
            KEEPER_ID,
            KEEPER_NAME,
            Tile {
                x: 3039,
                z: 3700,
                level: 0,
            },
        ),
        (
            PENDA,
            PENDA_KEY,
            PENDA_ID,
            PENDA_NAME,
            Tile {
                x: 2910,
                z: 3539,
                level: 0,
            },
        ),
    ] {
        let step = key_of(&data, id);
        assert_eq!(step.key_id, key_id, "{}", step.alias);
        assert_eq!(
            keeper_type(&step.keeper),
            Some((keeper, name)),
            "{}",
            step.alias
        );
        assert_eq!(spawn_of(step), spawn, "{}", step.alias);
    }
    // The five matcher-keepers publish no unique spawn, so not one of them
    // is a hunt: two name a packed type the family covered for a
    // non-unique jm2 NPC spawn, and three a category or a bare name that
    // is not one npc at all. No coordinate is invented for the first two
    // and no type list for the other three.
    for id in MATCHER_KEEPERS {
        assert!(key_of(&data, id).spawn.is_none(), "{id}");
    }
    for id in [2833, 2835] {
        assert!(keeper_type(&key_of(&data, id).keeper).is_some(), "{id}");
    }
    for id in [2837, 2839, 3605] {
        assert_eq!(keeper_type(&key_of(&data, id).keeper), None, "{id}");
    }
    // No selected pin is no key step, and neither is a talk step or any
    // other landed membership.
    assert_eq!(key_step(None, RIDDLE).map(|key| key.id), None);
    for id in [
        TALK,
        TALK_IDENTITY,
        MAP_EMPTY,
        SEARCH,
        UNGUARDED,
        GUARDED,
        CLUE,
        CASKET,
    ] {
        assert_eq!(key_step(Some(&data), id).map(|key| key.id), None, "{id}");
    }
}

/// The two unique-spawn type keepers driven through the whole hunt: the
/// walk to the published spawn with its `plane` as the verb's `level`, the
/// one Attack on the posted keeper, the kill, the Take of the key it drops,
/// and the idle the original riddle keeps once that key is on the posted
/// pack page.
#[test]
fn a_key_keeper_step_walks_attacks_and_takes_the_key_it_drops() {
    on_reset();
    let data = selected();
    for (id, key_id, keeper, name, spawn) in [
        (
            RIDDLE,
            KEEPER_KEY,
            KEEPER_ID,
            KEEPER_NAME,
            Tile {
                x: 3039,
                z: 3700,
                level: 0,
            },
        ),
        (
            PENDA,
            PENDA_KEY,
            PENDA_ID,
            PENDA_NAME,
            Tile {
                x: 2910,
                z: 3539,
                level: 0,
            },
        ),
    ] {
        let page = json!([[id, 1]]);
        let token = steady(&data, id);
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, keeper, name, spawn, 1, &[ATTACK]);

        // Not arrived: the walk is the published spawn itself, repeating
        // until the posted `here` holds.
        let walked = call(
            &data,
            token,
            page.clone(),
            key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({})),
        );
        assert_eq!(walked["kind"], "walk", "{id} {walked}");
        assert_eq!(walked["x"], spawn.x, "{id} {walked}");
        assert_eq!(walked["z"], spawn.z, "{id} {walked}");
        assert_eq!(walked["level"], spawn.level, "{id} {walked}");

        // Arrived with the keeper posted on its own tile: the one Attack,
        // carrying the posted name, the frozen action and the posted scene
        // index and nothing else.
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(attacked["kind"], "npc", "{id} {attacked}");
        assert_eq!(attacked["name"], name, "{id} {attacked}");
        assert_eq!(attacked["action"], ATTACK, "{id} {attacked}");
        assert_eq!(attacked["index"], 21, "{id} {attacked}");
        for absent in ["component_id", "id", "x", "z", "level", "message"] {
            assert!(attacked.get(absent).is_none(), "{id} {absent} {attacked}");
        }

        // Posted and alive: the hunt waits, and the Attack is never issued
        // twice for one owned index.
        let alive = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(alive["kind"], "wait", "{id} {alive}");
        assert_eq!(token_of(&alive), token, "{id} {alive}");

        // The kill: the owned index posted at zero health beside a posted
        // maximum with this token's own fight on it. The key is already on
        // the tile, so the kill lets the Take out on this same call.
        let dying = targeting(field(posted.clone(), "health", json!(0)), 0);
        let dropped = ground(key_id, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
        let taken = call(
            &data,
            token,
            page.clone(),
            key_scene(
                arrived.clone(),
                json!({ "npcs": [dying], "ground": [dropped.clone()] }),
            ),
        );
        assert_eq!(taken["kind"], "obj", "{id} {taken}");
        assert_eq!(taken["x"], spawn.x, "{id} {taken}");
        assert_eq!(taken["z"], spawn.z, "{id} {taken}");
        assert_eq!(taken["level"], spawn.level, "{id} {taken}");
        assert_eq!(taken["name"], "Key", "{id} {taken}");
        assert_eq!(taken["action"], TAKE, "{id} {taken}");

        // The key on the posted page ends the hunt: the original riddle
        // idles, the gate is not re-armed and no completion kind is
        // emitted — whatever the page still posts beside the key. The key
        // rides the same `(id, count)` page the identify reads, exactly as
        // the wrapper posts the pack page it is built from.
        let keyed = json!([[id, 1], [key_id, 1]]);
        for extra in [
            json!({ "npcs": [posted.clone()], "ground": [dropped.clone()] }),
            json!({}),
        ] {
            let mut scene = key_scene(arrived.clone(), extra);
            scene["inv"] = json!([inv(key_id, "Key", 1)]);
            let idle = call(&data, token, keyed.clone(), scene);
            assert_eq!(idle["kind"], "wait", "{id} {idle}");
            assert_eq!(token_of(&idle), token, "{id} {idle}");
            let text = idle.to_string();
            for forbidden in [
                "clue solved",
                "grind-ready",
                "\"done\"",
                "guardian-lost",
                "abandon",
                "supplies-needed",
            ] {
                assert!(!text.contains(forbidden), "{id} {forbidden} {idle}");
            }
        }
    }
}

/// The owned keeper that leaves the posted page inside the frozen grace is
/// this token's kill even without a posted zero health — and the kill walks
/// back to the published spawn before it Takes anything. A page that posted
/// no npc page at all cannot observe the keeper, so the hunt waits.
#[test]
fn the_owned_keeper_gone_inside_the_grace_is_the_kill_and_walks_back() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let token = steady(&data, RIDDLE);
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    let attacked = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted] })),
    );
    assert_eq!(attacked["kind"], "npc", "{attacked}");

    // No posted npc page this call: the owned keeper cannot be read, so
    // nothing walks and nothing is Taken behind it.
    let blind = call(
        &data,
        token,
        page.clone(),
        json!({ "here": here(spawn.x - 30, spawn.z, spawn.level) }),
    );
    assert_eq!(blind["kind"], "wait", "{blind}");

    // The owned index left the page inside the grace, from thirty tiles
    // off: the kill, and the walk back to the published spawn.
    let gone = call(
        &data,
        token,
        page.clone(),
        key_scene(
            here(spawn.x - 30, spawn.z, spawn.level),
            json!({ "npcs": [] }),
        ),
    );
    assert_eq!(gone["kind"], "walk", "{gone}");
    assert_eq!(gone["x"], spawn.x, "{gone}");
    assert_eq!(gone["z"], spawn.z, "{gone}");
    assert_eq!(gone["level"], spawn.level, "{gone}");

    // Arrived: the key the kill dropped one step off the spawn is Taken
    // with the landed `obj`, at the row's own posted tile.
    let dropped = ground(
        KEEPER_KEY,
        "Key",
        spawn.x + 1,
        spawn.z,
        spawn.level,
        &[TAKE],
    );
    let taken = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived, json!({ "ground": [dropped] })),
    );
    assert_eq!(taken["kind"], "obj", "{taken}");
    assert_eq!(taken["x"], spawn.x + 1, "{taken}");
    assert_eq!(taken["z"], spawn.z, "{taken}");
    assert_eq!(taken["level"], spawn.level, "{taken}");
    assert_eq!(taken["action"], TAKE, "{taken}");
}

/// The owned keeper gone outside the frozen grace without ever being seen
/// at zero health is not a kill: this hunt has no `guardian-lost`, no
/// invented respawn timer and no second kind. The token waits, and a key
/// already on the floor is not Taken before the kill it belongs to.
#[test]
fn a_keeper_gone_outside_the_grace_is_a_wait_and_never_a_lost_encounter() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let token = steady(&data, RIDDLE);
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    let attacked = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted] })),
    );
    assert_eq!(attacked["kind"], "npc", "{attacked}");

    age_keeper_seen(KILL_GRACE_MS + 1);
    let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
    let idle = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "ground": [dropped] })),
    );
    assert_eq!(idle["kind"], "wait", "{idle}");
    assert_eq!(token_of(&idle), token, "{idle}");
    let text = idle.to_string();
    for forbidden in ["guardian-lost", "keeper-lost", "obj", "\"done\"", "abandon"] {
        assert!(!text.contains(forbidden), "{forbidden} {idle}");
    }
}

/// A freeze that outlasted the remaining keeper grace still ends in the
/// kill: the thaw reclaims the frozen interval into the owned last-seen, so
/// the disappearance that follows it is read as this token's kill and the
/// key it left behind is Taken.
#[test]
fn a_freeze_across_the_keeper_grace_still_ends_in_the_kill() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let token = steady(&data, RIDDLE);
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    let attacked = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted] })),
    );
    assert_eq!(attacked["kind"], "npc", "{attacked}");

    froze_across_the_keeper_grace();
    let kill = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [] })),
    );
    assert_eq!(kill["kind"], "wait", "{kill}");
    assert!(!kill.to_string().contains("guardian-lost"), "{kill}");

    let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
    let taken = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived, json!({ "ground": [dropped] })),
    );
    assert_eq!(taken["kind"], "obj", "{taken}");
    assert_eq!(taken["action"], TAKE, "{taken}");
}

/// The key Take is the published spawn's own read: the posted ground row
/// must carry the key's own id, the posted `Take`, a posted name, the
/// spawn's own level and a tile inside `ARRIVE_RADIUS` of that spawn — and
/// the pack must have room for it. A full pack waits, because this arm
/// Drops no food, and a page that posted no slot count waits too.
#[test]
fn the_key_take_reads_the_posted_key_row_at_the_spawn() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let token = steady(&data, RIDDLE);
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    // The hunt's own Attack first: the pickup is never armed without the
    // kill this token's Attack went out for.
    let attacked = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
    );
    assert_eq!(attacked["kind"], "npc", "{attacked}");
    let dying = targeting(field(posted, "health", json!(0)), 0);
    // The kill, with nothing on the floor: the pickup is armed and waits.
    let kill = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [dying] })),
    );
    assert_eq!(kill["kind"], "wait", "{kill}");

    for (name, row) in [
        (
            "no Take",
            ground(
                KEEPER_KEY,
                "Key",
                spawn.x,
                spawn.z,
                spawn.level,
                &["Examine"],
            ),
        ),
        (
            "another id",
            ground(
                KEEPER_KEY + 1,
                "Key",
                spawn.x,
                spawn.z,
                spawn.level,
                &[TAKE],
            ),
        ),
        (
            "another level",
            ground(
                KEEPER_KEY,
                "Key",
                spawn.x,
                spawn.z,
                spawn.level + 1,
                &[TAKE],
            ),
        ),
        (
            "off the radius",
            ground(
                KEEPER_KEY,
                "Key",
                spawn.x + 2,
                spawn.z,
                spawn.level,
                &[TAKE],
            ),
        ),
        (
            "no name",
            ground(KEEPER_KEY, "", spawn.x, spawn.z, spawn.level, &[TAKE]),
        ),
    ] {
        let idle = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "ground": [row] })),
        );
        assert_eq!(idle["kind"], "wait", "{name} {idle}");
        assert_eq!(token_of(&idle), token, "{name} {idle}");
    }

    // A full pack: the Take waits rather than Dropping a food row for it.
    let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
    let full = call(
        &data,
        token,
        page.clone(),
        key_scene(
            arrived.clone(),
            json!({
                "ground": [dropped.clone()],
                "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
                "inv_size": 1,
            }),
        ),
    );
    assert_eq!(full["kind"], "wait", "{full}");

    // No posted slot count: the pack's fullness is not invented for it.
    let mut unknown = key_scene(arrived.clone(), json!({ "ground": [dropped.clone()] }));
    unknown.as_object_mut().expect("object").remove("inv_size");
    let waited = call(&data, token, page.clone(), unknown);
    assert_eq!(waited["kind"], "wait", "{waited}");

    // Room in the pack and the key on the spawn: the landed Take.
    let taken = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived, json!({ "ground": [dropped] })),
    );
    assert_eq!(taken["kind"], "obj", "{taken}");
    assert_eq!(taken["name"], "Key", "{taken}");
    assert_eq!(taken["action"], TAKE, "{taken}");
}

/// The key this keeper drops is the whole end of the hunt: on the posted
/// pack page the original riddle idles — no walk, no Attack, no gate
/// re-arm, no new kind, and never `'clue solved'`, `grind-ready` or `done`.
#[test]
fn a_held_key_ends_the_hunt_with_the_original_riddles_idle() {
    on_reset();
    let data = selected();
    for (id, key_id, keeper, name, spawn) in [
        (
            RIDDLE,
            KEEPER_KEY,
            KEEPER_ID,
            KEEPER_NAME,
            Tile {
                x: 3039,
                z: 3700,
                level: 0,
            },
        ),
        (
            PENDA,
            PENDA_KEY,
            PENDA_ID,
            PENDA_NAME,
            Tile {
                x: 2910,
                z: 3539,
                level: 0,
            },
        ),
    ] {
        let token = steady(&data, id);
        let posted = keeper_npc(21, keeper, name, spawn, 1, &[ATTACK]);
        // The keeper posted on its own spawn and the key already on the
        // posted page: no Attack. And a page with no `here` at all: no
        // walk.
        let keyed = json!([[id, 1], [key_id, 1]]);
        for extra in [
            key_scene(
                here(spawn.x, spawn.z, spawn.level),
                json!({ "npcs": [posted] }),
            ),
            json!({}),
        ] {
            let idle = call(&data, token, keyed.clone(), extra);
            assert_eq!(idle["kind"], "wait", "{id} {idle}");
            assert_eq!(token_of(&idle), token, "{id} {idle}");
            assert_eq!(idle["token"], json!(token), "{id} {idle}");
            for absent in ["x", "z", "level", "name", "action", "index"] {
                assert!(idle.get(absent).is_none(), "{id} {absent} {idle}");
            }
        }
    }
}

/// The hunt's own outcome set: the walk, the Attack, the key Take and the
/// idle `wait` — plus the yield the posted cooperative interrupt still wins
/// with. No Protect from Magic click, no Spade Dig, no collect verb and no
/// completion kind is ever this arm's.
#[test]
fn the_key_hunt_emits_only_walk_npc_obj_wait_and_yield() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    let dying = targeting(field(posted.clone(), "health", json!(0)), 0);
    let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
    let scenes = [
        // Not arrived: the walk.
        key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({})),
        // Arrived with the keeper posted: the one Attack.
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        // Posted and alive: the wait.
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        // The kill and the key it dropped: the Take.
        key_scene(
            arrived.clone(),
            json!({ "npcs": [dying], "ground": [dropped.clone()] }),
        ),
        // The key in hand: the original riddle's idle.
        key_scene(
            arrived.clone(),
            json!({ "inv": [inv(KEEPER_KEY, "Key", 1)] }),
        ),
    ];
    let token = steady(&data, RIDDLE);
    let mut kinds = Vec::new();
    for scene in scenes {
        let step = call(&data, token, page.clone(), scene);
        assert_eq!(token_of(&step), token, "{step}");
        kinds.push(step["kind"].as_str().unwrap_or("").to_string());
    }
    assert_eq!(
        kinds,
        vec!["walk", "npc", "wait", "obj", "wait"],
        "{kinds:?}"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| !["walk", "npc", "obj", "wait"].contains(&kind.as_str()))
            .count(),
        0,
        "{kinds:?}"
    );

    // The posted `hold || ours` interrupt still wins over the hunt.
    let mut interrupt = key_scene(arrived.clone(), json!({ "npcs": [posted] }));
    interrupt["hold"] = json!(true);
    let yielded = call(&data, token, page.clone(), interrupt);
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    assert_eq!(token_of(&yielded), token, "{yielded}");
    assert!(!yielded.to_string().contains("clue solved"), "{yielded}");
}

/// The frozen clock, the posted hitpoints and the token's own end keep
/// their precedence over the hunt: a frozen call emits no walk and no
/// Attack, and a posted effective hitpoints at zero is the `dead` terminal
/// — never `done`, and never a completion kind.
#[test]
fn freeze_yield_and_death_beat_the_key_hunt() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let token = steady(&data, RIDDLE);
    let danger = key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({}));
    on_pause();
    let paused = call(&data, token, page.clone(), danger.clone());
    assert_eq!(paused["kind"], "wait", "{paused}");
    on_resume();
    on_hold(true);
    let held_clock = call(&data, token, page.clone(), danger.clone());
    assert_eq!(held_clock["kind"], "wait", "{held_clock}");
    on_hold(false);
    for step in [&paused, &held_clock] {
        for absent in ["x", "z", "level", "action", "name", "index"] {
            assert!(step.get(absent).is_none(), "{absent} {step}");
        }
        assert_eq!(token_of(step), token, "{step}");
    }
    // Thawed and unheld, the walk is still there.
    let walked = call(&data, token, page.clone(), danger.clone());
    assert_eq!(walked["kind"], "walk", "{walked}");

    // The posted effective hitpoints read zero: the terminal wins over the
    // walk, the Attack and the Take alike.
    let dead = call(
        &data,
        token,
        page.clone(),
        key_scene(
            here(spawn.x, spawn.z, spawn.level),
            json!({
                "npcs": [keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK])],
                "inv": [inv(KEEPER_KEY, "Key", 1)],
                "hitpoints": 0,
            }),
        ),
    );
    assert_eq!(dead["kind"], "dead", "{dead}");
    assert!(!dead.to_string().contains("clue solved"), "{dead}");
    assert!(!dead.to_string().contains("\"done\""), "{dead}");
    let after = call(&data, token, page, danger);
    assert_eq!(after["kind"], "aborted", "{after}");
    assert_eq!(after["reason"], "stale", "{after}");
}

/// A different held step drops the live hunt with the step: coming back to
/// the key riddle walks and Attacks the posted keeper again rather than
/// resuming the index the previous session owned.
#[test]
fn a_different_held_step_drops_the_key_hunt() {
    on_reset();
    let data = selected();
    let page = json!([[RIDDLE, 1]]);
    let spawn = spawn_of(key_of(&data, RIDDLE));
    let arrived = here(spawn.x, spawn.z, spawn.level);
    let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
    let token = steady(&data, RIDDLE);
    let attacked = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
    );
    assert_eq!(attacked["kind"], "npc", "{attacked}");

    // A different membership row is held: the landed gate re-arms for it.
    let other = json!([[MAP_EMPTY, 1]]);
    let re_armed = call(
        &data,
        token,
        other.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
    );
    assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
    let logged = call(&data, token, other.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let _ = call(&data, token, other.clone(), json!({}));

    // Back to the key riddle: the gate re-arms again, and the next steady
    // call is the hunt's own start — the posted keeper is Attacked again
    // rather than read as the old session's kill.
    let back = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
    );
    assert_eq!(back["kind"], "callback.enabled", "{back}");
    let logged = call(&data, token, page.clone(), json!({ "resume": true }));
    assert_eq!(logged["kind"], "callback.log", "{logged}");
    let _ = call(&data, token, page.clone(), json!({}));
    let reborn = call(
        &data,
        token,
        page.clone(),
        key_scene(arrived, json!({ "npcs": [posted] })),
    );
    assert_eq!(reborn["kind"], "npc", "{reborn}");
    assert_eq!(reborn["action"], ATTACK, "{reborn}");
    assert_eq!(reborn["index"], 21, "{reborn}");
}
// ── the gate-toll shop trip ──

/// A posted navigator short for the selected Shantay pass.
fn shantay_short(data: &SelectedGameData) -> Value {
    let pass = data
        .item_by_alias(SHANTAY_PASS)
        .expect("selected Shantay pass");
    json!([{ "id": pass.id, "name": pass.name, "count": 1 }])
}

/// A posted Shantay row at the selected spawn, with the action the toll trip
/// is allowed to dispatch.
fn shantay_keeper() -> Value {
    json!({
        "index": 12,
        "id": SHANTAY_NPC_ID,
        "name": SHANTAY_NAME,
        "x": SHANTAY_X,
        "z": SHANTAY_Z,
        "level": SHANTAY_LEVEL,
        "distance": 1,
        "actions": [TRADE],
    })
}

/// The failed search walk is intercepted once, shops exactly the nominated
/// pass, closes the posted shop and resumes the original destination.
#[test]
fn gate_toll_buys_the_named_pass_once_and_walks_the_original_dest_back() {
    on_reset();
    let data = selected();
    let pass = data
        .item_by_alias(SHANTAY_PASS)
        .expect("selected Shantay pass");
    let page = json!([[SEARCH, 1]]);
    let packed = json!([[SEARCH, 1], [pass.id, 1]]);
    let short = shantay_short(&data);
    let away = here(3200, 3218, 1);
    let spawn = here(SHANTAY_X, SHANTAY_Z, SHANTAY_LEVEL);
    let token = steady(&data, SEARCH);

    let original = call(
        &data,
        token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(original["kind"], "walk", "{original}");
    assert_eq!(
        (&original["x"], &original["z"], &original["level"]),
        (&json!(3209), &json!(3218), &json!(1)),
        "{original}"
    );

    let to_shop = call(
        &data,
        token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(to_shop["kind"], "walk", "{to_shop}");
    assert_eq!(
        (&to_shop["x"], &to_shop["z"], &to_shop["level"]),
        (&json!(SHANTAY_X), &json!(SHANTAY_Z), &json!(SHANTAY_LEVEL)),
        "{to_shop}"
    );

    let trade = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": spawn,
            "npcs": [shantay_keeper()],
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(trade["kind"], "npc", "{trade}");
    assert_eq!(trade["name"], SHANTAY_NAME, "{trade}");
    assert_eq!(trade["action"], TRADE, "{trade}");
    assert_eq!(trade["index"], 12, "{trade}");

    let buy = call(
        &data,
        token,
        page.clone(),
        json!({
            "here": spawn,
            "shop_open": true,
            "shop_stock": [{
                "id": pass.id,
                "name": pass.name,
                "slot": 16,
                "component": 3900,
            }],
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(buy["kind"], "shop-button", "{buy}");
    assert_eq!(buy["shop"], BUY, "{buy}");
    assert_eq!(
        buy["name"],
        pass.name.as_deref().expect("selected pass name"),
        "{buy}"
    );
    assert_eq!(buy["id"], pass.id, "{buy}");
    assert_eq!(buy["slot"], 16, "{buy}");
    assert_eq!(buy["component"], 3900, "{buy}");
    assert_eq!(buy["chunk"], 1, "{buy}");

    let close = call(
        &data,
        token,
        packed.clone(),
        json!({
            "here": spawn,
            "shop_open": true,
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(close["kind"], "close-modal", "{close}");

    let resumed = call(
        &data,
        token,
        packed,
        json!({
            "here": spawn,
            "shop_open": false,
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(resumed["kind"], "walk", "{resumed}");
    assert_eq!(
        (&resumed["x"], &resumed["z"], &resumed["level"]),
        (&json!(3209), &json!(3218), &json!(1)),
        "{resumed}"
    );

    // The once-per-token latch survives the trip: the same named failure falls
    // through to the search arm instead of starting another shop trip.
    let latched = call(
        &data,
        token,
        page,
        json!({ "here": spawn, "walk_missing_carry": short }),
    );
    assert_eq!(latched["kind"], "walk", "{latched}");
    assert_eq!(
        (&latched["x"], &latched["z"], &latched["level"]),
        (&json!(3209), &json!(3218), &json!(1)),
        "{latched}"
    );
    on_reset();
}

/// Missing-short diagnostics that do not name an absent Shantay pass — and a
/// pass already held — never enter the shop trip.
#[test]
fn gate_toll_never_shops_an_unnamed_or_different_short_or_a_held_pass() {
    let data = selected();
    let pass = data
        .item_by_alias(SHANTAY_PASS)
        .expect("selected Shantay pass");
    let page = json!([[SEARCH, 1]]);
    let packed = json!([[SEARCH, 1], [pass.id, 1]]);
    let named = shantay_short(&data);
    let coins = json!([{ "id": 995, "name": "Coins", "count": 10 }]);
    let away = here(3200, 3218, 1);

    for (case, held, short) in [
        ("unnamed", page.clone(), json!([])),
        ("coins", page.clone(), coins),
        ("already-held", packed, named),
    ] {
        on_reset();
        let token = steady(&data, SEARCH);
        let first = call(
            &data,
            token,
            held.clone(),
            json!({ "here": away, "walk_missing_carry": short }),
        );
        assert_eq!(first["kind"], "walk", "{case}: {first}");
        let again = call(
            &data,
            token,
            held,
            json!({ "here": away, "walk_missing_carry": short }),
        );
        assert_eq!(again["kind"], "walk", "{case}: {again}");
        assert_eq!(
            (&again["x"], &again["z"], &again["level"]),
            (&json!(3209), &json!(3218), &json!(1)),
            "{case}: {again}"
        );
    }
    on_reset();
}

/// The machine envelope is read before an armed shop trip: cooperative yield
/// pauses it without advancing, while posted death ends the token.
#[test]
fn gate_toll_yield_and_death_precede_every_shop_verb() {
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let short = shantay_short(&data);
    let away = here(3200, 3218, 1);

    on_reset();
    let yielded_token = steady(&data, SEARCH);
    let first = call(
        &data,
        yielded_token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(first["kind"], "walk", "{first}");
    let yielded = call(
        &data,
        yielded_token,
        page.clone(),
        json!({
            "here": away,
            "hold": true,
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(yielded["kind"], "yield", "{yielded}");
    let thawed = call(
        &data,
        yielded_token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(thawed["kind"], "walk", "{thawed}");
    assert_eq!(
        (&thawed["x"], &thawed["z"], &thawed["level"]),
        (&json!(SHANTAY_X), &json!(SHANTAY_Z), &json!(SHANTAY_LEVEL)),
        "{thawed}"
    );

    on_reset();
    let dead_token = steady(&data, SEARCH);
    let first = call(
        &data,
        dead_token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(first["kind"], "walk", "{first}");
    let dead = call(
        &data,
        dead_token,
        page,
        json!({
            "here": away,
            "hitpoints": 0,
            "walk_missing_carry": short,
        }),
    );
    assert_eq!(dead["kind"], "dead", "{dead}");
    let stale = call(&data, dead_token, json!([[SEARCH, 1]]), json!({}));
    assert_eq!(stale["kind"], "aborted", "{stale}");
    assert_eq!(stale["reason"], "stale", "{stale}");
    on_reset();
}

/// A trip whose keeper never appears gives up with `no-shop`, keeps the live
/// token, and resumes the original destination without retrying the shop.
#[test]
fn gate_toll_no_shop_keeps_the_token_and_resumes_the_original_walk() {
    on_reset();
    let data = selected();
    let page = json!([[SEARCH, 1]]);
    let short = shantay_short(&data);
    let away = here(3200, 3218, 1);
    let spawn = here(SHANTAY_X, SHANTAY_Z, SHANTAY_LEVEL);
    let token = steady(&data, SEARCH);

    let first = call(
        &data,
        token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(first["kind"], "walk", "{first}");
    let to_shop = call(
        &data,
        token,
        page.clone(),
        json!({ "here": away, "walk_missing_carry": short }),
    );
    assert_eq!(to_shop["kind"], "walk", "{to_shop}");

    let waiting = call(
        &data,
        token,
        page.clone(),
        json!({ "here": spawn, "walk_missing_carry": short }),
    );
    assert_eq!(waiting["kind"], "wait", "{waiting}");
    force_bound();
    let failed = call(
        &data,
        token,
        page.clone(),
        json!({ "here": spawn, "walk_missing_carry": short }),
    );
    assert_eq!(failed["kind"], NO_SHOP, "{failed}");
    assert_eq!(failed["token"], token, "{failed}");

    let resumed = call(
        &data,
        token,
        page.clone(),
        json!({ "here": spawn, "walk_missing_carry": short }),
    );
    assert_eq!(resumed["kind"], "walk", "{resumed}");
    assert_eq!(resumed["token"], token, "{resumed}");
    assert_eq!(
        (&resumed["x"], &resumed["z"], &resumed["level"]),
        (&json!(3209), &json!(3218), &json!(1)),
        "{resumed}"
    );
    let latched = call(
        &data,
        token,
        page,
        json!({ "here": spawn, "walk_missing_carry": short }),
    );
    assert_eq!(latched["kind"], "walk", "{latched}");
    assert_eq!(latched["x"], 3209, "{latched}");
    on_reset();
}


// ── the Entrana strip, its restore and the abandon latch ──

/// `trail_clue_hard_riddle027`: the sole selected row whose own decode lands
/// in the cap box — `trail_coord=0_44_52_2_23` → `(2818, 3351, 0)`.
const ENTRANA: i32 = 3579;
const ENTRANA_X: i32 = 2818;
const ENTRANA_Z: i32 = 3351;
/// The two hard-trail dagger ids the strip unequips but never lists.
const DDS_POISONED: i32 = 1231;
/// A refused name the frozen vectors bind, and its item id.
const HELM: i32 = 1163;

/// One wrapper-marshalled worn row: the raw `host().snapshot.equipment`
/// shape, with the name the matcher folds and the id the DDS exclusion
/// joins.
fn worn(id: i32, name: &str, slot: i32) -> Value {
    json!({ "id": id, "name": name, "count": 1, "slot": slot })
}

/// The machine's own stripped list.
fn stripped() -> Vec<String> {
    RUNTIME.with(|rt| rt.borrow().stripped.clone())
}

/// The adapter's own read, straight through the dispatch seat.
fn owns() -> bool {
    dispatch(None, &json!({ "op": "ownsEquipment" }))["owns"] == true
}

/// The posted booth the bank trip opens, as the wrapper marshals
/// `nearest_booth`: the tile the player stands on beside it.
fn booth_page() -> Value {
    json!({
        "x": 2810,
        "z": 3350,
        "level": 0,
        "id": 2213,
        "name": "Bank booth",
        "op": "Use-quickly",
    })
}

/// The completion kinds no step of a strip or a reclaim may carry: the
/// three-step latch is the finished collect's own and nothing else's.
fn assert_no_completion(step: &Value) {
    let text = step.to_string();
    for forbidden in ["clue solved", "grind-ready", "\"done\""] {
        assert!(!text.contains(forbidden), "{forbidden} {step}");
    }
}

/// The frozen matcher's own vectors, ported whole: the twelve names the
/// frozen `entranaGear.test.ts` binds refused and its ten let through, plus
/// the same frozen regex' edges — the `\b` on a longer word, the
/// `two.handed` wildcard, the `gauntlets?` optional `s` and the
/// `body(?!\s+rune\b)` lookahead beside `Body runes`. Every expectation is
/// the frozen regex' own answer.
#[test]
fn the_frozen_entrana_matcher_folds_the_names_it_was_bound_to() {
    for name in [
        "Dragonhide body",
        "Dragonhide chaps",
        "Dragon vambraces",
        "Coif",
        "Dragonfire shield",
        "Legends cape",
        "Leather gloves",
        "Studded body",
        "Wizard hat",
        "Dragon dagger(p)",
        "Magic shortbow",
        "Maple longbow",
        "Rune full helm",
        "Two-handed sword",
        "Two handed",
        "Rune platebody",
        "Body runes",
        "Dragon battleaxe",
        "Rune kiteshield",
        "Cape of legends",
        "2h sword",
        "Battlestaff",
        "Staff of fire",
        "Rune plateskirt",
        "Skirt of silk",
        "Dragon sq shield",
        "Dragon square shield",
        "Snakeskin chaps",
        "snelm",
        "Cowl",
        "Hood",
        "Dragon gloves",
        "Rune claws",
        "Granite maul",
        "Rune cannon",
        "Med helm",
        "Full helm",
        "Rune chainbody",
        "Rune platelegs",
        "Rune defender",
        "Obsidian cape",
        "Fire cape",
        "God cape",
        "Rune cloak",
        "Air battlestaff",
        "Magic longbow",
        "Rune crossbow",
        "Rune javelin",
        "Rune dart",
        "Rune thrownaxe",
        "Rune knife",
        "Rune warhammer",
        "Rune spear",
        "Rune hasta",
        "Rune halberd",
        "Rune mace",
        "Rune scimitar",
        "Rune longsword",
        "Rune axe",
        "Rune pickaxe",
        "Dragon whip",
        "Bronze dagger",
        "Rune gloves",
        "Leather vambraces",
        "Anti-dragon shield",
        "Cape",
        "somebody body",
    ] {
        assert!(entrana_restricted_gear(name), "{name}");
    }
    for name in [
        "Amulet of glory",
        "Leather boots",
        "Rune arrow",
        "Body rune",
        "Shark",
        "Clue scroll",
        "Spade",
        "Sextant",
        "Superantipoison(4)",
        "Coins",
        "twohanded",
        "Somebody",
        "Shielded",
        "Swordfish",
        "Rune skirt",
        "Cannonball",
        "Brown apron",
        "Amulet of fury",
        "Zamorak monk top",
        "Priest gown",
        "Desert shirt",
        "Boots of lightness",
        "Rune boots",
        "Climbing boots",
        "Dragonstone",
        "Rune arrowtips",
        "Arrow shaft",
        "Coifed",
        "Hatchet",
        "Sharktooth",
        "Hooded",
        "Bodyguard",
    ] {
        assert!(!entrana_restricted_gear(name), "{name}");
    }
}

/// The strip's membership is the row's own selected decode and never a
/// copied coordinate, a casket row is not a member, and a row whose decode
/// is outside the box or off the level is not one either.
#[test]
fn the_entrana_membership_is_the_rows_own_decoded_coord() {
    let data = selected();
    assert_eq!(row(&data, ENTRANA).alias, "trail_clue_hard_riddle027");
    assert!(entrana_coord(row(&data, ENTRANA)));
    // The box's own ends are in it and one step outside is not.
    let inside = member(vec![param("trail_coord", "0_43_52_50_33")], None);
    assert!(entrana_coord(&inside), "the box's own west edge is in it");
    for value in [
        "0_43_52_49_33", // x 2817, one west of the box
        "0_44_52_63_33", // x 2879, one east
        "1_43_52_50_33", // the right square on the wrong level
    ] {
        assert!(
            !entrana_coord(&member(vec![param("trail_coord", value)], None)),
            "{value}"
        );
    }
    // A desc-only row carries no coord at all, and a casket never arms it
    // even when the id it is posted under is the box row's own.
    assert!(!entrana_coord(&member(vec![], None)));
    let mut casket = member(vec![param("trail_coord", "0_44_52_2_23")], None);
    casket.role = "casket".into();
    assert!(!entrana_coord(&casket), "a casket never arms the strip");
}

/// The strip's unequip pass: the posted worn rows the frozen matcher folds
/// go out as the landed `unequip` verb — the worn row's own `Remove`, which
/// the host's `wear` cannot do — one per call, the two hard-trail dagger
/// ids are unequipped but never listed, and the rows the matcher lets
/// through are never a verb at all. The deposit pass that follows takes
/// every regex-matching pack row, listed or not.
#[test]
fn the_strip_unequips_the_folded_names_and_never_lists_the_dds() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    // The dagger is first in posted order, then the helm, then the two the
    // monks let through.
    let posted = json!([
        worn(DDS_POISONED, "Dragon dagger(p)", 3),
        worn(HELM, "Rune full helm", 0),
        worn(1704, "Amulet of glory", 2),
        worn(3791, "Leather boots", 10),
    ]);
    let dagger = call(&data, token, page.clone(), json!({ "equipment": posted }));
    assert_eq!(dagger["kind"], "unequip", "{dagger}");
    assert_eq!(dagger["name"], "Dragon dagger(p)", "{dagger}");
    assert_eq!(dagger["token"], token, "{dagger}");
    assert!(
        stripped().is_empty(),
        "a hard-trail dagger id is never listed: {:?}",
        stripped()
    );
    assert_no_completion(&dagger);
    // It left the page: the helm is next, and the two non-matches are never
    // dispatched at.
    let left = json!([
        worn(HELM, "Rune full helm", 0),
        worn(1704, "Amulet of glory", 2),
        worn(3791, "Leather boots", 10),
    ]);
    let helm = call(&data, token, page.clone(), json!({ "equipment": left }));
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert_eq!(helm["name"], "Rune full helm", "{helm}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    // Both landed in the pack. The deposit pass is the matcher over the
    // posted pack page and never the listed names: the dagger id that was
    // never listed is first, because it is first in posted order — and it
    // is still not listed after it is deposited.
    let worn_free = json!([
        worn(1704, "Amulet of glory", 2),
        worn(3791, "Leather boots", 10),
    ]);
    let both = json!([
        inv(DDS_POISONED, "Dragon dagger(p)", 1),
        inv(HELM, "Rune full helm", 1),
    ]);
    let bank_ready = json!({
        "equipment": worn_free,
        "inv": both,
        "here": here(2810, 3350, 0),
        "nearest_booth": booth_page(),
    });
    let walked = call(&data, token, page.clone(), bank_ready.clone());
    assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
    let opened = call(&data, token, page.clone(), bank_ready.clone());
    assert_eq!(opened["kind"], "open-booth", "{opened}");
    let banked = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": worn_free, "inv": both, "bank_open": true }),
    );
    assert_eq!(banked["kind"], "deposit", "{banked}");
    assert_eq!(
        banked["name"], "Dragon dagger(p)",
        "the pack's own regex match is the candidate, listed or not: {banked}"
    );
    assert_eq!(
        stripped(),
        vec!["Rune full helm".to_string()],
        "and the dagger stays off the list"
    );
    // The helm's own deposit follows in posted order.
    let helm_pack = json!([inv(HELM, "Rune full helm", 1)]);
    let banked = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": worn_free, "inv": helm_pack, "bank_open": true }),
    );
    assert_eq!(banked["kind"], "deposit", "{banked}");
    assert_eq!(banked["name"], "Rune full helm", "{banked}");
    // A restricted name the player carried and never wore is the same
    // candidate: the predicate is the matcher over the pack page, so a
    // spare weapon that was never on the worn page is banked too — and it
    // is not listed either, because the list is the worn rows'.
    let carried = json!([inv(1181, "Rune platebody", 1)]);
    let banked = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": worn_free, "inv": carried, "bank_open": true }),
    );
    assert_eq!(banked["kind"], "deposit", "{banked}");
    assert_eq!(banked["name"], "Rune platebody", "{banked}");
    assert_eq!(
        stripped(),
        vec!["Rune full helm".to_string()],
        "a carried name is deposited but never listed"
    );
    assert!(owns());
    // Nothing restricted left in the pack: the interface closes and the
    // row's own search arm waits on the decoded tile with no locs posted.
    let settled = call(
        &data,
        token,
        page.clone(),
        json!({
            "equipment": worn_free,
            "here": here(ENTRANA_X, ENTRANA_Z, 0),
            "bank_open": true,
        }),
    );
    assert_eq!(settled["kind"], "close", "{settled}");
    let settled = call(
        &data,
        token,
        page.clone(),
        json!({
            "equipment": worn_free,
            "here": here(ENTRANA_X, ENTRANA_Z, 0),
        }),
    );
    assert_eq!(settled["kind"], "wait", "{settled}");
    assert!(
        !settled.to_string().contains("Amulet"),
        "a name the frozen matcher lets through is never dispatched at: {settled}"
    );
    assert!(owns(), "the helm is listed, so the adapter reads true");
    on_reset();
}

/// The strip's deposit pass: the listed names the posted pack holds are
/// deposited at the posted booth — the walk, the booth's own open, one
/// deposit per name, the close — and only then does the row's own arm walk.
#[test]
fn the_strip_deposits_the_listed_names_at_the_posted_booth() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    // The unequip landed: the worn page is empty and the pack holds it.
    let pack = json!([inv(HELM, "Rune full helm", 1)]);
    let banked = json!({
        "equipment": json!([]),
        "inv": pack,
        "here": here(2810, 3350, 0),
        "nearest_booth": booth_page(),
    });
    let walked = call(&data, token, page.clone(), banked.clone());
    assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
    assert_eq!(walked["token"], token, "{walked}");
    assert_no_completion(&walked);
    let opened = call(&data, token, page.clone(), banked.clone());
    assert_eq!(opened["kind"], "open-booth", "{opened}");
    assert_eq!(opened["x"], 2810, "{opened}");
    assert_eq!(opened["z"], 3350, "{opened}");
    assert_eq!(opened["id"], 2213, "{opened}");
    assert_eq!(opened["name"], "Bank booth", "{opened}");
    assert_eq!(opened["action"], "Use-quickly", "{opened}");
    // The interface is up: the deposit goes out by the name the strip put
    // in the pack.
    let deposited = call(
        &data,
        token,
        page.clone(),
        json!({ "inv": pack, "bank_open": true }),
    );
    assert_eq!(deposited["kind"], "deposit", "{deposited}");
    assert_eq!(deposited["name"], "Rune full helm", "{deposited}");
    // It landed: the interface closes, and the call after that settles the
    // strip and runs the row's own search arm — the name stays listed.
    let closed = call(
        &data,
        token,
        page.clone(),
        json!({ "inv": json!([]), "bank_open": true }),
    );
    assert_eq!(closed["kind"], "close", "{closed}");
    let walked_on = call(
        &data,
        token,
        page.clone(),
        json!({
            "inv": json!([]),
            "bank_open": false,
            "here": here(ENTRANA_X, ENTRANA_Z, 0),
        }),
    );
    assert_eq!(walked_on["kind"], "wait", "{walked_on}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    assert!(owns());
    on_reset();
}

/// The restore in front of the three-step latch: no `'clue solved'`, no
/// `grind-ready` and no `done` while a listed name is unclaimed, the pack's
/// own name worn back on first, and the three kinds in order once the list
/// empties.
#[test]
fn the_restore_runs_before_the_whole_three_step_latch() {
    on_reset();
    let data = selected();
    // One strip first, so this session owns a listed name.
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page,
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    assert!(owns());
    // The casket collect runs to its own bound with the name still listed.
    let casket = casket_of(&data, CLUE);
    let token = opened(&data, casket);
    let pack = json!([inv(HELM, "Rune full helm", 1)]);
    let scene = pages(json!([]), pack.clone(), json!(28), json!(-1));
    let waiting = call(&data, token, json!([]), scene.clone());
    assert_eq!(waiting["kind"], "wait", "{waiting}");
    assert_no_completion(&waiting);
    force_bound();
    // The collect is over and the reclaim owns the token: the pack's own
    // name goes back on, and no completion kind rides it.
    let wear_back = call(&data, token, json!([]), scene.clone());
    assert_eq!(wear_back["kind"], "wear", "{wear_back}");
    assert_eq!(wear_back["name"], "Rune full helm", "{wear_back}");
    assert_eq!(wear_back["token"], token, "{wear_back}");
    assert_no_completion(&wear_back);
    assert!(owns(), "the name is listed until it is worn again");
    // The worn page shows it again: the list empties and the latch runs its
    // three steps on this same live token.
    let back_on = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([worn(HELM, "Rune full helm", 0)]),
    });
    let solved = call(&data, token, json!([]), back_on.clone());
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    assert!(stripped().is_empty(), "the restore empties the list");
    assert!(!owns(), "and the adapter's own read follows it");
    let ready = call(&data, token, json!([]), back_on.clone());
    assert_eq!(ready["kind"], "grind-ready", "{ready}");
    assert_eq!(ready["token"], token, "{ready}");
    let done = call(&data, token, json!([]), back_on);
    assert_eq!(done["kind"], "done", "{done}");
    on_reset();
}

/// The restore's bank leg: the names the pack does not hold are walked to
/// the posted stand for, opened, claimed one `Withdraw-1` at a time, and the
/// interface closed before anything is worn back on.
#[test]
fn the_restore_claims_a_missing_name_at_the_bank() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page,
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    // The collect ends with nothing in the pack: the walk to the stand.
    let casket = casket_of(&data, CLUE);
    let token = opened(&data, casket);
    let scene = pages(json!([]), json!([]), json!(28), json!(-1));
    let _ = call(&data, token, json!([]), scene.clone());
    force_bound();
    let banked = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
    });
    let walked = call(&data, token, json!([]), banked.clone());
    assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
    assert_no_completion(&walked);
    // Arrived beside the posted booth: its own open, and then the claim.
    let arrived = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
    });
    let opened_booth = call(&data, token, json!([]), arrived.clone());
    assert_eq!(opened_booth["kind"], "open-booth", "{opened_booth}");
    let open = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": true,
    });
    let claimed = call(&data, token, json!([]), open.clone());
    assert_eq!(claimed["kind"], "withdraw", "{claimed}");
    assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
    assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
    assert_no_completion(&claimed);
    // The claim landed in the pack: the open interface closes first, and
    // only then does the wear pass put the name back on.
    let claimed_open = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": json!([inv(HELM, "Rune full helm", 1)]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": true,
    });
    let closed = call(&data, token, json!([]), claimed_open);
    assert_eq!(closed["kind"], "close", "{closed}");
    let claimed_closed = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": json!([inv(HELM, "Rune full helm", 1)]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": false,
    });
    let wear_back = call(&data, token, json!([]), claimed_closed);
    assert_eq!(wear_back["kind"], "wear", "{wear_back}");
    assert_eq!(wear_back["name"], "Rune full helm", "{wear_back}");
    assert_no_completion(&wear_back);
    // Worn again: the list empties and the exact status goes out.
    let back_on = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([worn(HELM, "Rune full helm", 0)]),
    });
    let solved = call(&data, token, json!([]), back_on);
    assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
    assert_eq!(solved["message"], "clue solved", "{solved}");
    assert!(stripped().is_empty());
    on_reset();
}

/// A listed name the page never puts back on is the named
/// `restore-incomplete` log — never a machine kind and never a completion
/// kind — and the list keeps it for the next attempt.
#[test]
fn a_name_that_will_not_go_back_on_stays_listed_and_logs() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page,
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    let casket = casket_of(&data, CLUE);
    let token = opened(&data, casket);
    let scene = pages(json!([]), json!([]), json!(28), json!(-1));
    let _ = call(&data, token, json!([]), scene.clone());
    force_bound();
    // The restore attempt: nothing in the pack, no booth posted at all, so
    // the walk goes out and the window is what ends the attempt.
    let bare = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
    });
    let walked = call(&data, token, json!([]), bare.clone());
    assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
    force_bound();
    let failed = call(&data, token, json!([]), bare.clone());
    assert_eq!(failed["kind"], "callback.log", "{failed}");
    let message = failed["message"].as_str().unwrap_or("");
    assert!(
        message.contains("restore-walk-failed"),
        "the named bank failure: {failed}"
    );
    assert_no_completion(&failed);
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    // The next call starts a fresh attempt, and the list is still what the
    // adapter reads: nothing posts the status while it is non-empty.
    let again = call(&data, token, json!([]), bare.clone());
    assert_eq!(again["kind"], "walk-nearest-bank", "{again}");
    assert!(owns());
    // A claim the posted bank never lands: the attempt gives up on the
    // name, the interface closes, and what is still missing is the named
    // `restore-incomplete` — the list keeps it and the latch stays blocked.
    let open = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "bank_open": true,
    });
    let claimed = call(&data, token, json!([]), open.clone());
    assert_eq!(claimed["kind"], "withdraw", "{claimed}");
    assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
    assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
    let closed = call(&data, token, json!([]), open.clone());
    assert_eq!(closed["kind"], "close", "{closed}");
    let down = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": json!([]),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "bank_open": false,
    });
    let incomplete = call(&data, token, json!([]), down.clone());
    assert_eq!(incomplete["kind"], "callback.log", "{incomplete}");
    let message = incomplete["message"].as_str().unwrap_or("");
    assert!(
        message.contains("restore-incomplete") && message.contains("Rune full helm"),
        "the named re-equip failure carries the names: {incomplete}"
    );
    assert_eq!(incomplete["token"], token, "{incomplete}");
    assert_no_completion(&incomplete);
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    // And the reclaim retries rather than finishing: a fresh attempt walks
    // again and no completion kind ever rides the still-listed name.
    let retry_walk = call(&data, token, json!([]), down);
    assert_eq!(retry_walk["kind"], "walk-nearest-bank", "{retry_walk}");
    assert_no_completion(&retry_walk);
    on_reset();
}

/// `ownsEquipment` is the machine's own list and `retry` is the machine's
/// own latch clear: retry never aborts the live token and never touches the
/// list.
#[test]
fn owns_equipment_reads_the_list_and_retry_clears_only_the_latch() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    assert!(!owns(), "an empty list owns nothing");
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert!(owns());
    let retried = dispatch(Some(&data), &json!({ "op": "retry" }));
    assert_eq!(retried["kind"], "retry", "{retried}");
    assert_eq!(retried["token"], token, "{retried}");
    assert!(owns(), "the list is not retry's: {:?}", stripped());
    // The live token is still the machine's own: its next step answers.
    let live = call(
        &data,
        token,
        page.clone(),
        json!({
            "equipment": json!([]),
            "inv": json!([inv(HELM, "Rune full helm", 1)]),
        }),
    );
    assert_eq!(live["kind"], "walk-nearest-bank", "{live}");
    on_reset();
}
/// An abandoned row remains refused across repeated begins until `retry`, while
/// identifying a different row clears the latch on the way past.
#[test]
fn an_abandoned_row_is_refused_until_retry_or_a_different_row() {
    on_stop();
    let data = selected();
    let search = json!([[SEARCH, 1]]);
    let other = json!([[MAP_EMPTY, 1]]);

    let token = token_of(&begin(&data, search.clone()));
    let left = dispatch(Some(&data), &json!({ "op": "abandon" }));
    assert_eq!(left["kind"], ABANDON, "{left}");
    assert_ne!(left["token"], token, "{left}");

    for _ in 0..2 {
        let refused = begin(&data, search.clone());
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], ABANDONED, "{refused}");
    }

    let retried = dispatch(Some(&data), &json!({ "op": "retry" }));
    assert_eq!(retried["kind"], "retry", "{retried}");
    let accepted = begin(&data, search.clone());
    assert_eq!(accepted["kind"], "token", "{accepted}");

    // Latch the same row again, then identify another selected row. That begin
    // is accepted and clears the old latch; returning to the first row is also
    // accepted rather than refused.
    let left_again = dispatch(Some(&data), &json!({ "op": "abandon" }));
    assert_eq!(left_again["kind"], ABANDON, "{left_again}");
    let different = begin(&data, other);
    assert_eq!(different["kind"], "token", "{different}");
    let back = begin(&data, search);
    assert_eq!(back["kind"], "token", "{back}");
    on_stop();
}


/// A connection boundary keeps the session's own strip list: `on_reset`
/// drops the live step and its token and nothing else, so the reclaim the
/// strip already owes survives a relog and `ownsEquipment` still reads
/// true. The fresh instance `on_stop` is what clears it.
#[test]
fn a_connection_boundary_keeps_the_stripped_list_and_a_stop_clears_it() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    assert!(owns());
    // The connection boundary: the live step and its token are gone.
    on_reset();
    assert_eq!(
        dispatch(Some(&data), &json!({ "op": "next", "token": token }))["kind"],
        "aborted",
        "the boundary kills the live token"
    );
    assert_eq!(
        stripped(),
        vec!["Rune full helm".to_string()],
        "the list the reclaim still owes outlives it"
    );
    assert!(owns(), "and the adapter still reads it");
    // A fresh session on the live machine still owes that reclaim: a begin
    // on the casket row reaches the collect, and its exit walks to the bank
    // for the name the earlier session banked, because the list outlived
    // the boundary.
    let casket = casket_of(&data, CLUE);
    let again = opened(&data, casket);
    let quiet = pages(json!([]), json!([]), json!(28), json!(-1));
    let _ = call(&data, again, json!([]), quiet.clone());
    force_bound();
    let walk = call(&data, again, json!([]), quiet);
    assert_eq!(walk["kind"], "walk-nearest-bank", "{walk}");
    // The fresh task instance clears the list with the step.
    on_stop();
    assert!(
        stripped().is_empty(),
        "Stop starts the session over: {:?}",
        stripped()
    );
    assert!(!owns(), "and the adapter reads an empty list");
    on_reset();
}

/// The restore's make-room deposit: a full pack at the trail's end banks
/// what the frozen predicate takes before the claim goes out, so the
/// withdrawn name has a slot to land in — and a bank that never lands the
/// deposit does not spin the same verb.
#[test]
fn a_full_pack_is_made_room_for_before_the_reclaim_claim() {
    on_reset();
    let data = selected();
    let page = json!([[ENTRANA, 1]]);
    let token = steady(&data, ENTRANA);
    let helm = call(
        &data,
        token,
        page.clone(),
        json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
    );
    assert_eq!(helm["kind"], "unequip", "{helm}");
    // The strip banks the helm, so the reclaim has to fetch it back.
    let banked = json!({
        "equipment": json!([]),
        "inv": json!([inv(HELM, "Rune full helm", 1)]),
        "here": here(2810, 3350, 0),
        "nearest_booth": booth_page(),
        "bank_open": true,
    });
    let _ = call(&data, token, page.clone(), banked.clone());
    let deposited = call(&data, token, page.clone(), banked.clone());
    assert_eq!(deposited["kind"], "deposit", "{deposited}");
    let _ = call(
        &data,
        token,
        page.clone(),
        json!({ "inv": json!([]), "bank_open": true, "nearest_booth": booth_page() }),
    );
    // The collect runs to its bound with the pack full of loot: 28 posted
    // rows and no room for the helm.
    let casket = casket_of(&data, CLUE);
    let token = opened(&data, casket);
    let full = full_pack();
    let scene = json!({
        "here": loot_tile(),
        "ground": json!([]),
        "inv": full,
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
    });
    let _ = call(&data, token, json!([]), scene.clone());
    force_bound();
    // The bank trip first, then the make-room deposit, and only then the
    // claim: the frozen `restoreStrippedGear` order.
    let open = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": full,
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": true,
    });
    let room = call(&data, token, json!([]), open.clone());
    assert_eq!(room["kind"], "deposit", "{room}");
    assert_eq!(
        room["name"], "Big bones",
        "a non-want row the trail facts do not name is banked to make room: {room}"
    );
    assert_no_completion(&room);
    // It landed: one slot free and the claim goes out.
    let freed = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": full_minus_one(),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": true,
    });
    let claimed = call(&data, token, json!([]), freed.clone());
    assert_eq!(claimed["kind"], "withdraw", "{claimed}");
    assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
    assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
    // The claim did not land inside one more call: `tried` ends the deposit
    // pass, the interface closes, and what is still missing is the named
    // log — the list keeps it, so the latch stays blocked. No second
    // deposit of the same row and no unbounded loop.
    let stuck = call(&data, token, json!([]), freed.clone());
    assert_eq!(stuck["kind"], "close", "{stuck}");
    let down = json!({
        "here": here(2810, 3350, 0),
        "ground": json!([]),
        "inv": full_minus_one(),
        "inv_size": 28,
        "main_modal_id": -1,
        "equipment": json!([]),
        "nearest_booth": booth_page(),
        "bank_open": false,
    });
    let incomplete = call(&data, token, json!([]), down);
    assert_eq!(incomplete["kind"], "callback.log", "{incomplete}");
    let message = incomplete["message"].as_str().unwrap_or("");
    assert!(message.contains("restore-incomplete"), "{incomplete}");
    assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
    on_reset();
}

#[test]
fn omitted_scalar_pages_fall_back_to_the_scene_and_the_echo_wins() {
    crate::observed::on_reset();
    let bare = json!({ "op": "next" });
    assert_eq!(posted_here(&bare), None, "no echo and no scene: no tile");
    assert_eq!(posted_inv_size(&bare), None);
    assert!(!posted_hold(&bare));

    crate::observed::post(3, |post| {
        post.here(crate::observed::Tile {
            x: 3200,
            z: 3218,
            level: 1,
        })
        .inv_size(28)
        .ours(true);
    });
    assert_eq!(
        posted_here(&bare),
        Some(Tile {
            x: 3200,
            z: 3218,
            level: 1
        })
    );
    assert_eq!(posted_inv_size(&bare), Some(28));
    assert!(posted_hold(&bare), "`ours` alone is the interrupt");

    let echoed = json!({
        "here": { "x": 1, "z": 2, "level": 0 },
        "inv_size": 0,
        "hold": false,
    });
    assert_eq!(
        posted_here(&echoed),
        Some(Tile {
            x: 1,
            z: 2,
            level: 0
        })
    );
    assert_eq!(posted_inv_size(&echoed), Some(0));
    assert!(!posted_hold(&echoed));
}
