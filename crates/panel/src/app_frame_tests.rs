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
use frontend_core::test_support::paint::TestPaint;
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::GameInputEvent;
use host_play::{SlotArm, SlotStatus};

use super::{frame_pumps, game_window_title, ui_frame, LiveHarness, LiveNull, PanelState};
use crate::session::{PanelWindow, Session, SlotIo};
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
    terminal_failure: Mutex<Option<crate::window::PanelError>>,
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
            terminal_failure: Mutex::new(None),
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
        self.frame_buttons_at(spot, down, false, pointer)
    }

    /// One frame with both physical mouse-button states.
    fn frame_buttons_at(
        &mut self,
        spot: [f32; 2],
        left_down: bool,
        right_down: bool,
        pointer: Pointer,
    ) -> bool {
        let io = self.ctx.io_mut();
        io.add_mouse_pos_event(spot);
        io.add_mouse_button_event(MouseButton::Left, left_down);
        io.add_mouse_button_event(MouseButton::Right, right_down);
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
                            ui_frame(ui, &mut gpu, &mut self.state, None, &self.terminal_failure);
                        ui.log_finish();
                        (exit, ui.with_bound_context(|| window_centre(&title)))
                    }
                    None => {
                        self.state.session.map_desk.begin_game_input_frame();
                        let exit = frame_pumps(&mut self.state, &self.terminal_failure);
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

/// The Game window's outer position and content-region size after the
/// frame that drew it. The Grid lays its cells out from the content
/// region, so a test aims at a cell from both.
fn window_pos_and_content(title: &str) -> Option<([f32; 2], [f32; 2])> {
    let name = CString::new(title).ok()?;
    // SAFETY: called with the frame's context bound; the name is a
    // NUL-terminated copy that outlives the call, and the window is read
    // only after the lookup found it.
    unsafe {
        let window = sys::igFindWindowByName(name.as_ptr());
        if window.is_null() || !(*window).Active {
            return None;
        }
        let content = (*window).ContentRegionRect;
        Some((
            [(*window).Pos.x, (*window).Pos.y],
            [content.Max.x - content.Min.x, content.Max.y - content.Min.y],
        ))
    }
}

/// Whether the docked tab of window `title` is visible in the current
/// frame: Dear ImGui draws a docked window's contents only on the frames
/// its tab is visible.
fn window_tab_visible(title: &str) -> bool {
    let Ok(name) = CString::new(title) else {
        return false;
    };
    // SAFETY: called with the frame's context bound; the name is a
    // NUL-terminated copy that outlives the call.
    unsafe {
        let window = sys::igFindWindowByName(name.as_ptr());
        !window.is_null() && (*window).DockTabIsVisible()
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
    #[cfg(feature = "js")]
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

#[cfg(feature = "js")]
const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

/// Property 2: script preparation, Start, reload validation and Stop settle
/// through frames alone. Every wait is a count of real frames until the
/// product's own state changes.
#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
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
    // The Script prefs window is docked, and Dear ImGui builds its dock
    // node's host window and tab bar on the rig's second frame. That frame
    // draws the tab labels only: a docked window's contents draw only while
    // its tab is visible, so the Quest Paths section is absent from it. A
    // reload that settled on that frame could not draw its result, although
    // the panel had still advanced it before the drawing. Wait for the
    // frame that draws the window before the reload starts: every frame
    // the reload can settle then draws the section, and the assertion below
    // holds the product's ordering.
    if rig.gpu.is_some() {
        rig.frames_until("the Script prefs window", |rig| {
            rig.text.contains(frontend_core::QUEST_PATHS_HEADING)
                && window_tab_visible(PanelWindow::ScriptPrefs.title())
        });
    }
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
    match drive {
        Drive::UiFrame => assert!(
            matches!(later.as_slice(), [InputEv::Up]),
            "the physical release routes outside the image, got {later:?}"
        ),
        Drive::Pumps => assert!(
            later.is_empty(),
            "the pump-only driver stages no physical release, got {later:?}"
        ),
    }
    rig.frame(Pointer::Away);
    assert!(
        drain(&rx).is_empty(),
        "a later empty frame does not replay the release"
    );
}

scenario!(captured_input, captured_input_is_submitted_once_per_frame);

/// The single Game view and Grid both release a captured mouse button
/// outside the image to the slot that received its press.
#[test]
fn game_and_grid_panes_release_mouse_buttons_outside_the_image() {
    for grid in [false, true] {
        for (name, left, right, button) in [("left", true, false, 1), ("right", false, true, 2)] {
            let pane = if grid { "grid" } else { "game" };
            let label = format!("mouse-up-{pane}-{name}");
            let (mut session, _dir) = session_with_play(&label, &["alice"]);
            session.multibox = grid;
            session.wall.grid = grid;
            session.focus.lock().unwrap().capture = true;
            let Some(mut rig) = Rig::new(Drive::UiFrame, session) else {
                return;
            };
            publish_ingame(rig.session(), "alice");
            for _ in 0..3 {
                assert!(!rig.frame(Pointer::Away));
            }
            let (sender, probe) = InputSender::probe();
            rig.session().map_desk.set_input_sender(Some(sender));
            let image = rig.game_centre.expect("the Game window is open");
            let mut image_hovered = false;
            for _ in 0..8 {
                assert!(!rig.frame_buttons_at(image, false, false, Pointer::Over));
                if drain(&probe)
                    .iter()
                    .any(|event| matches!(event, InputEv::Move { .. }))
                {
                    image_hovered = true;
                    break;
                }
            }
            assert!(image_hovered, "{pane}: the pointer reaches the Game Image");

            assert!(!rig.frame_buttons_at(image, left, right, Pointer::Over));
            let press = drain(&probe);
            assert!(
                press.iter().any(
                    |event| matches!(event, InputEv::Down { button: got, .. } if *got == button)
                ),
                "{pane} {name}: the press reaches its slot, got {press:?}"
            );

            let outside = [2.0, 2.0];
            assert!(!rig.frame_buttons_at(outside, left, right, Pointer::Away));
            assert!(
                drain(&probe).is_empty(),
                "{pane} {name}: moving outside does not release the held button"
            );
            if grid {
                rig.session()
                    .fleet_model
                    .staged_filter_mut()
                    .push_str("zzz");
                rig.session()
                    .submit(OperatorCommand::SetFleetFilter, Targets::Application);
                assert!(!rig.frame_buttons_at(outside, left, right, Pointer::Away));
                assert!(
                    drain(&probe).is_empty(),
                    "grid {name}: hiding the cell does not end the hold"
                );
            }

            assert!(!rig.frame_buttons_at(outside, false, false, Pointer::Away));
            assert_eq!(
                drain(&probe),
                [InputEv::Up],
                "{pane} {name}: the physical off-image release reaches the press slot"
            );
        }
    }
}

/// A Grid press inside the focused cell's script paint must not also click
/// the game. A later loaded cell with an open body must not clear the
/// focused cell's paint flag before the capture gate reads it.
#[test]
fn a_paint_press_in_the_focused_grid_cell_sends_no_game_down() {
    let (mut session, _dir) = session_with_play("grid-paint-press", &["alice", "bob"]);
    session.multibox = true;
    session.wall.grid = true;
    {
        let mut focus = session.focus.lock().unwrap();
        focus.capture = true;
        // Draw every wall member so both Grid cell bodies are open: the
        // defect needs a loaded cell after the focused one, drawing its
        // body. (Only-render-selected folds the unfocused bodies.)
        focus.only_render_selected = false;
        focus.wall_open = true;
        focus.wall = vec!["alice".to_string(), "bob".to_string()];
    }
    let Some(mut rig) = Rig::new(Drive::UiFrame, session) else {
        return;
    };
    publish_ingame(rig.session(), "alice");
    publish_ingame(rig.session(), "bob");
    TestPaint::new(Some("BoneBurier"), &["a row", "second row"])
        .publish(rig.session().core.play_for_test().unwrap(), "alice");
    for _ in 0..3 {
        assert!(!rig.frame(Pointer::Away));
    }
    // The pane reattaches its capture channel while the draw prefs settle;
    // take its place with a probe after the warm-up frames. The pane has
    // one capture feed, the focused slot's: a game Down for either cell's
    // slot is staged on this feed.
    let (sender, probe) = InputSender::probe();
    rig.session().map_desk.set_input_sender(Some(sender));

    let title = game_window_title(rig.session().focused_name().as_deref());
    let (window, avail) =
        window_pos_and_content(&title).expect("the Game window is open in Grid mode");
    let cells = crate::grid::grid_cells(2, avail);
    assert_eq!(cells.len(), 2, "both loaded slots have a Grid cell");
    let [cx, cy, cw, ch] = cells[0];
    // The script paint draws at the applet's lower left (`PANEL_RECT`,
    // `crates/frontend-core/src/scripts/paint_style.rs:14`), inside the
    // cell's Image. This point is over both: 34% across and 90% down the
    // first cell stays inside the panel for any cap-row height.
    let spot = [window[0] + cx + cw * 0.34, window[1] + cy + ch * 0.90];

    let mut in_paint = false;
    for _ in 0..8 {
        assert!(!rig.frame_buttons_at(spot, false, false, Pointer::Over));
        if rig.state.paint.pointer_inside_panel() {
            in_paint = true;
            break;
        }
    }
    assert!(
        in_paint,
        "the probe point lands inside the focused cell's script paint"
    );
    drain(&probe);

    assert!(!rig.frame_buttons_at(spot, true, false, Pointer::Over));
    let pressed = drain(&probe);
    assert!(
        !pressed
            .iter()
            .any(|event| matches!(event, InputEv::Down { .. })),
        "the paint press sends no game Down for either slot, got {pressed:?}"
    );
    assert!(
        rig.state.paint.pointer_inside_panel(),
        "the paint owns the press, not the game under it"
    );
    assert!(!rig.frame_buttons_at(spot, false, false, Pointer::Away));
    let released = drain(&probe);
    assert!(
        !released
            .iter()
            .any(|event| matches!(event, InputEv::Down { .. } | InputEv::Up)),
        "a paint press's release sends no button event to the game, got {released:?}"
    );

    // The other direction: a press on the same Image but outside the paint
    // still reaches the game, so the silence above is the paint's doing.
    let centre = [window[0] + cx + cw * 0.5, window[1] + cy + ch * 0.5];
    assert!(!rig.frame_buttons_at(centre, false, false, Pointer::Over));
    drain(&probe);
    assert!(!rig.frame_buttons_at(centre, true, false, Pointer::Over));
    let control = drain(&probe);
    assert!(
        control
            .iter()
            .any(|event| matches!(event, InputEv::Down { .. })),
        "a press inside the Image but outside the paint reaches the game, got {control:?}"
    );
    assert!(!rig.frame_buttons_at(centre, false, false, Pointer::Away));
    drain(&probe);
}

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
/// A Grid whose filter leaves no loaded profile frees the tile views its
/// cells held: the rail, which also disposes them, is not drawn in Grid.
#[test]
fn an_empty_filtered_grid_frees_its_tile_views() {
    let (mut session, _dir) = session_with_play("fleet-empty-grid", &["alice", "bob"]);
    session.multibox = true;
    session.wall.grid = true;
    let Some(mut rig) = Rig::new(Drive::UiFrame, session) else {
        return;
    };
    rig.frames_until("a Grid cell holds a tile view", |rig| {
        !rig.state.views.is_empty()
    });
    rig.session()
        .fleet_model
        .staged_filter_mut()
        .push_str("zzz");
    rig.session()
        .submit(OperatorCommand::SetFleetFilter, Targets::Application);
    rig.frame(Pointer::Away);
    assert!(
        rig.text.contains("no wall member matches the Fleet filter"),
        "the Grid is empty:\n{}",
        rig.text
    );
    assert!(
        rig.state.views.is_empty(),
        "the hidden cells' views are freed: {:?}",
        rig.state.views.keys().collect::<Vec<_>>()
    );
}

/// Publish `name` as a connected, in-game slot on public world `world`.
fn publish_on_world(session: &Session, name: &str, world: u16) {
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
        world: Some(world),
        connected: true,
        ingame: true,
        scene_state: 2,
        ..SlotStatus::default()
    });
}

