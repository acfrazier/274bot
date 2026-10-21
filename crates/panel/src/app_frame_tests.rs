//! The production panel frame path. A frame is `ui_frame`: the pump phase
//! (`frame_pumps`, after the input-staging reset), the drawing, then the
//! staged GameInput submit. These
//! tests run frames and never call `Session::pump_status` themselves, so a
//! frame that stops pumping the session (the round-3 regression: the call at
//! the top of the frame was replaced and no unit test noticed) fails here.
//!
//! Every scenario runs under two drivers. `UiFrame` calls the real
//! `ui_frame` over a headless wgpu device (skipped, as the other GPU tests
//! are, when the host has no adapter). `Pumps` needs no GPU: it runs the
//! pump phase and the staged submit that `ui_frame` calls, in the same
//! order, with the operator's pointer staged between them.

use std::ffi::CString;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dear_imgui_rs::{sys, ConfigFlags, Context, FramePrepareOptions, MouseButton};
use dear_imgui_wgpu as imgui_wgpu;
use frontend_core::command::Subject;
use frontend_core::scripts::{CompiledCardId, ScriptChoice, ScriptRunState};
use frontend_core::surface::{FrameBuf, InputEv, InputProbe, InputSender, SlotInput};
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::GameInputEvent;
use host_play::{SlotArm, SlotStatus};

use super::{frame_pumps, game_window_title, ui_frame, LiveHarness, LiveNull, PanelState};
use crate::session::{Session, SlotIo};
use crate::test_support::TestDir;
use crate::window::Gpu;
use frontend_core::{OperatorCommand, Targets};

const DISPLAY: [f32; 2] = [1440.0, 900.0];

/// Hang guard for a frame loop that waits on a worker thread. The pass
/// condition is always the product's own signal; this only bounds a true
/// hang.
const HANG_GUARD: Duration = Duration::from_secs(60);

/// How a scenario drives its frames.
#[derive(Clone, Copy, Debug)]
enum Drive {
    /// The real `ui_frame`.
    UiFrame,
    /// The input reset, `frame_pumps`, the operator's capture, the staged
    /// GameInput submit: `ui_frame`'s order without its drawing.
    Pumps,
}

/// Where the operator's pointer is for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pointer {
    /// Away from the Game image.
    Away,
    /// Over the Game image.
    Over,
    /// Left button pressed over the Game image.
    Click,
}

struct GpuRig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: imgui_wgpu::WgpuRenderer,
}

/// What one frame drew, from ImGui's own text log.
#[derive(Clone, Default)]
struct DrawnText(std::rc::Rc<std::cell::RefCell<String>>);

impl dear_imgui_rs::ClipboardBackend for DrawnText {
    fn get(&mut self) -> Option<String> {
        None
    }

    fn set(&mut self, value: &str) {
        value.clone_into(&mut self.0.borrow_mut());
    }
}

/// The panel state, an ImGui context and, for `UiFrame`, a headless GPU.
/// Field order is the drop order: the state and the renderer go before the
/// context, the context before the global ImGui test lock.
struct Rig {
    state: PanelState,
    gpu: Option<GpuRig>,
    live_failure: Mutex<Option<String>>,
    drawn: DrawnText,
    /// What the last `UiFrame` frame drew.
    text: String,
    /// Centre of the Game window as of the last `UiFrame` frame.
    game_centre: Option<[f32; 2]>,
    ctx: Context,
    _imgui: MutexGuard<'static, ()>,
}

impl Rig {
    /// `None` when `drive` needs a GPU the host does not have.
    fn new(drive: Drive, session: Session) -> Option<Self> {
        let gpu_parts = match drive {
            Drive::UiFrame => Some(crate::test_support::headless_gpu()?),
            Drive::Pumps => None,
        };
        let imgui = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let drawn = DrawnText::default();
        ctx.set_clipboard_backend(drawn.clone());
        let gpu = gpu_parts.map(|(device, queue)| {
            ctx.io_mut()
                .set_backend_flags(dear_imgui_rs::BackendFlags::RENDERER_HAS_TEXTURES);
            assert!(crate::window::add_panel_font(&mut ctx));
            let _ = ctx.font_atlas_mut().build();
            super::apply_ui_scale(ctx.style_mut(), 1.0);
            ctx.io_mut().set_config_flags(ConfigFlags::DOCKING_ENABLE);
            let renderer = imgui_wgpu::WgpuRenderer::new(
                imgui_wgpu::WgpuInitInfo::new(
                    device.clone(),
                    queue.clone(),
                    wgpu::TextureFormat::Bgra8UnormSrgb,
                ),
                &mut ctx,
            )
            .expect("WgpuRenderer init on the headless device");
            GpuRig {
                device,
                queue,
                renderer,
            }
        });
        Some(Self {
            state: PanelState::with_session(
                session,
                Arc::new(Mutex::new(crate::window::ShotState::default())),
            ),
            gpu,
            live_failure: Mutex::new(None),
            drawn,
            text: String::new(),
            game_centre: None,
            ctx,
            _imgui: imgui,
        })
    }

