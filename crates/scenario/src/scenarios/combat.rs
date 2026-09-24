use super::production::*;
use crate::*;
mod chaos_druid;
use chaos_druid::CHAOS_DRUID_FIXTURE_LOADOUTS;
pub(crate) use chaos_druid::*;
mod moss_giant;
use moss_giant::MOSS_GIANT_FIXTURE_LOADOUTS;
pub(crate) use moss_giant::*;
mod hill_giant;
use hill_giant::HILL_GIANT_FIXTURE_LOADOUTS;
pub(crate) use hill_giant::*;
mod auto_fighter;
pub(crate) use auto_fighter::*;
mod rock_crab;
use rock_crab::ROCK_CRAB_FIXTURE_LOADOUTS;
pub(crate) use rock_crab::*;
mod green_dragon;
pub(crate) use green_dragon::*;

pub(crate) const TROUT_ID: i32 = 333;
pub(crate) const COMBAT_SCIMITAR_ID: i32 = 1331;
const MIND_RUNE_ID: i32 = 558;
pub(crate) const BIG_BONES_ID: i32 = 532;
const NOTED_BIG_BONES_ID: i32 = 533;
pub(crate) const LIMPWURT_ROOT_ID: i32 = 225;
const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
pub(crate) const LAW_RUNE_ID: i32 = 563;
const AUTO_FIGHTER_MAGE_CASTS: i32 = 150;
const AUTO_FIGHTER_MAGE_AIR_RUNES: i32 = AUTO_FIGHTER_MAGE_CASTS * 2;
const AUTOCAST_MAGIC_VARP: i32 = 108;
const AUTOCAST_ARMED_VALUE: i32 = 3;
const RANGED_STAT: i32 = 4;
const COMBAT_MODE_VARP: i32 = 43;
const RAPID_COMBAT_MODE: i32 = 1;
const MAPLE_SHORTBOW_ID: i32 = 853;
pub(crate) const BRONZE_ARROW_ID: i32 = 882;
const RANGE_AMMO: i32 = 200;
/// Spare bank stock so the keep-list deposit can withdraw teleStock+1 again.
const CAMELOT_BANK_AIR: i32 = 20;
const CAMELOT_BANK_LAW: i32 = 5;
const GREEN_DRAGON_BASE_FOOD: i32 = 20;
const GREEN_DRAGON_FOOD: i32 = 12;
pub(crate) const COMBAT_ATTACK_LEVEL: i32 = 40;
const REMAINING_COMBAT_PREPARED_LEVEL: i32 = 70;
const BANK_PRESSURE_PREPARED_LEVEL: i32 = 99;
const COMBAT_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(300);
/// Runner dirty-snapshot increments, not engine ticks or wall seconds. Live
/// combat receipts run near two dirty increments/second; 750 keeps the named
/// 300s wall as the controlling bound instead of the ordinary 150-dirty arm.
const COMBAT_QUALIFICATION_WATCH_TICKS: u32 = 750;
/// Measured GreenDragon pressure reached its loaded deposit 268s after the
/// post-Start baseline, before the roughly 320-tile return and resumed combat.
/// Keep the same 2.5 dirty increments/second allowance as the 300s/750 arm.
const BANK_QUALIFICATION_WATCH_TICKS: u32 = 1500;
pub(crate) const RUNE_SCIMITAR_ID: i32 = 1333;
const DEFENCE_STAT: i32 = 1;
const RUNE_PLATELEGS_ID: i32 = 1079;
const RUNE_FULL_HELM_ID: i32 = 1163;
pub(crate) const BRASS_KEY_ID: i32 = 983;

const ROCK_CRAB_SPOT: WorldTile = WorldTile {
    x: 2704,
    z: 3726,
    level: 0,
};
const SEERS_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3491,
    level: 0,
};

const GREEN_DRAGON_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[
    FixtureLoadout {
        name: "Scenario Green Dragon food",
        carry: &[("Lobster", GREEN_DRAGON_BASE_FOOD as u32)],
    },
    FixtureLoadout {
        name: "Scenario Green Dragon trip food",
        carry: &[("Lobster", GREEN_DRAGON_FOOD as u32)],
    },
];
const FIRE_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Fire Giant food",
    carry: &[("Lobster", FIRE_GIANT_FOOD as u32)],
}];