/// A rig on the MultiBox rail (or the Grid) over `alice` on world 2 and
/// `bob` on world 3, settled until both caps are drawn; the server offers
/// `worlds`. `None` when the host has no GPU.
fn world_rig(label: &str, grid: bool, worlds: Option<&[u16]>) -> Option<(Rig, TestDir)> {
    let (mut session, dir) = session_with_play(label, &["alice", "bob"]);
    session.multibox = true;
    session.wall.grid = grid;
    if let Some(worlds) = worlds {
        session.core.set_public_worlds_for_test(worlds);
    }
    publish_on_world(&session, "alice", 2);
    publish_on_world(&session, "bob", 3);
    let mut rig = Rig::new(Drive::UiFrame, session)?;
    rig.frames_until("both caps are drawn", |rig| {
        drawn_caps(&rig.text, &["alice", "bob"]).len() == 2
    });
    rig.frame(Pointer::Away);
    Some((rig, dir))
}

/// The world marker of each cap one frame drew, in drawn order (the first
/// of the five items `rail_cap` records).
fn world_markers(rig: &mut Rig) -> Vec<[f32; 4]> {
    super::TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(Vec::clear);
    rig.frame(Pointer::Away);
    super::TEST_RAIL_ITEM_BOUNDS
        .with_borrow(|bounds| bounds.as_chunks::<5>().0.iter().map(|cap| cap[0]).collect())
}