    fn session(&mut self) -> &mut Session {
        &mut self.state.session
    }

    /// One frame. Returns whether the frame ended the run.
    fn frame(&mut self, pointer: Pointer) -> bool {
        // The pointer: over the Game window's centre (inside its image) or
        // off in the corner. A click is a button press on that frame and a
        // release on the next.
        let spot = match pointer {
            Pointer::Away => [2.0, 2.0],
            Pointer::Over | Pointer::Click => self.game_centre.unwrap_or([480.0, 450.0]),
        };
        self.frame_at(spot, pointer == Pointer::Click, pointer)
    }

    /// One frame with the pointer at `spot` and the left button `down`.
    /// `pointer` is what the `Pumps` driver stages for the Game image.
    fn frame_at(&mut self, spot: [f32; 2], down: bool, pointer: Pointer) -> bool {
        let io = self.ctx.io_mut();
        io.add_mouse_pos_event(spot);
        io.add_mouse_button_event(MouseButton::Left, down);
        self.ctx
            .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        let title = game_window_title(self.state.session.focused_name().as_deref());
        let (exit, centre) =
            {
                let ui = self.ctx.frame();
                match self.gpu.as_mut() {
                    Some(rig) => {
                        ui.log_to_clipboard(0u32);
                        let mut gpu = Gpu::new(&rig.device, &rig.queue, &mut rig.renderer);
                        let exit =
                            ui_frame(ui, &mut gpu, &mut self.state, None, &self.live_failure);
                        ui.log_finish();
                        (exit, ui.with_bound_context(|| window_centre(&title)))
                    }
                    None => {
                        self.state.session.map_desk.begin_game_input_frame();
                        let exit = frame_pumps(&mut self.state, &self.live_failure);
                        if !exit {
                            // The capture sites run between the pump phase and
                            // the submit: the pointer over the image stages a
                            // Move, a click adds the Down.
                            if pointer != Pointer::Away {
                                self.state.session.map_desk.stage_game_input(
                                    GameInputEvent::Captured(InputEv::Move { x: 10, y: 10 }),
                                );
                            }
                            if pointer == Pointer::Click {
                                self.state.session.map_desk.stage_game_input(
                                    GameInputEvent::Captured(InputEv::Down {
                                        button: 1,
                                        x: 10,
                                        y: 10,
                                    }),
                                );
                            }
                            if self.state.session.map_desk.has_staged_game_input() {
                                let submitted = self
                                    .state
                                    .session
                                    .submit(OperatorCommand::GameInput, Targets::Selected)
                                    .expect("the staged GameInput is submitted");
                                assert!(self
                                    .state
                                    .session
                                    .core
                                    .command_failure(submitted.request())
                                    .is_none());
                            }
                        }
                        (exit, None)
                    }
                }
            };
        self.ctx.render();
        if self.gpu.is_some() {
            self.text = self.drawn.0.take();
            self.game_centre = centre.or(self.game_centre);
        }
        exit
    }

    /// Frames until `done` holds, with the pointer away. Every frame is a
    /// real frame; the guard only bounds a hang. Returns the frame count.
    fn frames_until(&mut self, what: &str, mut done: impl FnMut(&mut Rig) -> bool) -> usize {
        let deadline = Instant::now() + HANG_GUARD;
        let mut frames = 0;
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "no frame settled {what} within {frames} frames"
            );
            self.frame(Pointer::Away);
            frames += 1;
            std::thread::yield_now();
        }
        frames
    }
}

