use super::*;
use crate::map::formats::MAX_POIS;
use crate::map::poi::CapabilityEvidence;
use client::config::{LocType, NpcType};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "navpois-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("pack")).unwrap();
        std::fs::create_dir_all(path.join("maps")).unwrap();
        std::fs::create_dir_all(path.join("scripts")).unwrap();
        Self(path)
    }
    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn npc(id: i32, name: &str, ops: &[&str]) -> NpcType {
    let mut row = NpcType {
        id,
        name: name.into(),
        ..NpcType::default()
    };
    row.op = ops.iter().map(|op| Some((*op).into())).collect();
    row
}

fn loc(id: i32, name: &str, ops: &[&str]) -> LocType {
    let mut row = LocType {
        id,
        name: name.into(),
        width: 1,
        length: 1,
        active: true,
        ..LocType::default()
    };
    row.op = ops.iter().map(|op| Some((*op).into())).collect();
    row
}

fn produce(root: &Path, npcs: &[NpcType], locs: &[LocType]) -> ProducedPois {
    produce_navpois(&ProduceRequest {
        revision: 289,
        content_root: root,
        npcs,
        locs,
        content_id: Digest([1; 32]),
        nav_sha256: Digest([2; 32]),
        source_sha256: Digest([3; 32]),
        generator_sha256: Digest([4; 32]),
    })
    .unwrap()
}

fn decode(bytes: &[u8]) -> ServicePois {
    ServicePois::decode_navpois(
        bytes,
        ServiceIdentity {
            revision: 289,
            content: Digest([1; 32]),
            nav_sha256: Digest([2; 32]),
            source_sha256: Digest([3; 32]),
            generator_sha256: Digest([4; 32]),
            policy: pois_policy_digest(),
        },
        Digest::of(bytes),
    )
    .unwrap()
}

fn bank_tree() -> Fixture {
    let fix = Fixture::new();
    fix.write(
        "pack/npc.pack",
        "1036=werewolfbanker\n902=magearena_banker\n",
    );
    fix.write(
        "pack/loc.pack",
        "3193=loc_3193\n3194=duel_chestopen\n2693=thbankchest\n4483=castlewars_bankchest\n",
    );
    fix.write(
        "scripts/interface_bank/configs/banker.npc",
        "[werewolfbanker]\nname=Banker\nop1=Talk-to\nop3=Bank\ncategory=bank_teller\n",
    );
    fix.write(
        "scripts/areas/area_mage_arena/configs/mage_arena.npc",
        "[magearena_banker]\nname=Gundai\nop1=Talk-to\n",
    );
    fix.write(
        "scripts/interface_bank/scripts/banker.rs2",
        "[opnpc3,_bank_teller] @openbank;\n[label,openbank]\n%bankcert = 0;\n",
    );
    fix.write(
        "scripts/areas/area_mage_arena/scripts/gundai.rs2",
        "[opnpc1,magearena_banker]\n~chatplayer(\"hi\");\ndef_int $choice = ~p_choice2(\"bank\", 1, \"no\", 2);\nif ($choice = 2) {\n    return;\n}\n@openbank;\n",
    );
    fix.write(
        "scripts/minigames/game_duelarena/scripts/misc_locs.rs2",
        "[oploc1,loc_3193]\n~open_chest(duel_chestopen);\n[oploc2,duel_chestopen]\n@openbank;\n",
    );
    fix.write(
        "scripts/general_use/scripts/chests.rs2",
        "[proc,open_chest](loc $other_chest)\nloc_change($other_chest, 300);\n",
    );
    fix.write(
        "scripts/areas/area_alkharid/scripts/shantay_chest.rs2",
        "[oploc1,thbankchest]\nif (map_members = ^false) {\n    return;\n}\n@openbank;\n",
    );
    fix.write(
        "scripts/minigames/game_castlewars/scripts/castlewars_bank.rs2",
        "[oploc1,castlewars_bankchest] @openbank;\n",
    );
    fix.write(
        "maps/m54_54.jm2",
        "==== MAP ====\n1 58 23 f2\n==== LOC ====\n==== NPC ====\n0 58 23: 1036\n0 58 25: 1036\n",
    );
    fix.write(
        "maps/m39_73.jm2",
        "==== MAP ====\n==== LOC ====\n==== NPC ====\n0 38 42: 902\n",
    );
    fix.write(
        "maps/m52_51.jm2",
        "==== MAP ====\n==== LOC ====\n0 53 5: 3193 10 2\n==== NPC ====\n",
    );
    fix.write(
        "maps/m51_48.jm2",
        "==== MAP ====\n==== LOC ====\n0 45 16: 2693 10 3\n==== NPC ====\n",
    );
    fix.write(
        "maps/m38_48.jm2",
        "==== MAP ====\n==== LOC ====\n0 12 19: 4483 10 3\n==== NPC ====\n",
    );
    fix.write(
        "maps/labels.txt",
        "=Lumbridge,3239,3233,1\n=Kingdom Of/Misthalin,3217,3321,2\n",
    );
    fix
}

