use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use super::{
    apply_only_render_selected, apply_ui_scale, boot_failure_is_fatal, boot_for, catalog_core_gate,
    chooser_should_open_popup, clamp_hop_label_px, debug_caption, drive_startup,
    edit_parameters_enabled, game_window_flags, hold_script_terminal_shot, live_null_tick,
    live_script_tick, live_smoke_tick, live_stress_tick, loading_text, log_follow_bottom,
    logout_enabled, manual_shot_label, parse_args, parse_live_args, progress_channel,
    random_status_text, request_clean_stop_capture, request_native_failure_capture, runner_config,
    script_failure_scenario, slot_startup_banner_line, smoke_settled, smoke_should_fire,
    startup_progress, Boot, CoreGate,
    LiveBoot, LiveNull, LiveScript, LiveSmoke, LiveStress, PanelState, ProfilePrepareJob,
    ProgressPhase, RunMode, ShotStatus, SoakCapture, StartupPreparation, BASE_WINDOW_H,
    BASE_WINDOW_W, LIVE_USAGE, NAV_FULL_SHOT_DRAIN, SMOKE_DEADLINE, SMOKE_SETTLE,
};
use crate::theme::{
    applet_offset, fit_applet, game_window_title, native_applet, panel_split_ratio, PANEL_WIDTH,
};
use crate::window::RedrawMode;
use dear_imgui_rs::{ConfigFlags, Id, WindowFlags};
use host_play::profile::ProfileEnvironment;
use host_play::SharedClientTemplate;

#[test]
fn logout_is_enabled_only_for_a_loaded_ingame_or_queued_focus() {
    assert!(!logout_enabled(false, true, true, false));
    assert!(!logout_enabled(true, false, true, false));
    assert!(!logout_enabled(true, true, false, false));
    assert!(logout_enabled(true, true, true, false));
    assert!(logout_enabled(true, true, false, true));
}

#[test]
fn headed_core_gate_rejects_scenario_only_pass_and_times_out() {
    let watch = host_play::catalog_core::CoreWatch::default();
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    let deadline = Instant::now() + Duration::from_secs(30);

    assert!(matches!(
        catalog_core_gate(Some(&watch), Some(deadline), Instant::now()),
        CoreGate::Pending
    ));
    assert!(matches!(
        catalog_core_gate(
            Some(&watch),
            Some(deadline),
            deadline + Duration::from_secs(1)
        ),
        CoreGate::Failed(_)
    ));
}

fn checked_fixture(revision: u16) -> (PathBuf, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let root = std::env::temp_dir().join(format!(
        "274bot-panel-profile-{revision}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    for jag in [
        "title",
        "config",
        "interface",
        "media",
        "versionlist",
        "textures",
        "wordenc",
        "sounds",
    ] {
        std::fs::copy(fixture.join(jag), cache.join(jag)).unwrap();
    }
    let manifest = fixture.join(format!("manifest-{revision}.json"));
    (root, cache, manifest)
}

#[test]
fn frontend_parser_prepares_real_clients_for_both_fixture_manifests() {
    for revision in [274_u16, 289] {
        let (root, cache, manifest) = checked_fixture(revision);
        let args = parse_args(
            [
                "--smoke".to_string(),
                "--profile".to_string(),
                format!("local-{revision}"),
                "--cache".to_string(),
                cache.display().to_string(),
                "--cache-manifest".to_string(),
                manifest.display().to_string(),
            ],
            None,
        )
        .expect("frontend and shared flags parse in either order");
        let env = ProfileEnvironment {
            home: Some(root.clone()),
            working_dir: Some(root.clone()),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..ProfileEnvironment::default()
        };
        let selection = args.profile.resolve_with_env(None, &env).unwrap();
        assert_eq!(selection.game_host(), "127.0.0.1");
        let profile = selection.bind().unwrap();
        let template = SharedClientTemplate::load(profile).unwrap();
        let client = template.prepare_client(274_000_001, true).unwrap();
        drop(client);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn boot_is_deferred_and_maps_live_smoke_and_vault_pass() {
    // Every slot-spawning path is deferred (returns a Boot to run on
    // the first frame after GPU init), never executed eagerly: mapping
    // a mode to a Boot must not unlock or call live_prepare — those
    // run only in `boot_execute`, after the first frame presents and
    // `on_gpu_init` injected the panel's device.
    assert!(matches!(
        boot_for(&RunMode::Live("null_raster".into()), None),
        Some(Boot::Live(LiveBoot::NullRaster))
    ));
    assert!(matches!(
        boot_for(&RunMode::Live("stress50".into()), None),
        Some(Boot::Live(LiveBoot::Stress50))
    ));
    assert!(matches!(
        boot_for(&RunMode::Live("stress50_full".into()), None),
        Some(Boot::Live(LiveBoot::Stress50Full))
    ));
    match boot_for(&RunMode::Live("script_walk".into()), Some("pass")) {
        Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "script_walk"),
        other => panic!("script_<name> must map to a deferred Script boot, got {other:?}"),
    }
    match boot_for(&RunMode::Live("nav_full".into()), None) {
        Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "nav_full"),
        other => panic!("nav_full must map to a deferred Script boot, got {other:?}"),
    }
    match boot_for(&RunMode::Interactive, Some("hunter2")) {
        Some(Boot::Unlock { pass }) => assert_eq!(pass, "hunter2"),
        other => panic!("BOT_VAULT_PASS must map to a deferred Unlock boot, got {other:?}"),
    }
    assert!(matches!(
        boot_for(&RunMode::Smoke, None),
        Some(Boot::Live(LiveBoot::Smoke))
    ));
    assert!(
        boot_for(&RunMode::Interactive, None).is_none(),
        "no boot with no live arg and no pass"
    );
    assert!(!boot_failure_is_fatal(&Boot::Unlock {
        pass: "secret".into()
    }));
    assert!(boot_failure_is_fatal(&Boot::Live(LiveBoot::Smoke)));
}

fn prepared_startup(boot: Boot) -> (PanelState, StartupPreparation, PathBuf) {
    let (root, cache, manifest) = checked_fixture(274);
    let options = host_play::ProfileOptions {
        profile: Some("local-274".into()),
        cache_dir: Some(cache),
        cache_manifest: Some(manifest),
        vault_path: Some(root.join("startup.vault")),
        ..host_play::ProfileOptions::default()
    };
    let env = ProfileEnvironment {
        home: Some(root.clone()),
        working_dir: Some(root.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    };
    let template = options
        .resolve_with_env(None, &env)
        .unwrap()
        .prepare_template()
        .unwrap();
    let mut state = PanelState::default();
    state.session.configure_profile(options).unwrap();
    let generation = state.session.profile_generation();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender.send(Ok(template)).unwrap();
    let (_, progress) = progress_channel(host_play::progress::ProfileProgress::steps(
        host_play::progress::ProfileProgressStage::LoadingGameData,
        4,
        4,
    ));
    let startup = StartupPreparation {
        prepare: Some(ProfilePrepareJob {
            generation,
            receiver,
            progress,
        }),
        validate: None,
        pending_boot: Some(boot),
        failed_generation: None,
    };
    (state, startup, root)
}

#[test]
fn normal_unlock_waits_for_worker_validation_then_uses_prepared_profile() {
    let (mut state, mut startup, root) = prepared_startup(Boot::Unlock {
        pass: "prepared-pass".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.session.play.is_none() && Instant::now() < deadline {
        drive_startup(&mut state, &mut startup);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(state.session.profile_bound());
    assert!(state.session.vault.is_some());
    assert!(state.session.play.is_some());
    assert!(state.session.slots.is_empty());
    assert!(startup_progress(&startup, state.session.profile_generation()).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_boot_stays_deferred_while_final_validation_is_in_flight() {
    let (mut state, mut startup, root) = prepared_startup(Boot::Live(LiveBoot::Smoke));
    drive_startup(&mut state, &mut startup);
    assert!(state.session.profile_bound());
    let validation = startup.validate.take().expect("final validation worker");
    assert!(state.session.profile_preparing());
    assert!(state.session.vault.is_none());
    assert!(state.session.play.is_none());
    assert!(state.session.slots.is_empty());
    assert!(validation
        .receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_ok());
    assert!(matches!(validation.boot, Boot::Live(LiveBoot::Smoke)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_progress_is_latest_only_and_generation_scoped() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let (observer, progress) = progress_channel(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        0,
        8,
    ));
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        3,
        8,
    ));
    observer.report(ProfileProgress::bytes(
        ProfileProgressStage::CheckingNavigationFiles,
        1024,
        4096,
    ));
    let (_sender, receiver) = std::sync::mpsc::sync_channel(1);
    let mut startup = StartupPreparation::new(None);
    startup.prepare = Some(ProfilePrepareJob {
        generation: 7,
        receiver,
        progress,
    });

    let current = startup_progress(&startup, 7).expect("matching generation progress");
    assert_eq!(current.phase, ProgressPhase::Preparing);
    assert_eq!(current.progress.completed, 1024);
    assert_eq!(current.progress.total, 4096);
    assert!(
        startup_progress(&startup, 8).is_none(),
        "stale work is hidden"
    );
    startup.prepare = None;
    assert!(
        startup_progress(&startup, 7).is_none(),
        "completion clears it"
    );
}

#[test]
fn preparation_failure_clears_progress_with_partial_session_state_absent() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let (root, cache, manifest) = checked_fixture(274);
    let mut state = PanelState::default();
    state
        .session
        .configure_profile(host_play::ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            ..host_play::ProfileOptions::default()
        })
        .unwrap();
    let generation = state.session.profile_generation();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender
        .send(Err("fixture preparation failed".into()))
        .unwrap();
    let (_, progress) = progress_channel(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        3,
        8,
    ));
    let mut startup = StartupPreparation {
        prepare: Some(ProfilePrepareJob {
            generation,
            receiver,
            progress,
        }),
        validate: None,
        pending_boot: Some(Boot::Unlock {
            pass: "not-used".into(),
        }),
        failed_generation: None,
    };

    drive_startup(&mut state, &mut startup);

    assert!(startup_progress(&startup, generation).is_none());
    assert!(!state.session.profile_preparing());
    assert!(!state.session.profile_bound());
    assert!(state.session.vault.is_none());
    assert!(state.session.play.is_none());
    assert!(state.session.slots.is_empty());
    assert_eq!(
        state.session.error.as_deref(),
        Some("fixture preparation failed")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn loading_text_fits_the_rail_and_never_rounds_partial_work_to_complete() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let partial = loading_text(
        ProgressPhase::FinalChecks,
        &ProfileProgress::bytes(ProfileProgressStage::CheckingNavigationFiles, 999, 1000),
    );

    assert!(partial.description.starts_with("Final checks"));
    assert_eq!(partial.filled.len() + partial.empty.len(), 20);
    assert_eq!(partial.percent, 99);
    assert!(partial.caption.contains("bytes checked"));
}

#[test]
fn loading_text_names_bundled_decode_and_custom_verify_passes() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let bundled = loading_text(
        ProgressPhase::Preparing,
        &ProfileProgress::steps(ProfileProgressStage::PreparingNavigation, 1, 1),
    );
    assert_eq!(bundled.description, "Loading navigation");
    assert_eq!(bundled.percent, 100);

    let custom = loading_text(
        ProgressPhase::Preparing,
        &ProfileProgress::bytes(ProfileProgressStage::CheckingNavigationFiles, 10, 100),
    );
    assert_eq!(custom.description, "Verifying custom navigation");
    assert_eq!(custom.percent, 10);
    assert!(custom.caption.contains("bytes checked"));
}

#[test]
fn log_follow_bottom_sticks_at_end_and_releases_when_scrolled_up() {
    assert!(log_follow_bottom(0.0, 0.0), "empty / first frame follows");
    assert!(log_follow_bottom(99.0, 100.0), "within 1 px of the bottom");
    assert!(log_follow_bottom(100.0, 100.0));
    assert!(!log_follow_bottom(50.0, 100.0), "scrolled up stays put");
}

#[test]
fn chooser_should_open_popup_table() {
    // First open: rising edge opens the popup and latches prev.
    assert_eq!(chooser_should_open_popup(true, false), (true, true));
    // Already open: no re-open while want stays true.
    assert_eq!(chooser_should_open_popup(true, true), (false, true));
    // Esc closed it: want drops to false and prev must fall so a later
    // `+ add bot` is a fresh rising edge.
    assert_eq!(chooser_should_open_popup(false, true), (false, false));
    assert_eq!(chooser_should_open_popup(false, false), (false, false));
}

#[test]
fn chooser_reopens_after_a_close() {
    let mut prev = false;
    let (open, np) = chooser_should_open_popup(true, prev);
    assert!(open, "first + add opens the chooser");
    prev = np;
    let (open, np) = chooser_should_open_popup(true, prev);
    assert!(!open, "already open: no reopen");
    prev = np;
    let (open, np) = chooser_should_open_popup(false, prev);
    assert!(!open);
    prev = np;
    assert!(!prev, "prev must track the close so + add can reopen");
    let (open, _np) = chooser_should_open_popup(true, prev);
    assert!(open, "the next + add bot reopens the chooser");
}

#[test]
fn edit_parameters_enabled_for_operator_bag() {
    assert!(edit_parameters_enabled(), "parameter editors ship in 0.1.6");
}

#[test]
fn loadout_combo_lists_store_names() {
    use script::{resolve_setting_options, Loadout, LoadoutsStore, SettingDef};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "274bot-panel-loadout-combo-{n}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = LoadoutsStore::at(dir.join("loadouts.json"));
    store.upsert(Loadout::new("guard"));
    store.upsert(Loadout::new("stall"));
    let def = SettingDef {
        id: "loadout".into(),
        ty: "string".into(),
        default: None,
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: Some("loadouts".into()),
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    };
    assert_eq!(
        resolve_setting_options(&def, &store, None),
        vec!["guard".to_string(), "stall".to_string()]
    );
}

#[test]
fn debug_captions_fit_the_strip() {
    assert_eq!(debug_caption("DebugPanel"), "Panel");
    assert_eq!(debug_caption("Lumbridge"), "Lumb");
    assert_eq!(debug_caption("maxme"), "maxme");
    assert_eq!(debug_caption("Teles"), "Teles");
    assert_eq!(debug_caption("TutSkip"), "TutSkip");
}

#[test]
fn hop_label_px_writes_clamp_to_8_28() {
    // The settings UI is the only writer of `hop_label_px`; it must
    // hold the NavSettings 8..=28 field invariant.
    assert_eq!(clamp_hop_label_px(0), 8);
    assert_eq!(clamp_hop_label_px(8), 8);
    assert_eq!(clamp_hop_label_px(11), 11);
    assert_eq!(clamp_hop_label_px(28), 28);
    assert_eq!(clamp_hop_label_px(100), 28);
}

#[test]
fn apply_only_render_selected_warns_before_unchecking() {
    // Checking "only render selected" on is immediate, no dialog.
    assert_eq!(apply_only_render_selected(false, true), (true, false));
    // Unchecking does not apply: keeps the safe default, opens the
    // warning instead.
    assert_eq!(apply_only_render_selected(true, false), (true, true));
    // No-op rows: the box already matches the flag.
    assert_eq!(apply_only_render_selected(true, true), (true, false));
    assert_eq!(apply_only_render_selected(false, false), (false, false));
}

#[test]
fn runner_config_docks_without_viewports() {
    let c = runner_config();
    assert!(c.docking.enable);
    assert!(
        !c.docking.auto_dockspace,
        "we own the game-left / panel-right split"
    );
    assert!(
        c.docking
            .dockspace_flags
            .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR),
        "single-bot hides the game/panel tab strip"
    );
    assert!(
        c.docking
            .dockspace_flags
            .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE),
        "dock splitters stay off — only grid uses OS-window resize"
    );
    assert!(c.ini_filename.is_none(), "no imgui.ini to restore");
    let flags = c.io_config_flags.expect("flags");
    assert!(flags.contains(ConfigFlags::DOCKING_ENABLE));
    assert!(!flags.contains(ConfigFlags::VIEWPORTS_ENABLE));
    assert!(matches!(c.redraw, RedrawMode::WaitUntil { fps } if (fps - 50.0).abs() < 0.01));
    assert_eq!(c.window_size, (BASE_WINDOW_W as f64, BASE_WINDOW_H as f64));
}

#[test]
fn single_bot_window_class_hides_tab_bar() {
    let c = super::game_window_class();
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING));
    assert!(!c.docking_always_tab_bar);
}

