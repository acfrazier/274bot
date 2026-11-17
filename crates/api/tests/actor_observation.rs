use api::snapshot::{actor_observation, ActorChatStyle, ActorSelector, GameSnapshot};
use client::client::{Client, ClientConfig, ClientNpc};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;

fn client() -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "absent-actor-cache".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.self_slot = 3;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    let mut player = ClientPlayer::at(20, 21);
    player.name = Some("Alice".into());
    player.primary_anim = 123;
    player.primary_anim_frame = 4;
    player.secondary_anim = 5;
    player.chat_message = Some("Hello".into());
    player.chat_colour = 2;
    player.chat_effect = 3;
    player.chat_timer = 100;
    client.local_player = Some(player);
    client
}

#[test]
fn selected_native_actor_preserves_pose_and_refreshes_timer_without_packet_generation() {
    let mut client = client();
    client.bump_gens(ServerProt::PLAYER_INFO);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    let posted = &snapshot.local_player().unwrap().player.actor;
    let first = actor_observation(&client, Some(ActorSelector::LocalPlayer)).unwrap();
    assert_eq!(first.name, posted.name.as_deref());
    assert_eq!(first.tile, posted.tile);
    assert_eq!(first.animation, posted.animation);
    assert_eq!(first.animation_frame, posted.animation_frame);
    assert_eq!(first.pose_animation, posted.pose_animation);
    assert_eq!(
        first.chat_style,
        ActorChatStyle {
            colour: 2,
            effect: 3
        }
    );
    assert_eq!(first.remaining_cycles, 100);
    let generation = client.gens.player_info;
    client.loop_cycle += 1;
    client.local_player.as_mut().unwrap().chat_timer = 99;
    assert_eq!(
        actor_observation(&client, Some(ActorSelector::LocalPlayer))
            .unwrap()
            .remaining_cycles,
        99
    );
    client.local_player.as_mut().unwrap().chat_message = None;
    client.local_player.as_mut().unwrap().chat_timer = 0;
    let ended = actor_observation(&client, Some(ActorSelector::LocalPlayer)).unwrap();
    assert_eq!(ended.overhead_text, None);
    assert_eq!(ended.remaining_cycles, 0);
    assert_eq!(client.gens.player_info, generation);
    assert_eq!(
        posted.overhead_text.as_deref(),
        Some("Hello"),
        "cached actor semantics stay unchanged"
    );
}

#[test]
fn actor_selection_requires_a_current_published_slot_and_never_uses_chat_ring_text() {
    let mut client = client();
    assert!(actor_observation(&client, None).is_none());
    assert!(actor_observation(&client, Some(ActorSelector::Player(7))).is_none());
    let mut bob = ClientPlayer::at(21, 21);
    bob.name = Some("Bob".into());
    client.players[7] = Some(Box::new(bob));
    assert!(actor_observation(&client, Some(ActorSelector::Player(7))).is_none());
    client.player_ids[0] = 7;
    client.player_count = 1;
    assert_eq!(
        actor_observation(&client, Some(ActorSelector::Player(7)))
            .unwrap()
            .name,
        Some("Bob")
    );
    client.npc[8] = Some(Box::new(ClientNpc::default()));
    client.npc_ids[0] = 8;
    client.npc_count = 1;
    assert_eq!(
        actor_observation(&client, Some(ActorSelector::Npc(8)))
            .unwrap()
            .index,
        8
    );
    client.local_player.as_mut().unwrap().chat_message = None;
    client.chat_text[0] = "Hello".into();
    assert_eq!(
        actor_observation(&client, Some(ActorSelector::LocalPlayer))
            .unwrap()
            .overhead_text,
        None
    );
    client.ingame = false;
    for actor in [
        ActorSelector::LocalPlayer,
        ActorSelector::Player(7),
        ActorSelector::Npc(8),
    ] {
        assert!(actor_observation(&client, Some(actor)).is_none());
    }
}
