use api::snapshot::{GameSnapshot, HintTileView};
use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;

fn client() -> Client {
    Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    })
}

#[test]
fn native_local_overhead_and_coordinate_hint_are_distinct_current_facts() {
    let mut client = client();
    client.ingame = true;
    client.self_slot = 7;
    let mut local = ClientPlayer::at(20, 12);
    local.entity.chat_message = Some("FIGHT!".into());
    local.entity.chat_timer = 2;
    client.local_player = Some(local);

    let mut other = ClientPlayer::at(21, 12);
    other.entity.chat_message = Some("OTHER".into());
    other.entity.chat_timer = 2;
    client.players[8] = Some(Box::new(other));
    client.player_ids[0] = 8;
    client.player_count = 1;

    client.chat_text[0] = "latest ring line".into();
    client.hint_type = 2;
    client.hint_tile_x = 2761;
    client.hint_tile_z = 9546;
    client.bump_gens(ServerProt::PLAYER_INFO);
    client.bump_gens(ServerProt::MESSAGE_GAME);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert_eq!(snapshot.local_overhead_text(), Some("FIGHT!"));
    assert_eq!(snapshot.chat(), Some("latest ring line"));
    assert_eq!(
        snapshot.hint_tile(),
        Some(&HintTileView { x: 2761, z: 9546 })
    );

    client.timeout_chat();
    assert!(!snapshot.rebuild(&client));
    assert_eq!(snapshot.local_overhead_text(), Some("FIGHT!"));

    client.timeout_chat();
    assert!(!snapshot.rebuild(&client));
    assert_eq!(
        snapshot.local_overhead_text(),
        None,
        "native expiry clears the local overhead rather than falling back to chat history"
    );

    client.hint_type = 1;
    assert!(!snapshot.rebuild(&client));
    assert_eq!(
        snapshot.hint_tile(),
        None,
        "non-coordinate hints are absent"
    );
}

#[test]
fn session_reset_clears_native_local_overhead_and_hint() {
    let mut client = client();
    client.ingame = true;
    let mut local = ClientPlayer::at(20, 12);
    local.entity.chat_message = Some("3".into());
    local.entity.chat_timer = 150;
    client.local_player = Some(local);
    client.hint_type = 2;
    client.hint_tile_x = 2800;
    client.hint_tile_z = 9500;
    client.bump_gens(ServerProt::PLAYER_INFO);
    client.bump_gens(ServerProt::REBUILD_NORMAL);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert_eq!(snapshot.local_overhead_text(), Some("3"));
    assert!(snapshot.hint_tile().is_some());

    client.logout();
    snapshot.reset_session(client.gens);
    assert_eq!(snapshot.local_overhead_text(), None);
    assert_eq!(snapshot.hint_tile(), None);
}