#[test]
fn rail_window_class_shows_tab_x() {
    let c = super::rail_window_class();
    assert!(
        !c.dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR),
        "hidden tab strip has no X"
    );
    assert!(c.docking_always_tab_bar, "tab X needs a visible tab bar");
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    const SRC: &str = include_str!("app.rs");
    let rail = SRC.split("fn rail_window(").nth(1).unwrap_or("");
    let rail_fn = rail
        .split("fn apply_only_render_selected")
        .next()
        .unwrap_or("");
    assert!(
        rail_fn.contains(".opened(") && rail_fn.contains("set_multibox(false)"),
        "rail tab X turns MultiBox off so ensure_window_fits can shrink"
    );
    assert!(
        !rail_fn.contains("NO_TITLE_BAR"),
        "NO_TITLE_BAR hides the tab that carries the X"
    );
    let frame = SRC.split("fn ui_frame").nth(1).unwrap_or("");
    assert!(
        frame.contains("rail_window_class"),
        "rail must not reuse the Game class that AUTO_HIDEs the tab bar"
    );
}

#[test]
fn chooser_docks_to_panel_never_rail_or_game() {
    let panel = Id::from(20u32);
    assert_eq!(super::chooser_dock_id(Some(panel)), Some(panel));
    assert_eq!(super::chooser_dock_id(None), None);
    const SRC: &str = include_str!("app.rs");
    let chooser = SRC
        .split("fn chooser_window")
        .nth(1)
        .unwrap_or("")
        .split("fn settings_window")
        .next()
        .unwrap_or("");
    assert!(
        chooser.contains("panel_window_class"),
        "Profiles must use the 274bot panel class"
    );
    assert!(
        !chooser.contains("rail_window_class"),
        "Profiles must never dock to the MultiBox rail"
    );
    assert!(
        chooser.contains("Appearing"),
        "spawn docks on the hidden→visible edge; rebuild must re-dock by name"
    );
}

#[test]
fn dock_host_redocks_profiles_onto_the_panel_node() {
    const SRC: &str = include_str!("app.rs");
    let host = SRC
        .split("fn dock_host")
        .nth(1)
        .unwrap_or("")
        .split("fn game_window_flags")
        .next()
        .unwrap_or("");
    assert!(
        host.contains("dock_panel_tabs"),
        "MultiBox rail / OS resize rebuild must re-dock Profiles onto the 274bot leaf"
    );
    let tabs = SRC
        .split("fn dock_panel_tabs")
        .nth(1)
        .unwrap_or("")
        .split("fn dock_host")
        .next()
        .unwrap_or("");
    for title in ["Profiles", "General config", "Nav config", "Script prefs"] {
        assert!(
            tabs.contains(title),
            "panel tab {title} must be DockBuilder::dock_window'd after a tree rebuild"
        );
    }
    assert!(
        !tabs.contains("Scripts") && !tabs.contains("Loadouts"),
        "overlay pickers stay floating over Game"
    );
}

#[test]
fn panel_window_class_allows_config_tabs() {
    let c = super::panel_window_class();
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_DOCKING_SPLIT));
    assert!(
        !c.dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING),
        "configs must be able to tab onto 274bot and undock later"
    );
    assert!(
        c.docking_allow_unclassed,
        "General/Nav are unclassed and must merge with the panel"
    );
    assert!(
        c.docking_always_tab_bar,
        "panel tab bar is the drop target for configs"
    );
}

#[test]
fn dockspace_does_not_lock_undock_on_every_node() {
    let f = super::dock_flags();
    assert!(f.contains(dear_imgui_rs::DockNodeFlags::NO_DOCKING_SPLIT));
    assert!(f.contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(
        !f.contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING),
        "NO_UNDOCKING on the dockspace would lock the panel against config tabs"
    );
}

#[test]
fn ensure_window_fits_uses_rail_open_falling_edge() {
    const SRC: &str = include_str!("app.rs");
    let body = SRC.split("fn ensure_window_fits").nth(1).unwrap_or("");
    let body = body.split("fn dock_host").next().unwrap_or("");
    assert!(
        body.contains("next_os_window_size") && body.contains("DockLayout::Rail"),
        "MultiBox off must re-shrink via the rail falling edge, not grow-only"
    );
}

#[test]
fn apply_ui_scale_scales_padding_for_retina() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    let mut ctx = dear_imgui_rs::Context::create();
    let before = ctx.style().window_padding();
    apply_ui_scale(ctx.style_mut(), 2.0);
    let after = ctx.style().window_padding();
    assert_eq!(before, [8.0, 8.0]);
    assert_eq!(after, [16.0, 16.0]);
}

#[test]
fn fit_applet_keeps_aspect_and_does_not_dpi_double() {
    assert_eq!(native_applet(), [765.0, 503.0]);
    assert_eq!(fit_applet([765.0, 503.0]), [765.0, 503.0]);
    let off = applet_offset([1200.0, 700.0], [765.0, 503.0]);
    assert!(
        (off[0] - (1200.0 - 765.0)).abs() < 0.01,
        "flush to the panel"
    );
    assert!((off[1] - (700.0 - 503.0) * 0.5).abs() < 0.01);
    // Grid cells downscale; the non-grid Game blit stays native_applet.
    assert_eq!(fit_applet([382.5, 251.5]), [382.5, 251.5]);
    let wide = fit_applet([2000.0, 503.0]);
    assert!((wide[1] - 503.0).abs() < 0.01);
    assert!(wide[0] <= 2000.0);
}

#[test]
fn game_window_flags_have_no_scrollbar() {
    let f = game_window_flags();
    assert!(f.contains(WindowFlags::NO_SCROLLBAR));
    assert!(f.contains(WindowFlags::NO_SCROLL_WITH_MOUSE));
    assert!(f.contains(WindowFlags::NO_RESIZE));
    assert!(!f.contains(WindowFlags::HORIZONTAL_SCROLLBAR));
}

