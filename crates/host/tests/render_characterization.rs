//! T4.1 characterization of today's render backend (contract §23 §2.1/§2.7).
//!
//! These tests drive the real `client::render::Renderer` (`mainredraw`, the
//! production frame entry) with the CPU (`CpuBackend`) and, where an adapter
//! exists, the GPU (`GpuBackend`) backend. They pin what the backend does now
//! so T4.2 (wrap) can prove it preserved it and T4.4 (fair) can see exactly
//! what it changes. The renderer-level tests construct heads directly; the
//! slot-level tests drive the production slot loop, whose heads attach
//! through the process render service since T4.2. The seam's names are the
//! vocabulary:
//!
//! - **head** (`HeadLease`): one `Renderer` attached to one slot `Client`.
//!   Today a head is created by the first paint and dropped on detach.
//! - **retained scene** (`HeadSnapshot::scene`): the last 3D image a head can
//!   present without a new scene build. Today that is the GPU backend's
//!   `scene_texture` or the CPU backend's `area_game`/`draw_area` pixels.
//! - **frame** (`FrameLease` → published output): one `mainredraw` call and
//!   the `FrameOutput` it returns.
//!
//! Set `T4_REQUIRE_GPU=1` to make a missing adapter a failure instead of a
//! skipped GPU variant, and `T4_REQUIRE_ASSETS=1` (with `ENGINE_DIR`) to make
//! a missing client cache a failure for the loading-splash assertions.

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use api::identity::{SlotKey, SlotKeyMinter};
use client::client::{Client, ClientConfig, ClientRevision};
use client::dash3d::{ClientNpc, TerrainOverlayShape};
use client::io::ClientStream;
use client::render::backend::{BackendKind, FrameOutput, GpuBackend};
use client::render::Renderer;
use host::render_head;
use host::{FrameBuf, Host, RandomClaim, ScriptRunPolicy, SlotInput, SlotRender};
use vault::ProfileSettings;

const FRAME_W: usize = 765;
const FRAME_H: usize = 503;
/// The 3D scene window inside the 765×503 frame (`area_game` at (4, 4)).
const SCENE_X: std::ops::Range<usize> = 4..516;
const SCENE_Y: std::ops::Range<usize> = 4..338;
/// Bounding box of "Loading - please wait." (`scene_loading_splash` centres
/// it at (256, 150) in `area_game`, which blits at (4, 4)).
const SPLASH_X: std::ops::Range<usize> = 180..340;
const SPLASH_Y: std::ops::Range<usize> = 135..165;

/// Process-wide renderer state (`Renderer::constructed`, the GPU profiling
/// ledger, the shared GPU context) makes these tests order-sensitive, so the
/// binary runs them one at a time.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn required(var: &str) -> bool {
    std::env::var(var).as_deref() == Ok("1")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Head {
    Cpu,
    Gpu,
}

/// Whether this machine has a GPU adapter (probed once per process).
static GPU_AVAILABLE: LazyLock<bool> = LazyLock::new(|| GpuBackend::try_new().is_ok());

/// CPU always; GPU when this machine has an adapter and the process is not
/// latched to the CPU path (`BOT_CPU=1`, which the slot honours).
fn heads() -> Vec<Head> {
    if required("BOT_CPU") {
        eprintln!("BOT_CPU=1: CPU heads only");
        return vec![Head::Cpu];
    }
    if *GPU_AVAILABLE {
        vec![Head::Cpu, Head::Gpu]
    } else {
        assert!(
            !required("T4_REQUIRE_GPU"),
            "T4_REQUIRE_GPU=1 but no GPU adapter is available"
        );
        eprintln!("no GPU adapter: the GPU variant is skipped");
        vec![Head::Cpu]
    }
}

/// Attach a head the way the slot does (`Renderer::new_prefer`), checking the
/// backend actually selected.
fn attach(head: Head) -> Renderer {
    let renderer = Renderer::new_prefer(false, head == Head::Gpu);
    let want = match head {
        Head::Cpu => BackendKind::Cpu,
        Head::Gpu => BackendKind::Gpu,
    };
    assert_eq!(renderer.backend_kind(), want, "{head:?} head");
    renderer
}

/// The real client cache when present (fonts for the loading splash).
fn assets() -> Option<String> {
    let dir = client::cache_dir();
    if dir.join("media").is_file() {
        return Some(dir.display().to_string());
    }
    assert!(
        !required("T4_REQUIRE_ASSETS"),
        "T4_REQUIRE_ASSETS=1 but {} has no media jag (set ENGINE_DIR)",
        dir.display()
    );
    eprintln!("no client cache: the loading-splash assertions are skipped");
    None
}