#[test]
fn canifis_bankers_keep_server_game_plane_zero() {
    let fix = bank_tree();
    let produced = produce(
        &fix.0,
        &[
            npc(1036, "Banker", &["Talk-to", "", "Bank"]),
            npc(902, "Gundai", &["Talk-to"]),
        ],
        &[
            loc(3193, "Closed chest", &["Open"]),
            loc(3194, "Open chest", &["", "Bank", "Shut"]),
            loc(2693, "Shantay chest", &["Open"]),
            loc(4483, "Bank chest", &["Use"]),
        ],
    );
    let doc = decode(&produced.bytes);
    let canifis: Vec<_> = doc
        .records
        .as_slice()
        .iter()
        .filter(|row| row.key.entity == EntityKind::Npc && row.key.id == 1036)
        .collect();
    assert_eq!(canifis.len(), 2);
    assert!(canifis.iter().all(|row| {
        matches!(row.key.source, SourceSpace::ServerGame { plane: 0 })
            && row.effective_plane == 0
            && row.kind == PoiKind::Bank
    }));
    assert_eq!(canifis[0].key.x, 3514);
    assert_eq!(canifis[0].key.z, 3479);
    assert_eq!(canifis[1].key.z, 3481);
}

#[test]
fn gundai_shantay_duel_and_castle_wars_are_source_service() {
    let fix = bank_tree();
    let doc = decode(
        &produce(
            &fix.0,
            &[
                npc(1036, "Banker", &["Talk-to", "", "Bank"]),
                npc(902, "Gundai", &["Talk-to"]),
            ],
            &[
                loc(3193, "Closed chest", &["Open"]),
                loc(3194, "Open chest", &["", "Bank", "Shut"]),
                loc(2693, "Shantay chest", &["Open"]),
                loc(4483, "Bank chest", &["Use"]),
            ],
        )
        .bytes,
    );
    let gundai = doc
        .records
        .as_slice()
        .iter()
        .find(|row| row.key.entity == EntityKind::Npc && row.key.id == 902)
        .unwrap();
    assert_eq!(gundai.key.x, 2534);
    assert_eq!(gundai.key.z, 4714);
    assert!(gundai.evidence.as_slice().iter().any(|ev| matches!(
        ev,
        CapabilityEvidence::SourceService {
            eligibility: Eligibility::Conditional,
            trigger: ServiceTrigger::Dialogue,
            ..
        }
    )));
    let shantay = doc
        .records
        .as_slice()
        .iter()
        .find(|row| row.key.entity == EntityKind::Loc && row.key.id == 2693)
        .unwrap();
    assert!(shantay.evidence.as_slice().iter().any(|ev| matches!(
        ev,
        CapabilityEvidence::SourceService {
            eligibility: Eligibility::Conditional,
            ..
        }
    )));
    let duel = doc
        .records
        .as_slice()
        .iter()
        .find(|row| row.key.entity == EntityKind::Loc && row.key.id == 3193)
        .unwrap();
    assert!(duel.evidence.as_slice().iter().any(|ev| matches!(
        ev,
        CapabilityEvidence::SourceService {
            trigger: ServiceTrigger::LocVariant { definition: 3194 },
            eligibility: Eligibility::Conditional,
            ..
        }
    )));
    assert!(doc
        .records
        .as_slice()
        .iter()
        .any(|row| row.key.entity == EntityKind::Loc && row.key.id == 4483));
    assert!(!doc
        .records
        .as_slice()
        .iter()
        .any(|row| row.key.entity == EntityKind::Loc && row.key.id == 3194));
}

#[test]
fn noop_open_chest_is_not_a_bank_variant() {
    let fix = Fixture::new();
    fix.write("pack/npc.pack", "1=unused\n");
    fix.write("pack/loc.pack", "3193=loc_3193\n3194=duel_chestopen\n");
    fix.write(
        "scripts/minigames/game_duelarena/scripts/misc_locs.rs2",
        "[oploc1,loc_3193]\n~open_chest(duel_chestopen);\n[oploc2,duel_chestopen]\n@openbank;\n",
    );
    fix.write(
        "scripts/general_use/scripts/chests.rs2",
        "[proc,open_chest](loc $other_chest)\nreturn;\n",
    );
    fix.write(
        "maps/m52_51.jm2",
        "==== MAP ====\n==== LOC ====\n0 53 5: 3193 10 2\n==== NPC ====\n",
    );
    let doc = decode(
        &produce(
            &fix.0,
            &[],
            &[
                loc(3193, "Closed chest", &["Open"]),
                loc(3194, "Open chest", &["", "Bank", "Shut"]),
            ],
        )
        .bytes,
    );
    assert!(
        !doc.records
            .as_slice()
            .iter()
            .any(|row| row.key.entity == EntityKind::Loc && row.key.id == 3193),
        "a no-op open_chest must not invent a closed-chest bank variant"
    );
}