#[test]
fn game_window_title_is_the_profile_name() {
    assert_eq!(game_window_title(Some("test")), "test");
    assert_eq!(game_window_title(None), "Game");
    assert_eq!(game_window_title(Some("")), "Game");
}

#[test]
fn panel_split_is_a_thin_right_slice() {
    let r = panel_split_ratio(1120.0);
    assert!((r - PANEL_WIDTH / 1120.0).abs() < 0.001);
    let wide = panel_split_ratio(2000.0);
    assert!(
        (wide * 2000.0 - PANEL_WIDTH).abs() < 0.01,
        "panel stays 330px on a wide host window, got {}",
        wide * 2000.0
    );
}

#[test]
fn parse_live_args_none_without_flag_or_env() {
    assert_eq!(
        parse_live_args([] as [&str; 0], None),
        Ok(RunMode::Interactive)
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("")),
        Ok(RunMode::Interactive)
    );
}

#[test]
fn parse_args_accepts_session_nav_paint_choice() {
    let parsed = parse_args(["--nav-paints", "on", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.nav_paints, Some(true));
    assert_eq!(parsed.mode, RunMode::Live("script_rock_crab".into()));

    let parsed = parse_args(["--nav-paints", "off", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.nav_paints, Some(false));
}

#[test]
fn parse_args_accepts_session_memory_choice_and_rejects_conflicts() {
    let parsed = parse_args(["--lowmem", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, Some(true));
    let parsed = parse_args(["--highmem", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, Some(false));
    let parsed = parse_args(["--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, None);
    assert!(matches!(
        parse_args(["--lowmem", "--highmem"], None),
        Err((2, message)) if message.contains("conflict")
    ));
}

#[test]
fn parse_args_rejects_malformed_session_nav_paint_choice() {
    assert!(matches!(
        parse_args(["--nav-paints", "maybe"], None),
        Err((2, message)) if message == "panel-play: --nav-paints expects on or off, got maybe"
    ));
    assert!(matches!(
        parse_args(["--nav-paints"], None),
        Err((2, message)) if message == "panel-play: --nav-paints needs on or off"
    ));
}

#[test]
fn parse_live_args_env_and_flag_null_raster() {
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("null_raster")),
        Ok(RunMode::Live("null_raster".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "null_raster"], None),
        Ok(RunMode::Live("null_raster".into()))
    );
}

#[test]
fn parse_live_args_env_and_flag_stress50() {
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("stress50")),
        Ok(RunMode::Live("stress50".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "stress50"], None),
        Ok(RunMode::Live("stress50".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "stress50_full"], None),
        Ok(RunMode::Live("stress50_full".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("stress50_full")),
        Ok(RunMode::Live("stress50_full".into()))
    );
}

#[test]
fn parse_live_args_accepts_nav_full() {
    assert_eq!(
        parse_live_args(["--live", "nav_full"], None),
        Ok(RunMode::Live("nav_full".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("nav_full")),
        Ok(RunMode::Live("nav_full".into()))
    );
}

#[test]
fn parse_live_args_flag_wins_over_env() {
    assert_eq!(
        parse_live_args(["--live", "null_raster"], Some("other")),
        Ok(RunMode::Live("null_raster".into()))
    );
}

#[test]
fn parse_live_args_smoke_flag_maps_to_smoke_mode() {
    assert_eq!(parse_live_args(["--smoke"], None), Ok(RunMode::Smoke));
    // A stray BOT_LIVE env does not demote --smoke.
    assert_eq!(
        parse_live_args(["--smoke"], Some("stress50")),
        Ok(RunMode::Smoke)
    );
    // --smoke wins over --live when both are passed.
    assert_eq!(
        parse_live_args(["--smoke", "--live", "null_raster"], None),
        Ok(RunMode::Smoke)
    );
}

#[test]
fn parse_live_args_unknown_name_is_usage_exit_2() {
    assert_eq!(
        parse_live_args(["--live", "nope"], None),
        Err((2, LIVE_USAGE.into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("other")),
        Err((2, LIVE_USAGE.into()))
    );
}

#[test]
fn parse_live_args_prod_is_interactive_not_unknown() {
    assert_eq!(parse_live_args(["--prod"], None), Ok(RunMode::Interactive));
    assert_eq!(
        parse_live_args(["--prod", "--live", "script_bone_burier"], None),
        Ok(RunMode::Live("script_bone_burier".into()))
    );
}

#[test]
fn parse_live_args_unknown_flag_and_missing_name() {
    assert_eq!(
        parse_live_args(["--wat"], None),
        Err((2, "panel-play: unknown --wat".into()))
    );
    assert_eq!(
        parse_live_args(["--live"], None),
        Err((2, "panel-play: --live needs a name".into()))
    );
}

#[test]
fn parse_live_args_help_is_usage_exit_0() {
    assert_eq!(
        parse_live_args(["--help"], None),
        Err((0, LIVE_USAGE.into()))
    );
    assert_eq!(parse_live_args(["-h"], None), Err((0, LIVE_USAGE.into())));
}

#[test]
fn parse_live_args_script_walk_accepted() {
    assert_eq!(
        parse_live_args(["--live", "script_walk"], None),
        Ok(RunMode::Live("script_walk".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("script_walk")),
        Ok(RunMode::Live("script_walk".into()))
    );
    // The smoke scenario is a registered scenario, so the script_
    // harness can also drive it manually.
    assert_eq!(
        parse_live_args(["--live", "script_render_smoke"], None),
        Ok(RunMode::Live("script_render_smoke".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_nav_routes"], None),
        Ok(RunMode::Live("script_nav_routes".into()))
    );
    // The courtyard paint-path scenario is a registered scenario, so
    // the script_ harness drives it like script_nav_routes.
    assert_eq!(
        parse_live_args(["--live", "script_nav_paint_path"], None),
        Ok(RunMode::Live("script_nav_paint_path".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier_v2_js"], None),
        Ok(RunMode::Live("script_bone_burier_v2_js".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier_v2_ts"], None),
        Ok(RunMode::Live("script_bone_burier_v2_ts".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier"], None),
        Ok(RunMode::Live("script_bone_burier".into()))
    );
}

#[test]
fn parse_live_args_unknown_script_rejected() {
    assert_eq!(
        parse_live_args(["--live", "script_nope"], None),
        Err((2, LIVE_USAGE.into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_"], None),
        Err((2, LIVE_USAGE.into()))
    );
}

#[test]
fn parse_args_prepare_fixture_is_offline_mode() {
    let parsed = parse_args(["--prepare-fixture", "thiever"], None).unwrap();
    assert_eq!(parsed.mode, RunMode::PrepareFixture("thiever".into()));
    assert!(!parsed.run_prepared);
}

#[test]
fn parse_args_run_prepared_requires_script_live() {
    let parsed = parse_args(
        [
            "--live",
            "script_thiever",
            "--run-prepared",
            "--fixture-path",
            "/tmp/t.json",
        ],
        None,
    )
    .unwrap();
    assert_eq!(parsed.mode, RunMode::Live("script_thiever".into()));
    assert!(parsed.run_prepared);
    assert_eq!(
        parsed.fixture_path.as_deref(),
        Some(std::path::Path::new("/tmp/t.json"))
    );
    assert!(parse_args(["--run-prepared"], None).is_err());
    assert!(parse_args(
        ["--prepare-fixture", "thiever", "--live", "script_thiever"],
        None
    )
    .is_err());
    assert!(parse_args(["--prepare-fixture", "thiever", "--run-prepared"], None).is_err());
    assert!(parse_args(["--prepare-fixture", "nope"], None).is_err());
    let ts = parse_args(["--prepare-fixture", "bone_burier_v2_ts"], None).unwrap();
    assert_eq!(ts.mode, RunMode::PrepareFixture("bone_burier_v2_ts".into()));
    let js = parse_args(["--prepare-fixture", "bone_burier_v2_js"], None).unwrap();
    assert_eq!(js.mode, RunMode::PrepareFixture("bone_burier_v2_js".into()));
    assert_eq!(
        scenario::fixture_preset_for("bone_burier_v2_ts").unwrap(),
        "bone_burier_v2"
    );
    assert_eq!(scenario::fixture_preset_for("thiever").unwrap(), "thiever");
    assert!(scenario::fixture_preset_for("bone_burier").is_err());
}

#[test]
fn parse_live_args_accepts_external_loader_without_scenario_catalog() {
    assert_eq!(
        parse_live_args(["--live", "script_external_loader"], None),
        Ok(RunMode::Live("script_external_loader".into()))
    );
    assert!(scenario::get("external_loader").is_none());
}

#[test]
fn parse_args_external_ts_refuses_relative_and_keeps_ordinary_watchers_off() {
    let err = parse_args(["--external-ts", "ExampleBot.ts"], None).unwrap_err();
    assert_eq!(err.0, 2);
    assert!(err.1.contains("relative"), "{}", err.1);
    let args = parse_args(["--live", "script_bone_burier"], None).unwrap();
    assert!(!args.external_core);
    assert!(args.external_ts.is_none());
    assert!(!args.catalog_core);
    assert!(!args.pair_core);
}

#[test]
fn pair_watch_cli_cases_resolve_two_prepared_actors_and_shared_start() {
    use host_play::paired_core::{
        AirObservation, FlaxObservation, PairCase, PairWatch, StartBarrier, AIR_RUINS, FLAX_FIELD,
        FLAX_MEET, MULE_TRADE_CAP, TRADE_CAP,
    };
    use scenario::StepKind;

    for (cli, case, card, mode_a, mode_b) in [
        (
            "script_nature_crafter_air",
            PairCase::Air,
            "NatureCrafter",
            "Master",
            "Runner",
        ),
        (
            "script_mule_crafter_air",
            PairCase::Mule,
            "MuleCrafter",
            "Crafter",
            "Mule",
        ),
        (
            "script_flax_runner",
            PairCase::Flax,
            "FlaxRunner",
            "Runner",
            "Spinner",
        ),
    ] {
        assert_eq!(
            parse_live_args(["--live", cli], None),
            Ok(RunMode::Live(cli.into())),
            "{cli} must resolve as a headed pair_watch live name"
        );
        let name = cli.strip_prefix("script_").unwrap();
        assert_eq!(PairCase::parse(name).unwrap(), case);
        let scenario = scenario::get(name).unwrap();
        assert_eq!(scenario.seed.profiles.len(), 2);
        assert_ne!(scenario.seed.profiles[0].0, scenario.seed.profiles[1].0);
        assert_eq!(scenario.settings.start_script, Some(card));
        assert_eq!(scenario.companions.len(), 1);
        assert_eq!(scenario.companions[0].profile, 1);
        assert!(scenario
            .steps
            .iter()
            .any(|step| matches!(step.kind, StepKind::StartScript)));

        let a = "alice";
        let b = "bob";
        let bag_a = host_play::paired_core::pair_settings(case, &[], 0, a, b).unwrap();
        let bag_b = host_play::paired_core::pair_settings(case, &[], 1, b, a).unwrap();
        assert_eq!(bag_a.get("mode").and_then(|v| v.as_str()), Some(mode_a));
        assert_eq!(bag_b.get("mode").and_then(|v| v.as_str()), Some(mode_b));
        let screen_a = client::util::JString::to_screen_name(a);
        let screen_b = client::util::JString::to_screen_name(b);
        assert_eq!(
            bag_a.get("partner").and_then(|v| v.as_str()),
            Some(screen_b.as_str())
        );
        assert_eq!(
            bag_b.get("partner").and_then(|v| v.as_str()),
            Some(screen_a.as_str())
        );

        let watch = PairWatch::default();
        watch.configure(case, a, b);
        match case {
            PairCase::Air | PairCase::Mule => {
                let first = AirObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(a.into()),
                    tile: Some(AIR_RUINS),
                    air_talisman: 1,
                    essence_unnoted: if case == PairCase::Mule {
                        MULE_TRADE_CAP
                    } else {
                        0
                    },
                    ..AirObservation::default()
                };
                let second = AirObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(b.into()),
                    tile: Some(AIR_RUINS),
                    air_talisman: 0,
                    essence_unnoted: if case == PairCase::Mule {
                        MULE_TRADE_CAP
                    } else {
                        TRADE_CAP
                    },
                    ..AirObservation::default()
                };
                watch.observe_air(a, first, false);
                watch.observe_air(b, second, false);
                assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
                watch.begin_shared_start(a, b).unwrap();
            }
            PairCase::Flax => {
                let runner = FlaxObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(a.into()),
                    tile: Some(FLAX_FIELD),
                    crafting: 1,
                    ..FlaxObservation::default()
                };
                let mut spinner = FlaxObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(b.into()),
                    tile: Some(FLAX_MEET),
                    crafting: 1,
                    ..FlaxObservation::default()
                };
                watch.observe_flax(a, runner, false);
                watch.observe_flax(b, spinner.clone(), false);
                assert_eq!(
                    watch.barrier(),
                    StartBarrier::Wait,
                    "Crafting 1 cannot spin flax"
                );
                spinner.crafting = 10;
                watch.observe_flax(b, spinner, false);
                assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
                watch.begin_shared_start(a, b).unwrap();
            }
            PairCase::Duel => unreachable!("this table is Air/Mule/Flax role bags"),
        }
    }

    let cli = "script_duel_arena";
    assert_eq!(
        parse_live_args(["--live", cli], None),
        Ok(RunMode::Live(cli.into())),
        "{cli} must resolve as a headed pair_watch live name"
    );
    assert_eq!(PairCase::parse("duel_arena").unwrap(), PairCase::Duel);
    let scenario = scenario::get("duel_arena").unwrap();
    assert_eq!(scenario.seed.profiles.len(), 2);
    assert_ne!(scenario.seed.profiles[0].0, scenario.seed.profiles[1].0);
    assert_eq!(
        scenario.settings.start_script,
        Some(PairCase::Duel.card_name())
    );
    assert_eq!(scenario.companions.len(), 1);
    assert!(scenario
        .steps
        .iter()
        .any(|step| matches!(step.kind, StepKind::StartScript)));
    let bag_a =
        host_play::paired_core::pair_settings(PairCase::Duel, &[], 0, "alice", "bob").unwrap();
    let bag_b =
        host_play::paired_core::pair_settings(PairCase::Duel, &[], 1, "bob", "alice").unwrap();
    assert!(bag_a.get("partner").is_none());
    assert!(bag_b.get("partner").is_none());
    let watch = PairWatch::default();
    watch.configure(PairCase::Duel, "alice", "bob");
    let first = host_play::paired_core::DuelObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some("alice".into()),
        tile: Some(host_play::paired_core::DUEL_CHALLENGE_ANCHOR),
        tick: 0,
        attack_xp: 0,
        strength_xp: 0,
        defence_xp: 0,
        hitpoints_xp: 0,
        in_combat: false,
        in_challenge_area: true,
        in_fight_pen: false,
        main_modal: -1,
        duel_offer_open: false,
        duel_confirm_open: false,
        duel_win_open: false,
        duel_partner: None,
        waiting_for_other: false,
        weapon_equipped: true,
        peer_visible: true,
    };
    let second = host_play::paired_core::DuelObservation {
        player: Some("bob".into()),
        ..first.clone()
    };
    watch.observe_duel("alice", first, false);
    watch.observe_duel("bob", second, false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
    watch.begin_shared_start("alice", "bob").unwrap();
}

#[test]
fn smoke_should_fire_table() {
    // Fires exactly once, only while armed, only at `ingame && scene 2`.
    let cases: &[(&str, bool, bool, bool, i32, bool)] = &[
        ("disarmed never fires", false, false, true, 2, false),
        ("fires at scene 2", true, false, true, 2, true),
        ("not before scene 2", true, false, true, 1, false),
        ("not while logged out", true, false, false, 2, false),
        ("not after scene 1", true, false, true, 0, false),
        ("not twice", true, true, true, 2, false),
        ("not after exit", true, true, true, 1, false),
    ];
    for (name, armed, fired, ingame, scene, expect) in cases {
        assert_eq!(
            smoke_should_fire(*armed, *fired, *ingame, *scene),
            *expect,
            "{name}"
        );
    }
}

#[test]
fn smoke_settled_table() {
    // The scene2 shot request reaches the render readback only once
    // the focused slot has held scene 2 for the full settle window and
    // is still ingame (the 1 fps renderer needs wall-clock time to
    // rasterize the world; an early capture is the title screen).
    let t0 = Instant::now();
    let cases: &[(&str, Option<Instant>, Instant, bool, bool)] = &[
        ("no scene 2 yet", None, t0, true, false),
        (
            "before the settle window",
            Some(t0),
            t0 + Duration::from_secs(1),
            true,
            false,
        ),
        (
            "at the settle boundary",
            Some(t0),
            t0 + SMOKE_SETTLE,
            true,
            true,
        ),
        (
            "after the settle window",
            Some(t0),
            t0 + Duration::from_secs(5),
            true,
            true,
        ),
        (
            "settled but logged out",
            Some(t0),
            t0 + Duration::from_secs(5),
            false,
            false,
        ),
    ];
    for (name, at, now, ingame, expect) in cases {
        assert_eq!(smoke_settled(*at, *now, *ingame), *expect, "{name}");
    }
}

#[test]
fn manual_shot_label_is_stamped_and_stays_normalized() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_616_000);
    let label = manual_shot_label(now);
    assert_eq!(label, "manual-2026-08-25T00-00-00");
    // The label is already in the 377 safe alphabet: the file name
    // normalizes to itself (the write path applies `safe_label`).
    assert_eq!(scenario::shot::safe_label(&label), label);
}

#[test]
fn manual_shot_without_a_focused_live_snapshot_is_not_enqueued() {
    let mut state = PanelState::default();
    super::enqueue_manual_shot(&mut state);
    assert!(state.shot_state.lock().unwrap().requests.is_empty());
}

#[test]
fn manual_shot_rejects_an_empty_published_snapshot() {
    let mut state = PanelState::default();
    state.session.focus.lock().unwrap().focused = Some("alice".into());
    state.session.nav_states.lock().unwrap().insert(
        "alice".into(),
        (
            api::snapshot::GameSnapshot::new(),
            nav::WorldState::default(),
        ),
    );
    super::enqueue_manual_shot(&mut state);
    assert!(state.shot_state.lock().unwrap().requests.is_empty());
}

/// A synthetic client that has already seeded: ingame, scene 2, a
/// mainland build base, and bumped family gens (same trick as the
/// scenario crate's own tests — no live server).
fn script_client() -> client::client::Client {
    let mut c = host::prepare_client(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        1,
        std::sync::Arc::new(client::config::Cache::default()),
        std::sync::Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(client::dash3d::ClientPlayer::at(20, 20));
    for prot in [
        client::io::ServerProt::PLAYER_INFO,
        client::io::ServerProt::REBUILD_NORMAL,
        client::io::ServerProt::UPDATE_STAT,
    ] {
        c.bump_gens(prot);
    }
    c
}

#[test]
fn live_script_tick_holds_pass_until_clean_script_stop() {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};

    let mut s = crate::session::Session::new();
    let pass = Scenario {
        name: "bone_stop",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 5 },
                budget_ticks: 5,
            },
        }],
        proof: Proof::Stat { id: 16, min: 5 },
        companions: vec![],
        settings: ScenarioSettings {
            wait_script_stop: Some("confirmed loaded current-generation bank exhaustion"),
            ..Default::default()
        },
    };
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = script_client();
    runner.tick(&mut c);
    c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
    runner.tick(&mut c);
    assert_eq!(runner.status(), scenario::RunnerStatus::Passed);
    assert_eq!(
        runner.wait_script_stop(),
        Some("confirmed loaded current-generation bank exhaustion")
    );
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None
    );
    assert!(!live.passed, "game-state PASS must wait for Idle + reason");
    s.live_script_stop_wait_started = Some(Instant::now() - Duration::from_secs(60));
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("clean-stop wait times out");
    assert!(error.contains("clean stop reason"), "{error}");
    assert!(live.failed.is_some());
}

#[test]
fn clean_stop_rearms_written_shot_once_and_can_complete() {
    let session = crate::session::Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    let client = script_client();
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    assert!(snapshot.ingame() && snapshot.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let label = "bone-stop";
    shots.lock().unwrap().mark_written(label);
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: Some(Instant::now() - NAV_FULL_SHOT_DRAIN),
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };

    request_clean_stop_capture(&mut live, &session, Some(&shots), Some(label))
        .expect("scene-2 snapshot can re-arm the original Written shot");
    assert!(
        live.clean_stop_capture_requested,
        "clean-stop capture latches after the original Written shot"
    );
    assert!(
        live.drain_started.is_none(),
        "re-arm resets only the capture drain window"
    );
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Requested,
        "the original Written label must be re-armed exactly once"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Written,
            Some(&shots),
        ),
        Ok(true),
        "the stale pre-Idle Written status must not complete before the new shot"
    );

    shots.lock().unwrap().mark_written(label);
    request_clean_stop_capture(&mut live, &session, Some(&shots), Some(label))
        .expect("latched clean-stop capture must not re-arm");
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Written,
        "a later clean-stop tick must not clear Written"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Requested,
            Some(&shots),
        ),
        Ok(false),
        "PASS can complete after the clean-stop capture writes"
    );
    assert!(live.failed.is_none());
}

#[test]
fn clean_stop_missing_scene_cannot_reuse_prior_written_capture() {
    let session = crate::session::Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    let mut client = script_client();
    client.scene_state = 1;
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("bone-stop");
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    let error = request_clean_stop_capture(&mut live, &session, Some(&shots), Some("bone-stop"))
        .expect_err("missing scene-2 must fail closed");
    assert!(error.contains("scene-2"), "{error}");
    assert!(live.clean_stop_capture_requested);
    assert!(matches!(
        shots.lock().unwrap().status("bone-stop"),
        ShotStatus::Failed(ref failed) if failed.contains("scene-2")
    ));
    assert!(shots.lock().unwrap().requests.is_empty());
}

/// Headed contract: PASS latches `passed` (the caller exits 0),
/// FAIL returns the message the caller turns into exit 1.
#[test]
fn live_script_tick_latches_pass_reports_soak_readbacks_and_fail() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };

    let mut s = crate::session::Session::new();
    // A runnable micro-scenario: the send sets run energy, the arm
    // waits for it, the proof asserts it.
    let pass = Scenario {
        name: "t",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 5 },
                budget_ticks: 5,
            },
        }],
        proof: Proof::Stat { id: 16, min: 5 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), RunnerStatus::Passed);
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None,
        "PASS latches; the caller exits 0"
    );
    assert!(live.passed);

    // Dedicated catalog_watch: scenario PASS alone waits for the shared
    // core, and an unqualified core becomes FAIL at its own deadline.
    let watch = host_play::catalog_core::CoreWatch::default();
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    s.install_catalog_core_watch(Some(watch.clone()));
    live.passed = false;
    live.announced_pass = false;
    live.core_deadline = Some(Instant::now() + Duration::from_secs(60));
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None
    );
    assert!(!live.passed, "scenario-only PASS is not catalog core PASS");
    live.core_deadline = Some(Instant::now() - Duration::from_secs(1));
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("unqualified core times out as FAIL");
    assert!(error.contains("catalog core did not qualify"), "{error}");

    live.failed = None;
    live.native_failure_capture_requested = false;
    live.core_deadline = Some(Instant::now() + Duration::from_secs(60));
    s.scenario
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_terminal_shot("core-pass");
    s.focus.lock().unwrap().focused = Some("catalogtest".into());
    let mut terminal_snapshot = api::snapshot::GameSnapshot::new();
    terminal_snapshot.rebuild(&script_client());
    s.nav_states.lock().unwrap().insert(
        "catalogtest".into(),
        (terminal_snapshot, nav::WorldState::default()),
    );
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("core-pass");
    let mut baseline = host_play::catalog_core::Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..host_play::catalog_core::Observation::default()
    };
    baseline.levels.insert("thieving".into(), 50);
    baseline.levels.insert("hitpoints".into(), 50);
    baseline.effective_levels.insert("thieving".into(), 50);
    baseline.effective_levels.insert("hitpoints".into(), 50);
    baseline.items.insert("Lobster".into(), 10);
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    baseline.xp.insert("thieving".into(), 1);
    baseline.items.insert("Coins".into(), 1);
    watch.observe("catalogtest", baseline, false);
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert!(!live.passed, "qualified core waits for its current capture");
    assert_eq!(
        shots.lock().unwrap().status("core-pass"),
        ShotStatus::Requested,
        "the scenario's earlier XP capture cannot discharge the terminal core proof"
    );
    shots.lock().unwrap().mark_written("core-pass");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, Some(&shots),),
        None
    );
    assert!(
        live.passed,
        "full shared core and its capture permit headed PASS"
    );
    watch.clear();

    live.passed = false;
    live.announced_pass = false;
    live.soak = true;
    live.soak_until = Some(Instant::now() + Duration::from_secs(60));
    live.soak_capture = SoakCapture::NotNeeded;
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None,
        "BUDGET_S soak prints PASS but does not latch exit"
    );
    assert!(!live.passed, "window stays open after proof PASS");
    assert!(live.announced_pass);
    assert!(matches!(
        live.soak_capture,
        SoakCapture::PostPass { ref label, .. } if label == "core-pass-postpass"
    ));
    assert_eq!(
        shots.lock().unwrap().status("core-pass-postpass"),
        ShotStatus::Requested,
        "PASS must enqueue a fresh post-pass checkpoint"
    );
    shots.lock().unwrap().mark_written("core-pass-postpass");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert_eq!(live.soak_capture, SoakCapture::WaitingForFinal);
    live.soak_until = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert!(matches!(
        live.soak_capture,
        SoakCapture::Final { ref label, .. } if label == "core-pass-soak-final"
    ));
    assert!(!live.passed, "final readback must precede exit 0");
    assert_eq!(
        shots.lock().unwrap().status("core-pass-soak-final"),
        ShotStatus::Requested,
        "deadline must enqueue a distinct final readback"
    );
    shots.lock().unwrap().mark_written("core-pass-soak-final");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert_eq!(live.soak_capture, SoakCapture::Complete);
    assert!(live.passed, "exit 0 only after the final readback writes");

    // A failed post-pass readback is a real harness failure, not a
    // successful soak with missing evidence.
    s.scenario
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_terminal_shot("fail-soak");
    shots.lock().unwrap().mark_written("fail-soak");
    let mut failed_soak = LiveScript {
        name: "script_fail_soak".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: true,
        soak_until: Some(Instant::now() + Duration::from_secs(60)),
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut failed_soak, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    shots.lock().unwrap().fail_labels(
        &["fail-soak-postpass".to_string()],
        "synthetic readback failure",
    );
    let error = live_script_tick(&mut failed_soak, &mut s, &ShotStatus::Written, Some(&shots))
        .expect("failed post-pass readback must fail the soak");
    assert!(error.contains("soak checkpoint") && error.contains("synthetic"));
    assert!(failed_soak.failed.is_some());

    // FAIL: a never-satisfiable arm within a 1-tick budget.
    let fail_scenario = Scenario {
        name: "f",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "never",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 999 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 999 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(fail_scenario);
    runner.set_scene_settle(Duration::ZERO);
    {
        let mut c = script_client();
        runner.tick(&mut c);
    }
    assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_f".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    // No terminal shot armed: the FAIL returns immediately.
    let msg = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("FAIL returns the message");
    assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
    assert!(live.failed.is_some());
}

/// A terminal-shot FAIL holds the exit until the shot writes (or the
/// drain lapses): the first frame returns `None` (the request landed
/// after `pump_shots` ran), the write frame returns the message.
#[test]
fn live_script_tick_holds_a_terminal_shot_fail_until_the_shot_writes() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };
    let mut s = crate::session::Session::new();
    let fail_scenario = Scenario {
        name: "f",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "never",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 999 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 999 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(fail_scenario);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot("t-fail");
    {
        let mut c = script_client();
        runner.tick(&mut c);
    }
    assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "nav_full".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None),
        None,
        "the first FAIL frame holds so the shot can land"
    );
    assert!(live.drain_started.is_some());
    // The shot writes on a later frame: the FAIL is returned.
    let msg = live_script_tick(&mut live, &mut s, &ShotStatus::Written, None)
        .expect("FAIL after the shot writes");
    assert!(live.failed.is_some());
    assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
}