fn combat_fixture_loadouts(card: &str) -> Option<&'static [FixtureLoadout]> {
    match card {
        "ChaosDruidKiller" => Some(CHAOS_DRUID_FIXTURE_LOADOUTS),
        "RockCrab" => Some(ROCK_CRAB_FIXTURE_LOADOUTS),
        "MossGiant" => Some(MOSS_GIANT_FIXTURE_LOADOUTS),
        "HillGiant" => Some(HILL_GIANT_FIXTURE_LOADOUTS),
        "GreenDragon" => Some(GREEN_DRAGON_FIXTURE_LOADOUTS),
        "FireGiant" => Some(FIRE_GIANT_FIXTURE_LOADOUTS),
        _ => None,
    }
}

/// Shared melee-core seed: legal stats, food, ordinary weapon, empty loot,
/// then Start. Banking policy is explicit in the inject; these cells are not
/// bank-roundtrip proof. Scenario watch is selected-style XP; catalog adds
/// two engagements, a verified defeat, and exact loot where required.
struct CombatCorePlan {
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    /// Pre-Start wear step: the step's own name plus the item id its
    /// `Proof::EquipmentId` arm acknowledges. The label belongs to the item
    /// ("Dragonfire shield", "Adamant scimitar"), so it stays accurate when
    /// the helper is reused outside the shield cells.
    wear: Option<(&'static str, i32)>,
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    /// The native quest prerequisite completed before the stat/inventory
    /// reset and the hostile-field teleport (never a fabricated stage: the
    /// quest's own completion script runs and its journal row is
    /// acknowledged before Start).
    complete_quest: Option<NativeQuestPrereq>,
    thieving: i32,
    agility: i32,
}

#[derive(Clone, Copy)]
struct PreparedCombatPlan {
    level: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    wear: &'static [(&'static str, i32)],
}

const TIER40_RUNE_ARMOUR_GIVE: &[(&str, i32, i32)] = &[
    ("rune_chainbody", RUNE_CHAINBODY_ID, 1),
    ("rune_platelegs", RUNE_PLATELEGS_ID, 1),
    ("rune_full_helm", RUNE_FULL_HELM_ID, 1),
];

const TIER40_RUNE_ARMOUR_WEAR: &[(&str, i32)] = &[
    (
        "wear and acknowledge Rune chainbody before hostile-field teleport",
        RUNE_CHAINBODY_ID,
    ),
    (
        "wear and acknowledge Rune platelegs before hostile-field teleport",
        RUNE_PLATELEGS_ID,
    ),
    (
        "wear and acknowledge Rune full helm before hostile-field teleport",
        RUNE_FULL_HELM_ID,
    ),
];

fn combat_core_scenario(plan: CombatCorePlan) -> Scenario {
    combat_core_scenario_with_preparation(plan, None)
}