/// Centre of the active window `title` (the Game window).
fn window_centre(title: &str) -> Option<[f32; 2]> {
    let name = CString::new(title).ok()?;
    // SAFETY: called with the frame's context bound; the name is a
    // NUL-terminated copy that outlives the call, and the window is read
    // only after the lookup found it.
    unsafe {
        let window = sys::igFindWindowByName(name.as_ptr());
        if window.is_null() || !(*window).Active {
            return None;
        }
        let (pos, size) = ((*window).Pos, (*window).Size);
        Some([pos.x + size.x / 2.0, pos.y + size.y / 2.0])
    }
}

fn empty_play() -> host_play::Play {
    frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    })
}

fn slot_io() -> SlotIo {
    SlotIo {
        input: SlotInput::new(),
        pixels: FrameBuf::new(),
    }
}

/// A session over a fresh vault holding `names`, with a `Play` and one
/// arm, rail entry and slot io per name, the first focused. The workers are
/// never spawned: tests publish status rows by hand.
fn session_with_play(label: &str, names: &[&str]) -> (Session, TestDir) {
    IsolatedStores::ensure_thread();
    let dir = TestDir::new(label);
    let mut vault = VaultFixture::create(&dir.join("v.vault"), "test-passphrase-01");
    for (i, name) in names.iter().enumerate() {
        vault.put(FixtureProfile::new(name, "pw").uid(1 + i as i32));
    }
    let mut session = Session::new();
    session.core.set_spawn_workers(false);
    install_vault(&mut session.core, vault);
    session.persist_ui = false;
    session
        .scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
    let mut play = empty_play();
    for name in names {
        play.attach_arm(name, SlotArm::new(42, false));
        session.core.fleet_mut().add(name);
        session.core.insert_slot_io(name, slot_io());
    }
    session.core.set_play(Some(play));
    if let Some(first) = names.first() {
        session.set_focus_for_test(first);
    }
    (session, dir)
}

/// Publish `name` as a connected, in-game slot at scene 2, the way the
/// slot thread does: into the host's shared status rows.
fn publish_ingame(session: &Session, name: &str) {
    let mut rows = session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap();
    rows.retain(|row| row.username != name);
    rows.push(SlotStatus {
        username: name.into(),
        connected: true,
        ingame: true,
        scene_state: 2,
        ..SlotStatus::default()
    });
}

fn drain(rx: &InputProbe) -> Vec<frontend_core::surface::InputEv> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// Run `$body(drive)` once per driver, each as its own test.
macro_rules! scenario {
    ($name:ident, $body:expr) => {
        mod $name {
            use super::*;

            #[test]
            fn pumps() {
                ($body)(Drive::Pumps);
            }

            #[test]
            fn ui_frame() {
                ($body)(Drive::UiFrame);
            }
        }
    };
}

/// Property 1: every frame advances the host. A status the slot published
/// is in the session's rows after one frame, with every operator window
/// closed.
fn frame_advances_the_host_status(drive: Drive) {
    let (session, _dir) = session_with_play("frame-host-status", &["alice"]);
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    assert!(
        !rig.session().wall.chooser_open && !rig.session().multibox,
        "the operator windows are closed"
    );
    publish_ingame(rig.session(), "alice");
    assert!(
        rig.session().statuses().is_empty(),
        "nothing polls the host before a frame runs"
    );
    assert!(!rig.frame(Pointer::Away));
    let rows = rig.session().statuses();
    assert!(
        rows.iter()
            .any(|row| row.username == "alice" && row.ingame && row.scene_state == 2),
        "one frame must poll the host and refresh the status rows: {rows:?}"
    );
    if matches!(drive, Drive::UiFrame) {
        assert!(
            rig.text.contains("state ingame scene 2"),
            "the same frame must draw the status it polled:\n{}",
            rig.text
        );
    }
}

scenario!(host_status, frame_advances_the_host_status);

