//! The identical memory-only case selector used by both comparison sources.
//!
//! The existing `t44_draw_memory` target owns candidate-specific retained-copy
//! and present-registration assertions. This target supplies raw, same-work A/A
//! and A/B samples without importing the candidate's renderer into the comparator.
#![cfg(feature = "draw-memory-case")]

use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use client::render::memory_case::Census;
use host::draw_memory_case::{self, Control, Observation};
use host::{render_head, FrameBuf, SlotInput};
use host_play::memory::{native_allocator_sample, rust_allocator_counts, CountingAllocator};
use host_play::{
    current_resident_bytes, mint_live_names, run_with_template, Play, ProfileOptions,
    SharedClientTemplate, SlotStatus,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const CONSUMER_PERIOD: Duration = Duration::from_millis(16);
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

struct Head {
    name: String,
    input: Arc<SlotInput>,
    mailbox: Arc<FrameBuf>,
    control: Arc<Control>,
    cpu: bool,
}

struct Consumer {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().expect("memory-case consumer completed");
        }
    }
}

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required"))
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .map(|value| value.parse().unwrap_or_else(|_| panic!("invalid {key}")))
        .unwrap_or(default)
}

fn census_json(census: Census) -> Value {
    json!({
        "coverage": "common renderer scalar and CPU owner capacities only; excludes private backend payload, shared Media, GPU resources and non-common caches",
        "diagnostic_inline_bytes": census.diagnostic_inline_bytes,
        "renderer_inline_bytes": census.renderer_inline_bytes,
        "backend_inline_bytes": census.backend_inline_bytes,
        "scene_cycle": census.scene_cycle,
        "scene_counter_kind": "renderer preparation cycle, not a GPU completion fence",
        "gpu_backend": census.gpu_backend,
        "draw_area_pixel_capacity_bytes": census.draw_area_pixel_capacity_bytes,
        "game_area_pixel_capacity_bytes": census.game_area_pixel_capacity_bytes,
        "title_area_pixel_capacity_bytes": census.title_area_pixel_capacity_bytes,
        "minimap_pixel_capacity_bytes": census.minimap_pixel_capacity_bytes,
        "compass_mask_line_offsets_capacity_bytes": census.compass_mask_line_offsets_capacity_bytes,
        "compass_mask_line_lengths_capacity_bytes": census.compass_mask_line_lengths_capacity_bytes,
        "minimap_mask_line_offsets_capacity_bytes": census.minimap_mask_line_offsets_capacity_bytes,
        "minimap_mask_line_lengths_capacity_bytes": census.minimap_mask_line_lengths_capacity_bytes,
        "tile_last_occupied_cycle_capacity_bytes": census.tile_last_occupied_cycle_capacity_bytes,
        "active_map_function_x_capacity_bytes": census.active_map_function_x_capacity_bytes,
        "active_map_function_z_capacity_bytes": census.active_map_function_z_capacity_bytes,
        "active_map_function_sprite_slots_capacity_bytes": census.active_map_function_sprite_slots_capacity_bytes,
        "active_map_function_sprite_pixels_capacity_bytes": census.active_map_function_sprite_pixels_capacity_bytes,
        "chat_string_capacity_bytes": census.chat_string_capacity_bytes,
        "texture_buffer_capacity_bytes": census.texture_buffer_capacity_bytes,
        "image_runes_slots_capacity_bytes": census.image_runes_slots_capacity_bytes,
        "image_runes_data_capacity_bytes": census.image_runes_data_capacity_bytes,
        "image_runes_palette_capacity_bytes": census.image_runes_palette_capacity_bytes,
        "image_titlebox_data_capacity_bytes": census.image_titlebox_data_capacity_bytes,
        "image_titlebox_palette_capacity_bytes": census.image_titlebox_palette_capacity_bytes,
        "image_titlebutton_data_capacity_bytes": census.image_titlebutton_data_capacity_bytes,
        "image_titlebutton_palette_capacity_bytes": census.image_titlebutton_palette_capacity_bytes,
        "cpu_output_pixel_capacity_bytes": census.cpu_output_pixel_capacity_bytes,
        "cpu_output_width": census.cpu_output_width,
        "cpu_output_height": census.cpu_output_height,
    })
}

