//! The slot frame closure hands the front end's hook the real client type
//! (ruling D4) and publishes each frame's snapshot before the hook runs.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use client::client::{Client, ClientConfig};
use host_play::SlotFrameInput;

use super::{FrameContext, FrameHook, FrameShared};

#[derive(Default)]
struct CountingHook {
    frames: AtomicUsize,
    downcasts: AtomicUsize,
}

impl FrameHook for CountingHook {
    fn frame(&self, client: &mut dyn Any, slot: &str, frame: &FrameContext<'_>) {
        assert_eq!(slot, "alice");
        assert!(!frame.walk_cancelled());
        assert_eq!(frame.walk_route(), (None, None));
        self.frames.fetch_add(1, Ordering::Relaxed);
        if client.downcast_mut::<Client>().is_some() {
            self.downcasts.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The core's slot frame over the real client and host-play's operations.
fn frame_closure(
    shared: Arc<FrameShared>,
    hook: Arc<dyn FrameHook>,
) -> impl Fn(&mut Client, &str, SlotFrameInput) + Send + Sync + 'static {
    super::slot_frame(
        shared,
        hook,
        super::ClientOps::new(
            host_play::player_here_tile,
            host_play::Host::publish_frontend_snapshot,
            host_play::step_walk_arm_follow,
            |client| client.gens.player,
        ),
    )
}

/// Bind the closure to the exact bound host-play gives the per-frame
/// callback, so the test calls it as a slot thread would.
fn per_frame<F>(f: F) -> F
where
    F: Fn(&mut Client, &str, SlotFrameInput) + Send + Sync + 'static,
{
    f
}

#[test]
fn hook_receives_the_real_client_and_a_published_snapshot() {
    let shared = Arc::new(FrameShared::new());
    let hook = Arc::new(CountingHook::default());
    let frame = per_frame(frame_closure(Arc::clone(&shared), hook.clone()));
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    });
    frame(&mut client, "alice", SlotFrameInput::default());
    frame(&mut client, "alice", SlotFrameInput::default());
    assert_eq!(hook.frames.load(Ordering::Relaxed), 2);
    assert_eq!(hook.downcasts.load(Ordering::Relaxed), 2);
    assert!(shared.with_snapshot("alice", |_| ()).is_some());
    assert!(shared.with_snapshot("bob", |_| ()).is_none());
}

#[test]
fn a_lifetime_end_drops_the_published_snapshot() {
    let shared = Arc::new(FrameShared::new());
    let frame = per_frame(frame_closure(
        Arc::clone(&shared),
        Arc::new(CountingHook::default()),
    ));
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    });
    frame(&mut client, "alice", SlotFrameInput::default());
    assert!(!shared.end_lifetime("alice"));
    assert!(shared.with_snapshot("alice", |_| ()).is_none());
    frame(&mut client, "alice", SlotFrameInput::default());
    assert!(shared.with_snapshot("alice", |_| ()).is_some());
}

/// A synthetic offline client with a fake mainland scene base (no server,
/// no network).
fn paint_client() -> Client {
    let mut c = host::prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(client::config::Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

fn ingame_client() -> Client {
    let mut c = paint_client();
    c.ingame = true;
    c.scene_state = 2;
    c.local_player = Some(client::client::ClientPlayer::at(5, 6));
    c.bump_gens(client::io::ServerProt::PLAYER_INFO);
    c
}

fn arm_walk(shared: &FrameShared, name: &str) {
    shared.walk_arms().lock().unwrap().insert(
        name.into(),
        Arc::new(std::sync::Mutex::new(host_play::WalkArm::default())),
    );
}

fn response_15_reconnect(c: &mut Client) {
    use std::io::{Read, Write};
    use std::time::Duration;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    c.config.port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = [0; 2];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 14);
        stream.write_all(&[0; 17]).unwrap();
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 18);
        let mut login = vec![0; header[1] as usize];
        stream.read_exact(&mut login).unwrap();
        stream.write_all(&[15]).unwrap();
    });
    c.login("snapshot", "test", true).unwrap();
    server.join().unwrap();
}

/// Records each frame's session-boundary flag.
#[derive(Default)]
struct BoundaryHook {
    boundaries: std::sync::Mutex<Vec<bool>>,
}