/// A session over a vault holding `alice`, with no `Play`: the locked state.
fn session_without_play(label: &str) -> (Session, TestDir) {
    IsolatedStores::ensure_thread();
    let dir = TestDir::new(label);
    let mut vault = VaultFixture::create(&dir.join("v.vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(1));
    let mut session = Session::new();
    session.core.set_spawn_workers(false);
    install_vault(&mut session.core, vault);
    session.persist_ui = false;
    assert!(!session.core.has_play());
    (session, dir)
}

/// Queue a credentials save of the existing profile `alice` and let the
/// core finish the write. The core has then settled the write, but only a
/// frame's session pump hands the outcome to the form and the banner.
fn save_alice_and_finish_the_write(session: &mut Session) {
    session.begin_edit_profile(Some("alice"));
    *session.profile_editor.draft_mut().username_mut() = "alice".into();
    *session.profile_editor.draft_mut().password_mut() = "changed".into();
    assert!(session.save_credentials(), "the save is accepted");
    session.core.flush_writes();
    assert!(
        session.profile_editor.save_state().notice().is_none() && session.error.is_none(),
        "nothing is shown before a frame settles the save"
    );
}

/// Property 1: every frame settles the durable profile writes, with
/// operator windows closed, with and without a `Play`.
fn frame_settles_a_durable_write(drive: Drive, with_play: bool) {
    let (session, _dir) = if with_play {
        session_with_play("frame-write-play", &["alice"])
    } else {
        session_without_play("frame-write-locked")
    };
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    assert_eq!(rig.session().core.has_play(), with_play);
    save_alice_and_finish_the_write(rig.session());
    assert!(!rig.frame(Pointer::Away));
    let notice = rig.session().profile_editor.save_state().notice();
    assert!(
        notice.is_some_and(frontend_core::FormNotice::is_saved),
        "one frame must settle the durable save onto its form: {notice:?}"
    );
    assert!(!rig.session().form_saving(), "the save settled");
}

scenario!(durable_write_with_play, |drive| {
    frame_settles_a_durable_write(drive, true)
});
scenario!(durable_write_locked, |drive| {
    frame_settles_a_durable_write(drive, false)
});

/// Property 1: a write that failed shows on the banner after one frame,
/// with and without a `Play`.
#[cfg(unix)]
fn frame_surfaces_a_failed_write(drive: Drive, with_play: bool) {
    use std::os::unix::fs::PermissionsExt;

    let (session, dir) = if with_play {
        session_with_play("frame-failed-play", &["alice"])
    } else {
        session_without_play("frame-failed-locked")
    };
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    // The vault's directory refuses the write.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    rig.session().begin_edit_profile(Some("alice"));
    *rig.session().profile_editor.draft_mut().username_mut() = "alice".into();
    *rig.session().profile_editor.draft_mut().password_mut() = "changed".into();
    assert!(rig.session().save_credentials(), "the edit is queued");
    rig.session().core.flush_writes();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        rig.session().error.is_none(),
        "nothing shows before a frame"
    );
    assert!(!rig.frame(Pointer::Away));
    assert!(
        rig.session()
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("credentials:")),
        "one frame must put the failed write on the banner, got {:?}",
        rig.session().error
    );
}

#[cfg(unix)]
scenario!(failed_write_with_play, |drive| {
    frame_surfaces_a_failed_write(drive, true)
});
#[cfg(unix)]
scenario!(failed_write_locked, |drive| {
    frame_surfaces_a_failed_write(drive, false)
});

/// Property 2: removal progress advances in the frame, before the status
/// refresh it feeds. A rail removal waits for the slot to disconnect; the
/// frame after the disconnect stops the lifetime.
fn frame_advances_a_pending_removal(drive: Drive) {
    let (session, _dir) = session_with_play("frame-removal", &["alice"]);
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    let arm = rig
        .session()
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .expect("alice's arm");
    publish_ingame(rig.session(), "alice");
    rig.frame(Pointer::Away);
    rig.session().rail_remove("alice");
    assert!(rig.session().core.removal_pending("alice"));
    assert!(
        !arm.stop.load(std::sync::atomic::Ordering::Relaxed),
        "a connected slot gets a clean logout first"
    );
    rig.session()
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .connected = false;
    rig.frame(Pointer::Away);
    assert!(
        arm.stop.load(std::sync::atomic::Ordering::Relaxed),
        "the frame after the disconnect must stop the removed slot"
    );
    assert!(!rig.session().core.removal_pending("alice"));
}

scenario!(pending_removal, frame_advances_a_pending_removal);

const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

