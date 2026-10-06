//! The two assigned Slayer quests' pre-Start restock setup.
use super::quester_live::{Cell, Mode, PathCell};
use api::snapshot::WorldTile;
use scenario::quester::FixtureLoadout;

const BANK_KIT: &[(&str, i32)] = &[
    ("rune_full_helm", 3),
    ("rune_chainbody", 3),
    ("rune_platelegs", 3),
    ("rune_kiteshield", 3),
    ("rune_scimitar", 3),
    ("coins", 1000),
    ("lobster", 30),
    ("4dose2attack", 3),
    ("4dose2strength", 3),
    ("4doseprayerrestore", 6),
];

pub struct Case<'a> {
    pub quest: &'static str,
    pub display: &'static str,
    pub label: &'a str,
    pub stage: &'a str,
    pub loadout: &'a str,
    pub items: &'a [(&'a str, i32)],
    pub stand: WorldTile,
    pub mode: Mode,
    /// A fixture-only varp variant of Demon's folded journal stage.
    pub fixture_varp: Option<i32>,
}

fn seed_step(command: String) -> scenario::Step {
    scenario::Step {
        name: "seed auxiliary state before final relog and Start",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 100,
        },
    }
}

pub fn cell(case: Case<'_>) -> Cell {
    let mut before_relog = BANK_KIT
        .iter()
        .map(|(alias, qty)| seed_step(format!("givebank {alias} {qty}")))
        .collect::<Vec<_>>();
    if let Some(value) = case.fixture_varp {
        // traiborn.rs2:2-3,77-80: varp 28 is the lost-key variant of
        // journal stage demon:2. This never changes the product Path.
        assert_eq!(case.quest, "demon");
        before_relog.push(seed_step(format!("setvar demonstart {value}")));
    }
    let fixture_varp = case.fixture_varp;
    let mut cell = super::quester_live::path_cell(PathCell {
        quest: case.quest,
        display: case.display,
        label: case.label.into(),
        stage: case.stage,
        loadout: Some(FixtureLoadout::Path(case.loadout)),
        extra_items: case.items,
        seed_vars: &[],
        stand: case.stand,
        mode: case.mode,
        before_relog,
    })
    .unwrap();
    let mut observe_start = cell.observe_start.take().expect("Path cell seed proof");
    cell.observe_start = Some(Box::new(move |snapshot| {
        let mut receipt = observe_start(snapshot)?;
        receipt["bank_restock_seed"] = serde_json::json!(BANK_KIT);
        receipt["fixture_varp_variant"] = serde_json::json!(fixture_varp);
        Ok(receipt)
    }));
    cell
}

pub fn stage(next: &str) -> Mode {
    Mode::Stage {
        expect: vec![next.into()],
    }
}