fn prepared_combat_core_scenario(
    plan: CombatCorePlan,
    preparation: PreparedCombatPlan,
) -> Scenario {
    let mut scenario = combat_core_scenario_with_preparation(plan, Some(preparation));
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

fn apply_combat_qualification_budget(
    scenario: &mut Scenario,
    deadline: Duration,
    watch_ticks: u32,
) {
    scenario.settings.deadline = deadline;
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat qualification has a Start step");
    for step in &mut scenario.steps[start + 1..] {
        step.wait.budget_ticks = step.wait.budget_ticks.max(watch_ticks);
    }
}

fn combat_core_scenario_with_preparation(
    plan: CombatCorePlan,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let CombatCorePlan {
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        wear,
        loot_empty,
        inject,
        complete_quest,
        thieving,
        agility,
    } = plan;
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    let combat_level = preparation
        .map(|prepared| prepared.level)
        .unwrap_or(COMBAT_ATTACK_LEVEL);
    if let Some(prereq) = complete_quest {
        steps.extend(quest_prereq_steps(prereq));
    }
    steps.push(Step {
        name: "prepare melee stats, food and gear on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {combat_level}"));
                cheat(c, &format!("setstat strength {combat_level}"));
                cheat(c, &format!("setstat hitpoints {combat_level}"));
                if let Some(preparation) = preparation {
                    cheat(c, &format!("setstat defence {}", preparation.level));
                }
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                if agility > 0 {
                    cheat(c, &format!("setstat agility {agility}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {weapon_alias} 1"));
                if food_count > 0 {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                if let Some(preparation) = preparation {
                    for &(alias, _, count) in preparation.extra_give {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: combat_level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Attack profile",
        Proof::Stat {
            id: 0,
            min: combat_level,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Strength profile",
        Proof::Stat {
            id: STRENGTH_STAT,
            min: combat_level,
        },
    ));
    if let Some(preparation) = preparation {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Defence profile",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: preparation.level,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Hitpoints profile",
        Proof::Stat {
            id: 3,
            min: combat_level,
        },
    ));
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if agility > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Agility before Start",
            Proof::Stat {
                id: AGILITY_STAT,
                min: agility,
            },
        ));
    }
    if food_count > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared food in pack before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge prepared weapon before Start",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    if let Some(preparation) = preparation {
        for &(_, id, count) in preparation.extra_give {
            steps.push(bank_fletcher_watch(
                "acknowledge prepared tier-40 armour before Start",
                Proof::ItemId { id, count },
            ));
        }
    }
    if let Some((label, id)) = wear {
        steps.push(wear_combat_item_step(label, id));
    }
    if let Some(preparation) = preparation {
        for &(label, id) in preparation.wear {
            steps.push(wear_combat_item_step(label, id));
        }
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded combat loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch Strength XP from the selected melee style after Start",
        xp,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            fixture_loadouts: combat_fixture_loadouts(card),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// A quest prerequisite the fixture completes through the **native** debug
/// quest-journal command `~quest` (`_test/scripts/cheats/cheat_quest.rs2`
/// `@debug_quests`): "Select Individual Quest." → the quest's row on the
/// paginated `p_choice5_header` list → "Complete.", which queues that
/// quest's own `[queue,<quest>_complete]` script. The path is targeted: the
/// required quest's completion is the only script queued, so no *other*
/// completion's rewards can arrive after Start (the demonstrated
/// `~completequests` leak: `QuestDone(Waterfall)` returned while ~60 queued
/// completions still owed stats/items/dialogs).
///
/// `dialog` is the quest's `quest_names_enum` label
/// (`content/scripts/general/configs/quest.enum`: index 34 `Lost City`,
/// index 50 `Waterfall Quest`) — the text the native list renders as
/// `<index>. <label>`; `journal` is the quest-tab row the native
/// `~send_quest_complete` helper paints green
/// (`content/scripts/player/interfaces/questlist.if` `[zanaris]`/`[waterfall]`,
/// `content/scripts/general/scripts/quests.rs2`), the acknowledgement the
/// fixture waits for.
///
/// The acknowledgement is the completion boundary itself, verified in clean
/// native content: `quest_zanaris.rs2` `[queue,zanaris_quest_complete]` sets
/// `%zanaris = ^zanaris_complete` (the var `levelrequire_zanaris_quest` gates
/// `opheld2 dragon_dagger` on, `levelrequire/scripts/levelrequire.rs2`) and
/// then calls `~send_quest_complete(questlist:zanaris, …)`; the FireGiant
/// counterpart `[queue,waterfall_quest_complete]`
/// (`quest_waterfall/scripts/quest_waterfall.rs2`) banks 2 diamonds, 2 gold
/// bars, 40 mithril seeds and 137,500 attack + strength XP before the same
/// helper. Every reward statement precedes the green, and `send_quest_complete`
/// is the last call in both scripts, so a fixture that waits for the green and
/// then resets stats/inventory cannot have quest rewards land after Start.
#[derive(Clone, Copy)]
pub(crate) struct NativeQuestPrereq {
    dialog: &'static str,
    journal: &'static str,
}

/// Shilo Village (`quest_names_enum` 43, journal row `[zombiequeen]` text
/// `Shilo Village`): `%zombiequeen >= ^zombiequeen_complete` (15) is the
/// wooden-gate membership (`quest_zombiequeen.rs2` `[oploc1,_shilo_woodengate]`).
pub(super) const SHILO_VILLAGE_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Shilo Village",
    journal: "Shilo Village",
};

/// The native quest-journal command and the labels of its individual-quest
/// path: the first dialog's "Select Individual Quest." branch button, the
/// paginated list's "Next." button and the per-quest "Complete."
/// confirmation. The labels are the dialog's own resume-button texts, so
/// the machine presses a button by what it says, never by page arithmetic.
const QUEST_JOURNAL_CHEAT: &str = "~quest";
const QUEST_SELECT_INDIVIDUAL: &str = "Select Individual Quest.";
const QUEST_PAGE_NEXT: &str = "Next.";
const QUEST_COMPLETE_LABEL: &str = "Complete.";