/// Property 2: script preparation, Start, reload validation and Stop settle
/// through frames alone. Every wait is a count of real frames until the
/// product's own state changes.
fn frames_settle_script_start_reload_and_stop(drive: Drive) {
    let (session, dir) = session_with_play("frame-scripts", &["alice"]);
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    let path = dir.join("bot.ts");
    std::fs::write(&path, BOT_TS).unwrap();
    rig.session().load_js(&path);
    rig.session().script_start_selected();
    assert!(
        rig.session().scripts.starts_pending(),
        "Start waits for its preparation"
    );
    rig.frames_until("the script Start", |rig| {
        rig.session().core.script_state("alice") == ScriptRunState::Running
            && !rig.session().scripts.starts_pending()
    });
    assert_eq!(rig.session().error, None);

    rig.session().begin_script_reload_clicked();
    assert!(rig.session().reload_validation_pending());
    rig.frames_until("the reload validation", |rig| {
        !rig.session().reload_validation_pending()
    });
    assert!(
        rig.session().take_reload_outcome().is_some(),
        "the reload publishes its outcome"
    );

    rig.session().script_stop();
    rig.frames_until("the script Stop", |rig| {
        rig.session().core.script_state("alice") == ScriptRunState::Idle
    });
}

scenario!(script_lifecycle, frames_settle_script_start_reload_and_stop);

/// Property 3: a status published to the host is visible to the live
/// decision of the same frame. The Null harness decides from the session's
/// status rows (refreshed by the pump), and the frame ends the run.
fn live_decision_sees_the_status_published_this_frame(drive: Drive) {
    let (mut session, _dir) = session_with_play("frame-live-null", &["test", "test2"]);
    session.core.insert_slot_io("test2", slot_io());
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    rig.state.live = Some(LiveHarness::Null(LiveNull {
        started: Instant::now(),
        saw_scene2: false,
        passed: false,
    }));
    assert!(
        !rig.frame(Pointer::Away),
        "no slot is in game yet: the live harness keeps the run going"
    );
    publish_ingame(rig.session(), "test");
    publish_ingame(rig.session(), "test2");
    assert!(
        rig.frame(Pointer::Away),
        "the frame that follows the publication must show it to the live decision"
    );
    assert!(rig.state.live.as_ref().is_some_and(LiveHarness::exit_pass));
}

scenario!(
    live_decision_sees_status,
    live_decision_sees_the_status_published_this_frame
);

/// The 289 game data a bound profile selects, prepared on the shared
/// worker as a profile bind does.
fn selected_game_data() -> Arc<api::game_data::SelectedGameData> {
    api::selected::FamilyPreparation::run(|_| {
        api::game_data::for_revision(api::selected::ClientRevision::R289)
            .expect("selected 289 game data")
    })
    .expect("spawn selected data worker")
    .join()
    .expect("selected data worker")
}

/// What the Quest Paths notice says while a reload is still running.
const PATHS_RUNNING: [&str; 2] = [
    "Reloading quest Paths…",
    "Reload Paths queued until background capacity is available.",
];

/// Property 2, Quester Paths (T3.3 r5, review finding 9): a Path reload
/// settles through frames alone, and the frame advances it before the
/// Quest Paths section decides what to show. The frame that settles the
/// reload therefore draws its result, never the reload as still running:
/// a frame that stops advancing the reload never settles it, and one that
/// advances it after the drawing draws the running notice.
fn frames_settle_a_quester_paths_reload(drive: Drive) {
    let (mut session, _dir) = session_with_play("frame-quester-paths", &["alice"]);
    session.script_prefs_open = true;
    session.script_sel = Some(ScriptChoice::compiled(CompiledCardId::QUESTER));
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    rig.session()
        .core
        .stage_quester_reload(Some(selected_game_data()));
    let request = rig
        .session()
        .submit(OperatorCommand::ReloadQuesterPaths, Targets::Application)
        .expect("the reload is admitted")
        .request();
    assert!(
        rig.session().quester_paths.is_running(),
        "the reload runs on the shared worker"
    );
    let settled = |rig: &mut Rig| {
        rig.session()
            .core
            .commands()
            .report(request)
            .and_then(|report| report.member(Subject::Application))
            .is_some_and(|member| member.terminal().is_some())
    };
    rig.frames_until("the Path reload", settled);
    assert!(!rig.session().quester_paths.is_running());
    let notice = rig
        .session()
        .quester_paths
        .notice_text()
        .expect("the reload reports its result")
        .to_owned();
    assert!(!PATHS_RUNNING.contains(&notice.as_str()), "{notice}");
    if rig.gpu.is_some() {
        assert!(
            rig.text.contains(&notice),
            "the frame that settled the reload draws its result:\n{}",
            rig.text
        );
        for running in PATHS_RUNNING {
            assert!(
                !rig.text.contains(running),
                "the frame that settled the reload drew it as running:\n{}",
                rig.text
            );
        }
    }
    frontend_core::test_support::scripts::reset_quester_registry();
}