/// A left click at `at`, then frames that show the result.
fn click_at(rig: &mut Rig, at: [f32; 2]) {
    rig.frame_at(at, false, Pointer::Away);
    rig.frame_at(at, true, Pointer::Away);
    rig.frame_at(at, false, Pointer::Away);
    rig.frame_at(at, false, Pointer::Away);
    rig.frame_at([2.0, 2.0], false, Pointer::Away);
}

/// The picker's rows as one frame draws them: each world, whether it is
/// ticked and its rectangle.
fn picker_rows(rig: &mut Rig) -> Vec<crate::world_picker::EntryRecord> {
    rig.frame(Pointer::Away);
    crate::world_picker::take_entries()
}

/// A click on a world marker of the rail or of a Grid cell opens the one
/// world picker for that bot, with its world ticked; picking another world
/// plans a Switch world for that bot alone, and the fleet toolbar on screen
/// (the rail's, or the side panel's in Grid) shows the plan's confirmation
/// (Confirm and Cancel); nothing runs before Confirm. Picking the world the
/// bot is on submits nothing. The marker was a plain disc.
fn a_world_marker_opens_the_picker_and_a_pick_plans_that_bot(grid: bool) {
    let label = if grid { "grid-world" } else { "rail-world" };
    let Some((mut rig, _dir)) = world_rig(label, grid, Some(&[1, 2, 3])) else {
        return;
    };
    let markers = world_markers(&mut rig);
    assert_eq!(markers.len(), 2, "one marker per loaded bot");
    assert!(!rig.session().fleet_open);
    let commands = rig.session().core.commands().reports().count();

    // alice's marker (the first cap): her own world 2 is ticked.
    click_at(&mut rig, centre(markers[0]));
    assert_eq!(rig.session().world_picker.target(), Some("alice"));
    let rows = picker_rows(&mut rig);
    assert_eq!(
        rows.iter()
            .map(|row| (row.world, row.ticked))
            .collect::<Vec<_>>(),
        [(1, false), (2, true), (3, false)]
    );
    assert!(
        rig.text.contains("Switch world of alice") && rig.text.contains("World 3"),
        "the frame drew the picker:\n{}",
        rig.text
    );
    assert!(rig.session().core.fleet_plan().is_none());

    // Her own world: nothing is submitted, the picker closes.
    let own = rows.iter().find(|row| row.world == 2).unwrap().rect;
    click_at(&mut rig, centre(own));
    assert!(!rig.session().world_picker.is_open());
    assert!(rig.session().core.fleet_plan().is_none());
    assert!(!rig.session().fleet_open);
    assert_eq!(rig.session().core.commands().reports().count(), commands);

    // bob's marker ticks his own world 3; picking 1 plans him alone.
    let markers = world_markers(&mut rig);
    click_at(&mut rig, centre(markers[1]));
    assert_eq!(rig.session().world_picker.target(), Some("bob"));
    let rows = picker_rows(&mut rig);
    assert_eq!(
        rows.iter()
            .map(|row| (row.world, row.ticked))
            .collect::<Vec<_>>(),
        [(1, false), (2, false), (3, true)]
    );
    let other = rows.iter().find(|row| row.world == 1).unwrap().rect;
    click_at(&mut rig, centre(other));
    let plan = rig.session().core.fleet_plan().expect("the plan waits");
    assert_eq!(plan.action(), frontend_core::FleetAction::SwitchWorld(1));
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["bob"]);
    let prompt = plan.prompt().to_owned();
    assert!(
        prompt.starts_with("Move 1 bot to world 1: bob."),
        "{prompt}"
    );
    // The rail's toolbar (Grid: the strip's fleet section) shows the plan,
    // so the Fleet window stays closed.
    assert!(!rig.session().fleet_open, "a toolbar shows the plan");
    rig.frame(Pointer::Away);
    assert!(
        rig.text.contains("Move 1 bot to world 1: bob.") && rig.text.contains("Confirm"),
        "the frame draws the plan row:\n{}",
        rig.text
    );
    let ran = rig
        .session()
        .core
        .commands()
        .reports()
        .any(|report| report.command() == OperatorCommand::SwitchWorld(1));
    assert!(!ran, "nothing runs before Confirm");
}