/// A terminal-shot PASS holds the exit until the shot writes, then
/// latches `passed` so the caller exits 0.
#[test]
fn live_script_tick_holds_a_terminal_shot_pass_until_the_shot_writes() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };
    let mut s = crate::session::Session::new();
    let pass = Scenario {
        name: "t",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 5 },
                budget_ticks: 5,
            },
        }],
        proof: Proof::Stat { id: 16, min: 5 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot("t-pass");
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), RunnerStatus::Passed);
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None),
        None,
        "the first PASS frame holds so the shot can land"
    );
    assert!(!live.passed);
    assert!(live.drain_started.is_some());
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, None),
        None
    );
    assert!(live.passed, "PASS after the shot writes; caller exits 0");

    live.passed = false;
    live.failed = None;
    live.announced_pass = false;
    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None)
        .expect("PASS with a missing terminal shot must fail after the drain bound");
    assert!(error.contains("terminal shot"), "error: {error}");
    assert!(error.contains("not written"), "error: {error}");
    assert!(live.failed.is_some(), "the missing capture latches failure");
}

#[test]
fn terminal_shot_drain_accepts_a_write_from_an_earlier_core_pending_frame() {
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        super::hold_terminal_shot(
            &mut live,
            Some("t-pass"),
            &crate::window::ShotStatus::Written,
        ),
        Ok(false)
    );
    assert!(live.drain_started.is_none());
}