fn identity(observation: Observation) -> (u64, u64) {
    (observation.slot, observation.head_generation)
}

fn active_observation(head: &Head) -> Option<Observation> {
    let attached = head.mailbox.head()?;
    head.control.latest().filter(|observation| {
        observation.slot == attached.slot().incarnation().get()
            && observation.head_generation == attached.generation().get()
    })
}

fn statuses(play: &Play, heads: &[Head]) -> Vec<SlotStatus> {
    let statuses = play.statuses();
    heads
        .iter()
        .map(|head| {
            let status = statuses
                .iter()
                .find(|status| status.username == head.name)
                .unwrap_or_else(|| panic!("{}: missing slot", head.name));
            assert!(
                status.connected && status.ingame,
                "{}: slot stopped",
                head.name
            );
            assert_eq!(
                status.scene_state, 2,
                "{}: simulation left the live scene",
                head.name
            );
            status.clone()
        })
        .collect()
}

fn wait_ingame(play: &Play, heads: &[Head]) {
    let deadline = Instant::now() + Duration::from_secs(60 + 30 * heads.len() as u64);
    loop {
        let statuses = play.statuses();
        if heads.iter().all(|head| {
            statuses.iter().any(|status| {
                status.username == head.name
                    && status.connected
                    && status.ingame
                    && status.scene_state == 2
            })
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "every memory-case slot must enter the game"
        );
        thread::park_timeout(Duration::from_millis(25));
    }
}

fn wait_live(play: &Play, heads: &[Head]) -> Vec<Observation> {
    let deadline = Instant::now() + Duration::from_secs(60 + 30 * heads.len() as u64);
    loop {
        statuses(play, heads);
        let observations: Option<Vec<_>> = heads
            .iter()
            .map(|head| {
                assert!(
                    head.mailbox.refusal().is_none(),
                    "{}: head refused",
                    head.name
                );
                active_observation(head).filter(|observation| {
                    observation.census.scene_cycle > 0
                        && observation.census.gpu_backend != head.cpu
                        && head.mailbox.generation() > 0
                })
            })
            .collect();
        if let Some(observations) = observations {
            return observations;
        }
        assert!(
            Instant::now() < deadline,
            "each admitted head must paint a real scene"
        );
        thread::park_timeout(Duration::from_millis(25));
    }
}

fn set_policy(play: &Play, heads: &[Head], policy: &str, focus: usize) {
    assert!(matches!(policy, "full" | "focus1"));
    for (index, head) in heads.iter().enumerate() {
        head.input.set_render_focused(index == focus);
        head.input.set_full_rate(policy == "full" || index == focus);
        play.wake(&head.name);
    }
}

fn sample(
    file: &mut BufWriter<std::fs::File>,
    phase: &str,
    index: usize,
    started: Instant,
    play: &Play,
    heads: &[Head],
    attached: bool,
) {
    // Read requested Rust live before allocating the transient sample object.
    let before = rust_allocator_counts();
    let rss = current_resident_bytes();
    let native = native_allocator_sample();
    #[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos"))]
    assert!(
        native.is_some(),
        "the supported native allocator sampler failed"
    );
    let statuses = statuses(play, heads);
    let gpu = client::profiling::gpu_bytes();
    let rows: Vec<_> = heads
        .iter()
        .zip(&statuses)
        .map(|(head, status)| {
            assert!(
                head.mailbox.refusal().is_none(),
                "{}: head refused",
                head.name
            );
            assert_eq!(
                head.mailbox.head().is_some(),
                attached,
                "{}: wrong attachment phase",
                head.name
            );
            let observation = active_observation(head);
            assert_eq!(
                observation.is_some(),
                attached,
                "{}: missing current census",
                head.name
            );
            json!({
                "name": head.name,
                "stream_epoch": status.stream_epoch,
                "login_present": status.login_started.is_some(),
                "frame_generation": head.mailbox.generation(),
                "slot": observation.map(|value| value.slot),
                "head_generation": observation.map(|value| value.head_generation),
                "paints": observation.map(|value| value.paints),
                "loading_paints": observation.map(|value| value.loading_paints),
                "simulation_scene_state": observation.map(|value| value.simulation_scene_state),
                "paint_scene_state": observation.map(|value| value.paint_scene_state),
                "owner_census": observation.map(|value| census_json(value.census)),
            })
        })
        .collect();
    let mut row = json!({
        "kind": "sample",
        "phase": phase,
        "phase_sample": index,
        "phase_elapsed_s": started.elapsed().as_secs_f64(),
        "rss_bytes": rss,
        "rust_allocator": { "allocation_calls": before.0, "allocated_bytes": before.1, "live_bytes": before.2 },
        "native_allocator": native,
        "logical_gpu": { "buffers": gpu.buffers, "textures": gpu.textures },
        "v8_domains": script_js::memory_profile::snapshots(),
        "heads": rows,
    });
    let owned = rust_allocator_counts();
    row["diagnostic_sample_rust"] = json!({
        "allocation_calls": owned.0.saturating_sub(before.0),
        "allocated_bytes": owned.1.saturating_sub(before.1),
        "live_bytes_delta": i128::from(owned.2) - i128::from(before.2),
        "coverage": "transient sample construction plus concurrent product allocations; not a per-owner allocation counter",
    });
    serde_json::to_writer(&mut *file, &row).expect("write memory sample");
    file.write_all(b"\n").expect("terminate memory sample");
    file.flush().expect("flush memory sample");
}

fn observe(
    file: &mut BufWriter<std::fs::File>,
    phase: &str,
    duration: Duration,
    play: &Play,
    heads: &[Head],
    attached: bool,
) {
    let started = Instant::now();
    let mut index = 0;
    loop {
        sample(file, phase, index, started, play, heads, attached);
        let next = started + SAMPLE_PERIOD * (index as u32 + 1);
        if next > started + duration {
            break;
        }
        thread::park_timeout(next.saturating_duration_since(Instant::now()));
        index += 1;
    }
}

#[test]
fn diagnostic_allocations_without_drawing() {
    const CHILD: &str = "T44_MEMORY_CASE_ALLOCATOR_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostic_allocations_without_drawing",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .status()
            .expect("run isolated diagnostic allocation receipt");
        assert!(status.success());
        return;
    }
    draw_memory_case::clear();
    let before = rust_allocator_counts();
    let control = Control::new();
    draw_memory_case::install(&control);
    let allocated = rust_allocator_counts();
    for _ in 0..100 {
        draw_memory_case::install(&control);
        assert!(control.latest().is_none());
    }
    assert_eq!(
        rust_allocator_counts(),
        allocated,
        "warm diagnostic bookkeeping allocates nothing"
    );
    draw_memory_case::clear();
    drop(control);
    let released = rust_allocator_counts();
    assert_eq!(released.2, before.2, "all diagnostic ownership is released");
    println!(
        "{}",
        json!({
            "kind": "diagnostic-allocations-without-drawing",
            "control_inline_bytes": std::mem::size_of::<Control>(),
            "observation_inline_bytes": std::mem::size_of::<Observation>(),
            "census_inline_bytes": std::mem::size_of::<Census>(),
            "allocation_calls": allocated.0 - before.0,
            "allocated_bytes": allocated.1 - before.1,
            "live_bytes": allocated.2 - before.2,
            "live_bytes_after_release": released.2 - before.2,
            "warm_installs": 100,
            "warm_allocation_calls": 0,
            "paint_calls": 0,
        })
    );
}

