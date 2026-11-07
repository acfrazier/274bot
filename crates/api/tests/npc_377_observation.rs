//! Java 377 Client.java:10521-10546; NpcType.java:359-372.
use api::snapshot::{Family, GameSnapshot};
use client::client::{Client, ClientConfig, ClientNpc};
use client::config::runtime_377::{
    CacheFields377, InterfaceFields377, MorphTable377, NpcFields377,
};
use client::config::{Cache, CacheRevision377, NpcType, VarBitType, VarpType};
use client::io::ClientRevision;
use std::sync::Arc;

fn fields(morph: Option<MorphTable377>) -> NpcFields377 {
    NpcFields377 {
        field_1451: -1,
        field_1430: -1,
        field_1440: -1,
        field_1434: true,
        morph,
    }
}
fn client(revision: ClientRevision, varbit: bool) -> Client {
    let cache = Cache {
        npcs: vec![
            NpcType {
                id: 0,
                name: "Placed identity".into(),
                size: 2,
                vislevel: 12,
                op: vec![Some("Root action".into())],
                ..Default::default()
            },
            NpcType {
                id: 1,
                name: "Juliet".into(),
                vislevel: 7,
                op: vec![Some("Talk-to".into())],
                ..Default::default()
            },
            NpcType {
                id: 2,
                name: "Different form".into(),
                vislevel: 19,
                op: vec![Some("Trade".into())],
                ..Default::default()
            },
        ],
        varps: vec![VarpType::default(), VarpType::default()],
        varbits: vec![VarBitType {
            basevar: 0,
            startbit: 1,
            endbit: 2,
            ..Default::default()
        }],
        revision_377: Some(Box::new(CacheRevision377 {
            config: CacheFields377 {
                npcs: vec![
                    fields(Some(MorphTable377 {
                        varbit: if varbit { 0 } else { -1 },
                        varp: 1,
                        targets: vec![1, -1, 2],
                    })),
                    fields(None),
                    fields(None),
                ],
                ..Default::default()
            },
            interfaces: InterfaceFields377 {
                client_code_layers: Default::default(),
                type1_fields: Vec::new(),
                null_string_masks: Vec::new(),
                initial_overlays: Vec::new(),
            },
        })),
        ..Default::default()
    };
    let mut client = Client::from_shared_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        },
        Arc::new(cache),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        revision,
    );
    client.ingame = true;
    client.scene_state = 2;
    client.var = vec![0, 0];
    client.gens.npc = 1;
    client.gens.varp = 1;
    client.npc[7] = Some(Box::new(ClientNpc {
        r#type: Some(0),
        ..Default::default()
    }));
    client.npc_ids[0] = 7;
    client.npc_count = 1;
    client
}

#[test]
fn revision_377_npc_varbit_and_varp_resolve_current_menu_without_changing_placed_type() {
    for varbit in [false, true] {
        let mut client = client(ClientRevision::R377, varbit);
        let mut snapshot = GameSnapshot::new();
        assert!(snapshot.rebuild_family(&client, Family::Npc));
        let npc = &snapshot.npcs()[0];
        assert_eq!(npc.r#type, Some(0));
        assert_eq!(npc.name.as_deref(), Some("Juliet"));
        assert_eq!(npc.actions, [Some("Talk-to".into())]);
        assert_eq!(npc.level, 7, "NPC level comes from the selected form");
        assert_eq!(npc.size, 2, "menu form must not replace placed actor size");
        assert!(!snapshot.rebuild_family(&client, Family::Npc));
        for (selector, expected) in [
            (2, Some("Different form")),
            (1, None),
            (3, None),
            (-1, None),
            (0, Some("Juliet")),
        ] {
            client.var[0] = selector << 1;
            client.var[1] = if varbit { 99 } else { selector };
            client.gens.varp += 1;
            assert!(
                snapshot.rebuild_family(&client, Family::Npc),
                "varp-only changes must refresh the menu form"
            );
            let npc = &snapshot.npcs()[0];
            assert_eq!(npc.r#type, Some(0));
            assert_eq!(
                npc.level,
                match selector {
                    2 => 19,
                    1 | 3 | -1 => 0,
                    _ => 7,
                },
                "NPC level follows its selected form; a null form has no level"
            );
            assert_eq!(npc.name.as_deref(), expected);
            assert_eq!(
                npc.actions.first().and_then(|op| op.as_deref()),
                expected.map(|name| if name == "Juliet" { "Talk-to" } else { "Trade" })
            );
            if expected.is_none() {
                assert!(npc.actions.is_empty());
            }
        }
        Arc::get_mut(&mut client.cache)
            .unwrap()
            .revision_377
            .as_mut()
            .unwrap()
            .config
            .npcs[1]
            .field_1434 = false;
        client.gens.npc += 1;
        assert!(snapshot.rebuild_family(&client, Family::Npc));
        assert!(snapshot.npcs()[0].name.is_none());
        assert_eq!(snapshot.npcs()[0].level, 0);
        assert!(
            snapshot.npcs()[0].actions.is_empty(),
            "Java's noninteractive current form offers no menu"
        );
    }
}

#[test]
fn revision_274_289_npc_snapshot_bytes_ignore_377_morph_tables_and_varp_changes() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let mut with_morph = client(revision, true);
        let mut legacy = client(revision, false);
        Arc::get_mut(&mut legacy.cache).unwrap().revision_377 = None;
        let mut actual = GameSnapshot::new();
        let mut control = GameSnapshot::new();
        assert!(actual.rebuild_family(&with_morph, Family::Npc));
        assert!(control.rebuild_family(&legacy, Family::Npc));
        let expected = serde_json::to_vec(control.npcs()).unwrap();
        assert_eq!(serde_json::to_vec(actual.npcs()).unwrap(), expected);
        assert_eq!(actual.npcs()[0].name.as_deref(), Some("Placed identity"));
        assert_eq!(actual.npcs()[0].level, 12);
        with_morph.var[0] = 2;
        with_morph.gens.varp += 1;
        assert!(!actual.rebuild_family(&with_morph, Family::Npc));
        assert_eq!(serde_json::to_vec(actual.npcs()).unwrap(), expected);
    }
}