/// The dialog this machine has already answered: the chat modal it was
/// rendered in and the option texts it carried. `chat.rs2` reuses one
/// interface per dialog shape (`multi4`, `multi5`, `multi3`) and
/// re-registers the *same* components as resume buttons for the question
/// that follows (`if_addresumebutton(multi5:com_5)`), and the engine
/// resumes a paused script with the clicked component
/// (`IfButtonHandler` → `p_pausebutton` → `last_com`). A press repeated
/// into the replaced dialog is therefore not inert — the page list's
/// `Next.` would advance a second page. One press per distinct dialog;
/// the step then waits for the next one, like the player it stands in for.
#[derive(Default)]
pub(crate) struct QuestJournalState {
    answered: Option<(i32, String)>,
}

/// One tick of the native quest-journal dialog machine: continue a
/// `BUTTON_CONTINUE` chat IF (the completion's level-ups), else answer the
/// option dialog the individual-quest path waits on (Select Individual
/// Quest. → the required quest's page row → Complete.), else close the
/// modal a completion opened (the quest scroll `send_quest_complete` opens
/// just before it paints the row green). Returns `false` when an option
/// dialog cannot be placed: the preparation step fails explicitly instead
/// of guessing a button.
pub(crate) fn answer_quest_journal_dialogs(
    client: &mut Client,
    snapshot: &GameSnapshot,
    prereq: NativeQuestPrereq,
    state: &mut QuestJournalState,
) -> bool {
    if snapshot.chat_continue_component_id() != -1 {
        let mut ix = Interactions::new(snapshot, client);
        return matches!(
            ix.continue_dialog(),
            SendResult::Sent { .. } | SendResult::Refused { .. }
        );
    }
    if !snapshot.chat_options().is_empty() {
        let identity = (
            snapshot.modals().chat,
            snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if state.answered.as_ref() == Some(&identity) {
            // Already answered this dialog; the next one has not landed.
            return true;
        }
        let choose = |wanted: &dyn Fn(&str) -> bool| {
            snapshot
                .chat_options()
                .iter()
                .position(|option| wanted(option.text.as_str()))
        };
        let choice = choose(&|text| text == QUEST_SELECT_INDIVIDUAL)
            .or_else(|| choose(&|text| text == QUEST_COMPLETE_LABEL))
            .or_else(|| choose(&|text| text.ends_with(prereq.dialog)))
            .or_else(|| choose(&|text| text == QUEST_PAGE_NEXT));
        let Some(choice) = choice else {
            return false;
        };
        let mut ix = Interactions::new(snapshot, client);
        return match ix.answer_choice(choice as i32 + 1) {
            SendResult::Sent { .. } => {
                state.answered = Some(identity);
                true
            }
            // Nothing pressable in that snapshot: retry on the next tick.
            SendResult::Refused { .. } => true,
        };
    }
    let m = snapshot.modals();
    if m.main == -1 && m.side == -1 && m.chat == -1 && m.tutorial == -1 {
        return true;
    }
    let mut ix = Interactions::new(snapshot, client);
    matches!(
        ix.close_modal(),
        SendResult::Sent { .. } | SendResult::Refused { .. }
    )
}

/// Pre-Start native quest prerequisite: send `~quest`, answer its
/// individual-quest dialogs until the required quest's "Complete." queued
/// that quest's own completion script, then drain the completion's
/// scroll/level-up dialogs until the native helper paints the quest's
/// journal row green. That green row is the completion boundary: the stat
/// and inventory reset, the prepared-gear acknowledgements and the
/// hostile-field teleport all follow it, so no quest reward can arrive
/// after Start. The step's tick budget bounds a preparation that never
/// acknowledges — the run fails before Start instead of starting without
/// the prerequisite.
pub(super) fn quest_prereq_steps(prereq: NativeQuestPrereq) -> [Step; 2] {
    // The machine is re-entered once per tick by the runner's `Repeat`
    // arm; its "already answered this dialog" state is behind a mutex so
    // the step closure stays `Fn + Send + Sync` (the `StepKind` bound).
    let state = std::sync::Mutex::new(QuestJournalState::default());
    [
        Step {
            name: "open the native quest-journal prerequisite dialog before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, QUEST_JOURNAL_CHEAT);
                    true
                }),
            },
            wait: Wait {
                arm: Proof::ChatChoice,
                budget_ticks: 20,
            },
        },
        Step {
            name: "answer the quest-journal dialogs until the required quest is acknowledged",
            kind: StepKind::Repeat {
                send: Box::new(move |c, snapshot| {
                    let Ok(mut state) = state.lock() else {
                        return false;
                    };
                    answer_quest_journal_dialogs(c, snapshot, prereq, &mut state)
                }),
            },
            wait: Wait {
                arm: Proof::QuestDone {
                    name: prereq.journal,
                },
                budget_ticks: 600,
            },
        },
    ]
}