scenario!(quester_paths_reload, frames_settle_a_quester_paths_reload);

/// Property 4: input staging is reset before capture, the captured events
/// are submitted once after collection, and a later empty frame does not
/// replay them; the frame that captures also pumps the host.
fn captured_input_is_submitted_once_per_frame(drive: Drive) {
    let (session, _dir) = session_with_play("frame-input", &["alice"]);
    session.focus.lock().unwrap().capture = true;
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    publish_ingame(rig.session(), "alice");
    for _ in 0..3 {
        rig.frame(Pointer::Away);
    }
    // The Game pane opening attached the slot's own capture channel; take
    // its place with a probe so the test sees what a frame submits.
    let (tx, rx) = InputSender::probe();
    rig.session().map_desk.set_input_sender(Some(tx));
    assert!(drain(&rx).is_empty(), "no capture, no input");

    // A stale staged event, left from an earlier frame, is reset before
    // this frame's capture and never submitted.
    rig.session()
        .map_desk
        .stage_game_input(GameInputEvent::Captured(InputEv::Key {
            down: true,
            ch: 65,
        }));
    rig.frame(Pointer::Away);
    assert!(
        drain(&rx).is_empty(),
        "a stale staged event must not reach the game"
    );
    assert!(!rig.session().map_desk.has_staged_game_input());

    rig.frame(Pointer::Over);
    let hover = drain(&rx);
    assert!(
        matches!(hover.as_slice(), [InputEv::Move { .. }]),
        "the pointer over the image submits one Move, got {hover:?}"
    );
    // The slot publishes a new scene just before the click frame; that
    // frame's pump refreshes the rows it captures against.
    rig.session()
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .scene_state = 3;
    rig.frame(Pointer::Click);
    assert_eq!(
        rig.session().statuses().first().map(|row| row.scene_state),
        Some(3),
        "the capturing frame must pump the host first"
    );
    let click = drain(&rx);
    assert!(
        matches!(
            click.as_slice(),
            [InputEv::Move { .. }, InputEv::Down { button: 1, .. }]
        ),
        "the click submits its Move and Down once, got {click:?}"
    );
    rig.frame(Pointer::Away);
    let later = drain(&rx);
    assert!(later.is_empty(), "a later empty frame replays {later:?}");
}

scenario!(captured_input, captured_input_is_submitted_once_per_frame);

/// The rail caps a frame drew (`name: brief`), in their drawn order.
fn drawn_caps<'a>(text: &str, names: &[&'a str]) -> Vec<&'a str> {
    let mut caps: Vec<(usize, &str)> = names
        .iter()
        .filter_map(|name| text.find(&format!("{name}: ")).map(|at| (at, *name)))
        .collect();
    caps.sort_unstable();
    caps.into_iter().map(|(_, name)| name).collect()
}

