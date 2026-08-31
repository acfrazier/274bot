//! Proof predicates: named assertions over a `GameSnapshot`. A step's
//! evidence arm and a scenario's terminal proof are the same type, so the
//! JSON evidence can name exactly which predicate passed or failed.

use api::obj_names::ObjNames;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::arrival::arrived;
use nav::tile::{chebyshev, Tile};

/// A named predicate over observable game state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof {
    /// `has_item("Bones")`: inventory holds `count` of the obj named
    /// `name`, resolved through the runner's shared obj table.
    Item { name: &'static str, count: i32 },
    /// `arrived(x, z, level)`: standing on the tile (or adjacent to it
    /// when the dest is solid).
    Arrived { x: i32, z: i32, level: i32 },
    /// `arrived_near(x, z, level, r)`: standing within chebyshev `r` of
    /// the tile on the level — for hops whose landing is randomised (the
    /// essence-mine exit portal teleports within a radius of the wizard's
    /// anchor).
    ArrivedNear {
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    },
    /// `in_essence_mine`: standing inside the Rune Essence mine enclosure
    /// (m45_75) — the entry teleport lands at a random
    /// `essence_mine_teleports` coord, never the pad exactly.
    EssenceMine,
    /// `chat_choice`: a chat modal with at least one choice button is
    /// open (a `p_choiceN` dialog waiting for an answer).
    ChatChoice,
    /// `quest_done("Rune Mysteries Quest")`: the quest tab paints the
    /// quest's row green — the same journal read the nav
    /// [`nav::WorldState`] gates quest edges on.
    QuestDone { name: &'static str },
    /// `stat(id) >= min`: a decoded stat-family value. The body decodes
    /// only run energy today (`id == 16`); other ids never hold.
    Stat { id: i32, min: i32 },
    /// A `MESSAGE_GAME`/`MESSAGE_PRIVATE` line containing `needle`.
    Chat { needle: &'static str },
    /// An NPC of `r#type` stands on the tile.
    NpcAt { r#type: usize, x: i32, z: i32 },
}

impl Proof {
    /// The predicate's evidence name (also the JSON `predicate` field).
    pub fn name(&self) -> String {
        match self {
            Proof::Item { name, count } => format!("has_item({name})>={count}"),
            Proof::Arrived { x, z, level } => format!("arrived({x},{z},{level})"),
            Proof::ArrivedNear {
                x,
                z,
                level,
                radius,
            } => format!("arrived_near({x},{z},{level},{radius})"),
            Proof::EssenceMine => "in_essence_mine".to_string(),
            Proof::ChatChoice => "chat_choice".to_string(),
            Proof::QuestDone { name } => format!("quest_done({name})"),
            Proof::Stat { id, min } => format!("stat({id})>={min}"),
            Proof::Chat { needle } => format!("chat(contains \"{needle}\")"),
            Proof::NpcAt { r#type, x, z } => format!("npc({type})@({x},{z})"),
        }
    }

    /// Whether the predicate holds on `snap`. `names` resolves `Item`
    /// predicates (name → obj id); `None` when the runner has no obj
    /// table, which makes every `Item` predicate fail.
    pub fn check(&self, snap: &GameSnapshot, names: Option<&ObjNames>) -> bool {
        match self {
            Proof::Item { name, count } => names
                .and_then(|n| n.by_name(name))
                .is_some_and(|id| snap.inv_count(id) >= *count),
            Proof::Arrived { x, z, level } => snap.tile().is_some_and(|(tx, tz, tl)| {
                arrived(
                    Tile {
                        x: tx,
                        z: tz,
                        level: tl,
                    },
                    Tile {
                        x: *x,
                        z: *z,
                        level: *level,
                    },
                    true,
                )
            }),
            Proof::ArrivedNear {
                x,
                z,
                level,
                radius,
            } => snap.tile().is_some_and(|(tx, tz, tl)| {
                tl == *level
                    && chebyshev(
                        Tile {
                            x: tx,
                            z: tz,
                            level: tl,
                        },
                        Tile {
                            x: *x,
                            z: *z,
                            level: *level,
                        },
                    ) <= *radius
            }),
            Proof::EssenceMine => snap.tile().is_some_and(|(tx, tz, tl)| {
                nav::essence::in_essence_mine(WorldTile {
                    x: tx,
                    z: tz,
                    level: tl,
                })
            }),
            Proof::ChatChoice => !snap.chat_options().is_empty(),
            Proof::QuestDone { name } => {
                nav::WorldState::from_snapshot(snap).quests.contains(*name)
            }
            Proof::Stat { id, min } => stat_value(snap, *id).is_some_and(|v| v >= *min),
            Proof::Chat { needle } => snap.chat().is_some_and(|c| c.contains(needle)),
            Proof::NpcAt { r#type, x, z } => snap
                .npcs()
                .iter()
                .any(|n| n.r#type == Some(*r#type) && n.x == *x && n.z == *z),
        }
    }
}

/// A decoded stat-family value, `None` for ids the body does not decode.
/// Today only run energy (stat 16) is decoded on the body; the rest of
/// the 0–24 stat table is a later decode.
fn stat_value(snap: &GameSnapshot, id: i32) -> Option<i32> {
    match id {
        16 => Some(snap.runenergy()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig, ClientNpc};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::config::ObjType;
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A synthetic mainland client: tile (3220, 3212, 0), one Bones in
    /// the inv iface, run energy 42, a chat line, ingame scene 2.
    fn seeded() -> Client {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 12));
        c.runenergy = 42;
        // The client's iface template already has the TYPE_INV widget;
        // fill it the way the server's `UPDATE_INV_FULL` does (stored
        // values are `obj_id + 1`: a real Bones id 526 stores as 527).
        match c.iface_id(|f| f.r#type == ComponentType::TYPE_INV) {
            Some(id) => {
                let inv = c.iface_mut(id).unwrap();
                inv.link_obj_type = Some(vec![527, 996]);
                inv.link_obj_number = Some(vec![1, 100]);
            }
            None => {
                let id = c.push_iface(IfType {
                    r#type: ComponentType::TYPE_INV,
                    ..Default::default()
                });
                c.set_iface_mut(
                    id,
                    IfTypeMut {
                        link_obj_type: Some(vec![527, 996]),
                        link_obj_number: Some(vec![1, 100]),
                        ..Default::default()
                    },
                );
            }
        }
        c.chat_text[0] = "Welcome to RuneScape".into();
        let mut npc = ClientNpc::default();
        npc.r#type = Some(708);
        npc.entity.x = 100;
        npc.entity.z = 200;
        c.npc[3] = Some(Box::new(npc));
        c.npc_ids[0] = 3;
        c.npc_count = 1;
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::UPDATE_INV_FULL,
            ServerProt::UPDATE_RUNENERGY,
            ServerProt::MESSAGE_GAME,
            ServerProt::REBUILD_NORMAL,
            ServerProt::NPC_INFO,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    fn snap(c: &mut Client) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.rebuild(c);
        s
    }

    fn names() -> ObjNames {
        ObjNames::from_objs(&[ObjType {
            id: 526,
            name: "Bones".into(),
            ..Default::default()
        }])
    }

    #[test]
    fn item_resolves_by_name_through_the_obj_table() {
        let s = snap(&mut seeded());
        assert!(Proof::Item {
            name: "Bones",
            count: 1
        }
        .check(&s, Some(&names())));
        assert!(!Proof::Item {
            name: "Bones",
            count: 2
        }
        .check(&s, Some(&names())));
        assert!(!Proof::Item {
            name: "Coins",
            count: 1
        }
        .check(&s, Some(&names())));
        // No obj table: every item predicate fails, never fakes a hit.
        assert!(!Proof::Item {
            name: "Bones",
            count: 1
        }
        .check(&s, None));
    }

    #[test]
    fn arrived_matches_the_player_world_tile() {
        let s = snap(&mut seeded());
        assert!(Proof::Arrived {
            x: 3220,
            z: 3212,
            level: 0
        }
        .check(&s, None));
        assert!(!Proof::Arrived {
            x: 3220,
            z: 3211,
            level: 0
        }
        .check(&s, None));
        assert_eq!(
            Proof::Arrived {
                x: 3220,
                z: 3212,
                level: 0
            }
            .name(),
            "arrived(3220,3212,0)"
        );
    }

    #[test]
    fn arrived_near_matches_within_the_radius_on_the_level() {
        let s = snap(&mut seeded()); // player at (3220, 3212, 0)
        assert!(Proof::ArrivedNear {
            x: 3220,
            z: 3214,
            level: 0,
            radius: 2
        }
        .check(&s, None));
        assert!(!Proof::ArrivedNear {
            x: 3220,
            z: 3215,
            level: 0,
            radius: 2
        }
        .check(&s, None), "cheb 3 is outside radius 2");
        assert!(!Proof::ArrivedNear {
            x: 3220,
            z: 3212,
            level: 1,
            radius: 2
        }
        .check(&s, None), "the radius is per-level");
        assert_eq!(
            Proof::ArrivedNear {
                x: 3220,
                z: 3212,
                level: 0,
                radius: 2
            }
            .name(),
            "arrived_near(3220,3212,0,2)"
        );
    }

    #[test]
    fn essence_mine_holds_only_inside_the_enclosure() {
        let mut c = seeded();
        let mut s = snap(&mut c); // player at (3220, 3212): outside m45_75
        assert!(!Proof::EssenceMine.check(&s, None));
        // Teleport the player into the mine (world (2912,4833) = scene
        // (-288, 1633) on the fixture base): the predicate holds.
        let mut lp = ClientPlayer::at(-288, 1633);
        lp.entity.x = -288 * 128 + 64;
        lp.entity.z = 1633 * 128 + 64;
        c.local_player = Some(lp);
        c.bump_gens(ServerProt::PLAYER_INFO);
        s.rebuild(&mut c);
        assert!(Proof::EssenceMine.check(&s, None));
        assert_eq!(Proof::EssenceMine.name(), "in_essence_mine");
    }

    #[test]
    fn chat_choice_holds_only_when_a_choice_modal_is_open() {
        let mut c = seeded();
        let mut s = snap(&mut c);
        assert!(!Proof::ChatChoice.check(&s, None), "no chat modal on the seed");
        // A `p_choice2_header`-shape modal: root 100 with two BUTTON_OK
        // children (the quest-seed dialogs).
        for (i, text) in ["Black Arm Gang.", "Phoenix Gang."].iter().enumerate() {
            let id = 101 + i as usize;
            c.set_iface(
                id,
                IfType {
                    id: id as i32,
                    layer_id: 100,
                    ..Default::default()
                },
            );
            c.set_iface_mut(
                id,
                IfTypeMut {
                    button_type: client::config::if_type::ButtonType::BUTTON_OK,
                    text: (*text).to_string(),
                    ..Default::default()
                },
            );
        }
        c.set_iface(
            100,
            IfType {
                id: 100,
                layer_id: 100,
                children: Some(vec![101, 102]),
                ..Default::default()
            },
        );
        c.chat_modal_id = 100;
        c.bump_gens(ServerProt::IF_OPENCHAT);
        s.rebuild(&mut c);
        assert!(Proof::ChatChoice.check(&s, None));
        assert_eq!(Proof::ChatChoice.name(), "chat_choice");
    }

    #[test]
    fn quest_done_matches_the_green_journal_row() {
        let mut c = seeded();
        let mut s = snap(&mut c);
        assert!(
            !Proof::QuestDone {
                name: "Rune Mysteries Quest"
            }
            .check(&s, None),
            "no quest tab on the seed"
        );
        // The quest tab (side 2) with one TYPE_TEXT row: "Rune Mysteries
        // Quest" still red (stored `0xF80000`), "Lost City" green (stored
        // `0xF800`, the client's decode of the journal's 15-bit green) —
        // only the green row proves the quest done, exactly like the nav
        // WorldState gate.
        c.side_icon[2] = 700;
        c.set_iface(
            700,
            IfType {
                id: 700,
                children: Some(vec![701, 702]),
                ..Default::default()
            },
        );
        c.set_iface(
            701,
            IfType {
                id: 701,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            701,
            IfTypeMut {
                text: "Rune Mysteries Quest".into(),
                colour: 0xF80000,
                ..Default::default()
            },
        );
        c.set_iface(
            702,
            IfType {
                id: 702,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            702,
            IfTypeMut {
                text: "Lost City".into(),
                colour: 0xF800,
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::IF_OPENMAIN);
        s.rebuild(&mut c);
        assert!(
            Proof::QuestDone {
                name: "Lost City"
            }
            .check(&s, None)
        );
        assert!(
            !Proof::QuestDone {
                name: "Rune Mysteries Quest"
            }
            .check(&s, None),
            "the red row is not done"
        );
        assert_eq!(
            Proof::QuestDone {
                name: "Rune Mysteries Quest"
            }
            .name(),
            "quest_done(Rune Mysteries Quest)"
        );
    }

    #[test]
    fn stat_16_is_run_energy_others_never_hold() {
        let s = snap(&mut seeded());
        assert!(Proof::Stat { id: 16, min: 42 }.check(&s, None));
        assert!(!Proof::Stat { id: 16, min: 43 }.check(&s, None));
        // stat(7) is not decoded on the body yet — honest false.
        assert!(!Proof::Stat { id: 7, min: 1 }.check(&s, None));
    }

    #[test]
    fn chat_matches_the_ring_head_line() {
        let s = snap(&mut seeded());
        assert!(Proof::Chat { needle: "Welcome" }.check(&s, None));
        assert!(!Proof::Chat { needle: "arrived" }.check(&s, None));
    }

    #[test]
    fn npc_at_matches_type_and_tile() {
        let s = snap(&mut seeded());
        assert!(Proof::NpcAt {
            r#type: 708,
            x: 100,
            z: 200
        }
        .check(&s, None));
        assert!(!Proof::NpcAt {
            r#type: 709,
            x: 100,
            z: 200
        }
        .check(&s, None));
    }
}