#[test]
fn labels_are_search_anchors_not_walk_targets() {
    let fix = bank_tree();
    let doc = decode(
        &produce(
            &fix.0,
            &[npc(1036, "Banker", &["Talk-to", "", "Bank"])],
            &[],
        )
        .bytes,
    );
    let labels: Vec<_> = doc
        .records
        .as_slice()
        .iter()
        .filter(|row| row.key.entity == EntityKind::Label)
        .collect();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].name.as_str(), "Lumbridge");
    assert_eq!(labels[1].name.as_str(), "Kingdom Of/Misthalin");
    assert!(labels.iter().all(|row| {
        row.walk_target.is_none()
            && matches!(row.key.source, SourceSpace::ServerGame { plane: 0 })
            && matches!(row.kind, PoiKind::Label { .. })
    }));
}

#[test]
fn missing_labels_are_coverage_not_frozen_fallback() {
    let fix = bank_tree();
    std::fs::remove_file(fix.0.join("maps/labels.txt")).unwrap();
    let produced = produce(
        &fix.0,
        &[npc(1036, "Banker", &["Talk-to", "", "Bank"])],
        &[],
    );
    let doc = decode(&produced.bytes);
    assert_eq!(doc.coverage.place_labels, CoverageLevel::Unavailable);
    assert!(doc
        .coverage
        .unresolved
        .as_slice()
        .iter()
        .any(|issue| issue.reason == CoverageReason::MissingLabels));
    assert!(!doc
        .records
        .as_slice()
        .iter()
        .any(|row| row.key.entity == EntityKind::Label));
}

#[test]
fn extra_npc_tokens_are_not_placements() {
    let fix = Fixture::new();
    fix.write("pack/npc.pack", "1=banker1\n");
    fix.write(
        "scripts/interface_bank/scripts/banker.rs2",
        "[opnpc3,banker1] @openbank;\n",
    );
    fix.write(
        "maps/m50_50.jm2",
        "==== MAP ====\n==== NPC ====\n0 1 1: 1 10 1\n0 2 2: 1\n",
    );
    let doc = decode(&produce(&fix.0, &[npc(1, "Banker", &["Talk-to", "", "Bank"])], &[]).bytes);
    let npcs: Vec<_> = doc
        .records
        .as_slice()
        .iter()
        .filter(|row| row.key.entity == EntityKind::Npc)
        .collect();
    assert_eq!(npcs.len(), 1);
    assert_eq!(npcs[0].key.x, 3202);
    assert_eq!(npcs[0].key.z, 3202);
}

#[test]
fn identical_inputs_emit_identical_bytes() {
    let fix = bank_tree();
    let npcs = [npc(1036, "Banker", &["Talk-to", "", "Bank"])];
    let first = produce(&fix.0, &npcs, &[]).bytes;
    let second = produce(&fix.0, &npcs, &[]).bytes;
    assert_eq!(first, second);
    assert!(first.len() > 77);
}

#[test]
fn cheat_scripts_under_test_are_ignored() {
    let fix = Fixture::new();
    fix.write("pack/npc.pack", "1=banker1\n");
    fix.write(
        "scripts/_test/scripts/cheats/cheat_bank.rs2",
        "[opnpc1,banker1] @openbank;\n",
    );
    fix.write(
        "maps/m50_50.jm2",
        "==== MAP ====\n==== NPC ====\n0 1 1: 1\n",
    );
    let doc = decode(&produce(&fix.0, &[npc(1, "Banker", &["Talk-to"])], &[]).bytes);
    assert!(!doc
        .records
        .as_slice()
        .iter()
        .any(|row| row.key.entity == EntityKind::Npc));
}
#[test]
fn record_count_stays_inside_the_shared_limit() {
    const { assert!(MAX_POIS >= 4096) };
}

#[test]
fn generator_identity_tracks_source_bytes() {
    let a = pois_generator_identity(&[("src/map/services.rs", "fn a() {}")]);
    let b = pois_generator_identity(&[("src/map/services.rs", "fn b() {}")]);
    assert_ne!(a, b);
    assert_eq!(a.len(), 64);
}