/// An in-game client with a built 3D scene: a 16×32-tile coloured plain in
/// front of a fixed camera. `cache_dir` selects real fonts or none.
fn scene_client(cache_dir: Option<&str>) -> Client {
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: cache_dir.unwrap_or("/nonexistent-t4-cache").into(),
            members: true,
            lowmem: false,
        },
        ClientRevision::R289,
    );
    client.set_draw(true);
    client.ingame = true;
    client.scene_state = 2;
    client.world.fill_base_level(0);
    for x in 8..24 {
        for z in 8..40 {
            let hsl = ((x * 7 + z * 13) % 60) * 1024 + 2624;
            client.world.set_ground(
                0,
                x,
                z,
                TerrainOverlayShape::PLAIN,
                0,
                -1,
                0,
                0,
                0,
                0,
                hsl,
                hsl,
                hsl,
                hsl,
                0,
                0,
                0,
                0,
                0x00ff00,
                0,
            );
        }
    }
    look_from(&mut client, 16 * 128, 4 * 128, 0);
    client
}

fn look_from(client: &mut Client, x: i32, z: i32, yaw: i32) {
    client.cam_x = x;
    client.cam_z = z;
    client.cam_y = -600;
    client.cam_pitch = 200;
    client.cam_yaw = yaw;
}

/// One production frame (`mainredraw`), read back as 0xRRGGBB pixels.
fn frame(renderer: &mut Renderer, client: &mut Client) -> Vec<i32> {
    pixels(&renderer.mainredraw(client))
}

fn pixels(output: &FrameOutput) -> Vec<i32> {
    let px = match output {
        FrameOutput::PixMap(map) => map.pixels.clone(),
        FrameOutput::Texture(handle) => handle.read_back(),
    };
    assert_eq!(px.len(), FRAME_W * FRAME_H);
    px.into_iter().map(|p| p & 0xff_ffff).collect()
}

fn in_scene(i: usize) -> bool {
    SCENE_X.contains(&(i % FRAME_W)) && SCENE_Y.contains(&(i / FRAME_W))
}

fn in_splash(i: usize) -> bool {
    SPLASH_X.contains(&(i % FRAME_W)) && SPLASH_Y.contains(&(i / FRAME_W))
}

/// Scene-window pixels that differ between two frames.
fn scene_diff(a: &[i32], b: &[i32]) -> usize {
    (0..a.len())
        .filter(|&i| in_scene(i) && a[i] != b[i])
        .count()
}

