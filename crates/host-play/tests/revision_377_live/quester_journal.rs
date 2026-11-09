//! Native Quester ownership checks for the 377 quest journal paths.
use super::*;
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use script::native::NativePhase;
use script::{ScriptLifecycleReceipt, ScriptTerminalState};
use std::io::Write;
use std::sync::Mutex;
use vault::{Profile, ProfileSettings};

#[derive(Clone, Debug, serde::Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    xp: i32,
    level: i32,
    mining_level: i32,
    cooking_level: i32,
    qp: i32,
    quest_colour: Option<i32>,
    inventory: Vec<(i32, i32)>,
    main_text: Vec<String>,
    main_root: i32,
    dialogue: Vec<String>,
    chat: Vec<(i32, String)>,
    combat_level: i32,
    combat_stats: [i32; 4],
    hitpoints: i32,
    in_combat: bool,
    local_index: Option<usize>,
    nearby_npcs: Vec<NpcSample>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct NpcSample {
    index: usize,
    npc_type: Option<usize>,
    name: Option<String>,
    tile: WorldTile,
    level: i32,
    in_combat: bool,
    target: Option<api::snapshot::ActorTargetView>,
    overhead: Option<String>,
}

// Forty-three in these stats gives combat 49 (canon Player.ts:1429-1434).
// That is the first level exempt from the palace's level-24 zombie hunts
// (canon all.npc:2289; Npc.ts:1017). Keep the production danger policy at default.
const COMBAT_STATS_FIXTURE: &[&str] = &[
    "setstat attack 43",
    "setstat strength 43",
    "setstat defence 43",
    "setstat hitpoints 43",
];

#[derive(Clone, Copy)]
struct CombatFoodSeed {
    command: &'static str,
    item_id: i32,
    count: i32,
    receipt_label: &'static str,
    phase_label: &'static str,
}

const COMBAT_FOOD_SEED: CombatFoodSeed = CombatFoodSeed {
    command: "give trout 5",
    item_id: 333,
    count: 5,
    receipt_label: "labelled combat-only area fixture: Attack/Strength/Defence/Hitpoints 43, combat at least 49, five trout; default danger policy",
    phase_label: "SEED own-account combat-only area fixture",
};
const SQUIRE_COMBAT_FOOD_SEED: CombatFoodSeed = CombatFoodSeed {
    command: "give lobster 6",
    item_id: 379,
    count: 6,
    receipt_label: "labelled Squire combat-only area and food fixture: Attack/Strength/Defence/Hitpoints 43, combat at least 49, six lobsters for blurite; default danger policy",
    phase_label: "SEED own-account food: Squire blurite step (six lobsters)",
};

// The journal needs Mining 10, and the fresh pie recipe needs Cooking 10.
// The Squire fixture supplies both the two-bar fast-path input and six lobsters
// for its blurite step; the run still exercises the Path's other acquisition.
// Sources: quest_squire/scripts/squire_journal.rs2:7-12;
// skill_cooking/configs/cooking_inv/configs/pies/pies.struct:13-17.
const SQUIRE_MINING_LEVEL: i32 = 10;
const SQUIRE_SMITHING_LEVEL: i32 = 1;
const SQUIRE_COOKING_LEVEL: i32 = 10;
const SQUIRE_REQUIREMENT_FIXTURE: &[&str] = &["setstat mining 10", "setstat cooking 10"];
const SQUIRE_BANK_SEED_LABEL: &str = "SEED own-account bank: Squire two-bar fast-path input";
const SQUIRE_BANK_SEED_COMMAND: &str = "givebank iron_bar 2";

#[derive(Clone, Copy)]
struct Quest {
    key: &'static str,
    label: &'static str,
    var: &'static str,
    row_label: &'static str,
    terminal: &'static str,
    skill: i32,
    xp_delta: i32,
    qp_delta: i32,
    qp_reward: &'static str,
}
fn combat_food_seed(quest: Quest) -> CombatFoodSeed {
    if quest.key == "squire" {
        SQUIRE_COMBAT_FOOD_SEED
    } else {
        COMBAT_FOOD_SEED
    }
}