pub(super) fn wear_combat_item_step(name: &'static str, id: i32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).wear(id),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id },
            budget_ticks: 200,
        },
    }
}

/// Select and acknowledge the native aggressive melee style after wielding.
pub(crate) fn is_aggressive_combat_style(label: &str) -> bool {
    label.to_ascii_lowercase().contains("aggressive")
}

pub(super) fn select_strength_combat_style_step() -> Step {
    Step {
        name: "select and acknowledge native Strength combat style before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                let Some(root) = snapshot
                    .side_tabs()
                    .iter()
                    .find(|tab| tab.index == 0 && tab.available)
                    .map(|tab| tab.root_component_id)
                else {
                    return false;
                };
                let Some(style) =
                    api::query::widget_search::combat_style_labels(snapshot, root, 43)
                        .into_iter()
                        .find(|style| style.mode == 1 && is_aggressive_combat_style(&style.label))
                else {
                    return false;
                };
                let ctx = ReadContext::new(snapshot);
                let Some(widget) = ctx.component(style.component_id) else {
                    return false;
                };
                matches!(
                    Interactions::new(snapshot, c).press(widget),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::VarpExact { id: 43, value: 1 },
            budget_ticks: 200,
        },
    }
}

/// Prepared bow/quiver branch. The frozen script owns combat-mode selection,
/// target engagement, ammunition use and (for RockCrab) projectile recovery.
#[allow(clippy::too_many_arguments)]
fn combat_range_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    inject: &'static [ScriptSettingInject],
    dormant_rocks: bool,
) -> Scenario {
    let ranged_xp = Proof::StatXpGain {
        id: RANGED_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Ranged stats, food, bow and arrows on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat ranged {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "~clearinv");
                cheat(c, "give maple_shortbow 1");
                cheat(c, &format!("give bronze_arrow {RANGE_AMMO}"));
                cheat(c, &format!("give {food_alias} {food_count}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: RANGED_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Hitpoints before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge prepared range food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ),
        (
            "acknowledge Maple shortbow before wielding",
            Proof::ItemId {
                id: MAPLE_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "acknowledge exact Bronze arrow stack before equipping",
            Proof::ItemId {
                id: BRONZE_ARROW_ID,
                count: RANGE_AMMO,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Maple shortbow before hostile-field teleport",
        MAPLE_SHORTBOW_ID,
    ));
    steps.push(wear_combat_item_step(
        "equip and acknowledge Bronze arrows before hostile-field teleport",
        BRONZE_ARROW_ID,
    ));
    steps.push(Step {
        name: "teleport into the ranged field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    if dormant_rocks {
        steps.push(bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ));
    }
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the frozen script select rapid ranged mode",
        Proof::Varp {
            id: COMBAT_MODE_VARP,
            min: RAPID_COMBAT_MODE,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Ranged XP from actual ammunition combat after Start",
        ranged_xp,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: ranged_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Shared bank-cell fixture: the same safe-tile preparation and acknowledgement
/// order as `combat_core_scenario`, plus the acknowledged bank stock the trip
/// has to draw from, the scoped weapon itself (seeded, acknowledged carried,
/// then worn — never a card stash: the card's own deposit must not be able to
/// stash it), and the bank/return watch chain after Start.
#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    thieving: i32,
    bank_alias: &'static str,
    bank_food_count: i32,
    watches: &[(&'static str, Proof)],
) -> Scenario {
    combat_bank_scenario_with_preparation(
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        loot_empty,
        inject,
        thieving,
        bank_alias,
        bank_food_count,
        watches,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario_with_preparation(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    thieving: i32,
    bank_alias: &'static str,
    bank_food_count: i32,
    watches: &[(&'static str, Proof)],
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    let combat_level = preparation
        .map(|prepared| prepared.level)
        .unwrap_or(COMBAT_ATTACK_LEVEL);
    steps.push(Step {
        name: "prepare melee stats, trip stock and bank stock on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {combat_level}"));
                cheat(c, &format!("setstat strength {combat_level}"));
                cheat(c, &format!("setstat hitpoints {combat_level}"));
                if let Some(preparation) = preparation {
                    cheat(c, &format!("setstat defence {}", preparation.level));
                }
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {weapon_alias} 1"));
                // Zero-count food is a declared empty baseline, never a
                // `give`: the native `::give` handler clamps to at least
                // one (`Math.max(1, …)`), so `give cake 0` seeded the Cake
                // the ardy_fighter_bank baseline is supposed to lack.
                // Full-pack prepared cells wear their kit first, then stock
                // food below. Giving all armour plus 24/26 food in one debug
                // command batch would exceed the native 28-slot pack.
                if food_count > 0 && preparation.is_none() {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                if let Some(preparation) = preparation {
                    for &(alias, _, count) in preparation.extra_give {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                if bank_food_count > 0 {
                    cheat(c, &format!("givebank {bank_alias} {bank_food_count}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: combat_level,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Attack profile",
            Proof::Stat {
                id: 0,
                min: combat_level,
            },
        ),
        (
            "acknowledge prepared Strength profile",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: combat_level,
            },
        ),
        (
            "acknowledge prepared Hitpoints profile",
            Proof::Stat {
                id: 3,
                min: combat_level,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if let Some(preparation) = preparation {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Defence profile",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: preparation.level,
            },
        ));
    }
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if food_count > 0 && preparation.is_none() {
        steps.push(bank_fletcher_watch(
            "acknowledge the trip's carried food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    if let Some(preparation) = preparation {
        for &(_, id, count) in preparation.extra_give {
            steps.push(bank_fletcher_watch(
                "acknowledge prepared tier-40 armour before Start",
                Proof::ItemId { id, count },
            ));
        }
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded kill loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the weapon carried onto the safe tile",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    steps.push(wear_combat_item_step(
        "wield and acknowledge the weapon before the hostile-field teleport",
        weapon_id,
    ));
    if let Some(preparation) = preparation {
        for &(label, id) in preparation.wear {
            steps.push(wear_combat_item_step(label, id));
        }
        if food_count > 0 {
            steps.push(Step {
                name: "stock prepared trip food after wearing the combat kit",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &format!("give {food_alias} {food_count}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ItemId {
                        id: food_id,
                        count: food_count,
                    },
                    budget_ticks: 200,
                },
            });
        }
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    // Bank-first cells deliberately start with banking (no food and/or seeded
    // deposit cargo). Their ordered watches require combat after the return;
    // waiting for incidental auto-retaliation XP here gates the wrong phase.
    const COMBAT_BANK_SKIPS_PREFIGHT_XP: &[&str] = &[
        "chaos_druid_bank",
        "moss_giant_bank_start",
        "green_dragon_tele_prepared",
    ];
    if !COMBAT_BANK_SKIPS_PREFIGHT_XP.contains(&name) {
        steps.push(bank_fletcher_watch(
            "watch Strength XP from the selected melee style after Start",
            xp,
        ));
    }
    for (step_name, arm) in watches {
        steps.push(bank_fletcher_watch(step_name, *arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            fixture_loadouts: combat_fixture_loadouts(card),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn remaining_prepared_combat_bank_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    bank_alias: &'static str,
    bank_food_count: i32,
    prepared_level: i32,
    deadline: Duration,
    watch_ticks: u32,
    watches: &[(&'static str, Proof)],
) -> Scenario {
    let mut scenario = combat_bank_scenario_with_preparation(
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        loot_empty,
        inject,
        0,
        bank_alias,
        bank_food_count,
        watches,
        Some(PreparedCombatPlan {
            level: prepared_level,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    );
    apply_combat_qualification_budget(&mut scenario, deadline, watch_ticks);
    scenario
}

mod ardy_fighter;
mod fire_giant;
pub(crate) use ardy_fighter::*;
use fire_giant::FIRE_GIANT_FOOD;
pub(crate) use fire_giant::*;