#[test]
fn native_failure_rearms_written_shot_and_waits_for_current_capture() {
    let session = crate::session::Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    let client = script_client();
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    assert!(snapshot.ingame() && snapshot.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let label = "thiever";
    shots.lock().unwrap().mark_written(label);
    let mut live = LiveScript {
        name: "script_thiever".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };

    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    request_native_failure_capture(&mut live, &session, Some(&shots), Some(label));
    assert!(live.native_failure_capture_requested);
    assert!(live.drain_started.is_none(), "re-arm resets the old drain");
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Requested,
        "a prior Written capture must be re-armed"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Written,
            Some(&shots),
        ),
        Ok(true),
        "the stale pre-tick Written status must not exit before the new shot"
    );
    assert!(live.drain_started.is_some());

    shots.lock().unwrap().mark_written(label);
    request_native_failure_capture(&mut live, &session, Some(&shots), Some(label));
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Written,
        "a later failure tick must not re-arm the current capture"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Requested,
            Some(&shots),
        ),
        Ok(false),
        "exit is released only after the current capture is written"
    );
}

#[test]
fn native_failure_missing_scene_cannot_reuse_prior_written_capture() {
    let session = crate::session::Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    let mut client = script_client();
    client.scene_state = 1;
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("thiever");
    super::enqueue_current_terminal_shot(&session, &shots, "thiever");
    assert!(matches!(
        shots.lock().unwrap().status("thiever"),
        ShotStatus::Failed(error) if error.contains("scene-2")
    ));
    assert!(shots.lock().unwrap().requests.is_empty());
}

#[test]
fn native_failure_receipt_uses_inner_scenario_identity() {
    let evidence = scenario::Evidence {
        scenario: "thiever".into(),
        outcome: "PASS",
        predicate: "stat(16)>=0".into(),
        ticks: 1,
        elapsed_ms: 2,
        message: None,
        tile: None,
        inv: Vec::new(),
        stat: None,
        chat: Vec::new(),
        scene: 2,
    };
    assert_eq!(
        script_failure_scenario("script_thiever", Some(&evidence)),
        "thiever"
    );
    assert_eq!(script_failure_scenario("script_thiever", None), "thiever");
}