// quest_sheep.rs2:5 gives 150 XP. The fresh account also spins 20 balls:
// spinning.rs2:60-64 and spinning.struct:8-12 give 2.5 XP per ball.
const SHEEP: Quest = Quest {
    key: "sheep",
    label: "Sheep Shearer",
    row_label: "Sheep Shearer",
    var: "sheep",
    terminal: "22",
    skill: 12,
    xp_delta: 200,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const RUNE: Quest = Quest {
    key: "runemysteries",
    label: "Rune Mysteries",
    row_label: "Rune Mysteries Quest",
    var: "runemysteries",
    terminal: "6",
    skill: 20,
    xp_delta: 0,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const ROMEO: Quest = Quest {
    key: "romeojuliet",
    label: "Romeo & Juliet",
    row_label: "Romeo & Juliet",
    var: "rjquest",
    terminal: "100",
    skill: 0,
    xp_delta: 0,
    qp_delta: 5,
    qp_reward: "5 Quest Points",
};
// Magic is stat index 6 (quester_imp.rs). Crafting is 12 (sheep). Attack is 0.
// Vampire's reward skill is Attack, which the combat fixture also raises, so
// its witness compares the delta against the post-fixture baseline.
const HETTY: Quest = Quest {
    key: "hetty",
    label: "Witch's Potion",
    row_label: "Witch's Potion",
    var: "hetty",
    terminal: "3",
    skill: 6,
    xp_delta: 325,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const GOBDIP: Quest = Quest {
    key: "gobdip",
    label: "Goblin Diplomacy",
    row_label: "Goblin Diplomacy",
    var: "goblinquest",
    terminal: "6",
    skill: 12,
    xp_delta: 200,
    qp_delta: 5,
    qp_reward: "5 Quest Points",
};
const VAMPIRE: Quest = Quest {
    key: "vampire",
    label: "Vampire Slayer",
    row_label: "Vampire Slayer",
    var: "vampire",
    terminal: "3",
    skill: 0,
    xp_delta: 4825,
    qp_delta: 3,
    qp_reward: "3 Quest Points",
};
const DEMON: Quest = Quest {
    key: "demon",
    label: "Demon Slayer",
    row_label: "Demon Slayer",
    var: "demonstart",
    terminal: "30",
    skill: 6,
    xp_delta: 0,
    qp_delta: 3,
    qp_reward: "3 Quest Points",
};
// The 377 journal row is "The Knight's Sword" (content/scripts/interfaces/questlist.if:231-244).
// The completion reward and terminal var come from content/scripts/quests/quest_squire/scripts/quest_squire.rs2:219-223.
// Smithing is stat index 13, the reward skill.
const SQUIRE: Quest = Quest {
    key: "squire",
    label: "Knight's Sword",
    row_label: "The Knight's Sword",
    var: "squire",
    terminal: "7",
    skill: 13,
    xp_delta: 12_725,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};

fn needs_combat_fixture(key: &str) -> bool {
    matches!(
        key,
        "runemysteries" | "hetty" | "gobdip" | "vampire" | "demon" | "squire"
    )
}

fn split_completion(key: &str) -> Option<&'static str> {
    match key {
        "squire" => Some("Knight's Sword Quest!"),
        "romeojuliet" => Some("Romeo & Juliet Quest!"),
        "hetty" => Some("Witches Potion Quest!"),
        "gobdip" => Some("Goblin Diplomacy Quest!"),
        "vampire" => Some("Vampire Slayer Quest!"),
        "demon" => Some("Demon Slayer Quest!"),
        _ => None,
    }
}

impl Sample {
    fn read(client: &Client, snapshot: &GameSnapshot, quest: Quest) -> Self {
        let stat = snapshot.stats().iter().find(|row| row.index == quest.skill);
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            xp: stat.map_or(0, |row| row.xp),
            level: stat.map_or(1, |row| row.base),
            mining_level: snapshot
                .stats()
                .iter()
                .find(|row| row.name == "mining")
                .map_or(1, |row| row.base),
            cooking_level: snapshot
                .stats()
                .iter()
                .find(|row| row.name == "cooking")
                .map_or(1, |row| row.base),
            qp: client.var.get(101).copied().unwrap_or(-1),
            quest_colour: snapshot
                .quest_statuses()
                .iter()
                .find(|row| row.name == quest.row_label)
                .map(|row| row.colour),
            inventory: snapshot
                .inventory()
                .iter()
                .map(|row| (row.def.id, row.count))
                .collect(),
            main_root: snapshot.modals().main,
            main_text: snapshot.main_modal_texts().to_vec(),
            dialogue: snapshot.chat_modal_texts().to_vec(),
            chat: snapshot
                .chat_lines()
                .iter()
                .map(|row| (row.sequence, row.text.clone()))
                .collect(),
            combat_level: snapshot.local_player().map_or(0, |p| p.player.combat_level),
            combat_stats: ["attack", "strength", "defence", "hitpoints"].map(|name| {
                snapshot
                    .stats()
                    .iter()
                    .find(|s| s.name == name)
                    .map_or(0, |s| s.base)
            }),
            hitpoints: snapshot
                .stats()
                .iter()
                .find(|s| s.name == "hitpoints")
                .map_or(0, |s| s.effective),
            in_combat: snapshot
                .local_player()
                .is_some_and(|p| p.player.actor.in_combat),
            local_index: snapshot.local_player().map(|p| p.player.index),
            nearby_npcs: snapshot
                .npcs()
                .iter()
                .filter(|n| n.distance <= 18)
                .map(|n| NpcSample {
                    index: n.index,
                    npc_type: n.r#type,
                    name: n.name.clone(),
                    tile: n.tile,
                    level: n.level,
                    in_combat: n.in_combat,
                    target: n.target,
                    overhead: n.overhead_text.clone(),
                })
                .collect(),
        }
    }
    fn fresh(&self) -> bool {
        self.xp == 0 && self.qp == 0 && self.quest_colour == Some(0xF80000)
    }
    fn safe_to_relog(&self) -> bool {
        !self.in_combat
            && !self.nearby_npcs.iter().any(|npc| {
                npc.in_combat
                    && npc.target.is_some_and(|target| {
                        target.kind == api::snapshot::ActorKind::Player
                            && Some(target.index) == self.local_index
                    })
            })
    }
    fn combat_can_relog(&self) -> bool {
        self.combat_level >= 49
            && self.combat_stats.iter().all(|level| *level == 43)
            && self.hitpoints == 43
            && self.safe_to_relog()
    }
    fn combat_fixture_ready(&self, quest: Quest) -> bool {
        let food = combat_food_seed(quest);
        self.combat_can_relog()
            && self
                .inventory
                .iter()
                .filter(|(id, _)| *id == food.item_id)
                .map(|(_, count)| count)
                .sum::<i32>()
                == food.count
    }
    fn quest_fixture_ready(&self, quest: Quest) -> bool {
        match quest.key {
            "squire" => {
                self.combat_fixture_ready(quest)
                    && self.mining_level == SQUIRE_MINING_LEVEL
                    && self.level == SQUIRE_SMITHING_LEVEL
                    && self.cooking_level == SQUIRE_COOKING_LEVEL
            }
            key if needs_combat_fixture(key) => self.combat_fixture_ready(quest),
            _ => true,
        }
    }
    fn quest_pre_relog_ready(&self, quest: Quest) -> bool {
        if quest.key == "squire" {
            // setstat skill rows are verified after the clean relog, when the
            // server publishes them. Before it, wait only for the visible combat fixture.
            self.combat_level >= 49 && self.safe_to_relog()
        } else {
            self.combat_can_relog()
        }
    }
    fn complete(&self, q: Quest, base_xp: i32) -> bool {
        let completion = if let Some(second) = split_completion(q.key) {
            self.main_text
                .iter()
                .any(|s| s.contains("You have completed the"))
                && self.main_text.iter().any(|s| s.contains(second))
        } else {
            self.main_text
                .iter()
                .any(|s| s.contains(&format!("You have completed the {} Quest!", q.label)))
        };
        let reward = match q.key {
            "sheep" => {
                self.main_text.iter().any(|s| s.contains("150 Crafting XP"))
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "runemysteries" => {
                self.main_text.iter().any(|s| s == "Runecrafting skill")
                    && self.main_text.iter().any(|s| s == "Air talisman")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "hetty" => {
                self.main_text.iter().any(|s| s == "325 Magic XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "gobdip" => {
                self.main_text.iter().any(|s| s == "200 Crafting XP")
                    && self.main_text.iter().any(|s| s == "A Gold Bar")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "vampire" => {
                self.main_text.iter().any(|s| s == "4,825 Attack XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "demon" => {
                self.main_text.iter().any(|s| s == "Silverlight")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "squire" => {
                self.main_text.iter().any(|s| s == "12,725 Smithing XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "romeojuliet" => self.main_text.iter().any(|s| s == q.qp_reward),
            _ => false,
        };
        // Draynor adds variable Attack XP before Vampire's reward. Squire's
        // dwarf drops add no Smithing XP, so its reward delta remains exact.
        let xp_delta = self.xp.checked_sub(base_xp);
        let xp_ok = if q.key == "vampire" {
            xp_delta.is_some_and(|gained| gained >= q.xp_delta)
        } else {
            xp_delta == Some(q.xp_delta)
        };
        xp_ok
            && self.qp == q.qp_delta
            && self.quest_colour == Some(0x00F800)
            && completion
            && reward
    }
}

fn native_run_completed(receipt: Option<&ScriptLifecycleReceipt>, generation: Option<u64>) -> bool {
    receipt.is_some_and(|r| {
        Some(r.runtime_generation) == generation && r.state == ScriptTerminalState::Completed
    })
}

fn witness_predicate(sample: &Sample, quest: Quest, base_xp: i32, baseline: bool) -> bool {
    if baseline {
        if quest.key == "vampire" {
            // The labelled combat fixture raises Attack, the reward skill.
            sample.qp == 0
                && sample.quest_colour == Some(0xF80000)
                && sample.combat_fixture_ready(quest)
        } else {
            sample.fresh() && base_xp == sample.xp && sample.quest_fixture_ready(quest)
        }
    } else {
        sample.complete(quest, base_xp)
    }
}

#[test]
fn journal_witness_rejects_stale_or_partial_product_evidence() {
    let fresh = Sample {
        tick: 1,
        tile: Some((3200, 3200, 0)),
        xp: 0,
        level: 1,
        mining_level: 1,
        cooking_level: 1,
        qp: 0,
        quest_colour: Some(0xF80000),
        inventory: vec![],
        main_root: 0,
        main_text: vec![],
        dialogue: vec![],
        chat: vec![],
        combat_level: 3,
        combat_stats: [1, 1, 1, 10],
        hitpoints: 10,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    assert!(witness_predicate(&fresh, SHEEP, 0, true));
    assert!(
        !witness_predicate(&fresh, RUNE, 0, true),
        "Rune Mysteries must reject the original unprepared combat fixture"
    );
    let mut prepared = fresh.clone();
    prepared.combat_level = 49;
    prepared.combat_stats = [43; 4];
    prepared.hitpoints = 43;
    prepared.inventory = vec![(333, 5)];
    assert!(witness_predicate(&prepared, RUNE, 0, true));
    let mut old_area_fixture = prepared.clone();
    old_area_fixture.combat_level = 46;
    old_area_fixture.combat_stats = [40; 4];
    old_area_fixture.hitpoints = 40;
    assert!(
        !old_area_fixture.combat_can_relog(),
        "combat 46 does not clear the default level-24 sewer-zombie policy"
    );
    for invalid in 0..4 {
        let mut sample = prepared.clone();
        match invalid {
            0 => sample.combat_level = 48,
            1 => sample.inventory.clear(),
            2 => sample.hitpoints = 10,
            _ => sample.in_combat = true,
        }
        assert!(!witness_predicate(&sample, RUNE, 0, true));
        if invalid == 1 {
            assert!(
                sample.combat_can_relog(),
                "mainland inventory is published only after the clean relog"
            );
        }
    }
    for quest in [SHEEP, RUNE, ROMEO] {
        let mut completed = Sample {
            tick: 2,
            tile: fresh.tile,
            xp: quest.xp_delta,
            level: 1,
            mining_level: 1,
            cooking_level: 1,
            qp: quest.qp_delta,
            quest_colour: Some(0x00F800),
            inventory: vec![],
            main_root: 0,
            main_text: vec![],
            dialogue: vec![],
            chat: vec![],
            combat_level: 3,
            combat_stats: [1, 1, 1, 10],
            hitpoints: 10,
            in_combat: false,
            local_index: Some(1),
            nearby_npcs: vec![],
        };
        completed.main_text = match quest.key {
            "sheep" => vec![
                "You have completed the Sheep Shearer Quest!".into(),
                "150 Crafting XP".into(),
                "1 Quest Point".into(),
            ],
            "runemysteries" => vec![
                "You have completed the Rune Mysteries Quest!".into(),
                "Runecrafting skill".into(),
                "Air talisman".into(),
                "1 Quest Point".into(),
            ],
            _ => vec![
                "You have completed the".into(),
                "Romeo & Juliet Quest!".into(),
                "5 Quest Points".into(),
            ],
        };
        assert!(
            witness_predicate(&completed, quest, 0, false),
            "{} terminal witness",
            quest.key
        );
        completed.quest_colour = Some(0xF80000);
        assert!(
            !witness_predicate(&completed, quest, 0, false),
            "{} pending quest row",
            quest.key
        );
        completed.quest_colour = Some(0x00F800);
        assert!(
            !witness_predicate(&completed, quest, quest.xp_delta + 1, false),
            "{} incorrect XP delta",
            quest.key
        );
    }
    assert!(!native_run_completed(None, Some(1)));
    for state in [
        ScriptTerminalState::Stopped,
        ScriptTerminalState::Failed,
        ScriptTerminalState::Cancelled,
    ] {
        let receipt = ScriptLifecycleReceipt {
            runtime_generation: 4,
            state,
            tick: 1,
            reason: "fixture".into(),
        };
        assert!(!native_run_completed(Some(&receipt), Some(4)));
    }
    let completed = ScriptLifecycleReceipt {
        runtime_generation: 4,
        state: ScriptTerminalState::Completed,
        tick: 2,
        reason: "complete".into(),
    };
    assert!(native_run_completed(Some(&completed), Some(4)));
    assert!(!native_run_completed(Some(&completed), Some(5)));
}

struct Witness {
    snapshot: GameSnapshot,
    pump: Pump,
    log: fs::File,
    account: String,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    quest: Quest,
    relog_requested: bool,
    fixture_seeded: bool,
    relog_detached: bool,
    relogged: bool,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    base_xp: Option<i32>,
    reads: usize,
    pending: Option<i32>,
    read_tick: Option<u32>,
    last_tick: Option<u32>,
    product_captured: bool,
    native_complete: bool,
    error: Option<String>,
}
impl Witness {
    fn record(&mut self, value: Value) -> Result<(), String> {
        writeln!(self.log, "{value}").map_err(|e| e.to_string())?;
        self.log.flush().map_err(|e| e.to_string())
    }
    fn capture(&mut self, client: &mut Client, phase: &str, sample: &Sample) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let draw = client.draw;
        client.set_draw(true);
        let out = renderer.mainredraw(client);
        client.set_draw(draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = out else {
            return Err("377 Quester capture did not return CPU framebuffer".into());
        };
        let mut seeds = vec!["own-account tutorial bypass and mainland position, then clean relog"];
        if needs_combat_fixture(self.quest.key) {
            seeds.push(combat_food_seed(self.quest).receipt_label);
        }
        if self.quest.key == "squire" {
            seeds.push("labelled own-account quest minima: Mining 10 (content/scripts/quests/quest_squire/scripts/squire_journal.rs2:7-12) and Cooking 10 (content/scripts/skill_cooking/configs/cooking_inv/configs/pies/pies.struct:13-17); Smithing remains fresh at level 1 / 0 XP");
            seeds.push(SQUIRE_BANK_SEED_LABEL);
        }
        let bank_seeds = if self.quest.key == "squire" {
            json!([{"label":SQUIRE_BANK_SEED_LABEL,"item_alias":"iron_bar","count":2,"command":SQUIRE_BANK_SEED_COMMAND,"scope":"own-account bank"}])
        } else {
            json!([])
        };
        let quest_item_seeds = if self.quest.key == "squire" { 2 } else { 0 };
        let receipt = json!({"account":self.account,"phase":phase,"revision":377,"quest":self.quest.key,"sample":sample,"baseline":self.baseline,"driver":"production Play.script_start(Quester), bundled 377 quest path","renderer":"real Client CpuPix3D framebuffer","seeds":seeds,"bank_seeds":bank_seeds,"quest_state_seeds":0,"quest_item_seeds":quest_item_seeds,"reward_xp_seeds":0,"quest_point_seeds":0});
        self.record(receipt.clone())?;
        self.jobs.push(self.writer.submit(EvidenceRequest {
            png_path: self.directory.join(format!("{phase}.png")),
            width: pixels.width as u32,
            height: pixels.height as u32,
            pixels: pixels.pixels,
            color: PngColor::Rgba,
            sidecar: Some(EvidenceSidecar {
                path: self.directory.join(format!("{phase}.json")),
                receipt,
                patch_error: None,
            }),
        }));
        Ok(())
    }
    fn frame(&mut self, client: &mut Client, hold: bool) -> Result<(), String> {
        host::publish_snapshot(&mut self.snapshot, client, self.pump.drain_client(client));
        if self.relog_requested && !self.snapshot.ingame() {
            self.relog_detached = true
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let s = Sample::read(client, &self.snapshot, self.quest);
        self.latest = Some(s.clone());
        if self.last_tick != Some(s.tick) {
            self.record(json!({"phase":"observation","sample":s}))?;
            self.last_tick = Some(s.tick);
        }
        if !self.relogged {
            if self.relog_detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|t| t.index == 3 && t.available)
            {
                self.relogged = true;
                self.record(json!({"phase":"fixture-relogged","sample":s}))?;
            } else {
                if !self.relog_requested
                    && s.tick >= 3
                    && s.tile
                        .is_some_and(|(x, z, l)| lumbridge(WorldTile { x, z, level: l }))
                {
                    if needs_combat_fixture(self.quest.key) {
                        if !self.fixture_seeded {
                            let requirement_fixture = if self.quest.key == "squire" {
                                SQUIRE_REQUIREMENT_FIXTURE
                            } else {
                                &[]
                            };
                            let bank_seed: &[&str] = if self.quest.key == "squire" {
                                &[SQUIRE_BANK_SEED_COMMAND]
                            } else {
                                &[]
                            };
                            let food_seed = combat_food_seed(self.quest);
                            for command in COMBAT_STATS_FIXTURE
                                .iter()
                                .chain(std::iter::once(&food_seed.command))
                                .chain(requirement_fixture.iter())
                                .chain(bank_seed.iter())
                            {
                                let command = *command;
                                if !interact::cheat(client, command).is_sent() {
                                    return Err(format!(
                                        "combat fixture command refused: {command}"
                                    ));
                                }
                                let phase = if command == food_seed.command {
                                    food_seed.phase_label
                                } else {
                                    match command {
                                        "setstat mining 10" => {
                                            "SEED quest Mining 10 minimum (content/scripts/quests/quest_squire/scripts/squire_journal.rs2:7-12)"
                                        }
                                        "setstat cooking 10" => {
                                            "SEED redberry-pie Cooking 10 minimum (content/scripts/skill_cooking/configs/cooking_inv/configs/pies/pies.struct:13-17; content/scripts/skill_cooking/configs/cooking_source/cooking_generic.dbrow:201-212)"
                                        }
                                        SQUIRE_BANK_SEED_COMMAND => SQUIRE_BANK_SEED_LABEL,
                                        _ => "SEED own-account combat-only area fixture",
                                    }
                                };
                                if command == SQUIRE_BANK_SEED_COMMAND {
                                    self.record(json!({"phase":phase,"command":command,"scope":"own-account bank"}))?;
                                } else {
                                    self.record(json!({"phase":phase,"command":command}))?;
                                }
                            }
                            self.fixture_seeded = true;
                            return Ok(());
                        }
                        // Tutorial login has no inventory side tab yet. Squire's
                        // skill minima and the food fixture are verified after relog.
                        if !s.quest_pre_relog_ready(self.quest) {
                            return Ok(());
                        }
                        self.record(json!({"phase":"combat fixture visible; skill minima will be verified after clean relog","sample":s}))?;
                    }
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("could not cleanly logout mainland fixture".into());
                    }
                    self.relog_requested = true;
                    self.record(json!({"phase":"SEED own-account tutorial/mainland preparation; clean relog","sample":s}))?;
                }
                return Ok(());
            }
        }
        if self.baseline.is_none() {
            // The first post-relog frame can publish combat level and food
            // before named stat rows. Do not fail that partial frame.
            if !s.quest_fixture_ready(self.quest) {
                return Ok(());
            }
            if !witness_predicate(&s, self.quest, s.xp, true) {
                return Err(format!("fresh-account quest conditions failed: {s:?}"));
            }
            self.base_xp = Some(s.xp);
            self.capture(client, "fresh", &s)?;
            self.baseline = Some(s.clone());
            return Ok(());
        }
        if s.complete(self.quest, self.base_xp.unwrap_or(0)) && !self.product_captured {
            self.capture(client, "product-completion", &s)?;
            self.product_captured = true;
        }
        let expected = [
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: 0", self.quest.var),
            ),
            ("getvar qp".into(), "get qp: 0".into()),
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: {}", self.quest.var, self.quest.terminal),
            ),
            (
                "getvar qp".into(),
                format!("get qp: {}", self.quest.qp_delta),
            ),
        ];
        if self.reads < 4 && (self.reads < 2 || (self.product_captured && self.native_complete)) {
            if let Some(seq) = self.pending {
                if s.chat
                    .iter()
                    .any(|(n, text)| *n > seq && text == &expected[self.reads].1)
                {
                    self.record(json!({"phase":"SERVER READBACK observed","command":expected[self.reads].0,"reply":expected[self.reads].1,"sample":s}))?;
                    self.reads += 1;
                    self.pending = None;
                }
            } else if self.read_tick != Some(s.tick) {
                let before = s.chat.iter().map(|(n, _)| *n).max().unwrap_or(-1);
                if !interact::cheat(client, &expected[self.reads].0).is_sent() {
                    return Err("own-account getvar read refused".into());
                }
                self.record(json!({"phase":"SERVER READBACK request, own-account read-only","command":expected[self.reads].0,"after_sequence":before}))?;
                self.pending = Some(before);
                self.read_tick = Some(s.tick);
            }
        }
        Ok(())
    }
}

fn run_quest(q: Quest) -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let account = host_play::mint_live_names(1).remove(0);
    let directory = required_path("QUESTER_377_EVIDENCE_DIR")?.join(&account);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let log = fs::File::create(directory.join("observations.jsonl")).map_err(|e| e.to_string())?;
    let writer = Arc::new(EvidenceWriter::new(4));
    let state = Arc::new(Mutex::new(Witness {
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        log,
        account: account.clone(),
        directory,
        writer: writer.clone(),
        jobs: Vec::new(),
        quest: q,
        relog_requested: false,
        relog_detached: false,
        relogged: false,
        fixture_seeded: false,
        baseline: None,
        latest: None,
        base_xp: None,
        reads: 0,
        pending: None,
        read_tick: None,
        last_tick: None,
        product_captured: false,
        native_complete: false,
        error: None,
    }));
    let frame = state.clone();
    let name = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |c, u, f| {
            if u == name {
                let mut s = frame.lock().unwrap();
                if s.error.is_none() {
                    if let Err(e) = s.frame(c, f.hold) {
                        s.error = Some(e)
                    }
                }
            }
        },
    )?;
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: host_play::profile_password(&account).into(),
            uid: fresh_uid(300),
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    let budget = if matches!(q.key, "hetty" | "gobdip" | "vampire" | "demon" | "squire") {
        3600
    } else {
        1500
    };
    let deadline = Instant::now() + Duration::from_secs(budget);
    let prep = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut relogin_requested = false;
    let mut complete = false;
    let mut generation = None;
    let mut lifecycle = None;
    let mut last_status = String::new();
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("native Quester lifecycle: {error}"));
        }
        let status = play.script_native_status(&account);
        let receipt = play.script_lifecycle_receipt(&account);
        let mut s = state.lock().map_err(|_| "witness poisoned")?;
        if let Some(error) = &s.error {
            return Err(error.clone());
        }
        if !relogin_requested && s.relog_detached {
            drop(s);
            if let Some(arm) = play.arm(&account) {
                arm.arm_explicit_login();
                relogin_requested = true;
            }
            continue;
        }
        // Initial readbacks must precede Start, not race the first quest action.
        if !started && s.baseline.is_some() && s.reads == 2 {
            s.record(json!({"phase":"native Quester start", "quest":q.key}))?;
            drop(s);
            play.script_start(
                &account,
                script::CompiledId("Quester"),
                serde_json::Map::from_iter([("quests".into(), json!([q.key]))]),
            )?;
            started = true;
            continue;
        }
        if let Some(status) = status {
            let detail = format!("{status:?}");
            if detail != last_status {
                let event = json!({"phase":"native status", "status":detail, "latest":s.latest});
                s.record(event)?;
                last_status = detail;
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                return Err(format!("native Quester failed: {status:?}"));
            }
            generation.get_or_insert(status.run.run);
        }
        if !complete && native_run_completed(receipt.as_ref(), generation) {
            s.record(json!({"phase":"native lifecycle", "receipt":receipt}))?;
            lifecycle = receipt;
            complete = true;
            s.native_complete = true;
        }
        if complete && s.product_captured && s.reads == 4 {
            let event = json!({"outcome":"PASS", "account":account, "native_lifecycle":lifecycle, "baseline":s.baseline, "latest":s.latest});
            s.record(event)?;
            return Ok(());
        }
        if Instant::now() >= deadline || (!started && Instant::now() >= prep) {
            return Err(format!(
                "quest witness deadline started={started} complete={complete} reads={} latest={:?}",
                s.reads, s.latest
            ));
        }
        drop(s);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let mut s = state.lock().map_err(|_| "witness poisoned")?;
    let event = json!({"phase":"final","outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),"quest":q.key,"latest":s.latest});
    s.record(event)?;
    let jobs = s.jobs.clone();
    drop(s);
    for job in jobs {
        let outcome = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("screenshot write timed out")?;
        if let Some(e) = outcome.error {
            return Err(e);
        }
    }
    result
}

#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_sheep_native() -> Result<(), String> {
    run_quest(SHEEP)
}
#[test]
#[ignore = "requires LIVE=1, owned qm3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_runemysteries_native() -> Result<(), String> {
    run_quest(RUNE)
}
#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_romeojuliet_native() -> Result<(), String> {
    run_quest(ROMEO)
}
fn batch2_product(quest: Quest, extra: &[&str]) -> Sample {
    let mut sample = Sample {
        tick: 2,
        tile: Some((3200, 3200, 0)),
        xp: quest.xp_delta,
        level: 1,
        mining_level: 1,
        cooking_level: 1,
        qp: quest.qp_delta,
        quest_colour: Some(0x00F800),
        inventory: vec![],
        main_root: 0,
        main_text: vec!["You have completed the".into()],
        dialogue: vec![],
        chat: vec![],
        combat_level: 49,
        combat_stats: [43; 4],
        hitpoints: 43,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    if let Some(second) = split_completion(quest.key) {
        sample.main_text.push(second.into());
    }
    sample.main_text.push(quest.qp_reward.into());
    sample
        .main_text
        .extend(extra.iter().map(|line| (*line).into()));
    sample
}

#[test]
fn batch2_witness_accepts_the_377_scroll_and_rejects_a_red_row() {
    for (quest, extra) in [
        (HETTY, &["325 Magic XP"][..]),
        (GOBDIP, &["200 Crafting XP", "A Gold Bar"][..]),
        (VAMPIRE, &["4,825 Attack XP"][..]),
        (DEMON, &["Silverlight"][..]),
        (SQUIRE, &["12,725 Smithing XP"][..]),
    ] {
        let base = if quest.key == "vampire" { 1154 } else { 0 };
        let mut sample = batch2_product(quest, extra);
        sample.xp = base + quest.xp_delta;
        if quest.key == "vampire" {
            sample.xp += 40;
        }
        assert!(
            witness_predicate(&sample, quest, base, false),
            "{} product",
            quest.key
        );
        sample.quest_colour = Some(0xF80000);
        assert!(
            !witness_predicate(&sample, quest, base, false),
            "{}",
            quest.key
        );
    }
}

#[test]
fn squire_witness_rejects_invalid_baseline_and_partial_product_evidence() {
    let baseline = Sample {
        tick: 1,
        tile: Some((3000, 3150, 0)),
        xp: 0,
        level: SQUIRE_SMITHING_LEVEL,
        mining_level: SQUIRE_MINING_LEVEL,
        cooking_level: SQUIRE_COOKING_LEVEL,
        qp: 0,
        quest_colour: Some(0xF80000),
        inventory: vec![(379, 6)],
        main_root: 0,
        main_text: vec![],
        dialogue: vec![],
        chat: vec![],
        combat_level: 49,
        combat_stats: [43; 4],
        hitpoints: 43,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    assert!(witness_predicate(&baseline, SQUIRE, baseline.xp, true));
    let mut five_trout = baseline.clone();
    five_trout.inventory = vec![(333, 5)];
    assert!(
        !witness_predicate(&five_trout, SQUIRE, baseline.xp, true),
        "Squire requires its lobster food, not the other combat fixture's trout"
    );
    let mut short_lobster_seed = baseline.clone();
    short_lobster_seed.inventory = vec![(379, 5)];
    assert!(
        !witness_predicate(&short_lobster_seed, SQUIRE, baseline.xp, true),
        "Squire's blurite step requires six lobsters"
    );
    let mut pre_relog = baseline.clone();
    pre_relog.level = 1;
    pre_relog.mining_level = 1;
    pre_relog.cooking_level = 1;
    assert!(pre_relog.quest_pre_relog_ready(SQUIRE));
    pre_relog.in_combat = true;
    assert!(!pre_relog.quest_pre_relog_ready(SQUIRE));

    let mut invalid_baseline = baseline.clone();
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL;
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL;
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL;
    invalid_baseline.xp = 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.xp = 0;
    assert!(!witness_predicate(&baseline, SQUIRE, baseline.xp + 1, true));
    invalid_baseline.qp = 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.qp = 0;
    invalid_baseline.quest_colour = Some(0x00F800);
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));

    let mut product = batch2_product(SQUIRE, &["12,725 Smithing XP"]);
    product.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(witness_predicate(&product, SQUIRE, baseline.xp, false));
    for unexpected_xp in [-1, 1, 12, 13, 25, 26] {
        let mut invalid_product = product.clone();
        invalid_product.xp += unexpected_xp;
        assert!(
            !witness_predicate(&invalid_product, SQUIRE, baseline.xp, false),
            "unexpected Smithing XP delta: {unexpected_xp}"
        );
    }
    product.qp += 1;
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));
    product.qp = SQUIRE.qp_delta;
    product.quest_colour = Some(0xF80000);
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));
    product.quest_colour = Some(0x00F800);
    product
        .main_text
        .retain(|line| line != "Knight's Sword Quest!");
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));

    let mut wrong_reward_text = batch2_product(SQUIRE, &["12,750 Smithing XP"]);
    wrong_reward_text.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(!witness_predicate(
        &wrong_reward_text,
        SQUIRE,
        baseline.xp,
        false
    ));

    let mut product_without_xp = batch2_product(SQUIRE, &[]);
    product_without_xp.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(!witness_predicate(
        &product_without_xp,
        SQUIRE,
        baseline.xp,
        false
    ));
}

#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_hetty_native() -> Result<(), String> {
    run_quest(HETTY)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_gobdip_native() -> Result<(), String> {
    run_quest(GOBDIP)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_vampire_native() -> Result<(), String> {
    run_quest(VAMPIRE)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_demon_native() -> Result<(), String> {
    run_quest(DEMON)
}

#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_squire_native() -> Result<(), String> {
    run_quest(SQUIRE)
}
