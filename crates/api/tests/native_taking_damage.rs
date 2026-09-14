//! Native local-player taking_damage scalar: positive type-1 hitmarks,
//! expiry without a player-generation advance, and fail-closed logout.

use api::snapshot::GameSnapshot;
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

fn plant_local(c: &mut Client) {
    c.ingame = true;
    c.self_slot = 0;
    c.local_player = Some(ClientPlayer::at(10, 10));
}

fn set_hit(c: &mut Client, slot: usize, value: i32, dtype: i32, cycle: i32) {
    let lp = c.local_player.as_mut().expect("local");
    lp.entity.damage_values[slot] = value;
    lp.entity.damage_types[slot] = dtype;
    lp.entity.damage_cycles[slot] = cycle;
}

fn active_until(c: &Client) -> i32 {
    c.loop_cycle + 70
}

#[test]
fn positive_active_type1_hit_is_taking_damage() {
    let mut client = client();
    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 5, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(snapshot.taking_damage());
}

#[test]
fn zero_poison_and_blocked_hits_are_not_taking_damage() {
    let mut client = client();
    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 0, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(!snapshot.taking_damage(), "zero value type-1");

    let until = active_until(&client);
    set_hit(&mut client, 0, 3, 2, until);
    assert!(!snapshot.rebuild(&client));
    // refresh_native_facts still runs even when no family gen moves.
    assert!(!snapshot.taking_damage(), "poison type 2");

    let until = active_until(&client);
    set_hit(&mut client, 0, 0, 0, until);
    assert!(!snapshot.rebuild(&client));
    assert!(!snapshot.taking_damage(), "blocked type 0");
}

#[test]
fn positive_hit_survives_later_miss_slot() {
    let mut client = client();
    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 8, 1, until);
    set_hit(&mut client, 1, 0, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(snapshot.taking_damage());
}

#[test]
fn hit_expires_without_player_generation_advance() {
    let mut client = client();
    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 4, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);

    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(snapshot.taking_damage());

    let player_gen = client.gens.player;
    client.loop_cycle = until; // boundary: cycle > loop_cycle fails
    assert!(!snapshot.rebuild_from_drain(&client, false));
    assert_eq!(client.gens.player, player_gen);
    assert!(
        !snapshot.taking_damage(),
        "expiry must clear without PLAYER_INFO"
    );
}

#[test]
fn logged_out_or_missing_player_fails_closed() {
    let mut client = client();
    client.ingame = true;
    // no local player
    client.bump_gens(ServerProt::PLAYER_INFO);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(!snapshot.taking_damage());

    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 9, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snapshot.rebuild(&client));
    assert!(snapshot.taking_damage());

    client.ingame = false;
    assert!(!snapshot.rebuild(&client));
    assert!(!snapshot.taking_damage());

    client.ingame = true;
    client.local_player = None;
    assert!(!snapshot.rebuild(&client));
    assert!(!snapshot.taking_damage());
}

#[test]
fn session_reset_clears_taking_damage() {
    let mut client = client();
    plant_local(&mut client);
    let until = active_until(&client);
    set_hit(&mut client, 0, 2, 1, until);
    client.bump_gens(ServerProt::PLAYER_INFO);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&client));
    assert!(snapshot.taking_damage());

    client.logout();
    snapshot.reset_session(client.gens);
    assert!(!snapshot.taking_damage());
}
