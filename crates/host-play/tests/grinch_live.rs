//! Real runner-owned scratch-engine Grinch evidence capture.
//!
//! These ignored tests require the parent orchestration contract described in
//! `GRINCH-1020/design.md` section 6. They never start or stop an engine. Each
//! run records its typed trace and oracle receipt below `LIVE_EVIDENCE_DIR`.

#[path = "support/grinch_driver.rs"]
mod grinch_driver;
#[path = "support/grinch_oracle.rs"]
mod grinch_oracle;

use grinch_driver::Case;

fn capture(case: Case) {
    grinch_driver::capture_live(case).unwrap_or_else(|error| {
        panic!(
            "Grinch live evidence capture failed for {}: {error}",
            case.slug()
        )
    });
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_tradeable_drops_live() {
    capture(Case::ATradeableDrops);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_green_mask_selection_live() {
    capture(Case::AGreenMaskSelection);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_negative_candidates_live() {
    capture(Case::ANegativeCandidates);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_private_untradeable_drops_live() {
    capture(Case::APrivateUntradeableDrops);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_pumpkin_respawns_live() {
    capture(Case::BPumpkinRespawns);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_contested_pumpkin_live() {
    capture(Case::BContestedPumpkin);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_scythe_duplicate_live() {
    capture(Case::BScytheDuplicate);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_bunny_ears_duplicate_live() {
    capture(Case::BBunnyEarsDuplicate);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_yoyo_repeat_live() {
    capture(Case::BYoYoRepeat);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_deposit_28_live() {
    capture(Case::CDeposit28);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_deposit_27_preserve_item_live() {
    capture(Case::CDeposit27PreserveItem);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_inventory_blocked_live() {
    capture(Case::CInventoryBlocked);
}

#[test]
#[ignore = "requires four captured A-variant scratch traces and verified process-tree stops"]
fn grinch_a_cell_oracle_live() {
    grinch_driver::replay_cell_a()
        .unwrap_or_else(|error| panic!("Grinch A replay failed: {error}"));
}

#[test]
#[ignore = "requires five captured scratch traces and verified full-process-tree stop evidence"]
fn grinch_b_cell_oracle_live() {
    grinch_driver::replay_cell_b()
        .unwrap_or_else(|error| panic!("Grinch B replay failed: {error}"));
}

#[test]
#[ignore = "requires three captured C-variant scratch traces and verified process-tree stops"]
fn grinch_c_cell_oracle_live() {
    grinch_driver::replay_cell_c()
        .unwrap_or_else(|error| panic!("Grinch C replay failed: {error}"));
}