/// Differing scene-window pixels outside / inside the splash box.
fn split_diff(a: &[i32], b: &[i32]) -> (usize, usize) {
    let (mut outside, mut inside) = (0, 0);
    for i in (0..a.len()).filter(|&i| in_scene(i) && a[i] != b[i]) {
        if in_splash(i) {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (outside, inside)
}

fn lit_scene(px: &[i32]) -> usize {
    (0..px.len()).filter(|&i| in_scene(i) && px[i] != 0).count()
}

const SCENE_PIXELS: usize = (516 - 4) * (338 - 4);

/// Family **scene**: a live head (`scene_state == 2`) builds a new 3D scene
/// on every paint, so its output follows the camera. Guards the freeze tests
/// against a backend that never redraws.
#[test]
fn scene_live_paint_rebuilds_the_scene_every_frame() {
    let _serial = serial();
    for head in heads() {
        let mut renderer = attach(head);
        let mut client = scene_client(None);
        let first = frame(&mut renderer, &mut client);
        assert!(
            lit_scene(&first) > SCENE_PIXELS / 4,
            "{head:?}: the fixture scene must cover the window ({} lit)",
            lit_scene(&first)
        );
        let again = frame(&mut renderer, &mut client);
        assert_eq!(
            scene_diff(&first, &again),
            0,
            "{head:?}: an unchanged camera must reproduce the same scene"
        );
        look_from(&mut client, 15 * 128, 3 * 128, 96);
        let moved = frame(&mut renderer, &mut client);
        assert!(
            scene_diff(&first, &moved) > SCENE_PIXELS / 4,
            "{head:?}: a live paint must rebuild the scene for the new camera"
        );
    }
}

/// Family **scene**: while the slot is loading (`scene_state == 1`), each
/// head keeps presenting the last 3D scene it built (the Java `area_game`
/// freeze) and does no new scene build, whatever the camera and world do.
/// The loading splash is the only change: the CPU head draws "Loading -
/// please wait." over the frozen scene; today's GPU head composites the
/// retained texture with overlay coverage only, so the splash text is not
/// visible on GPU (a CPU/GPU divergence T4.2/T4.4 must account for).
#[test]
fn scene_freeze_retains_the_last_scene_while_loading() {
    let _serial = serial();
    let cache = assets();
    for head in heads() {
        let mut renderer = attach(head);
        let mut client = scene_client(cache.as_deref());
        frame(&mut renderer, &mut client);
        let last_live = frame(&mut renderer, &mut client);

        // The slot enters loading, then the camera moves and the old region
        // is unloaded under it: a scene build now would look different.
        client.scene_state = 1;
        look_from(&mut client, 13 * 128, 2 * 128, 160);
        client.world.reset_map();
        let frozen = frame(&mut renderer, &mut client);
        let (outside, inside) = split_diff(&last_live, &frozen);
        assert_eq!(
            outside, 0,
            "{head:?}: the frozen scene must be the last live scene outside the splash"
        );
        match (head, cache.is_some()) {
            (Head::Cpu, true) => assert!(
                inside > 100,
                "CPU: the loading splash must draw over the frozen scene ({inside} px)"
            ),
            (Head::Gpu, true) => assert_eq!(
                inside, 0,
                "GPU: today the retained texture hides the uncovered splash text"
            ),
            (_, false) => assert_eq!(inside, 0, "{head:?}: no fonts, no splash"),
        }
        let frozen_again = frame(&mut renderer, &mut client);
        assert_eq!(
            scene_diff(&frozen, &frozen_again),
            0,
            "{head:?}: repeated loading frames must not accumulate or rebuild"
        );

        // Loading ends: the next paint is a real scene boundary again.
        client.scene_state = 2;
        let rebuilt = frame(&mut renderer, &mut client);
        assert!(
            scene_diff(&frozen, &rebuilt) > SCENE_PIXELS / 4,
            "{head:?}: scene_state 2 must replace the frozen scene"
        );
    }
}

/// A ready NPC (`r#type` set; no NPC config or model is read on the
/// freeze path) whose health bar is visible: `entity_overlays` draws it as
/// two `fill_rect`s, 15 px green then 15 px red, 5 px high, with no sprite or
/// font, so the fixture is cache-free.
fn health_bar_npc(client: &mut Client, x: i32, z: i32) {
    let mut npc = ClientNpc {
        r#type: Some(0),
        ..Default::default()
    };
    npc.entity.x = x;
    npc.entity.z = z;
    npc.entity.height = 100;
    npc.entity.health = 10;
    npc.entity.total_health = 20;
    npc.entity.combat_cycle = client.loop_cycle + 1_000;
    client.npc[0] = Some(Box::new(npc));
    client.npc_ids[0] = 0;
    client.npc_count = 1;
}

/// The scene-window pixels of `px` that differ from `base`, as a bounding
/// box `(x0, y0, x1, y1)` (frame coordinates, exclusive end) with the count
/// of changed pixels and how many are `Colour::GREEN` / `Colour::RED`.
#[derive(Debug, PartialEq, Eq)]
struct OverlayChange {
    bbox: (usize, usize, usize, usize),
    changed: usize,
    green: usize,
    red: usize,
    in_splash: usize,
}

fn overlay_change(base: &[i32], px: &[i32]) -> OverlayChange {
    let mut change = OverlayChange {
        bbox: (usize::MAX, usize::MAX, 0, 0),
        changed: 0,
        green: 0,
        red: 0,
        in_splash: 0,
    };
    for i in (0..px.len()).filter(|&i| in_scene(i) && base[i] != px[i]) {
        let (x, y) = (i % FRAME_W, i / FRAME_W);
        let b = &mut change.bbox;
        *b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
        change.changed += 1;
        change.green += usize::from(px[i] == 0x00ff00);
        change.red += usize::from(px[i] == 0xff0000);
        change.in_splash += usize::from(in_splash(i));
    }
    change
}

/// Asserts `change` is exactly one 30×5 health bar (75 green and 75 red
/// pixels) at `bbox`, clear of the splash box.
fn assert_one_health_bar(
    head: Head,
    what: &str,
    change: &OverlayChange,
    bbox: (usize, usize, usize, usize),
) {
    assert_eq!(
        (change.changed, change.green, change.red, change.in_splash),
        (150, 75, 75, 0),
        "{head:?} {what}: exactly one 30x5 health bar must change over the frozen scene ({change:?})"
    );
    assert_eq!(
        change.bbox, bbox,
        "{head:?} {what}: the bar sits where the current NPC position projects"
    );
}

/// Frame-space boxes of the fixture's health bar. With the `scene_client`
/// camera (x 2048, z 512, y −600, pitch 200, yaw 0, origin (256, 167)) and the
/// bar at height 115 over flat ground, the integer projection puts the bar
/// centre at (256, 81) for NPC x 2048 / z 1536 and at (80, 81) for x 1664;
/// the bar spans (−15..15, −3..2) around it and `area_game` blits at (4, 4).
const BAR_AHEAD: (usize, usize, usize, usize) = (245, 79, 275, 84);
const BAR_LEFT: (usize, usize, usize, usize) = (69, 79, 99, 84);

/// Family **scene** (overlay over a retained scene): while the slot is
/// loading (`scene_state == 1`), a game overlay appears, moves and clears
/// over the frozen scene. The overlay is an NPC health bar from
/// `entity_overlays` (cache-free, like the GPU crate's overlay fixtures).
///
/// - **GPU** keeps the retained scene texture and redraws the entity and
///   interface overlays every frame (`GpuBackend::scene` →
///   `draw_scene_overlays` while `freeze_last_scene`): the bar appears, follows
///   the NPC with the old pixels restored to the frozen scene, does not
///   accumulate, and clears back to the exact frozen frame. The scene is not
///   rebuilt: `reset_map` emptied the world and the camera did not move, so a
///   rebuild would differ everywhere.
/// - **CPU**, today, skips the whole scene stage while `scene_state != 2`
///   (`CpuBackend::scene`), so `entity_overlays` does not run and the frozen
///   scene window does not change at all. This pins current behaviour only;
///   which overlays a frozen CPU head must show is T4.4's decision (§23 §2.6).
#[test]
fn scene_freeze_overlay_changes_and_clears_over_the_retained_scene() {
    let _serial = serial();
    let cache = assets();
    for head in heads() {
        let mut renderer = attach(head);
        let mut client = scene_client(cache.as_deref());
        frame(&mut renderer, &mut client);
        let last_live = frame(&mut renderer, &mut client);

        // Loading starts and the region is unloaded; the camera stays, so
        // both health-bar positions project into the same scene window.
        client.scene_state = 1;
        client.world.reset_map();
        let frozen = frame(&mut renderer, &mut client);
        assert_eq!(
            split_diff(&last_live, &frozen).0,
            0,
            "{head:?}: the frozen scene is the last live scene outside the splash"
        );

        // The overlay appears: NPC 8 tiles ahead of the camera.
        health_bar_npc(&mut client, 16 * 128, 12 * 128);
        let shown = frame(&mut renderer, &mut client);
        let shown_change = overlay_change(&frozen, &shown);
        let shown_again = frame(&mut renderer, &mut client);

        // The overlay changes: the NPC moves 3 tiles to the left.
        let loop_cycle = client.loop_cycle;
        let npc = client.npc[0].as_mut().expect("fixture NPC");
        npc.entity.x = 13 * 128;
        let moved = frame(&mut renderer, &mut client);
        let moved_change = overlay_change(&frozen, &moved);

        // The overlay clears: the combat cycle expires.
        let npc = client.npc[0].as_mut().expect("fixture NPC");
        npc.entity.combat_cycle = loop_cycle;
        let cleared = frame(&mut renderer, &mut client);
        eprintln!(
            "{head:?}: freeze overlay shown {:?} moved {:?}",
            shown_change, moved_change
        );

        match head {
            Head::Gpu => {
                assert_one_health_bar(head, "shown", &shown_change, BAR_AHEAD);
                assert_eq!(
                    scene_diff(&shown, &shown_again),
                    0,
                    "GPU: an unchanged overlay over the frozen scene must not accumulate"
                );
                assert_one_health_bar(head, "moved", &moved_change, BAR_LEFT);
                assert_eq!(
                    scene_diff(&frozen, &cleared),
                    0,
                    "GPU: a cleared overlay must leave exactly the frozen scene"
                );
            }
            Head::Cpu => {
                for (what, px) in [
                    ("shown", &shown),
                    ("shown again", &shown_again),
                    ("moved", &moved),
                    ("cleared", &cleared),
                ] {
                    assert_eq!(
                        scene_diff(&frozen, px),
                        0,
                        "CPU {what}: today the frozen scene window runs no entity overlays"
                    );
                }
            }
        }

        // No NPC config exists in this fixture, so drop the NPC before the
        // next live scene build would look it up.
        client.npc[0] = None;
        client.npc_count = 0;
        client.scene_state = 2;
        let rebuilt = frame(&mut renderer, &mut client);
        assert!(
            scene_diff(&frozen, &rebuilt) > SCENE_PIXELS / 4,
            "{head:?}: scene_state 2 must replace the frozen scene (the world was reset)"
        );
    }
}

/// Family **scene**: in game with no scene at all (`scene_state == 0`, the
/// state before the first rebuild). The GPU head invalidates its retained
/// texture and seals the scene window as opaque 2D (no 3D left); the CPU head
/// skips every scene stage and keeps presenting its persistent `draw_area`,
/// so the last frame stays on screen. Another CPU/GPU divergence.
#[test]
fn scene_no_scene_state_drops_gpu_texture_but_cpu_keeps_its_framebuffer() {
    let _serial = serial();
    for head in heads() {
        let mut renderer = attach(head);
        let mut client = scene_client(None);
        frame(&mut renderer, &mut client);
        let live = frame(&mut renderer, &mut client);
        client.scene_state = 0;
        let empty = frame(&mut renderer, &mut client);
        match head {
            Head::Cpu => assert_eq!(
                scene_diff(&live, &empty),
                0,
                "CPU: scene_state 0 leaves draw_area untouched"
            ),
            Head::Gpu => assert_eq!(
                lit_scene(&empty),
                0,
                "GPU: scene_state 0 must stop compositing the retained texture"
            ),
        }
    }
}

/// Family **head**: the retained scene belongs to one head. A head attached
/// while the slot is loading presents an empty scene (the explicit
/// PendingScene of §23 §2.6; plus the splash on CPU, or on GPU where no
/// texture hides it), never another head's last scene; it resolves at the
/// next real scene boundary from the same `Client`, with no reconnect and no
/// map rebuild.
#[test]
fn head_attached_during_loading_starts_with_an_empty_scene() {
    let _serial = serial();
    let cache = assets();
    for head in heads() {
        let mut client = scene_client(cache.as_deref());
        let mut old = attach(head);
        frame(&mut old, &mut client);
        let old_live = frame(&mut old, &mut client);
        client.scene_state = 1;
        frame(&mut old, &mut client);
        // Detach (the slot drops its renderer), then attach a fresh head.
        drop(old);
        let mut new = attach(head);
        let pending = frame(&mut new, &mut client);
        let lit = lit_scene(&pending);
        let lit_outside_splash = (0..pending.len())
            .filter(|&i| in_scene(i) && !in_splash(i) && pending[i] != 0)
            .count();
        assert_eq!(
            lit_outside_splash, 0,
            "{head:?}: a new head has no scene to present ({lit} lit)"
        );
        if cache.is_some() {
            assert!(lit > 100, "{head:?}: the splash shows over the empty scene");
        }
        client.scene_state = 2;
        let resolved = frame(&mut new, &mut client);
        assert_eq!(
            scene_diff(&old_live, &resolved),
            0,
            "{head:?}: the new head builds the same scene from the retained Client"
        );
    }
}

/// Family **head**: two heads keep separate retained scenes. Each frozen
/// head presents its own last scene, whatever the other head drew since; the
/// retained image is per head, not a process-wide last-composed frame.
#[test]
fn head_retained_scenes_are_per_head_not_per_process() {
    let _serial = serial();
    for head in heads() {
        let mut a_client = scene_client(None);
        let mut b_client = scene_client(None);
        look_from(&mut b_client, 15 * 128, 3 * 128, 96);
        let mut a = attach(head);
        let mut b = attach(head);
        frame(&mut a, &mut a_client);
        let a_live = frame(&mut a, &mut a_client);
        frame(&mut b, &mut b_client);
        let b_live = frame(&mut b, &mut b_client);
        assert!(
            scene_diff(&a_live, &b_live) > SCENE_PIXELS / 4,
            "the fixture heads must show different scenes"
        );
        a_client.scene_state = 1;
        b_client.scene_state = 1;
        let a_frozen = frame(&mut a, &mut a_client);
        let b_frozen = frame(&mut b, &mut b_client);
        let a_again = frame(&mut a, &mut a_client);
        assert_eq!(split_diff(&a_live, &a_frozen).0, 0, "{head:?}: head A");
        assert_eq!(split_diff(&b_live, &b_frozen).0, 0, "{head:?}: head B");
        assert_eq!(
            split_diff(&a_live, &a_again).0,
            0,
            "{head:?}: head A after head B's frame"
        );
    }
}

/// Family **frame**: a CPU frame is an owned snapshot. It does not change
/// when the head paints again and it outlives the head that produced it.
#[test]
fn frame_cpu_output_is_an_owned_snapshot() {
    let _serial = serial();
    let mut renderer = attach(Head::Cpu);
    let mut client = scene_client(None);
    let first = renderer.mainredraw(&mut client);
    let first_px = pixels(&first);
    look_from(&mut client, 15 * 128, 3 * 128, 96);
    let second = pixels(&renderer.mainredraw(&mut client));
    assert!(scene_diff(&first_px, &second) > SCENE_PIXELS / 4);
    assert_eq!(
        scene_diff(&first_px, &pixels(&first)),
        0,
        "a later paint must not change a published CPU frame"
    );
    drop(renderer);
    assert_eq!(
        scene_diff(&first_px, &pixels(&first)),
        0,
        "a published CPU frame outlives its head"
    );
}

/// Family **frame**: a GPU frame is a handle to the head's single stable
/// present texture, not a snapshot. A handle taken for frame N shows frame
/// N+1 once the head paints again, and stays readable (showing the last
/// frame) after the head is dropped. Consumers therefore read a GPU frame
/// before the next paint; the slot drains its mailbox on detach.
#[test]
fn frame_gpu_output_aliases_the_heads_present_texture() {
    let _serial = serial();
    if !heads().contains(&Head::Gpu) {
        return;
    }
    let mut renderer = attach(Head::Gpu);
    let mut client = scene_client(None);
    let first = renderer.mainredraw(&mut client);
    let first_px = pixels(&first);
    look_from(&mut client, 15 * 128, 3 * 128, 96);
    let second = renderer.mainredraw(&mut client);
    let second_px = pixels(&second);
    assert!(scene_diff(&first_px, &second_px) > SCENE_PIXELS / 4);
    assert_eq!(
        scene_diff(&second_px, &pixels(&first)),
        0,
        "an earlier GPU frame handle reads the head's latest present texture"
    );
    drop(renderer);
    assert_eq!(
        scene_diff(&second_px, &pixels(&first)),
        0,
        "a GPU frame handle keeps the texture alive after its head is dropped"
    );
}

/// Family **head**: a GPU head owns its per-head textures and buffers; the
/// shared context (pipelines, atlas) stays with the process. Dropping the
/// head returns the tracked GPU bytes to the pre-attach level, which is the
/// per-head render-only cost the M1 evidence reports.
#[test]
fn head_gpu_resources_are_released_when_the_head_is_dropped() {
    let _serial = serial();
    if !heads().contains(&Head::Gpu) {
        return;
    }
    client::profiling::enable();
    // The first head also creates the shared process context; measure the
    // per-head cost on a second head.
    let mut client = scene_client(None);
    {
        let mut warm = attach(Head::Gpu);
        frame(&mut warm, &mut client);
    }
    let base = client::profiling::gpu_bytes();
    let mut renderer = attach(Head::Gpu);
    frame(&mut renderer, &mut client);
    frame(&mut renderer, &mut client);
    let attached = client::profiling::gpu_bytes();
    let per_head = (attached.buffers + attached.textures) - (base.buffers + base.textures);
    eprintln!(
        "M1 per-head GPU bytes: textures {} buffers {} total {per_head}",
        attached.textures - base.textures,
        attached.buffers - base.buffers
    );
    assert!(
        attached.textures > base.textures,
        "an attached GPU head owns its own textures"
    );
    drop(renderer);
    let detached = client::profiling::gpu_bytes();
    assert_eq!(
        (detached.buffers, detached.textures),
        (base.buffers, base.textures),
        "detach must release every per-head GPU allocation"
    );
}

/// Controls for one real slot loop (`Host::run_client`) driven by a test.
#[derive(Default)]
struct SlotControl {
    draw: AtomicBool,
    stop: AtomicBool,
    frames: AtomicU64,
    dynamic_sprite_bytes: AtomicUsize,
}

/// What a test inspects on the slot's `Client` once its loop stops. A
/// `Client` is not `Send`: the slot thread builds and owns it, as in
/// production (`Host::spawn_slot`).
struct SessionAfter {
    bytes_out_start: u64,
    bytes_out_end: Option<u64>,
    ingame: bool,
    scene_state: i32,
    stamp_kept: bool,
    ground_mesh: bool,
}

/// Run the production slot loop (`Host::run_client`) on its own thread over
/// an in-game scene client, connected to `server` when given, while `body`
/// steers it. The observe hook applies `draw` each frame the way the panel's
/// focus switch does, and reports busy so the loop stays on the 20 ms frame
/// cadence.
fn with_slot<R>(
    server: Option<SocketAddr>,
    input: &Arc<SlotInput>,
    mailbox: &Arc<FrameBuf>,
    control: &SlotControl,
    body: impl FnOnce() -> R,
) -> (R, SessionAfter) {
    std::thread::scope(|scope| {
        let slot = scope.spawn(move || {
            let mut client = scene_client(None);
            if let Some(addr) = server {
                client.stream =
                    Some(ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap());
            }
            client.last_response = Some(Instant::now());
            let bytes_out_start = client.stream.as_ref().map_or(0, ClientStream::bytes_out);
            Host::run_client(
                &mut client,
                "t4slot",
                ProfileSettings::default(),
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                Arc::new(Mutex::new(String::new())),
                Some(Arc::clone(input)),
                Some(SlotRender::new(slot_key(), Arc::clone(mailbox))),
                None,
                ScriptRunPolicy::default(),
                |client, _, _, _, _| {
                    client.set_draw(control.draw.load(Ordering::SeqCst));
                    control.frames.fetch_add(1, Ordering::SeqCst);
                    control.dynamic_sprite_bytes.store(
                        client.world.dynamic_sprite_storage_bytes(),
                        Ordering::SeqCst,
                    );
                    true
                },
                |_| control.stop.load(Ordering::SeqCst),
                |_| RandomClaim::Host,
            );
            let square = client.world.square(0, 8, 8);
            SessionAfter {
                bytes_out_start,
                bytes_out_end: client.stream.as_ref().map(ClientStream::bytes_out),
                ingame: client.ingame,
                scene_state: client.scene_state,
                stamp_kept: square.is_some_and(|s| s.overlay_stamp.is_some()),
                ground_mesh: square.is_some_and(|s| s.quick_ground.is_some()),
            }
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        control.stop.store(true, Ordering::SeqCst);
        let after = slot.join().expect("slot thread");
        let result = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (result, after)
    })
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Wait until the slot has run `n` more frames (observe calls).
fn wait_frames(control: &SlotControl, n: u64) {
    let target = control.frames.load(Ordering::SeqCst) + n;
    wait_for("slot frames", Duration::from_secs(10), || {
        control.frames.load(Ordering::SeqCst) >= target
    });
}

/// A fresh `SlotKey` per slot loop, as Play mints one per spawned worker.
fn slot_key() -> SlotKey {
    static SLOTS: SlotKeyMinter = SlotKeyMinter::new();
    SLOTS.mint().expect("test slot key")
}

fn wait_paints(mailbox: &FrameBuf, n: u64) {
    let target = mailbox.generation() + n;
    wait_for("slot paints", Duration::from_secs(10), || {
        mailbox.generation() >= target
    });
}

fn take_pixels(mailbox: &FrameBuf, head: Head) -> Vec<i32> {
    let frame = mailbox.take().expect("a painted frame in the mailbox");
    match (&frame, head) {
        (FrameOutput::PixMap(_), Head::Cpu) | (FrameOutput::Texture(_), Head::Gpu) => {}
        _ => panic!("{head:?} slot published the wrong frame kind"),
    }
    pixels(&frame)
}

/// Family **head** (ADR 0001): on the real slot loop, draw off detaches the
/// head (the renderer and its GPU resources go, the mailbox is drained so no
/// stale frame is published), and draw on attaches a new head that rebuilds
/// the same scene from the retained `Client`. The game socket, session and
/// sim world survive: no new connection, no reconnect, no map rebuild.
#[test]
fn slot_head_detach_and_reattach_keep_the_session_without_reconnect() {
    let _serial = serial();
    for head in heads() {
        client::profiling::enable();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let input = SlotInput::new();
        input.set_full_rate(true);
        input.set_prefer_cpu(head == Head::Cpu);
        let mailbox = FrameBuf::new();
        let control = SlotControl::default();
        let constructed = Renderer::constructed();
        let gpu_base = client::profiling::gpu_bytes();
        let ((), after) = with_slot(Some(addr), &input, &mailbox, &control, || {
            let (server, _) = listener.accept().unwrap();
            server.set_nonblocking(true).unwrap();
            listener.set_nonblocking(true).unwrap();
            control.draw.store(true, Ordering::SeqCst);
            wait_paints(&mailbox, 3);
            assert_eq!(
                Renderer::constructed(),
                constructed + 1,
                "{head:?}: one head"
            );
            let attached = take_pixels(&mailbox, head);
            assert!(lit_scene(&attached) > SCENE_PIXELS / 4, "{head:?}: scene");

            // Detach.
            control.draw.store(false, Ordering::SeqCst);
            wait_frames(&control, 5);
            let generation = mailbox.generation();
            assert!(
                mailbox.take().is_none(),
                "{head:?}: detach drains the mailbox before dropping the head"
            );
            wait_frames(&control, 5);
            assert_eq!(mailbox.generation(), generation, "{head:?}: no paints");
            assert_eq!(
                control.dynamic_sprite_bytes.load(Ordering::SeqCst),
                0,
                "{head:?}: detach releases transient draw-sprite backing storage"
            );
            if head == Head::Gpu {
                let detached = client::profiling::gpu_bytes();
                assert_eq!(
                    (detached.buffers, detached.textures),
                    (gpu_base.buffers, gpu_base.textures),
                    "GPU: detach released the head's GPU allocations"
                );
            }

            // Reattach.
            control.draw.store(true, Ordering::SeqCst);
            wait_paints(&mailbox, 3);
            assert_eq!(
                Renderer::constructed(),
                constructed + 2,
                "{head:?}: attach builds a fresh head"
            );
            let reattached = take_pixels(&mailbox, head);
            assert_eq!(
                scene_diff(&attached, &reattached),
                0,
                "{head:?}: the new head rebuilds the same scene from the retained Client"
            );
            // Checked while the slot still owns its Client (the socket
            // closes when the slot thread drops it).
            assert!(
                matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
                "{head:?}: attach/detach must never open a new game connection"
            );
            let mut probe = [0u8; 1];
            assert!(
                !matches!(server.peek(&mut probe), Ok(0)),
                "{head:?}: the client never closed its game socket"
            );
        });
        let bytes_out_end = after.bytes_out_end.expect("the game socket survives");
        assert!(
            bytes_out_end >= after.bytes_out_start,
            "{head:?}: the same stream's counters, not a fresh connection's"
        );
        assert!(after.ingame, "{head:?}: still in game");
        assert_eq!(after.scene_state, 2, "{head:?}: no map rebuild");
        assert!(after.stamp_kept, "{head:?}: the sim ground stamp survives");
        assert!(
            after.ground_mesh,
            "{head:?}: the reattached head rematerialized its ground mesh"
        );
    }
}

/// Family **cadence**: the slot's render policy today. A focused
/// (full-rate) head paints on every 20 ms frame; a background (watch) head
/// paints at most once a second while its simulation keeps the 20 ms frame
/// loop; refocusing paints on the very next frame. The policy is decided by
/// the slot, not the backend, so one CPU head characterizes it.
#[test]
fn slot_cadence_focused_full_rate_versus_background_one_fps() {
    let _serial = serial();
    let input = SlotInput::new();
    input.set_prefer_cpu(true);
    input.set_full_rate(true);
    let mailbox = FrameBuf::new();
    let control = SlotControl::default();
    with_slot(None, &input, &mailbox, &control, || {
        control.draw.store(true, Ordering::SeqCst);
        wait_paints(&mailbox, 1);

        let (paints, frames) = window(&mailbox, &control, Duration::from_secs(1));
        assert!(
            paints >= 25 && paints + 2 >= frames,
            "focused: a paint on every frame ({paints} paints / {frames} frames)"
        );

        input.set_full_rate(false);
        wait_frames(&control, 3);
        let (paints, frames) = window(&mailbox, &control, Duration::from_millis(3200));
        assert!(
            (2..=4).contains(&paints) && frames >= 80,
            "background: 1 fps paints on a 20 ms sim loop ({paints} paints / {frames} frames)"
        );

        let before = mailbox.generation();
        input.set_full_rate(true);
        wait_frames(&control, 2);
        assert!(
            mailbox.generation() > before,
            "refocus paints on the next frame"
        );
    });
}

/// Paints and slot frames over `span`.
fn window(mailbox: &FrameBuf, control: &SlotControl, span: Duration) -> (u64, u64) {
    let (g0, f0) = (mailbox.generation(), control.frames.load(Ordering::SeqCst));
    std::thread::sleep(span);
    (
        mailbox.generation() - g0,
        control.frames.load(Ordering::SeqCst) - f0,
    )
}

/// T4.2 wrap, family **head**: a slot's head attaches to the process render
/// service with its `(SlotKey, head_generation)`, and reattach is a new
/// generation of the same slot. A GPU head runs each 3D scene submission
/// under a frame lease; a CPU head takes no scene-workspace reservation.
#[test]
fn wrap_heads_carry_slot_and_generation_and_only_gpu_scenes_lease_the_workspace() {
    let _serial = serial();
    for head in heads() {
        let input = SlotInput::new();
        input.set_full_rate(true);
        input.set_prefer_cpu(head == Head::Cpu);
        let mailbox = FrameBuf::new();
        let control = SlotControl::default();
        with_slot(None, &input, &mailbox, &control, || {
            let before = render_head::workspace_stats();
            control.draw.store(true, Ordering::SeqCst);
            wait_paints(&mailbox, 5);
            let first = mailbox.head().expect("a drawing slot has an attached head");
            let after = render_head::workspace_stats();
            let leased = after.leased() - before.leased();
            match head {
                Head::Gpu => assert!(
                    leased >= 5,
                    "GPU: every scene submission leases the workspace ({leased} for 5 paints)"
                ),
                Head::Cpu => assert_eq!(leased, 0, "CPU: no scene-workspace reservation"),
            }
            assert_eq!(
                after.unleased(),
                before.unleased(),
                "{head:?}: no unleased scene"
            );

            control.draw.store(false, Ordering::SeqCst);
            wait_frames(&control, 3);
            assert_eq!(mailbox.head(), None, "{head:?}: detach retires the head");
            control.draw.store(true, Ordering::SeqCst);
            wait_paints(&mailbox, 1);
            let second = mailbox.head().expect("draw on reattaches");
            assert_eq!(
                second.slot(),
                first.slot(),
                "{head:?}: the same slot worker"
            );
            assert_ne!(
                second.generation(),
                first.generation(),
                "{head:?}: reattach is a new head generation"
            );
        });
    }
}

/// T4.2 wrap: two GPU heads painting at full rate share the one scene
/// workspace. Both keep painting, every scene runs under a lease, and a head
/// that finds the workspace held waits for its own grant (the retained wait).
#[test]
fn wrap_gpu_heads_share_the_scene_workspace_and_both_paint() {
    let _serial = serial();
    if !heads().contains(&Head::Gpu) {
        return;
    }
    let inputs = [SlotInput::new(), SlotInput::new()];
    for input in &inputs {
        input.set_full_rate(true);
        input.set_prefer_cpu(false);
    }
    let mailboxes = [FrameBuf::new(), FrameBuf::new()];
    let controls = [SlotControl::default(), SlotControl::default()];
    with_slot(None, &inputs[0], &mailboxes[0], &controls[0], || {
        with_slot(None, &inputs[1], &mailboxes[1], &controls[1], || {
            for control in &controls {
                control.draw.store(true, Ordering::SeqCst);
            }
            for mailbox in &mailboxes {
                wait_paints(mailbox, 1);
            }
            let before = render_head::workspace_stats();
            let starts = mailboxes.each_ref().map(|mailbox| mailbox.generation());
            std::thread::sleep(Duration::from_millis(1500));
            let after = render_head::workspace_stats();
            let paints: Vec<u64> = mailboxes
                .iter()
                .zip(starts)
                .map(|(mailbox, start)| mailbox.generation() - start)
                .collect();
            let leased = after.leased() - before.leased();
            let waited = after.waited() - before.waited();
            println!(
                "wrap shared workspace: paints {paints:?}, leased {leased}, waited {waited}, wait {:?}, max wait {:?}",
                after.wait() - before.wait(),
                after.max_wait()
            );
            assert!(
                paints.iter().all(|&paints| paints >= 20),
                "both GPU heads keep painting: {paints:?}"
            );
            // The two counters are read at slightly different instants while
            // both heads paint, so a paint in flight at either window edge
            // can be leased on one side of the edge and published on the
            // other: at most one paint per head per edge. Every other
            // painted scene ran under a lease.
            let edges = 2 * paints.len() as u64;
            assert!(
                leased + edges >= paints.iter().sum::<u64>(),
                "every painted scene ran under a lease ({leased} for {paints:?})"
            );
            assert_eq!(after.unleased(), before.unleased());
            let (first, second) = (mailboxes[0].head().unwrap(), mailboxes[1].head().unwrap());
            assert_ne!(first.slot(), second.slot(), "two slot workers, two heads");
        });
    });
}