/// One fleet model behind every panel surface: the real frame draws the
/// MultiBox rail, the Grid and the Fleet window from the same rows, filter
/// and order. A profile that is not loaded is listed (folded) in the rail,
/// never as a tile; the Fleet window's filter hides the same tiles in the
/// rail and the Grid, and a filtered-out mark survives.
#[test]
fn the_rail_grid_and_fleet_window_draw_one_filtered_order() {
    let (mut session, _dir) = session_with_play("fleet-one-order", &["carol", "alice", "bob"]);
    frontend_core::test_support::session::vault_upsert(
        &mut session.core,
        FixtureProfile::new("dave", "pw").uid(9),
    );
    session.multibox = true;
    session.fleet_open = true;
    let Some(mut rig) = Rig::new(Drive::UiFrame, session) else {
        return;
    };
    let names = ["alice", "bob", "carol", "dave"];
    // The docked rail lays out over the first frames.
    rig.frames_until("the rail caps", |rig| {
        !drawn_caps(&rig.text, &names).is_empty()
    });
    assert_eq!(
        drawn_caps(&rig.text, &names),
        ["alice", "bob", "carol"],
        "load order, and no tile for the profile that is not loaded:\n{}",
        rig.text
    );
    assert!(
        rig.text.contains("Not loaded (1)"),
        "the folded group names the profile count:\n{}",
        rig.text
    );
    assert!(
        rig.text.contains("not loaded"),
        "the Fleet window lists dave as not loaded:\n{}",
        rig.text
    );

    let alice = rig.session().core.profile_identity("alice").unwrap();
    rig.session().fleet_model.set_mark_for_test(alice, true);
    rig.session()
        .fleet_model
        .staged_filter_mut()
        .push_str("bob");
    rig.session()
        .submit(OperatorCommand::SetFleetFilter, Targets::Application);
    rig.frame(Pointer::Away);
    assert_eq!(
        drawn_caps(&rig.text, &names),
        ["bob"],
        "the rail shows the Fleet window's filter:\n{}",
        rig.text
    );
    assert!(rig.session().fleet_model.marks().contains(alice));
    assert_eq!(rig.session().fleet_model.counts().hidden_marked(), 1);

    rig.session().wall.grid = true;
    rig.frame(Pointer::Away);
    rig.frame(Pointer::Away);
    assert_eq!(
        drawn_caps(&rig.text, &names),
        ["bob"],
        "the Grid shows the same filtered cells:\n{}",
        rig.text
    );

    rig.session().fleet_model.staged_filter_mut().clear();
    rig.session()
        .submit(OperatorCommand::SetFleetFilter, Targets::Application);
    rig.session().submit(
        OperatorCommand::SetFleetSort(frontend_core::FleetOrder::Name),
        Targets::Application,
    );
    rig.frame(Pointer::Away);
    assert_eq!(
        drawn_caps(&rig.text, &names),
        ["alice", "bob", "carol"],
        "the Grid follows the Fleet window's sort:\n{}",
        rig.text
    );
}

const REORDER_NAMES: [&str; 3] = ["alice", "bob", "carol"];

fn centre([min_x, min_y, max_x, max_y]: [f32; 4]) -> [f32; 2] {
    [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0]
}

/// The rail cap names of one frame, in drawn order (a `UiFrame` rig with the
/// pointer away). `rail_cap` records five items per cap: world marker, mark
/// box, name, fold, remove.
fn rail_cap_names(rig: &mut Rig) -> Vec<[f32; 4]> {
    super::TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(Vec::clear);
    rig.frame(Pointer::Away);
    super::TEST_RAIL_ITEM_BOUNDS
        .with_borrow(|bounds| bounds.as_chunks::<5>().0.iter().map(|cap| cap[2]).collect())
}

fn saved_names(rig: &mut Rig) -> Vec<String> {
    rig.session().core.profile_order().names().to_vec()
}

/// A rig on the MultiBox rail over `REORDER_NAMES`, settled until every cap
/// is drawn; `None` when the host has no GPU.
fn reorder_rig(label: &str) -> Option<(Rig, TestDir)> {
    let (mut session, dir) = session_with_play(label, &REORDER_NAMES);
    session.multibox = true;
    let mut rig = Rig::new(Drive::UiFrame, session)?;
    rig.frames_until("the rail caps", |rig| {
        drawn_caps(&rig.text, &REORDER_NAMES).len() == REORDER_NAMES.len()
    });
    rig.frame(Pointer::Away);
    Some((rig, dir))
}

/// Press on `from`, move over several frames, release on `to`, settle.
fn rail_drag(rig: &mut Rig, from: [f32; 2], to: [f32; 2]) {
    rig.frame_at(from, false, Pointer::Away);
    rig.frame_at(from, true, Pointer::Away);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        let at = [
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
        ];
        rig.frame_at(at, true, Pointer::Away);
    }
    rig.frame_at(to, true, Pointer::Away);
    rig.frame_at(to, false, Pointer::Away);
    rig.frame_at(to, false, Pointer::Away);
}