impl FrameHook for BoundaryHook {
    fn frame(&self, _client: &mut dyn Any, _slot: &str, frame: &FrameContext<'_>) {
        self.boundaries
            .lock()
            .unwrap()
            .push(frame.session_boundary());
    }
}

impl BoundaryHook {
    fn last(&self) -> bool {
        *self.boundaries.lock().unwrap().last().expect("a frame ran")
    }
}

#[test]
fn a_logout_clears_published_facts_armed_work_and_the_step_latch() {
    let shared = Arc::new(FrameShared::new());
    let hook = Arc::new(BoundaryHook::default());
    let frame = per_frame(frame_closure(Arc::clone(&shared), hook.clone()));
    let mut c = ingame_client();
    frame(&mut c, "alice", SlotFrameInput::default());
    assert!(!hook.last());
    arm_walk(&shared, "alice");
    shared
        .step_latch
        .lock()
        .unwrap()
        .insert("alice".into(), (c.gens.player, (3205, 3206, 0)));

    c.logout();
    frame(&mut c, "alice", SlotFrameInput::default());
    assert!(hook.last());
    assert_eq!(
        shared.with_snapshot("alice", |snap| (
            snap.ingame(),
            snap.local_player().is_none()
        )),
        Some((false, true))
    );
    assert!(!shared.walk_arms().lock().unwrap().contains_key("alice"));
    assert!(!shared.step_latch.lock().unwrap().contains_key("alice"));
}

#[test]
fn a_response_15_replacement_waits_for_the_post_grant_player_packet() {
    let shared = Arc::new(FrameShared::new());
    let hook = Arc::new(BoundaryHook::default());
    let frame = per_frame(frame_closure(Arc::clone(&shared), hook.clone()));
    let mut c = ingame_client();
    frame(&mut c, "alice", SlotFrameInput::default());
    arm_walk(&shared, "alice");

    response_15_reconnect(&mut c);
    frame(&mut c, "alice", SlotFrameInput::default());
    assert!(hook.last());
    assert_eq!(
        shared.with_snapshot("alice", |snap| snap.local_player().is_none()),
        Some(true)
    );
    assert!(!shared.walk_arms().lock().unwrap().contains_key("alice"));

    let mut player = client::io::Packet::new(vec![0xe0, 0x50, 0xc0, 0]);
    c.psize = 4;
    c.handle_packet(client::io::ServerProt::PLAYER_INFO, &mut player);
    frame(&mut c, "alice", SlotFrameInput::default());
    assert!(!hook.last());
    assert_eq!(
        shared.with_snapshot("alice", |snap| snap.local_player().is_some()),
        Some(true)
    );
}

#[test]
fn a_same_name_lifetime_resets_but_a_scene_change_preserves_work() {
    let shared = Arc::new(FrameShared::new());
    let hook = Arc::new(BoundaryHook::default());
    let frame = per_frame(frame_closure(Arc::clone(&shared), hook.clone()));
    let mut c = ingame_client();
    frame(&mut c, "alice", SlotFrameInput::default());
    arm_walk(&shared, "alice");
    shared
        .step_latch
        .lock()
        .unwrap()
        .insert("alice".into(), (c.gens.player, (1, 1, 0)));

    c.scene_state = 1;
    c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    frame(&mut c, "alice", SlotFrameInput::default());
    assert!(!hook.last());
    assert!(!crate::walk_permissions::manual_walk_may_follow(
        true, false
    ));
    assert!(!crate::walk_permissions::manual_walk_may_follow(
        false, true
    ));
    assert!(shared.walk_arms().lock().unwrap().contains_key("alice"));
    assert!(shared.step_latch.lock().unwrap().contains_key("alice"));

    assert!(shared.end_lifetime("alice"));
    assert!(shared.with_snapshot("alice", |_| ()).is_none());
    assert!(!shared.walk_arms().lock().unwrap().contains_key("alice"));
    assert!(!shared.step_latch.lock().unwrap().contains_key("alice"));

    let mut fresh = paint_client();
    frame(&mut fresh, "alice", SlotFrameInput::default());
    assert!(!shared.walk_arms().lock().unwrap().contains_key("alice"));
}