#[test]
fn pair_terminal_decision_requests_scene2_snapshots_for_both_actors() {
    use std::collections::HashSet;

    let mut s = crate::session::Session::new();
    let scenario = scenario::get("nature_crafter_air").expect("paired cell");
    let mut runner = scenario::ScenarioRunner::new(scenario);
    runner.set_live_names(&["alice".into(), "bob".into()]);
    *s.scenario.lock().unwrap() = Some(runner);

    let watch = host_play::paired_core::PairWatch::default();
    watch.configure(host_play::paired_core::PairCase::Air, "alice", "bob");
    s.install_paired_core_watch(Some(watch));

    let mut alice_client = script_client();
    let mut alice_player = client::dash3d::ClientPlayer::at(20, 20);
    alice_player.name = Some("NatureMaster".into());
    alice_client.local_player = Some(alice_player);
    let mut snap_a = api::snapshot::GameSnapshot::new();
    snap_a.rebuild(&alice_client);

    let mut bob_client = script_client();
    bob_client.local_player = None;
    let mut snap_b = api::snapshot::GameSnapshot::new();
    snap_b.rebuild(&bob_client);

    assert!(snap_a.ingame() && snap_a.scene_state() == 2);
    assert!(snap_b.ingame() && snap_b.scene_state() == 2);
    s.nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snap_a, nav::WorldState::default()));
    s.nav_states
        .lock()
        .unwrap()
        .insert("bob".into(), (snap_b, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let mut live = LiveScript {
        name: "script_nature_crafter_air".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: Some(Instant::now() - Duration::from_secs(1)),
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, Some(&shots)),
        None,
        "pair terminal FAIL holds until both actor shots are requested"
    );
    let guard = shots.lock().unwrap();
    assert_eq!(guard.requests.len(), 2, "both actors must be captured");
    let mut actors = HashSet::new();
    let mut payloads = HashSet::new();
    for request in &guard.requests {
        let label = &request.label;
        let json = &request.snapshot_json;
        let value: serde_json::Value = serde_json::from_str(json).expect("snapshot json");
        assert_eq!(value.get("ingame"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(value.get("scene_state"), Some(&serde_json::json!(2)));
        let actor = value
            .get("actor")
            .and_then(|v| v.as_str())
            .expect("actor label metadata is the profile identity");
        assert_eq!(
            request.actor.as_deref(),
            Some(actor),
            "queued capture must be bound to the selected actor, not a label clone"
        );
        assert!(
            label.contains(actor),
            "label {label} must name actor {actor}"
        );
        match actor {
            "alice" => {
                assert_eq!(
                    value.pointer("/player/player/actor/name"),
                    Some(&serde_json::json!("NatureMaster")),
                    "observed player name must be retained"
                );
                assert!(
                    value
                        .get("player")
                        .and_then(|player| player.get("name"))
                        .is_none(),
                    "actor metadata must not overwrite or synthesize player.name"
                );
            }
            "bob" => {
                assert_eq!(
                    value.get("player"),
                    Some(&serde_json::Value::Null),
                    "missing-player stays missing"
                );
            }
            other => panic!("unexpected actor {other}"),
        }
        actors.insert(actor.to_string());
        payloads.insert(json.clone());
    }
    assert_eq!(
        actors.len(),
        2,
        "paired headed snapshots must identify two distinct actors"
    );
    assert_eq!(
        payloads.len(),
        2,
        "paired headed snapshots must retain distinct observed payloads"
    );
}

fn dummy_slot() -> crate::session::SlotIo {
    crate::session::SlotIo {
        input: host::SlotInput::new(),
        pixels: host::FrameBuf::new(),
    }
}

fn insert_named_scene2(session: &crate::session::Session, name: &str, player: Option<&str>) {
    let mut client = script_client();
    match player {
        Some(player_name) => {
            let mut player = client::dash3d::ClientPlayer::at(20, 20);
            player.name = Some(player_name.into());
            client.local_player = Some(player);
        }
        None => client.local_player = None,
    }
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.ingame() && snap.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert(name.into(), (snap, nav::WorldState::default()));
}

fn sidecar_actor_and_scene(json: &str) -> (String, i64) {
    let value: serde_json::Value = serde_json::from_str(json).expect("sidecar json");
    (
        value
            .get("actor")
            .and_then(|v| v.as_str())
            .expect("actor")
            .to_string(),
        value
            .get("scene_state")
            .and_then(|v| v.as_i64())
            .expect("scene_state"),
    )
}

#[test]
fn pump_shots_does_not_promote_two_pair_actors_from_an_unready_buffer() {
    let mut state = PanelState::default();
    state.session.slots.insert("alice".into(), dummy_slot());
    state.session.slots.insert("bob".into(), dummy_slot());
    state.session.focus.lock().unwrap().focused = Some("alice".into());
    insert_named_scene2(&state.session, "alice", Some("NatureMaster"));
    insert_named_scene2(&state.session, "bob", None);
    {
        let mut shots = state.shot_state.lock().unwrap();
        shots.enqueue_for_actor(
            "air-alice".into(),
            "{\"actor\":\"alice\"}".into(),
            "alice".into(),
        );
        shots.enqueue_for_actor("air-bob".into(), "{\"actor\":\"bob\"}".into(), "bob".into());
    }

    assert_eq!(super::pump_shots(&mut state), 0);
    {
        let shots = state.shot_state.lock().unwrap();
        assert!(
            shots.wanted.is_empty(),
            "unready selected buffer must not capture"
        );
        assert_eq!(shots.requests.len(), 2);
        assert_eq!(shots.status("air-alice"), ShotStatus::Requested);
        assert_eq!(shots.status("air-bob"), ShotStatus::Requested);
    }

    state.last_upload = Some(("alice".into(), 1));
    assert_eq!(super::pump_shots(&mut state), 0);
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "air-alice");
        assert_eq!(
            sidecar_actor_and_scene(&shots.wanted[0].1),
            ("alice".into(), 2),
            "promoted sidecar must be the current scene2 snapshot, not the enqueue-time clone"
        );
        assert_eq!(shots.requests.len(), 1);
        assert_eq!(shots.requests[0].actor.as_deref(), Some("bob"));
    }

    state.shot_state.lock().unwrap().wanted.clear();
    state.shot_state.lock().unwrap().mark_written("air-alice");
    state.last_upload = Some(("alice".into(), 1));
    super::pump_shots(&mut state);
    assert_eq!(
        state.session.focused_name().as_deref(),
        Some("bob"),
        "next queued actor is selected only after the previous capture writes"
    );
    assert!(
        state.shot_state.lock().unwrap().wanted.is_empty(),
        "bob is focused but not yet presented"
    );

    state.last_upload = Some(("bob".into(), 2));
    super::pump_shots(&mut state);
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "air-bob");
        assert_eq!(
            sidecar_actor_and_scene(&shots.wanted[0].1),
            ("bob".into(), 2)
        );
        assert!(shots.requests.is_empty());
    }
}

#[test]
fn pump_shots_fails_a_missing_pair_actor_without_deadlocking() {
    let mut state = PanelState::default();
    state.shot_state.lock().unwrap().enqueue_for_actor(
        "air-ghost".into(),
        "{\"actor\":\"ghost\"}".into(),
        "ghost".into(),
    );
    assert_eq!(super::pump_shots(&mut state), 0);
    let shots = state.shot_state.lock().unwrap();
    match shots.status("air-ghost") {
        ShotStatus::Failed(error) => {
            assert!(
                error.contains("not a live slot"),
                "missing actor must fail closed: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(shots.requests.is_empty());
    assert!(shots.wanted.is_empty());
}

#[test]
fn pump_shots_does_not_promote_a_presented_actor_that_left_scene2() {
    let mut state = PanelState::default();
    state.session.slots.insert("alice".into(), dummy_slot());
    state.session.focus.lock().unwrap().focused = Some("alice".into());
    state.last_upload = Some(("alice".into(), 1));
    state.session.nav_states.lock().unwrap().insert(
        "alice".into(),
        (
            api::snapshot::GameSnapshot::new(),
            nav::WorldState::default(),
        ),
    );
    state.shot_state.lock().unwrap().enqueue_for_actor(
        "air-alice".into(),
        "{\"actor\":\"alice\",\"ingame\":true,\"scene_state\":2}".into(),
        "alice".into(),
    );
    assert_eq!(super::pump_shots(&mut state), 0);
    let shots = state.shot_state.lock().unwrap();
    assert!(
        shots.wanted.is_empty(),
        "stale scene2 sidecar must not prove a currently offworld actor"
    );
    assert_eq!(shots.status("air-alice"), ShotStatus::Requested);
    assert_eq!(
        shots.requests[0].snapshot_json,
        "{\"actor\":\"alice\",\"ingame\":true,\"scene_state\":2}"
    );
}

fn actor_snapshot_serialization_count() -> u64 {
    super::ACTOR_SNAPSHOT_SERIALIZATIONS.with(|count| count.get())
}

#[test]
fn pump_shots_does_not_serialize_sidecar_without_a_pending_actor_capture() {
    let mut state = PanelState::default();
    state.session.slots.insert("alice".into(), dummy_slot());
    state.session.focus.lock().unwrap().focused = Some("alice".into());
    insert_named_scene2(&state.session, "alice", Some("NatureMaster"));
    state.last_upload = Some(("alice".into(), 1));

    let before = actor_snapshot_serialization_count();
    assert_eq!(super::pump_shots(&mut state), 0);
    assert_eq!(
        actor_snapshot_serialization_count(),
        before,
        "ordinary focused scene2 must not serialize a pair sidecar with no actor-tagged request"
    );
    assert!(state.shot_state.lock().unwrap().wanted.is_empty());
    assert!(state.shot_state.lock().unwrap().requests.is_empty());

    state
        .shot_state
        .lock()
        .unwrap()
        .enqueue("manual".into(), "{\"untagged\":true}".into());
    let before = actor_snapshot_serialization_count();
    assert_eq!(super::pump_shots(&mut state), 0);
    assert_eq!(
        actor_snapshot_serialization_count(),
        before,
        "untagged promote must not require pair sidecar serialization"
    );
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "manual");
        assert_eq!(shots.wanted[0].1, "{\"untagged\":true}");
        assert!(shots.requests.is_empty());
    }
}

#[test]
fn pair_deadline_fails_remaining_queued_actors() {
    let s = crate::session::Session::new();
    let scenario = scenario::get("nature_crafter_air").expect("paired cell");
    let mut runner = scenario::ScenarioRunner::new(scenario);
    runner.set_live_names(&["alice".into(), "bob".into()]);
    *s.scenario.lock().unwrap() = Some(runner);
    let watch = host_play::paired_core::PairWatch::default();
    watch.configure(host_play::paired_core::PairCase::Air, "alice", "bob");
    s.install_paired_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().enqueue_for_actor(
        "nature_crafter_air-alice".into(),
        "{}".into(),
        "alice".into(),
    );
    shots.lock().unwrap().enqueue_for_actor(
        "nature_crafter_air-bob".into(),
        "{}".into(),
        "bob".into(),
    );
    let mut live = LiveScript {
        name: "script_nature_crafter_air".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: Some(Instant::now() - NAV_FULL_SHOT_DRAIN),
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    let error = super::hold_script_terminal_shot(
        &mut live,
        &s,
        Some("nature_crafter_air"),
        &ShotStatus::Missing,
        Some(&shots),
    )
    .expect_err("drain lapse is FAIL");
    assert!(error.contains("not written"), "{error}");
    let guard = shots.lock().unwrap();
    assert!(matches!(
        guard.status("nature_crafter_air-alice"),
        ShotStatus::Failed(_)
    ));
    assert!(matches!(
        guard.status("nature_crafter_air-bob"),
        ShotStatus::Failed(_)
    ));
    assert!(guard.requests.is_empty());
    assert!(guard.wanted.is_empty());
}

#[test]
fn record_presented_upload_ignores_a_failed_take_after_focus_switch() {
    let mut last = Some(("alice".into(), 3));
    super::record_presented_upload(&mut last, "bob".into(), 4, false);
    assert_eq!(last, Some(("alice".into(), 3)));
    super::record_presented_upload(&mut last, "bob".into(), 4, true);
    assert_eq!(last, Some(("bob".into(), 4)));
}

fn passed_prereq_runner() -> scenario::ScenarioRunner {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
    let pass = Scenario {
        name: "t",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 5 },
                budget_ticks: 5,
            },
        }],
        proof: Proof::Stat { id: 16, min: 5 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot(host_play::external_loader::PREREQ_SHOT);
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), scenario::RunnerStatus::Passed);
    runner
}

