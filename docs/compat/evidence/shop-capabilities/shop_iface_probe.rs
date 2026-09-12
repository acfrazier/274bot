// Exact probe used for the shop interface decode evidence in this directory.
// It is NOT a committed test (no engine cache is assumed in CI): copy it to
// `crates/api/tests/shop_iface_probe.rs` in a clean checkout and run
//
//   ENGINE_DIR=<engine> cargo test -p api --test shop_iface_probe -- --nocapture
//
// once per selected cache. `client::cache_dir()` resolves
// `$ENGINE_DIR/data/pack/client`, so the two runs below produced the two logs
// in this directory:
//
//   ENGINE_DIR=/Users/acfrazier/experiments/Server/engine          # 274
//   ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine    # 289
//
// The committed, reproducible check for the same identities is
// `crates/api/tests/snapshot.rs::packed_shop_interfaces_post_main_stock_and_player_pack`.

use client::client::{Client, ClientConfig};

#[test]
fn probe_shop_iface() {
    let cache = client::bot_target::cache_dir();
    if !cache.join("interface").is_file() {
        panic!("no interface jag at {}", cache.display());
    }
    let c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: cache.display().to_string(),
        members: true,
        lowmem: false,
    });
    println!("cache={} ifaces_len={}", cache.display(), c.ifaces_len());
    for id in 3818usize..=3826 {
        match c.if_(id) {
            Some(com) => println!(
                "id={} type={:?} layer={} children={:?} iop={:?} text={:?}",
                id, com.r#type, com.layer_id, com.children, com.iop, com.text
            ),
            None => println!("id={} ABSENT", id),
        }
    }
    // Walk the player-container root.
    for root in [3822usize, 3823, 3900] {
        println!("-- subtree {root}");
        let mut queue = vec![root];
        let mut seen = 0usize;
        while let Some(id) = queue.pop() {
            let Some(com) = c.if_(id) else { continue };
            seen += 1;
            println!(
                "  walk id={} type={:?} layer={} iop={:?} text={:?}",
                id, com.r#type, com.layer_id, com.iop, com.text
            );
            if let Some(children) = com.children.clone() {
                for child in children {
                    if child >= 0 {
                        queue.push(child as usize);
                    }
                }
            }
        }
        println!("walked {seen} components from {root}");
    }
}