/// A real drag of one rail tile's name onto another changes the saved Load
/// order, and the next frames draw the rail and the Grid in the new order.
/// The drag neither focuses the bot nor toggles a mark.
#[test]
fn dragging_a_rail_tile_reorders_the_saved_order_in_the_rail_and_the_grid() {
    let Some((mut rig, _dir)) = reorder_rig("rail-drag") else {
        return;
    };
    let saved = saved_names(&mut rig);
    assert_eq!(
        drawn_caps(&rig.text, &REORDER_NAMES),
        saved,
        "the rail draws the saved order"
    );
    let focused = rig.session().focused_name();
    let caps = rail_cap_names(&mut rig);
    assert_eq!(caps.len(), 3, "one cap per loaded profile");
    let before = crate::session::reorders_submitted();

    // The first tile onto the last: it goes after it.
    rail_drag(&mut rig, centre(caps[0]), centre(caps[2]));
    assert_eq!(crate::session::reorders_submitted(), before + 1);
    let mut expected = saved.clone();
    let first = expected.remove(0);
    expected.push(first);
    assert_eq!(saved_names(&mut rig), expected, "the saved order moved");
    rig.frame(Pointer::Away);
    rig.frame(Pointer::Away);
    assert_eq!(
        drawn_caps(&rig.text, &REORDER_NAMES),
        expected,
        "the rail follows the saved order:\n{}",
        rig.text
    );
    assert_eq!(
        rig.session().focused_name(),
        focused,
        "a drag does not focus the bot"
    );
    assert!(
        rig.session().fleet_model.marks().is_empty(),
        "a drag does not mark"
    );
    assert!(rig.session().error.is_none(), "{:?}", rig.session().error);

    rig.session().wall.grid = true;
    rig.frame(Pointer::Away);
    rig.frame(Pointer::Away);
    assert_eq!(
        drawn_caps(&rig.text, &REORDER_NAMES),
        expected,
        "the Grid follows the saved order:\n{}",
        rig.text
    );
}

/// Dropping a tile on the tile above it puts it before that tile.
#[test]
fn dragging_a_rail_tile_up_lands_before_the_target() {
    let Some((mut rig, _dir)) = reorder_rig("rail-drag-up") else {
        return;
    };
    let saved = saved_names(&mut rig);
    let caps = rail_cap_names(&mut rig);
    rail_drag(&mut rig, centre(caps[2]), centre(caps[0]));
    let mut expected = saved;
    let last = expected.pop().unwrap();
    expected.insert(0, last);
    assert_eq!(saved_names(&mut rig), expected);
}

/// Outside Load order a rail drag does nothing: no reorder is submitted and
/// the saved order stays.
#[test]
fn a_rail_drag_does_nothing_while_the_fleet_is_sorted_by_name() {
    let Some((mut rig, _dir)) = reorder_rig("rail-drag-name") else {
        return;
    };
    rig.session().submit(
        OperatorCommand::SetFleetSort(frontend_core::FleetOrder::Name),
        Targets::Application,
    );
    rig.frame(Pointer::Away);
    let saved = saved_names(&mut rig);
    let caps = rail_cap_names(&mut rig);
    assert_eq!(caps.len(), 3);
    let before = crate::session::reorders_submitted();
    rail_drag(&mut rig, centre(caps[0]), centre(caps[2]));
    assert_eq!(crate::session::reorders_submitted(), before);
    assert_eq!(saved_names(&mut rig), saved);
}

/// A steady frame in Load order, even with the pointer over a tile, submits
/// nothing and rebuilds no label: the cached cap strings keep their buffers.
#[test]
fn a_steady_rail_frame_submits_nothing_and_keeps_its_labels() {
    let Some((mut rig, _dir)) = reorder_rig("rail-steady") else {
        return;
    };
    let caps = rail_cap_names(&mut rig);
    let label_buffers = |rig: &Rig| {
        let mut buffers: Vec<*const u8> = rig
            .state
            .fleet_labels
            .iter()
            .map(|labels| labels.title.as_ptr())
            .collect();
        buffers.push(rig.state.not_loaded_label.as_ptr());
        buffers
    };
    let buffers = label_buffers(&rig);
    let before = crate::session::reorders_submitted();
    for _ in 0..6 {
        rig.frame_at(centre(caps[1]), false, Pointer::Away);
    }
    assert_eq!(crate::session::reorders_submitted(), before);
    assert_eq!(label_buffers(&rig), buffers);
}