fn drive_external_watch_to_capture(watch: &host_play::external_loader::ExternalWatch) {
    use host_play::external_loader::{BONES_COUNT, FROZEN_SHA256, NOTHING_CHANGED, SCRIPT_NAME};
    use std::path::Path;
    let t0 = Instant::now();
    watch.configure(
        "alice",
        PathBuf::from("/tmp/ExampleBot.ts"),
        FROZEN_SHA256.into(),
    );
    watch.note_scene(true, 2);
    watch.note_inventory(t0, "alice", BONES_COUNT, 0);
    watch.note_prereq_passed();
    watch.note_load(
        1,
        SCRIPT_NAME,
        Path::new("/tmp/ExampleBot.ts"),
        "file:/tmp/ExampleBot.ts",
        "compiled-a",
        true,
        false,
    );
    watch.begin_start(t0).unwrap();
    let lines: Vec<String> = (1..=10)
        .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
        .collect();
    watch.note_logs(t0, "alice", &lines);
    watch.note_inventory(t0, "alice", 12, 45);
    let stop = t0 + Duration::from_millis(8);
    watch.request_stop(stop);
    watch.note_logs(
        stop,
        "alice",
        &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
    );
    watch.note_stop(stop + Duration::from_millis(40), true, false);
    watch.note_reload_unchanged(NOTHING_CHANGED);
    watch.note_reload_changed(
        1,
        true,
        false,
        Path::new("/tmp/ExampleBot.ts"),
        "file:/tmp/ExampleBot.ts",
        "sha-before",
        "sha-after",
        "compiled-a",
        "compiled-b",
        true,
        false,
    );
}

fn alice_scene2_session() -> crate::session::Session {
    let s = crate::session::Session::new();
    *s.scenario.lock().unwrap() = Some(passed_prereq_runner());
    let mut client = script_client();
    let mut player = client::dash3d::ClientPlayer::at(20, 20);
    player.name = Some("Alice".into());
    client.local_player = Some(player);
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.ingame() && snap.scene_state() == 2);
    s.nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snap, nav::WorldState::default()));
    s
}

#[test]
fn earlier_fixture_written_cannot_discharge_external_terminal_hold() {
    let mut s = alice_scene2_session();
    let watch = host_play::external_loader::ExternalWatch::default();
    drive_external_watch_to_capture(&watch);
    s.install_external_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::PREREQ_SHOT);

    let mut live = LiveScript {
        name: "script_external_loader".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None,
        "prereq Written must not latch PASS before the post-run shot"
    );
    assert!(!live.passed);
    assert_eq!(
        shots
            .lock()
            .unwrap()
            .status(host_play::external_loader::TERMINAL_SHOT),
        ShotStatus::Requested
    );

    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::TERMINAL_SHOT);
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None
    );
    assert!(
        live.passed,
        "PASS only after the external terminal shot writes"
    );
}

#[test]
fn missing_external_terminal_shot_fails_after_drain() {
    let mut s = alice_scene2_session();
    let watch = host_play::external_loader::ExternalWatch::default();
    drive_external_watch_to_capture(&watch);
    s.install_external_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::PREREQ_SHOT);

    let mut live = LiveScript {
        name: "script_external_loader".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None
    );
    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots))
        .expect("missing external shot is FAIL");
    assert!(error.contains("external_loader terminal"), "{error}");
    assert!(error.contains("not written"), "{error}");
    assert!(live.failed.is_some());
    assert!(!live.passed);
}

#[test]
fn pump_shots_marks_completion_only_after_the_png_and_snapshot_pair_write() {
    let dir = std::env::temp_dir().join(format!(
        "274bot-panel-shot-pump-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut state = PanelState {
        shot_dir: Some(dir.clone()),
        ..PanelState::default()
    };
    state
        .shot_state
        .lock()
        .unwrap()
        .done
        .push(crate::window::ShotCapture {
            label: "gnome_chop".into(),
            snapshot_json: "{\"scene\":2}".into(),
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            #[cfg(feature = "render-diagnostics")]
            pixel_roi: None,
        });

    assert_eq!(super::pump_shots(&mut state), 1);
    assert_eq!(
        state.shot_state.lock().unwrap().status("gnome_chop"),
        ShotStatus::Written
    );
    let mut extensions = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    extensions.sort();
    assert_eq!(extensions, ["json", "png"]);
    std::fs::remove_dir_all(dir).unwrap();
}

fn smoke_at(started: Instant) -> LiveSmoke {
    LiveSmoke {
        started,
        last_step: None,
        failed: None,
        saw_scene2_at: None,
        passed: false,
    }
}

#[test]
fn live_smoke_tick_passes_when_the_shot_is_written() {
    let mut s = crate::session::Session::new();
    let mut live = smoke_at(Instant::now());
    let statuses = [st("test", true, 2)];
    // A written shot (pump_shots drained `done`) exits the smoke: the
    // tick latches passed and the caller turns it into exit 0.
    assert_eq!(live_smoke_tick(&mut live, &mut s, &statuses, 1), None);
    assert!(live.passed, "the written scene2 shot passes the smoke");
}

#[test]
fn live_smoke_tick_latches_scene2_once_and_reports_a_missing_write() {
    let mut s = crate::session::Session::new();
    s.focus.lock().unwrap().focused = Some("test".into());
    let mut live = smoke_at(Instant::now());
    // Before scene 2 nothing latches.
    assert_eq!(
        live_smoke_tick(&mut live, &mut s, &[st("test", true, 1)], 0),
        None
    );
    assert!(!live.saw_scene2_at.is_some());
    // Scene 2 on the focused slot latches exactly once (the pure
    // trigger), and a late deadline names the missing write.
    let scene2 = [st("test", true, 2)];
    assert_eq!(live_smoke_tick(&mut live, &mut s, &scene2, 0), None);
    assert!(live.saw_scene2_at.is_some());
    live.started = Instant::now() - SMOKE_DEADLINE;
    let err = live_smoke_tick(&mut live, &mut s, &scene2, 0).expect("deadline");
    assert!(err.contains("never written within 300s"), "err: {err}");
}

#[test]
fn live_smoke_tick_deadline_reports_scene2_never_reached() {
    let mut s = crate::session::Session::new();
    s.focus.lock().unwrap().focused = Some("test".into());
    let mut live = smoke_at(Instant::now() - SMOKE_DEADLINE);
    let err = live_smoke_tick(&mut live, &mut s, &[st("test", false, 0)], 0).expect("deadline");
    assert!(
        err.contains("never reached scene 2 within 300s"),
        "err: {err}"
    );
    assert!(live.failed.is_some(), "the deadline latches the failure");
}

fn st(name: &str, ingame: bool, scene: i32) -> host_play::SlotStatus {
    host_play::SlotStatus {
        username: name.into(),
        ingame,
        scene_state: scene,
        ..Default::default()
    }
}

#[test]
fn latched_title_banner_does_not_promise_automatic_connect() {
    let status = host_play::SlotStatus {
        username: "alice".into(),
        startup_phase: host_play::StartupPhase::Queueing,
        login_latched: true,
        ..Default::default()
    };
    let (message, show_elapsed) = slot_startup_banner_line(&status).expect("latched banner");
    assert_eq!(message, "Logged out — select Log in to reconnect");
    assert!(!show_elapsed);
}

#[test]
fn unlatched_queue_banner_keeps_position_and_elapsed() {
    let status = host_play::SlotStatus {
        username: "alice".into(),
        startup_phase: host_play::StartupPhase::Queueing,
        queue_position: 2,
        queue_total: 5,
        login_latched: false,
        ..Default::default()
    };
    let (message, show_elapsed) = slot_startup_banner_line(&status).expect("queue banner");
    assert!(message.contains("2/5"));
    assert!(show_elapsed);
}

#[test]
fn login_rearm_clears_latched_display_for_connect_wait() {
    let status = host_play::SlotStatus {
        username: "alice".into(),
        startup_phase: host_play::StartupPhase::Queueing,
        login_latched: false,
        ..Default::default()
    };
    let (message, show_elapsed) = slot_startup_banner_line(&status).expect("connect banner");
    assert_eq!(message, "Waiting to connect");
    assert!(show_elapsed);
}

#[test]
fn preparing_startup_banner_keeps_elapsed_timer() {
    let status = host_play::SlotStatus {
        username: "alice".into(),
        startup_phase: host_play::StartupPhase::Preparing,
        login_latched: false,
        ..Default::default()
    };
    let (message, show_elapsed) = slot_startup_banner_line(&status).expect("preparing banner");
    assert_eq!(message, "Preparing client");
    assert!(show_elapsed);
}

#[test]
fn stale_latched_flag_still_overrides_connect_wait_banner() {
    let status = host_play::SlotStatus {
        username: "alice".into(),
        startup_phase: host_play::StartupPhase::Queueing,
        login_latched: true,
        ..Default::default()
    };
    let (message, show_elapsed) = slot_startup_banner_line(&status).expect("latched overrides");
    assert_eq!(message, "Logged out — select Log in to reconnect");
    assert!(!show_elapsed);
}

fn live_at(started: Instant) -> LiveNull {
    LiveNull {
        started,
        saw_scene2: false,
        passed: false,
    }
}

#[test]
fn live_null_tick_waits_until_two_scene2() {
    let mut live = live_at(Instant::now());
    let statuses = [st("test", true, 1), st("test2", false, 0)];
    assert_eq!(live_null_tick(&mut live, &statuses), None);
    assert!(!live.saw_scene2);
}

#[test]
fn live_null_tick_timeout_before_scene2() {
    let mut live = live_at(Instant::now() - Duration::from_secs(120));
    let statuses = [st("test", true, 2)];
    let err = live_null_tick(&mut live, &statuses).expect("timeout");
    assert!(err.contains("1/2"), "{err}");
    assert!(err.contains("120s"), "{err}");
}

#[test]
fn live_null_tick_passes_at_scene2_without_freeze() {
    let mut live = live_at(Instant::now());
    let scene2 = [st("test", true, 2), st("test2", true, 2)];
    assert_eq!(live_null_tick(&mut live, &scene2), None);
    assert!(live.passed);
    assert!(live.saw_scene2);
    assert_eq!(live_null_tick(&mut live, &scene2), None, "stay passed");
}

fn stress_at(started: Instant) -> LiveStress {
    LiveStress {
        started,
        last_announced: 0,
        passed: false,
        name: "stress50",
        host: "127.0.0.1".into(),
        port: 43594,
    }
}

fn ready_n(n: usize) -> Vec<host_play::SlotStatus> {
    (0..n).map(|i| st(&format!("s{i:02}"), true, 2)).collect()
}

#[test]
fn live_stress_tick_announces_1_10_50_and_stays_passed() {
    let mut live = stress_at(Instant::now());
    assert_eq!(live_stress_tick(&mut live, &[]), None);
    assert_eq!(live.last_announced, 0);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(1)), None);
    assert_eq!(live.last_announced, 1);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(10)), None);
    assert_eq!(live.last_announced, 10);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(50)), None);
    assert_eq!(live.last_announced, 50);
    assert!(live.passed);
    assert_eq!(
        live_stress_tick(&mut live, &ready_n(50)),
        None,
        "stay passed"
    );
    assert!(live.passed);
}