#[test]
#[ignore = "LIVE=1 exact-source drawing memory A/A and A/B, claimed quiet worker required"]
fn t44_cpu_draw_memory_ab() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let n = required("M3_N").parse::<usize>().expect("positive M3_N");
    assert!(n > 0);
    let backend = required("M3_BACKEND");
    assert!(matches!(backend.as_str(), "cpu" | "gpu" | "mixed"));
    let cpu_heads = match backend.as_str() {
        "cpu" => n,
        "gpu" => 0,
        _ => required("M3_CPU_HEADS").parse().expect("M3_CPU_HEADS"),
    };
    assert!(cpu_heads <= n);
    assert_eq!(
        std::env::var("BOT_CPU").as_deref() == Ok("1"),
        cpu_heads == n
    );
    let focus = env_or("M3_FOCUS_INDEX", 0usize);
    assert!(focus < n);
    let warmup = Duration::from_secs(env_or("M3_WARMUP_S", 10u64));
    let duration = Duration::from_secs(env_or("M3_OBSERVE_S", 30u64));
    assert!(warmup >= Duration::from_secs(10) && duration >= Duration::from_secs(30));
    let output = PathBuf::from(required("M3_OUT"));
    std::fs::create_dir_all(&output).expect("create comparison directory");
    let mut file = BufWriter::new(
        std::fs::File::create(output.join("draw-memory-case.jsonl")).expect("create raw samples"),
    );
    writeln!(
        file,
        "{}",
        json!({
            "kind": "header", "schema": "t44-draw-memory-case/v1",
            "qualification": "raw comparison arm; not an A/A spread or A/B verdict",
            "git_rev": required("M3_GIT_REV"), "client_rev": required("M3_CLIENT_REV"),
            "arm": required("T44_MEMORY_ARM"), "n": n, "cpu_heads": cpu_heads, "backend": backend,
            "diagnostic_source_sha256": required("T44_MEMORY_DIAGNOSTIC_SHA256"),
            "warmup_s": warmup.as_secs(), "observe_s": duration.as_secs(), "focus_index": focus,
            "consumer_period_ns": CONSUMER_PERIOD.as_nanos(),
        })
    )
    .expect("write header");
    file.flush().expect("flush header");
    client::profiling::enable();
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_or("M3_PORT", 44594u16)),
        http_port: Some(env_or("M3_HTTP_PORT", 1080u16)),
        engine_dir: Some(required("ENGINE_DIR").into()),
        nav_pack: Some(required("NAV_PACK").into()),
        ..ProfileOptions::default()
    };
    let bound = options
        .resolve(None)
        .expect("resolve Engine L profile")
        .bind_runtime()
        .expect("bind Engine L profile");
    let names = mint_live_names(n);
    let heads: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let input = SlotInput::new();
            input.set_prefer_cpu(index < cpu_heads);
            Head {
                name: name.clone(),
                input,
                mailbox: FrameBuf::new(),
                control: Control::new(),
                cpu: index < cpu_heads,
            }
        })
        .collect();
    let profiles: Vec<_> = heads
        .iter()
        .enumerate()
        .map(|(index, head)| Profile {
            username: head.name.clone(),
            password: head.name.clone().into(),
            uid: 274_500_000 + index as i32,
            settings: ProfileSettings {
                auto_login: true,
                lowmem: true,
                raster: if head.cpu {
                    RasterMode::Cpu
                } else {
                    RasterMode::Gpu
                },
                ..ProfileSettings::default()
            },
        })
        .collect();
    let channels: HashMap<_, _> = heads
        .iter()
        .map(|head| {
            (
                head.name.clone(),
                (Arc::clone(&head.input), Arc::clone(&head.mailbox)),
            )
        })
        .collect();
    let controls: HashMap<_, _> = heads
        .iter()
        .map(|head| (head.name.clone(), Arc::clone(&head.control)))
        .collect();
    let draw = Arc::new(AtomicBool::new(false));
    let frame_draw = Arc::clone(&draw);
    let template = SharedClientTemplate::load(bound).expect("load identical shared cache");
    let play = run_with_template(
        template,
        false,
        profiles,
        move |name| {
            let (input, mailbox) = &channels[name];
            (Some(Arc::clone(input)), Some(Arc::clone(mailbox)))
        },
        move |client, name, _| {
            draw_memory_case::install(&controls[name]);
            client.set_draw(frame_draw.load(Ordering::Acquire));
        },
    )
    .expect("create real Play");
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let mailboxes: Vec<_> = heads.iter().map(|head| Arc::clone(&head.mailbox)).collect();
    let consumer = Consumer {
        stop,
        join: Some(thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                for mailbox in &mailboxes {
                    drop(mailbox.take());
                }
                render_head::pump(Instant::now());
                thread::park_timeout(CONSUMER_PERIOD);
            }
        })),
    };
    wait_ingame(&play, &heads);
    let sessions = statuses(&play, &heads);
    observe(&mut file, "headless_warmup", warmup, &play, &heads, false);
    observe(&mut file, "headless", duration, &play, &heads, false);
    draw.store(true, Ordering::Release);
    set_policy(&play, &heads, "full", focus);
    let initial = wait_live(&play, &heads);
    for policy in ["full", "focus1"] {
        set_policy(&play, &heads, policy, focus);
        observe(
            &mut file,
            &format!("{policy}_warmup"),
            warmup,
            &play,
            &heads,
            true,
        );
        let before = wait_live(&play, &heads);
        observe(&mut file, policy, duration, &play, &heads, true);
        let after = wait_live(&play, &heads);
        for ((head, before), after) in heads.iter().zip(before).zip(after) {
            assert_eq!(
                identity(before),
                identity(after),
                "{}: head changed during observation",
                head.name
            );
            assert!(
                after.census.scene_cycle > before.census.scene_cycle,
                "{}: no real scene progress in {policy}",
                head.name
            );
        }
    }
    set_policy(&play, &heads, "full", focus);
    let before_loading: Vec<_> = heads
        .iter()
        .map(|head| head.control.latest().unwrap().loading_paints)
        .collect();
    for head in &heads {
        head.control.set_loading(true);
        play.wake(&head.name);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        statuses(&play, &heads);
        if heads.iter().zip(&before_loading).all(|(head, before)| {
            active_observation(head)
                .is_some_and(|value| value.loading_paints > *before && value.paint_scene_state == 1)
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "every real head must observe loading input"
        );
        thread::park_timeout(Duration::from_millis(25));
    }
    let frozen = wait_live(&play, &heads);
    observe(&mut file, "loading_freeze", duration, &play, &heads, true);
    for (head, before) in heads.iter().zip(&frozen) {
        let after = active_observation(head).expect("frozen head remains attached");
        assert_eq!(
            after.census.scene_cycle, before.census.scene_cycle,
            "{}: loading input rebuilt a scene",
            head.name
        );
        assert!(
            after.loading_paints > before.loading_paints,
            "{}: loading paints stopped",
            head.name
        );
    }
    draw.store(false, Ordering::Release);
    for head in &heads {
        head.control.set_loading(false);
        play.wake(&head.name);
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while heads.iter().any(|head| head.mailbox.head().is_some()) {
        statuses(&play, &heads);
        assert!(Instant::now() < deadline, "draw off must retire every head");
        thread::park_timeout(Duration::from_millis(25));
    }
    let detached: Vec<_> = heads
        .iter()
        .map(|head| {
            assert!(head.mailbox.take().is_none());
            head.mailbox.generation()
        })
        .collect();
    observe(&mut file, "detached", duration, &play, &heads, false);
    for (head, generation) in heads.iter().zip(detached) {
        assert_eq!(
            head.mailbox.generation(),
            generation,
            "{}: paint after detach",
            head.name
        );
    }
    draw.store(true, Ordering::Release);
    set_policy(&play, &heads, "full", focus);
    let reattached = wait_live(&play, &heads);
    for ((head, before), after) in heads.iter().zip(initial).zip(reattached) {
        assert_eq!(before.slot, after.slot, "{}: slot changed", head.name);
        assert_ne!(
            before.head_generation, after.head_generation,
            "{}: head was not replaced",
            head.name
        );
    }
    observe(&mut file, "reattached_warmup", warmup, &play, &heads, true);
    observe(&mut file, "reattached", duration, &play, &heads, true);
    for (before, after) in sessions.iter().zip(statuses(&play, &heads)) {
        assert_eq!(
            before.stream_epoch, after.stream_epoch,
            "{}: reconnect during comparison",
            before.username
        );
        assert_eq!(
            before.login_started, after.login_started,
            "{}: new login during comparison",
            before.username
        );
    }
    for head in &heads {
        if let Some(arm) = play.arm(&head.name) {
            arm.request_logout();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while play.statuses().iter().any(|status| status.ingame) {
        assert!(
            Instant::now() < deadline,
            "every comparison slot must log out"
        );
        thread::park_timeout(Duration::from_millis(25));
    }
    drop(consumer);
    writeln!(
        file,
        "{}",
        json!({"kind": "complete", "all_slots_logged_out": true})
    )
    .expect("write completion");
    file.flush().expect("flush completion");
}