#[test]
fn a_rail_world_marker_opens_the_picker_and_a_pick_plans_that_bot() {
    a_world_marker_opens_the_picker_and_a_pick_plans_that_bot(false);
}

#[test]
fn a_grid_world_marker_opens_the_picker_and_a_pick_plans_that_bot() {
    a_world_marker_opens_the_picker_and_a_pick_plans_that_bot(true);
}

/// On a server with one world the marker is not clickable: a click opens no
/// picker, plans nothing and opens no window.
#[test]
fn a_world_marker_is_not_clickable_on_a_one_world_server() {
    for grid in [false, true] {
        let label = if grid {
            "grid-world-local"
        } else {
            "rail-world-local"
        };
        let Some((mut rig, _dir)) = world_rig(label, grid, None) else {
            return;
        };
        let markers = world_markers(&mut rig);
        assert_eq!(markers.len(), 2, "the markers are still drawn");
        click_at(&mut rig, centre(markers[0]));
        assert!(!rig.session().world_picker.is_open());
        assert!(rig.session().core.fleet_plan().is_none());
        assert!(!rig.session().fleet_open);
        assert_eq!(rig.session().error, None);
    }
}

/// The real frame: the rail's fleet toolbar stays put while the tiles
/// region under it scrolls (30 caps do not fit), open or folded; folded,
/// the tiles region is taller and a waiting plan stays on screen.
#[test]
fn the_rail_tiles_scroll_under_the_sticky_toolbar() {
    let names: Vec<String> = (0..30).map(|i| format!("bot{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut session, _dir) = session_with_play("rail-sticky", &refs);
    session.multibox = true;
    let Some(mut rig) = Rig::new(Drive::UiFrame, session) else {
        eprintln!("the_rail_tiles_scroll_under_the_sticky_toolbar: no headless GPU, skipped");
        return;
    };
    rig.frames_until("every cap is drawn", |rig| {
        drawn_caps(&rig.text, &refs).len() == refs.len()
    });
    // One frame's toolbar header and tiles region.
    let frame = |rig: &mut Rig| {
        crate::fleet::take_layout();
        rig.frame(Pointer::Away);
        let layout = crate::fleet::take_layout();
        let tiles = super::TEST_RAIL_TILES.get().expect("the tiles region");
        (layout, tiles)
    };
    for open in [true, false] {
        rig.session().ui.fleet_toolbar_open = open;
        let (before, tiles) = frame(&mut rig);
        assert!(
            tiles.scroll_max_y > 0.0,
            "open {open}: 30 caps overflow the tiles region"
        );
        // Wheel over the tiles region until it scrolls.
        let over = centre(tiles.rect);
        for _ in 0..4 {
            rig.ctx.io_mut().add_mouse_wheel_event([0.0, -3.0]);
            rig.frame_at(over, false, Pointer::Away);
        }
        let (after, scrolled) = frame(&mut rig);
        assert!(
            scrolled.scroll_y > 0.0,
            "open {open}: the tiles region scrolled"
        );
        assert_eq!(
            after.fold_header, before.fold_header,
            "open {open}: the toolbar did not move"
        );
        assert_eq!(after.site, Some(crate::fleet_toolbar::Site::Rail));
        assert_eq!(after.actions.is_empty(), !open, "open {open}");
        if !open {
            // A plan stays visible under the folded header.
            rig.session()
                .plan_fleet_action(frontend_core::FleetAction::Login);
            let (folded, folded_tiles) = frame(&mut rig);
            assert!(folded.plan.is_some(), "the folded toolbar keeps the plan");
            assert!(folded_tiles.rect[1] > folded.fold_header[3]);
        }
    }
}

/// PANEL-STATE-GAP-1023: the rail's caps (`name: label`, the sidecar the
/// operator saw) read `idle` for all nine bots after Stop, including the
/// three whose script was walking. Under `UiFrame` the drawn text says the
/// same.
fn stopped_scripts_leave_no_walking_label_on_the_rail(drive: Drive) {
    const NINE: [&str; 9] = [
        "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan",
    ];
    let (mut session, _dir) = session_with_play("psg-rail", &NINE);
    session.multibox = true;
    let Some(mut rig) = Rig::new(drive, session) else {
        return;
    };
    #[cfg(not(feature = "js"))]
    {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        rig.session()
            .core
            .play_mut_for_test()
            .unwrap()
            .bind_script_test_data(data);
    }
    for name in NINE {
        publish_ingame(rig.session(), name);
        #[cfg(feature = "js")]
        frontend_core::test_support::scripts::play_start_load(
            &rig.session().core,
            name,
            "export function tick(api) {}".into(),
            frontend_core::scripts::LoadShape::NativeTick,
            None,
            Vec::new(),
        )
        .unwrap();
        #[cfg(not(feature = "js"))]
        {
            let card = CompiledCardId::lookup("Gatherer").expect("compiled Gatherer card");
            rig.session().persist_successful_assignment(
                name,
                frontend_core::test_support::scripts::Assignment::compiled(card),
            );
            rig.session().core.flush_writes();
            rig.session()
                .script_start_profile(name)
                .expect("compiled Start");
        }
    }
    let wait = |rig: &mut Rig, want: ScriptRunState| {
        for name in NINE {
            rig.frames_until("the script state", |rig| {
                let session = rig.session();
                if let Some(play) = session.core.play_for_test() {
                    play.pump_script_lifecycle(name);
                }
                session.core.script_state(name) == want
            });
        }
    };
    wait(&mut rig, ScriptRunState::Running);
    {
        let play = rig.session().core.play_for_test().unwrap();
        let mut rows = play.statuses.lock().unwrap();
        for row in rows
            .iter_mut()
            .filter(|row| NINE[..3].contains(&row.username.as_str()))
        {
            (row.walk_x, row.walk_z, row.walk_level) = (2550, 3348, 0);
        }
    }
    let caps = |rig: &mut Rig| -> Vec<String> {
        rig.frame(Pointer::Away);
        rig.state.sync_fleet();
        rig.state
            .fleet_labels
            .iter()
            .map(|labels| labels.title.trim_end_matches("###cap").to_owned())
            .collect()
    };
    let expect = |label: &str| -> Vec<String> {
        NINE.iter().map(|name| format!("{name}: {label}")).collect()
    };
    // The rail rows refresh from the core one frame after the script state
    // changes (Session::pump_status runs before the core poll), so a native
    // Start that is already Running needs frames before the labels follow.
    let running = expect("running");
    rig.frames_until("every rail label shows running", |rig| caps(rig) == running);
    assert_eq!(caps(&mut rig), running);

    let stops = NINE.map(|name| {
        rig.session()
            .submit(OperatorCommand::StopScript, Targets::Profile(name))
            .expect("Stop is admitted")
            .request()
    });
    wait(&mut rig, ScriptRunState::Idle);
    // A native Stop can reach Idle during submit. Its command report still
    // settles through frames, which ingest the ended run into the fleet model.
    rig.frames_until("the Stop reports", |rig| {
        let core = &rig.session().core;
        stops.iter().all(|request| {
            core.commands()
                .report(*request)
                .is_some_and(|report| report.is_settled())
        })
    });
    assert_eq!(caps(&mut rig), expect("idle"), "Stop left a walking label");
    if matches!(drive, Drive::UiFrame) {
        assert!(!rig.text.contains(": running"), "{}", rig.text);
    }
}

scenario!(
    stopped_rail_labels,
    stopped_scripts_leave_no_walking_label_on_the_rail
);