#[test]
fn live_stress_tick_timeout_before_50() {
    let mut live = stress_at(Instant::now() - Duration::from_secs(600));
    let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
    assert_eq!(err, "live stress50: 1/50 up after 600s");
    assert!(!live.passed);
    assert_eq!(live.last_announced, 1);
}

#[test]
fn live_stress_tick_full_name_in_timeout() {
    let mut live = LiveStress {
        started: Instant::now() - Duration::from_secs(600),
        last_announced: 0,
        passed: false,
        name: "stress50_full",
        host: "127.0.0.1".into(),
        port: 43594,
    };
    let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
    assert_eq!(err, "live stress50_full: 1/50 up after 600s");
}

#[test]
fn live_stress_tick_counts_full_clients_up() {
    let mut live = stress_at(Instant::now());
    // Every member is a full Client: "up" requires scene 2, so loading
    // slots do not count toward the 50.
    let mut rows = vec![st("s00", true, 2)];
    for i in 1..50 {
        rows.push(st(&format!("s{i:02}"), true, 1));
    }
    assert_eq!(live_stress_tick(&mut live, &rows), None);
    assert!(!live.passed, "49 loading Clients are not 50 up");
    assert_eq!(live.last_announced, 1);
    for r in rows.iter_mut() {
        r.scene_state = 2;
    }
    assert_eq!(live_stress_tick(&mut live, &rows), None);
    assert!(live.passed, "50 scene-2 Clients pass");
    assert_eq!(live.last_announced, 50);
}

#[test]
fn random_status_text_names_kind_hold_and_off() {
    let r = host::RandomStatus {
        kind: Some(api::RandomKind::Dialog),
        name: Some("mysterious old man".into()),
        toggle: true,
        hold: true,
        ..Default::default()
    };
    assert_eq!(
        random_status_text(&r).as_deref(),
        Some("dialog: mysterious old man (hold)")
    );
    let r = host::RandomStatus {
        kind: Some(api::RandomKind::Dialog),
        name: Some("mysterious old man".into()),
        toggle: false,
        ..Default::default()
    };
    assert_eq!(
        random_status_text(&r).as_deref(),
        Some("dialog: mysterious old man (off)")
    );
    let r = host::RandomStatus {
        kind: Some(api::RandomKind::Lamp),
        name: Some("genie".into()),
        toggle: true,
        ..Default::default()
    };
    assert_eq!(random_status_text(&r).as_deref(), Some("lamp: genie"));
    let r = host::RandomStatus::default();
    assert_eq!(random_status_text(&r), None, "no event, no row");
}

#[test]
fn random_status_text_kebab_cases_lost_kinds() {
    let r = host::RandomStatus {
        kind: Some(api::RandomKind::LostTool),
        toggle: true,
        ..Default::default()
    };
    assert_eq!(random_status_text(&r).as_deref(), Some("lost-tool: ?"));
    let r = host::RandomStatus {
        kind: Some(api::RandomKind::LostGear),
        toggle: true,
        ..Default::default()
    };
    assert_eq!(random_status_text(&r).as_deref(), Some("lost-gear: ?"));
}

#[test]
fn config_section_scopes_accent_to_header_not_body() {
    const SRC: &str = include_str!("app.rs");
    let fn_src = SRC.split("fn config_section").nth(1).unwrap_or("");
    let fn_src = fn_src
        .split("fn global_capture_section")
        .next()
        .unwrap_or("");
    assert!(
        fn_src.contains("FrameBorderSize(1.0)"),
        "header orange border needs a visible frame border"
    );
    let after_header = fn_src.split("ui.collapsing_header").nth(1).unwrap_or("");
    assert!(
        !after_header.contains("push_style_color(StyleColor::Text, ACCENT)"),
        "accent text must not wrap the open section body"
    );
    assert!(
        fn_src.contains("body(ui, session)"),
        "section body runs outside header style scope"
    );
}

#[test]
fn panel_heading_toggle_hides_status_section() {
    use crate::ui_state::{panel_section_visible, set_panel_section_visible, PanelUiState};
    let mut ui = PanelUiState::default();
    assert!(panel_section_visible(&ui, "status"));
    set_panel_section_visible(&mut ui, "status", false);
    assert!(!panel_section_visible(&ui, "status"));
}

#[test]
fn parameters_and_script_prefs_share_show_parameters_rail() {
    use crate::ui_state::{panel_section_visible, set_panel_section_visible, PanelUiState};
    let mut ui = PanelUiState::default();
    assert!(!ui.show_parameters_rail);
    set_panel_section_visible(&mut ui, "parameters", true);
    assert!(ui.show_parameters_rail);
    assert!(panel_section_visible(&ui, "parameters"));
    ui.show_parameters_rail = false;
    assert!(!panel_section_visible(&ui, "parameters"));
}

#[test]
fn global_config_slot_name_then_capture_then_focused_50_then_panel() {
    const SRC: &str = include_str!("app.rs");
    let g = SRC.split("fn global_config_section").nth(1).unwrap_or("");
    let g = g.split("\nfn ").next().unwrap_or("");
    let slot = g.find("\"Slot:\"").expect("Slot: label above capture");
    let capture = g
        .find("global_capture_section")
        .expect("capture stays in Global");
    let focused_50 = g
        .find("focused 50 fps")
        .expect("focused 50 fps lives in Global");
    let panel = g
        .find("panel_heading_toggles")
        .expect("Panel heading toggles stay in Global");
    assert!(
        slot < capture,
        "Slot: sits below the heading, above capture"
    );
    assert!(
        focused_50 < panel,
        "focused 50 fps sits above the Panel subsection"
    );
    assert!(
        !g.contains("auto-login"),
        "auto-login belongs on the profile editor, not Global"
    );
}

#[test]
fn settings_window_drops_slot_and_random_rows() {
    const SRC: &str = include_str!("app.rs");
    let settings = SRC.split("fn settings_window").nth(1).unwrap_or("");
    let settings = settings.split("\nfn ").next().unwrap_or("");
    assert!(
        settings.contains("config_section(ui, session, \"Global\""),
        "Global row stays"
    );
    assert!(
        settings.contains("config_section(ui, session, \"render\""),
        "render (raster/mem) stays in General config"
    );
    assert!(
        !settings.contains("config_section(ui, session, \"slot\""),
        "slot row is gone; the focused name is a Global label"
    );
    assert!(
        !settings.contains("config_section(ui, session, \"random\""),
        "random/lamp belong on the profile editor"
    );
    assert!(
        !settings.contains("slot_capture_section"),
        "auto-login is not a General config control"
    );
    assert!(
        !settings.contains("slot_random_section"),
        "guardian toggles are not a General config control"
    );
}

#[test]
fn slot_render_section_no_longer_owns_focused_50() {
    const SRC: &str = include_str!("app.rs");
    let r = SRC.split("fn slot_render_section").nth(1).unwrap_or("");
    let r = r.split("\nfn ").next().unwrap_or("");
    assert!(
        !r.contains("focused 50 fps"),
        "focused 50 fps moved to Global, above Panel"
    );
    assert!(
        r.contains("raster_picker"),
        "Game-pane raster/mem stay under render"
    );
}

#[test]
fn chooser_edit_hosts_per_profile_login_and_random() {
    const SRC: &str = include_str!("app.rs");
    let chooser = SRC.split("fn chooser_window").nth(1).unwrap_or("");
    let chooser = chooser.split("fn settings_window").next().unwrap_or("");
    assert!(
        chooser.contains("slot_capture_section"),
        "auto-login moved onto the profile editor"
    );
    assert!(
        chooser.contains("slot_random_section"),
        "random/lamp moved onto the profile editor"
    );
    const SRC_COPY: &str = include_str!("app.rs");
    let random = SRC_COPY
        .split("fn slot_random_section")
        .nth(1)
        .unwrap_or("");
    assert!(
        random.contains("this profile"),
        "copy names the edited profile, not a global slot"
    );
    assert!(
        !random.contains("focus a profile to edit"),
        "edit form is already on this profile"
    );
    let save = chooser
        .find("button_with_size(\"Save\"")
        .expect("Save stays on the editor");
    let auto = chooser
        .find("slot_capture_section")
        .expect("auto-login in editor");
    assert!(auto < save, "per-profile settings sit above Save/Cancel");
}

#[test]
fn chooser_locked_vault_shows_unlock_not_empty_copy() {
    const SRC: &str = include_str!("app.rs");
    let chooser = SRC.split("fn chooser_window").nth(1).unwrap_or("");
    let chooser = chooser.split("fn settings_window").next().unwrap_or("");
    let locked = chooser
        .find("vault.is_none()")
        .expect("Profiles must branch on a locked vault");
    let unlock = chooser
        .find("vault_unlock_prompt")
        .expect("locked Profiles reuses the panel unlock UI");
    let empty = chooser
        .find("vault is empty")
        .expect("empty copy stays for a truly empty unlocked vault");
    assert!(
        locked < unlock && unlock < empty,
        "unlock UI while locked; empty copy only after the vault is open"
    );
    let profile = SRC.split("fn profile_section").nth(1).unwrap_or("");
    let profile = profile.split("\nfn ").next().unwrap_or("");
    assert!(
        profile.contains("vault_unlock_prompt"),
        "panel profile heading and Profiles share one unlock prompt"
    );
    assert!(
        !profile.contains("##vault-pass"),
        "pass field lives in the shared prompt, not forked in profile_section"
    );
}

#[test]
fn panel_subsection_exposes_chrome_color_pickers() {
    const SRC: &str = include_str!("app.rs");
    let panel = SRC.split("fn panel_heading_toggles").nth(1).unwrap_or("");
    let panel = panel.split("\nfn ").next().unwrap_or("");
    assert!(
        panel.contains("chrome_color_field") || panel.contains("nav_color_field"),
        "Panel chrome colours use the same hex picker pattern as Nav"
    );
    assert!(
        panel.contains("accent") || panel.contains("ACCENT"),
        "named theme consts are exposed as pickers"
    );
}

/// File-loaded cards have empty description/tags. A trailing
/// SetCursorScreenPos after the badge used to EndChild past CursorMaxPos
/// and abort ErrorCheckUsingSetCursorPosToExtendParentBoundaries
/// (panel-play SIGABRT on Browse, window `##scard-File-trade_bot`).
#[test]
fn browse_file_card_without_desc_does_not_assert_on_endchild() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    let iso = script::IsolatedEnv::enter("browse-scard-assert");
    let path = iso.dir.join("trade_bot.js");
    std::fs::write(
        &path,
        "export default class TradeBot extends LoopingBot { loop() {} }\n",
    )
    .unwrap();
    let mut s = crate::session::Session::new();
    s.js = script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.load_js(&path);
    assert_eq!(s.error, None, "load: {:?}", s.error);
    assert_eq!(
        s.js.cards()[0].description,
        "",
        "File cards have no registry description — this is the abort path"
    );
    s.script_browse_open = true;
    let mut ctx = dear_imgui_rs::Context::create();
    // AUTO_RESIZE_Y measures on frame 1 and applies on frame 2.
    for _ in 0..3 {
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            super::browse_window(ui, &mut s);
        }
        ctx.render();
    }
}
