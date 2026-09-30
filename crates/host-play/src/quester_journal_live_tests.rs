//! Headless LIVE proofs for Quester journal evidence and the released Cook card.
//!
//! The synthetic card deliberately uses the Rune Mysteries journal branches from
//! the R289 content pack while retaining the checked Cook Path header.  It is a
//! fixture, not a second product Path: the test installs it through the
//! test-only [`ScriptStartHandle::start_test_script`] seam and exercises the
//! ordinary host pump.
//!
//! The tests are ignored because they need the shared local R289 engine.  Run
//! one at a time with a throwaway HOME, for example:
//!
//! ```text
//! LIVE=1 cargo test -p host-play --lib live_quester_journal_synthetic_runemysteries -- --ignored --nocapture --test-threads=1
//! LIVE=1 cargo test -p host-play --lib live_quester_cook_reaches_complete -- --ignored --nocapture --test-threads=1
//! ```
//!
//! On Windows, headed proof additionally requires the opt-in feature and both
//! headed environment variables.  `BOT_CPU=1` selects the CPU pixmap path;
//! leaving it unset exercises GPU frames through WindowTarget readback:
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_JOURNAL_HEADED=1 BOT_JOURNAL_CAPTURE_DIR=guest_home\.274bot\smoke\journal cargo test -p host-play --release --features journal-paint-proof --lib live_quester_journal_synthetic_runemysteries -- --ignored --nocapture --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(all(windows, feature = "journal-paint-proof"))]
use std::sync::mpsc;

use api::interact;
use api::quest_facts::QuestCatalog;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey, Truth};
use api::snapshot::GameSnapshot;
use client::client::MiniMenuAction;
#[cfg(all(windows, feature = "journal-paint-proof"))]
use client::client::present::{PresentTarget, WindowTarget};
#[cfg(all(windows, feature = "journal-paint-proof"))]
use client::client::{GameShell, APPLET_H, APPLET_W};
#[cfg(all(windows, feature = "journal-paint-proof"))]
use client::render::backend::FrameOutput;
#[cfg(all(windows, feature = "journal-paint-proof"))]
use host::InputEv;
use host::{FrameBuf, SlotInput};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::compile::{compile_uncached_for_test, decode_cook};
use script::quester::path::{
    PathDocument, PredicateDocument, ProgressColourDocument, ProgressDocument,
    ProgressFlagDocument, ProgressRuleDocument, SequenceDocument,
};
use script::quester::runner::Quester;
use vault::{Profile, ProfileSettings};

use super::{run_with_template, ProfileOptions, ScriptStartHandle, SharedClientTemplate};

const SYNTHETIC_QUEST: &str = "runemysteries";
const ACCOUNT_UID: i32 = 274_279_003;
const SYNTHETIC_TIMEOUT: Duration = Duration::from_secs(240);
const COOK_TIMEOUT: Duration = Duration::from_secs(1000);
const JOURNAL_ROOT_R289: i32 = 8134;
const JOURNAL_TITLE_COMPONENT_R289: i32 = 8144;
#[cfg(all(windows, feature = "journal-paint-proof"))]
const JOURNAL_ROOT: i32 = 8134;
#[cfg(all(windows, feature = "journal-paint-proof"))]
const JOURNAL_TITLE_COMPONENT: i32 = 8144;
const JOURNAL_BUTTON: i32 = 42;
const JOURNAL_CLOSE_X: i32 = 496;
const JOURNAL_CLOSE_Y: i32 = 8;

#[cfg(all(windows, feature = "journal-paint-proof"))]
const MODAL_ROI: (usize, usize, usize, usize) = (4, 516, 4, 338);
#[cfg(all(windows, feature = "journal-paint-proof"))]
const SIDE_ROI: (usize, usize, usize, usize) = (553, 743, 205, 466);
#[cfg(all(windows, feature = "journal-paint-proof"))]
const TABS_TOP_ROI: (usize, usize, usize, usize) = (516, 765, 160, 205);
#[cfg(all(windows, feature = "journal-paint-proof"))]
const TABS_BOTTOM_ROI: (usize, usize, usize, usize) = (496, 765, 466, 503);

#[derive(Clone, Copy, Debug, Default)]
struct PaintObservation {
    sequence: u64,
    paint_generation: u64,
    modal_root: i32,
    journal_paint_hidden: bool,
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
type CaptureMutex<T> = parking_lot::Mutex<T>;
#[cfg(not(all(windows, feature = "journal-paint-proof")))]
type CaptureMutex<T> = std::sync::Mutex<T>;

struct PaintProbe {
    metadata: Arc<CaptureMutex<PaintObservation>>,
    mailbox: Arc<FrameBuf>,
    #[cfg(all(windows, feature = "journal-paint-proof"))]
    observations: mpsc::SyncSender<PaintObservation>,
}

struct CaptureHandle {
    #[cfg(all(windows, feature = "journal-paint-proof"))]
    inner: HeadedCapture,
}

impl CaptureHandle {
    fn from_env(label: &str) -> Option<Self> {
        let headed = std::env::var("BOT_JOURNAL_HEADED").as_deref() == Ok("1");
        let capture_dir = std::env::var_os("BOT_JOURNAL_CAPTURE_DIR").map(PathBuf::from);
        if !headed && capture_dir.is_none() {
            return None;
        }
        let Some(capture_dir) = capture_dir else {
            panic!("BOT_JOURNAL_HEADED=1 requires explicit BOT_JOURNAL_CAPTURE_DIR");
        };
        if !headed {
            panic!(
                "BOT_JOURNAL_CAPTURE_DIR requires BOT_JOURNAL_HEADED=1; headed capture is opt-in"
            );
        }
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            Some(Self {
                inner: HeadedCapture::start(label, capture_dir),
            })
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            drop((label, capture_dir));
            panic!(
                "BOT_JOURNAL_HEADED=1 is Windows-only and requires --features journal-paint-proof"
            );
        }
    }

    fn slots(&self) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>) {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.slots()
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            (None, None)
        }
    }

    fn probe(&self) -> Option<PaintProbe> {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            Some(PaintProbe {
                metadata: Arc::clone(&self.inner.metadata),
                mailbox: Arc::clone(&self.inner.mailbox),
                observations: self.inner.observations.clone(),
            })
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            None
        }
    }

    fn saw_hidden_root(&self) -> bool {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.saw_hidden_root()
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            false
        }
    }

    fn saw_visible_after_hidden(&self) -> bool {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.saw_visible_after_hidden()
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            false
        }
    }

    fn saw_closed_after_click(&self) -> bool {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.saw_closed_after_click()
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            false
        }
    }

    fn normal_closed_paint(&self) -> bool {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.check_fatal();
            let state = self.inner.state.lock();
            state.frames > 0
                && !state.last_observation.journal_paint_hidden
                && state.last_observation.modal_root != JOURNAL_ROOT
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
            false
        }
    }

    fn click(&self, x: i32, y: i32) {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.click(x, y);
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = (self, x, y);
        }
    }

    fn assert_no_flash(&self) {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.assert_no_flash();
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
        }
    }

    fn finish(self) {
        #[cfg(all(windows, feature = "journal-paint-proof"))]
        {
            self.inner.finish();
        }
        #[cfg(not(all(windows, feature = "journal-paint-proof")))]
        {
            let _ = self;
        }
    }
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
struct HeadedCapture {
    input: Arc<SlotInput>,
    mailbox: Arc<FrameBuf>,
    metadata: Arc<CaptureMutex<PaintObservation>>,
    observations: mpsc::SyncSender<PaintObservation>,
    input_tx: mpsc::Sender<InputEv>,
    commands: mpsc::Sender<CaptureCommand>,
    state: Arc<CaptureMutex<CaptureState>>,
    output_dir: PathBuf,
    worker: Option<thread::JoinHandle<()>>,
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
enum CaptureCommand {
    ArmInput,
    Stop,
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
struct CaptureState {
    baseline: u64,
    last_generation: u64,
    frames: u64,
    cpu_frames: u64,
    gpu_frames: u64,
    fatal: Option<String>,
    saw_hidden_root: bool,
    saw_visible_after_hidden: bool,
    close_requested: bool,
    saw_closed_after_click: bool,
    visible_template: Option<Vec<u32>>,
    quiet_frame: Option<Vec<u32>>,
    restored_frame: Option<Vec<u32>>,
    hidden_frames: Vec<(u64, Vec<u32>)>,
    last_observation: PaintObservation,
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
impl HeadedCapture {
    fn start(label: &str, root: PathBuf) -> Self {
        std::fs::create_dir_all(&root).expect("create headed journal capture directory");
        let run_dir = root.join(format!(
            "{}-{}-{}",
            capture_label(label),
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&run_dir).expect("create headed journal capture run");

        let input = SlotInput::new();
        let prefer_cpu = std::env::var("BOT_CPU").as_deref() == Ok("1");
        input.set_prefer_cpu(prefer_cpu);
        input.set_full_rate(false);
        input.set_enabled(false);
        let mailbox = FrameBuf::new();
        let metadata = Arc::new(CaptureMutex::new(PaintObservation::default()));
        let (observations, observation_rx) = mpsc::sync_channel(256);
        let (input_tx, input_rx) = mpsc::channel();
        input.connect_rx(input_rx);
        let (commands, command_rx) = mpsc::channel();
        let state = Arc::new(CaptureMutex::new(CaptureState {
            baseline: 0,
            last_generation: 0,
            frames: 0,
            cpu_frames: 0,
            gpu_frames: 0,
            fatal: None,
            saw_hidden_root: false,
            saw_visible_after_hidden: false,
            close_requested: false,
            saw_closed_after_click: false,
            visible_template: None,
            quiet_frame: None,
            restored_frame: None,
            hidden_frames: Vec::new(),
            last_observation: PaintObservation::default(),
        }));
        let ready_state = Arc::clone(&state);
        let ready_mailbox = Arc::clone(&mailbox);
        let ready_metadata = Arc::clone(&metadata);
        let ready_input = Arc::clone(&input);
        let ready_input_tx = input_tx.clone();
        let ready_output = run_dir.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let title = format!("274bot journal proof: {label}");
        let worker = thread::Builder::new()
            .name(format!("journal-capture-{label}"))
            .spawn(move || {
                let mut target = match WindowTarget::open(APPLET_W as u32, APPLET_H as u32, &title)
                {
                    Ok(target) => target,
                    Err(error) => {
                        let message = format!("open headed journal proof window: {error}");
                        ready_state.lock().fatal = Some(message.clone());
                        let _ = ready_tx.send(Err(message));
                        return;
                    }
                };
                let baseline = ready_mailbox.generation();
                {
                    let mut state = ready_state.lock();
                    state.baseline = baseline;
                    state.last_generation = baseline;
                }
                let _ = ready_tx.send(Ok(()));
                capture_loop(
                    &mut target,
                    ready_mailbox,
                    ready_metadata,
                    observation_rx,
                    ready_input,
                    ready_input_tx,
                    command_rx,
                    ready_state,
                    ready_output,
                );
            })
            .expect("spawn headed journal capture worker");
        match ready_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("headed journal capture worker did not initialize")
        {
            Ok(()) => {}
            Err(error) => panic!("{error}"),
        }
        Self {
            input,
            mailbox,
            metadata,
            observations,
            input_tx,
            commands,
            state,
            output_dir: run_dir,
            worker: Some(worker),
        }
    }

    fn slots(&self) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>) {
        (
            Some(Arc::clone(&self.input)),
            Some(Arc::clone(&self.mailbox)),
        )
    }

    fn check_fatal(&self) {
        if let Some(error) = self.state.lock().fatal.clone() {
            panic!("headed journal capture failed: {error}");
        }
    }

    fn saw_hidden_root(&self) -> bool {
        self.check_fatal();
        self.state.lock().saw_hidden_root
    }

    fn saw_visible_after_hidden(&self) -> bool {
        self.check_fatal();
        self.state.lock().saw_visible_after_hidden
    }

    fn saw_closed_after_click(&self) -> bool {
        self.check_fatal();
        self.state.lock().saw_closed_after_click
    }

    fn click(&self, x: i32, y: i32) {
        self.check_fatal();
        {
            let mut state = self.state.lock();
            state.close_requested = true;
            state.saw_closed_after_click = false;
        }
        self.input.set_enabled(true);
        self.input_tx
            .send(InputEv::Move { x, y })
            .expect("headed journal input worker stopped");
        self.input_tx
            .send(InputEv::Down { button: 1, x, y })
            .expect("headed journal input worker stopped");
        self.input_tx
            .send(InputEv::Up)
            .expect("headed journal input worker stopped");
        self.commands
            .send(CaptureCommand::ArmInput)
            .expect("headed journal capture worker stopped");
    }

    fn assert_no_flash(&self) {
        self.check_fatal();
        let state = self.state.lock();
        let visible = state
            .visible_template
            .as_ref()
            .expect("headed proof did not capture a visible Rune Mysteries journal template");
        let quiet = state
            .quiet_frame
            .as_ref()
            .expect("headed proof did not capture a hidden Rune Mysteries journal frame");
        assert!(
            state.saw_hidden_root,
            "headed proof never observed journal paint ownership at main root {JOURNAL_ROOT}"
        );
        assert!(
            state.saw_visible_after_hidden,
            "headed proof never observed normal journal paint after ownership release"
        );
        assert!(
            state.restored_frame.is_some(),
            "headed proof did not retain a post-Stop normal journal frame"
        );
        assert!(
            !state.hidden_frames.is_empty(),
            "no owned journal frames captured"
        );
        for (generation, pixels) in &state.hidden_frames {
            let (changed, total) = roi_diff(visible, pixels, MODAL_ROI);
            assert!(
                changed * 10 >= total,
                "journal modal flashed in owned painted frame {generation}: only {changed}/{total} main pixels differ from the visible control"
            );
        }
        let (modal_changed, modal_total) = roi_diff(visible, quiet, MODAL_ROI);
        assert!(
            modal_changed * 10 >= modal_total,
            "quiet journal frame still resembles the visible modal in its main ROI: {modal_changed}/{modal_total} pixels changed"
        );
        let (side_changed, side_total) = roi_diff(visible, quiet, SIDE_ROI);
        let (top_changed, top_total) = roi_diff(visible, quiet, TABS_TOP_ROI);
        let (bottom_changed, bottom_total) = roi_diff(visible, quiet, TABS_BOTTOM_ROI);
        let retained =
            side_total + top_total + bottom_total - (side_changed + top_changed + bottom_changed);
        let retained_total = side_total + top_total + bottom_total;
        assert!(
            retained * 4 >= retained_total * 3,
            "quiet journal frame lost retained side/tab chrome: {retained}/{retained_total} pixels retained"
        );
    }

    fn finish(mut self) {
        let _ = self.commands.send(CaptureCommand::Stop);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("headed journal capture worker panicked");
        }
        let state = self.state.lock();
        let runtime_model = std::env::var("BOT_RUNTIME_MODEL")
            .or_else(|_| std::env::var("BOT_ENGINE_MODEL"))
            .unwrap_or_else(|_| "R289".into());
        let backend = if state.gpu_frames > 0 {
            "gpu-readback"
        } else {
            "cpu"
        };
        let summary = serde_json::json!({
            "runtime_model": runtime_model,
            "backend": backend,
            "baseline_generation": state.baseline,
            "last_generation": state.last_generation,
            "frames": state.frames,
            "cpu_frames": state.cpu_frames,
            "gpu_frames": state.gpu_frames,
            "journal_root": JOURNAL_ROOT,
            "journal_title_component": JOURNAL_TITLE_COMPONENT,
            "saw_hidden_root": state.saw_hidden_root,
            "saw_visible_after_hidden": state.saw_visible_after_hidden,
            "saw_closed_after_click": state.saw_closed_after_click,
            "owned_frames_checked": state.hidden_frames.len(),
            "fatal": state.fatal,
            "metadata_relation": "first callback after the completed paint generation; modal root is current server state",
        });
        std::fs::write(
            self.output_dir.join("summary.json"),
            serde_json::to_vec_pretty(&summary).expect("encode headed journal capture summary"),
        )
        .expect("write headed journal capture summary");
        println!(
            "headed journal capture runtime_model={runtime_model} backend={backend} frames={} generation={}..{} output={}",
            state.frames,
            state.baseline + 1,
            state.last_generation,
            self.output_dir.display(),
        );
        if let Some(error) = state.fatal.clone() {
            panic!("headed journal capture failed: {error}");
        }
    }
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
impl Drop for HeadedCapture {
    fn drop(&mut self) {
        let _ = self.commands.send(CaptureCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
#[allow(clippy::too_many_arguments)]
fn capture_loop(
    target: &mut WindowTarget,
    mailbox: Arc<FrameBuf>,
    metadata: Arc<CaptureMutex<PaintObservation>>,
    observations: mpsc::Receiver<PaintObservation>,
    input: Arc<SlotInput>,
    input_tx: mpsc::Sender<InputEv>,
    commands: mpsc::Receiver<CaptureCommand>,
    state: Arc<CaptureMutex<CaptureState>>,
    output_dir: PathBuf,
) {
    let mut shell = GameShell::new();
    let mut last_mouse = (shell.mouse_x, shell.mouse_y);
    let mut last_button = shell.mouse_button;
    let mut disable_input_at = None;
    loop {
        let mut stop = false;
        while let Ok(command) = commands.try_recv() {
            match command {
                CaptureCommand::ArmInput => {
                    input.set_enabled(true);
                    disable_input_at = Some(Instant::now() + Duration::from_millis(120));
                }
                CaptureCommand::Stop => stop = true,
            }
        }
        if stop {
            let generation = mailbox.generation();
            let last_generation = state.lock().last_generation;
            if generation <= last_generation {
                break;
            }
        }
        if !target.poll(&mut shell) {
            state.lock().fatal =
                Some("headed journal proof window was closed before capture finished".into());
            break;
        }
        if shell.mouse_x != last_mouse.0 || shell.mouse_y != last_mouse.1 {
            if input.enabled() {
                let _ = input_tx.send(InputEv::Move {
                    x: shell.mouse_x,
                    y: shell.mouse_y,
                });
            }
            last_mouse = (shell.mouse_x, shell.mouse_y);
        }
        if shell.mouse_button != last_button {
            input.set_enabled(true);
            disable_input_at = Some(Instant::now() + Duration::from_millis(120));
            if shell.mouse_button == 0 {
                let _ = input_tx.send(InputEv::Up);
            } else {
                let _ = input_tx.send(InputEv::Down {
                    button: shell.mouse_button,
                    x: shell.mouse_x,
                    y: shell.mouse_y,
                });
            }
            last_button = shell.mouse_button;
        }
        if let Some(frame) = mailbox.take() {
            let generation = mailbox.generation();
            let baseline = state.lock().baseline;
            if generation > baseline {
                let mut state = state.lock();
                if generation != state.last_generation + 1 {
                    state.fatal = Some(format!(
                        "frame generation gap during headed proof: expected {}, captured {generation}",
                        state.last_generation + 1
                    ));
                    break;
                }
                // The callback precedes this pass's lease update/raster. The
                // first callback tagged with the completed generation reads
                // the flag used by that paint, rather than its predecessor.
                let observation = loop {
                    match observations.recv_timeout(Duration::from_secs(1)) {
                        Ok(observation) if observation.paint_generation < generation => {}
                        Ok(observation) if observation.paint_generation == generation => {
                            break observation;
                        }
                        Ok(observation) => {
                            state.fatal = Some(format!(
                                "paint metadata skipped generation {generation}: reached {}",
                                observation.paint_generation
                            ));
                            return;
                        }
                        Err(error) => {
                            if matches!(commands.try_recv(), Ok(CaptureCommand::Stop)) {
                                stop = true;
                            }
                            let latest = *metadata.lock();
                            // Teardown follows a closed, revoked journal. A
                            // final non-owned frame can have no next callback;
                            // retain its old generation explicitly in JSON.
                            if stop && !latest.journal_paint_hidden {
                                break latest;
                            }
                            state.fatal = Some(format!(
                                "missing post-paint metadata for generation {generation}: {error}"
                            ));
                            return;
                        }
                    }
                };
                match frame {
                    FrameOutput::PixMap(pix) => {
                        let width = pix.width;
                        let height = pix.height;
                        let pixels = pix
                            .pixels
                            .iter()
                            .take((APPLET_W * APPLET_H) as usize)
                            .map(|pixel| *pixel as u32 & 0x00ff_ffff)
                            .collect::<Vec<_>>();
                        if width != APPLET_W
                            || height != APPLET_H
                            || pixels.len() < (APPLET_W * APPLET_H) as usize
                        {
                            state.fatal = Some(format!(
                                "headed proof received non-applet CPU frame {}x{} ({} pixels)",
                                width,
                                height,
                                pixels.len()
                            ));
                            break;
                        }
                        if let Err(error) =
                            write_capture_frame(&output_dir, generation, &observation, &pixels)
                        {
                            state.fatal = Some(error);
                            break;
                        }
                        remember_frame(&mut state, generation, &observation, &pixels);
                        state.cpu_frames += 1;
                        target.present(FrameOutput::PixMap(pix));
                    }
                    FrameOutput::Texture(handle) => {
                        let width = handle.width as i32;
                        let height = handle.height as i32;
                        let pixels = handle
                            .read_back()
                            .into_iter()
                            .map(|pixel| pixel as u32 & 0x00ff_ffff)
                            .collect::<Vec<_>>();
                        if width != APPLET_W
                            || height != APPLET_H
                            || pixels.len() < (APPLET_W * APPLET_H) as usize
                        {
                            state.fatal = Some(format!(
                                "headed proof received non-applet GPU frame {}x{} ({} pixels)",
                                width,
                                height,
                                pixels.len()
                            ));
                            break;
                        }
                        if let Err(error) =
                            write_capture_frame(&output_dir, generation, &observation, &pixels)
                        {
                            state.fatal = Some(error);
                            break;
                        }
                        remember_frame(&mut state, generation, &observation, &pixels);
                        state.gpu_frames += 1;
                        target.present(FrameOutput::Texture(handle));
                    }
                }
                state.last_generation = generation;
                state.frames += 1;
            }
        }
        // Play is stopped before this worker receives Stop; drain one final
        // parked generation so the end-of-journal paint is not discarded.
        if stop {
            break;
        }
        if disable_input_at.is_some_and(|deadline| Instant::now() >= deadline) {
            input.set_enabled(false);
            disable_input_at = None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
fn write_capture_frame(
    output_dir: &Path,
    generation: u64,
    observation: &PaintObservation,
    pixels: &[u32],
) -> Result<(), String> {
    let stem = format!("frame-{generation:020}");
    let png_path = output_dir.join(format!("{stem}.png"));
    let json_path = output_dir.join(format!("{stem}.json"));
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            *pixel as u8,
            u8::MAX,
        ]);
    }
    let file = std::fs::File::create(&png_path)
        .map_err(|error| format!("create {}: {error}", png_path.display()))?;
    let mut encoder = png::Encoder::new(file, APPLET_W as u32, APPLET_H as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("write {} header: {error}", png_path.display()))?;
    writer
        .write_image_data(&rgba)
        .map_err(|error| format!("write {} pixels: {error}", png_path.display()))?;
    let metadata = serde_json::json!({
        "generation": generation,
        "modal_root": observation.modal_root,
        "journal_paint_hidden": observation.journal_paint_hidden,
        "metadata_sequence": observation.sequence,
        "metadata_paint_generation": observation.paint_generation,
        "flag_sample_matches_paint": observation.paint_generation == generation,
        "metadata_relation": "first callback after the completed paint generation; modal root is current server state",
        "journal_root": JOURNAL_ROOT,
        "journal_title_component": JOURNAL_TITLE_COMPONENT,
        "width": APPLET_W,
        "height": APPLET_H,
    });
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&metadata)
            .map_err(|error| format!("encode {} metadata: {error}", json_path.display()))?,
    )
    .map_err(|error| format!("write {}: {error}", json_path.display()))
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
fn remember_frame(
    state: &mut CaptureState,
    generation: u64,
    observation: &PaintObservation,
    pixels: &[u32],
) {
    state.last_observation = *observation;
    if observation.journal_paint_hidden {
        state.saw_hidden_root |= observation.modal_root == JOURNAL_ROOT;
        state.hidden_frames.push((generation, pixels.to_vec()));
        if state.quiet_frame.is_none() && observation.modal_root == JOURNAL_ROOT {
            state.quiet_frame = Some(pixels.to_vec());
        }
    } else if observation.modal_root == JOURNAL_ROOT {
        if state.visible_template.is_none() {
            state.visible_template = Some(pixels.to_vec());
        }
        if state.saw_hidden_root {
            state.saw_visible_after_hidden = true;
            if state.restored_frame.is_none() {
                state.restored_frame = Some(pixels.to_vec());
            }
        }
    } else if state.close_requested {
        state.saw_closed_after_click = true;
    }
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
fn roi_diff(
    first: &[u32],
    second: &[u32],
    (x0, x1, y0, y1): (usize, usize, usize, usize),
) -> (usize, usize) {
    let mut changed = 0;
    let mut total = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            total += 1;
            if first[y * APPLET_W as usize + x] != second[y * APPLET_W as usize + x] {
                changed += 1;
            }
        }
    }
    (changed, total)
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
fn update_paint_observation(probe: &PaintProbe, client: &client::client::Client) {
    let mut observation = probe.metadata.lock();
    observation.sequence = observation.sequence.wrapping_add(1);
    observation.modal_root = client.main_modal_id;
    observation.journal_paint_hidden = client.journal_paint_hidden();
    observation.paint_generation = probe.mailbox.generation();
    let _ = probe.observations.try_send(*observation);
}

#[cfg(not(all(windows, feature = "journal-paint-proof")))]
fn update_paint_observation(probe: &PaintProbe, client: &client::client::Client) {
    let mut observation = probe.metadata.lock().expect("journal paint metadata lock");
    observation.sequence = observation.sequence.wrapping_add(1);
    observation.modal_root = client.main_modal_id;
    observation.journal_paint_hidden = client.journal_paint_hidden();
    observation.paint_generation = probe.mailbox.generation();
}

#[cfg(all(windows, feature = "journal-paint-proof"))]
fn capture_label(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum SetupMode {
    Synthetic,
    Cook,
}

#[derive(Default)]
struct SetupState {
    primed: bool,
    logout_after: Option<Instant>,
    logout_sent: bool,
    saw_offline: bool,
    relog_ready: bool,
    journal_capture: bool,
    journal_capture_ready: bool,
    journal_expected_display: Option<String>,
    journal_expected_title: Option<String>,
    journal_last_modal: Option<i32>,
    journal_open_count: u32,
    journal_close_count: u32,
    journal_click_count: u32,
    journal_title_seen: bool,
    journal_titles: Vec<String>,
    journal_title_mismatch: Option<String>,
    open_unowned_journal: bool,
}

/// The live engine is shared by the operator's tunnel.  Keep its profile and
/// cache paths explicit and never let a test resolve `~/.274bot`.
struct ThrowawayHome {
    path: PathBuf,
    previous: Option<String>,
}

impl ThrowawayHome {
    fn enter(label: &str) -> Self {
        let previous = std::env::var("HOME").ok();
        let path = std::env::temp_dir().join(format!(
            "274bot-quester-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("create throwaway HOME");
        std::env::set_var("HOME", &path);
        Self { path, previous }
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn live() -> bool {
    std::env::var("LIVE").as_deref() == Ok("1")
}

fn live_options(home: &Path) -> ProfileOptions {
    let default_port = if cfg!(windows) { 44594 } else { 45594 };
    let default_http_port = if cfg!(windows) { 1080 } else { 2080 };
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default_port);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default_http_port);
    let nav_pack = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .or_else(|| {
            let own = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/nav/289/274bot.navpack");
            own.exists().then_some(own)
        });
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        unpack_dir: Some(home.join("unpack")),
        nav_pack,
        engine_dir: std::env::var_os("BOT_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    }
}

fn account_name() -> String {
    format!(
        "jq{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis() % 1_000_000_000)
            .unwrap_or(0)
    )
}

fn profile(name: &str) -> Profile {
    Profile {
        username: name.to_string(),
        password: name.to_string().into(),
        uid: ACCOUNT_UID,
        settings: ProfileSettings::default(),
    }
}

fn prime(client: &mut client::client::Client, mode: SetupMode) {
    // Tutorial completion must be followed by a relog before the quest tab is
    // trusted.  The quest varp/item cheats only choose a deterministic fixture
    // state; all journal clicks and subsequent actions remain host-owned.
    interact::mainland_hop(client);
    match mode {
        SetupMode::Synthetic => {
            let _ = interact::cheat(client, "setvar runemysteries 3");
            let _ = interact::cheat(client, "give research_package 1");
        }
        SetupMode::Cook => {
            let _ = interact::cheat(client, "~clearinv");
            let _ = interact::cheat(client, "setvar cook 0");
        }
    }
}

fn post_relog(client: &mut client::client::Client, mode: SetupMode) {
    let (x, z) = match mode {
        // Aubury is the real Rune Mysteries branch-3 advance target.
        SetupMode::Synthetic => (3253, 3401),
        SetupMode::Cook => (3209, 3215),
    };
    assert_eq!(
        interact::cheat(client, &interact::tele_args(0, x, z)),
        client::CheatSend::Sent
    );
}

fn normalize_journal_title(title: &str) -> &str {
    title.strip_prefix("@dre@").unwrap_or(title).trim()
}

fn observed_journal_title(snapshot: &GameSnapshot) -> Option<String> {
    snapshot
        .widgets()
        .iter()
        .find(|widget| {
            widget.root_component_id == JOURNAL_ROOT_R289
                && widget.component_id == JOURNAL_TITLE_COMPONENT_R289
                && !widget.hidden
        })
        .and_then(|widget| widget.text.clone())
}

fn observe_journal(client: &mut client::client::Client, state: &mut SetupState) {
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let modal = snapshot.modals().main;
    if state.journal_last_modal != Some(modal) {
        if modal == JOURNAL_ROOT_R289 && state.journal_last_modal != Some(JOURNAL_ROOT_R289) {
            state.journal_open_count += 1;
        }
        if state.journal_last_modal == Some(JOURNAL_ROOT_R289) && modal != JOURNAL_ROOT_R289 {
            state.journal_close_count += 1;
        }
        state.journal_last_modal = Some(modal);
    }
    if modal == JOURNAL_ROOT_R289 {
        if let Some(title) = observed_journal_title(&snapshot) {
            state.journal_title_seen = true;
            if state.journal_titles.last() != Some(&title) {
                state.journal_titles.push(title.clone());
            }
            if state
                .journal_expected_title
                .as_deref()
                .is_some_and(|expected| normalize_journal_title(&title) != expected)
            {
                state.journal_title_mismatch = Some(title);
            }
        }
    }
    let row_component = client.menu_param_c.first().copied();
    if client.menu_action.first().copied() == Some(MiniMenuAction::IF_BUTTON) {
        if let (Some(component), Some(display)) =
            (row_component, state.journal_expected_display.as_deref())
        {
            let journal_row = snapshot
                .quest_statuses()
                .iter()
                .any(|row| row.component_id == component && row.name.eq_ignore_ascii_case(display));
            if journal_row {
                state.journal_click_count += 1;
                // `doAction` already consumed this command. Clear only the
                // component slot so a later identical click is observable
                // without collecting or injecting any production traffic.
                if let Some(component) = client.menu_param_c.get_mut(0) {
                    *component = -1;
                }
            }
        }
    }
    state.journal_capture_ready = true;
}

#[derive(Debug, Clone)]
struct JournalProbe {
    capture_ready: bool,
    main_modal: Option<i32>,
    opens: u32,
    closes: u32,
    clicks: u32,
    title_seen: bool,
    titles: Vec<String>,
    title_mismatch: Option<String>,
}

fn arm_journal_capture(state: &Arc<Mutex<SetupState>>, display: &str, title: &str) {
    let mut state = state.lock().expect("journal capture state");
    state.journal_capture = true;
    state.journal_capture_ready = false;
    state.journal_expected_display = Some(display.to_string());
    state.journal_expected_title = Some(title.to_string());
    state.journal_last_modal = None;
    state.journal_open_count = 0;
    state.journal_close_count = 0;
    state.journal_click_count = 0;
    state.journal_title_seen = false;
    state.journal_titles.clear();
    state.journal_title_mismatch = None;
}

fn journal_probe(state: &Arc<Mutex<SetupState>>) -> JournalProbe {
    let state = state.lock().expect("journal capture state");
    JournalProbe {
        capture_ready: state.journal_capture_ready,
        main_modal: state.journal_last_modal,
        opens: state.journal_open_count,
        closes: state.journal_close_count,
        clicks: state.journal_click_count,
        title_seen: state.journal_title_seen,
        titles: state.journal_titles.clone(),
        title_mismatch: state.journal_title_mismatch.clone(),
    }
}

fn assert_exact_journal_titles(probe: &JournalProbe, expected: &str, label: &str) {
    assert!(probe.title_seen, "{label} never exposed the journal title");
    assert!(
        probe.title_mismatch.is_none(),
        "{label} observed a foreign journal title: {:?}",
        probe.title_mismatch
    );
    assert!(
        probe
            .titles
            .iter()
            .all(|title| normalize_journal_title(title) == expected),
        "{label} observed unexpected journal titles: {:?}",
        probe.titles
    );
}

fn wait_until_fast(label: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out after {timeout:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_journal_open(
    state: &Arc<Mutex<SetupState>>,
    expected_title: &str,
    label: &str,
) -> JournalProbe {
    wait_until_fast(label, Duration::from_secs(20), || {
        let probe = journal_probe(state);
        probe.capture_ready && probe.main_modal == Some(JOURNAL_ROOT_R289) && probe.title_seen
    });
    wait_until_fast("journal row click capture", Duration::from_secs(2), || {
        journal_probe(state).clicks >= 1
    });
    let probe = journal_probe(state);
    assert_exact_journal_titles(&probe, expected_title, label);
    assert_eq!(
        probe.main_modal,
        Some(JOURNAL_ROOT_R289),
        "{label} must interrupt while the owned journal is open"
    );
    probe
}

fn frame_hook(
    state: Arc<Mutex<SetupState>>,
    mode: SetupMode,
    metadata: Option<PaintProbe>,
) -> impl Fn(&mut client::client::Client, &str, bool) + Send + Sync + 'static {
    move |client, _name, _hold| {
        if let Some(metadata) = &metadata {
            client.set_draw(true);
            update_paint_observation(metadata, client);
        }
        let now = Instant::now();
        let mut state = state.lock().expect("quester live setup lock");
        if !client.ingame {
            if state.logout_sent {
                state.saw_offline = true;
            }
            return;
        }
        if client.scene_state != 2 {
            return;
        }
        if state.journal_capture {
            observe_journal(client, &mut state);
        }
        if !state.primed {
            prime(client, mode);
            state.primed = true;
            state.logout_after = Some(now + Duration::from_secs(2));
            return;
        }
        if !state.logout_sent && state.logout_after.is_some_and(|deadline| now >= deadline) {
            let ifaces = Arc::clone(&client.ifaces);
            state.logout_sent = interact::logout(client, &ifaces);
            return;
        }
        if state.logout_sent && state.saw_offline && !state.relog_ready {
            post_relog(client, mode);
            state.relog_ready = true;
        }
        if state.open_unowned_journal && !client.journal_paint_hidden() {
            assert!(
                interact::press(client, JOURNAL_BUTTON),
                "unowned Rune Mysteries journal button must be accepted"
            );
            state.open_unowned_journal = false;
        }
    }
}

fn launch_live(
    mode: SetupMode,
    label: &str,
) -> (
    ThrowawayHome,
    super::Play,
    Arc<Mutex<SetupState>>,
    String,
    Option<CaptureHandle>,
) {
    let home = ThrowawayHome::enter(label);
    let options = live_options(&home.path);
    let server_profile = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .bind()
        .expect("bind local-289 profile");
    let template =
        SharedClientTemplate::load(Arc::clone(&server_profile)).expect("load live template");
    let name = account_name();
    let state = Arc::new(Mutex::new(SetupState::default()));
    let capture = CaptureHandle::from_env(label);
    let metadata = capture.as_ref().and_then(CaptureHandle::probe);
    let slots = capture
        .as_ref()
        .map(CaptureHandle::slots)
        .unwrap_or((None, None));
    let hook = Arc::clone(&state);
    let play = run_with_template(
        template,
        false,
        vec![profile(name.as_str())],
        move |_| (slots.0.clone(), slots.1.clone()),
        frame_hook(hook, mode, metadata),
    )
    .expect("start live Quester Play");
    (home, play, state, name, capture)
}

fn wait_until(label: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out after {timeout:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_relogged(play: &super::Play, state: &Arc<Mutex<SetupState>>) {
    wait_until("live account relog", Duration::from_secs(90), || {
        state.lock().expect("setup state").relog_ready
            && play.statuses().iter().any(|status| {
                status.username.starts_with("jq") && status.ingame && status.scene_state == 2
            })
    });
}

fn field<'a>(status: &'a ScriptStatus, key: &str) -> &'a StatusValue {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
        .unwrap_or_else(|| panic!("status {key:?} missing from {status:?}"))
}

fn text<'a>(status: &'a ScriptStatus, key: &str) -> &'a str {
    match field(status, key) {
        StatusValue::Text(value) => value,
        other => panic!("status {key:?} is not Text: {other:?}"),
    }
}

fn truth(status: &ScriptStatus, key: &str) -> Truth {
    match field(status, key) {
        StatusValue::Truth(value) => *value,
        other => panic!("status {key:?} is not Truth: {other:?}"),
    }
}

fn progress_evidence(status: &ScriptStatus, key: &str) -> EvidenceStamp {
    match field(status, key) {
        StatusValue::Quest(progress) => progress.evidence,
        other => panic!("status {key:?} is not Quest progress: {other:?}"),
    }
}

fn wait_status(
    play: &super::Play,
    name: &str,
    timeout: Duration,
    label: &str,
    mut predicate: impl FnMut(&ScriptStatus) -> bool,
) -> Arc<ScriptStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = play.script_native_status(name) {
            if predicate(&status) {
                return status;
            }
            assert_ne!(
                status.phase,
                NativePhase::Blocked,
                "{label} blocked: {status:#?}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out; status={:?}",
            play.script_native_status(name)
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_read_journal_active(
    play: &super::Play,
    name: &str,
    target: api::selected::RunKey,
    expected_lines: &str,
) -> Arc<ScriptStatus> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = play.script_native_status(name) {
            if status.run == target
                && matches!(status.phase, NativePhase::Waiting | NativePhase::Working)
                && truth(&status, "needs_read") == Truth::True
                && text(&status, "journal_lines") == expected_lines
            {
                return status;
            }
        }
        assert!(
            Instant::now() < deadline,
            "ReadJournal never exposed a pending Waiting/Working status; status={:?}",
            play.script_native_status(name)
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_test_start(handle: &ScriptStartHandle, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match handle.poll_start(name) {
            script::StartPoll::Pending => {}
            script::StartPoll::Settled(script::StartOutcome::Ready) => return,
            script::StartPoll::Settled(outcome) => panic!("Quester Start failed: {outcome:?}"),
            script::StartPoll::NotOwed => panic!("Quester Start outcome was lost"),
        }
        assert!(Instant::now() < deadline, "Quester Start did not settle");
        thread::sleep(Duration::from_millis(50));
    }
}

fn synthetic_document(no_match: bool) -> PathDocument {
    let mut document = decode_cook().expect("decode checked Cook Path");
    document.id = FactKey::new(SYNTHETIC_QUEST);
    document.display_name = "Synthetic Rune Mysteries journal".into();
    let role = &mut document.roles[0];
    role.progress_binding = FactKey::new("journal:runemysteries");

    let template = role.sequences[0].clone();
    let stages = ["rm:0", "rm:1", "rm:2", "rm:3", "rm:4", "rm:5", "rm:6"];
    role.sequences = stages
        .iter()
        .enumerate()
        .map(|(index, stage)| {
            let mut sequence = SequenceDocument {
                stage: FactKey::new(stage),
                required: Vec::new(),
                terminal: *stage == "rm:6",
                recovery_entry: None,
                steps: Vec::new(),
            };
            if !sequence.terminal {
                let mut step = template.steps[0].clone();
                step.id = FactKey::new(&format!("synthetic-rm-step-{index}"));
                step.advances = *stage == "rm:3";
                step.skip_if = PredicateDocument::Any(Vec::new());
                if *stage == "rm:3" {
                    // This is the real content branch: at varp 3 the journal
                    // asks for the Research Package at Aubury. The host
                    // dialogue machine performs the talk; it is not faked by
                    // changing the journal/status wire.
                    step.args = serde_json::json!({
                        "npc": "aubury",
                        "leash": 8,
                        "prefer": ["I have been sent here with a package for you."],
                    });
                    step.settle = PredicateDocument::Fact {
                        kind: "stage_in".into(),
                        version: 1,
                        args: serde_json::json!({
                            "quest": SYNTHETIC_QUEST,
                            "any": ["rm:4"],
                        }),
                    };
                } else {
                    // Keep every fixture branch other than the exercised
                    // Aubury branch passive. In particular, do not inherit a
                    // Cook Talk action that could emit unrelated wire input.
                    step.kind = "wait".into();
                    step.version = 1;
                    step.args = serde_json::json!({
                        "until": {
                            "Fact": {
                                "kind": "quest_colour",
                                "version": 1,
                                "args": {
                                    "quest": SYNTHETIC_QUEST,
                                    "is": "complete",
                                },
                            },
                        },
                        "max_ticks": 1000,
                    });
                    step.settle = PredicateDocument::Fact {
                        kind: "stage_in".into(),
                        version: 1,
                        args: serde_json::json!({
                            "quest": SYNTHETIC_QUEST,
                            "any": [format!("rm:{}", index + 1)],
                        }),
                    };
                }
                sequence.steps.push(step);
            }
            sequence
        })
        .collect();
    // These branches mirror content/scripts/quests/quest_runemysteries/scripts/
    // runemysteries_journal.rs2 (R289): varps 1..5 are the authored body
    // branches and the final else branch is the completed journal.

    let rules = if no_match {
        (1..=6)
            .rev()
            .map(|stage| ProgressRuleDocument {
                stage: FactKey::new(&format!("rm:{stage}")),
                all: vec!["fixture line that is never emitted".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(stage),
            })
            .collect()
    } else {
        vec![
            ProgressRuleDocument {
                stage: FactKey::new("rm:6"),
                all: vec!["quest complete".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(6),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:5"),
                all: vec!["aubury was interested".into(), "research notes".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(5),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:4"),
                all: vec!["took the research package".into(), "delivered it".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(4),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:3"),
                all: vec!["research package".into(), "aubury".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(3),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:2"),
                all: vec!["gave the talisman".into(), "head wizard".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(2),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:1"),
                all: vec!["spoke to duke horacio".into(), "strange".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(1),
            },
        ]
    };
    role.progress = Some(ProgressDocument {
        colour: ProgressColourDocument {
            not_started: FactKey::new("rm:0"),
            in_progress: FactKey::new("rm:1"),
            complete: FactKey::new("rm:6"),
        },
        rules,
        flags: vec![ProgressFlagDocument {
            flag: FactKey::new("rm:research-package"),
            all: vec!["research package".into()],
            any: Vec::new(),
            count: None,
        }],
        monotonic: false,
    });
    document
}

fn compile_synthetic(
    selected: &api::game_data::SelectedGameData,
    quests: &QuestCatalog,
    no_match: bool,
) -> Arc<script::quester::compile::CompiledPath> {
    let path = compile_uncached_for_test(&synthetic_document(no_match), selected, quests)
        .expect("compile synthetic Rune Mysteries Path");
    assert!(
        path.progress.rules.len() >= 3,
        "fixture must compile at least three progress rules"
    );
    println!(
        "synthetic compiled progress rules (no_match={no_match}): {}",
        path.progress.rules.len()
    );
    path
}

fn start_synthetic(
    handle: &ScriptStartHandle,
    play: &super::Play,
    name: &str,
    path: Arc<script::quester::compile::CompiledPath>,
    quests: &Arc<QuestCatalog>,
    selected: &Arc<api::game_data::SelectedGameData>,
) -> api::selected::RunKey {
    let run = handle
        .start_test_script(
            name,
            Box::new(Quester::new(
                api::selected::RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                path,
                Arc::clone(quests),
            )),
            Some(Arc::clone(selected)),
        )
        .expect("install synthetic Quester");
    assert_eq!(
        run,
        play.script_native_run(name)
            .expect("synthetic run key after install")
    );
    play.wake(name);
    run
}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_journal_synthetic_runemysteries() {
    assert!(
        live(),
        "live_quester_journal_synthetic_runemysteries requires LIVE=1"
    );
    let (home, play, setup, name, capture) = launch_live(SetupMode::Synthetic, "journal");
    wait_relogged(&play, &setup);

    let selected = api::game_data::for_revision(ClientRevision::R289).expect("R289 game data");
    let quests =
        Arc::new(QuestCatalog::from_identity(selected.quest_identity()).expect("quest catalog"));
    let path = compile_synthetic(&selected, &quests, false);
    let restart_path = Arc::clone(&path);
    let no_match_path = compile_synthetic(&selected, &quests, true);
    let handle = play.script_start_handle();
    let mut run = handle
        .start_test_script(
            &name,
            Box::new(Quester::new(
                api::selected::RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                path,
                Arc::clone(&quests),
            )),
            Some(Arc::clone(&selected)),
        )
        .expect("install synthetic Quester");
    assert_eq!(
        run,
        play.script_native_run(&name).expect("synthetic run key")
    );
    play.wake(&name);
    if let Some(capture) = capture.as_ref() {
        wait_until(
            "headed initial synthetic journal read",
            SYNTHETIC_TIMEOUT,
            || {
                capture.saw_hidden_root()
                    && play
                        .script_native_status(&name)
                        .is_some_and(|status| truth(&status, "needs_read") == Truth::True)
            },
        );
        eprintln!(
            "journal-proof-stop event=requested run={run:?} unix_ns={}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("journal proof clock")
                .as_nanos()
        );
        assert!(
            play.script_native_stop(&name, run),
            "forced Stop must revoke the initial synthetic journal read"
        );
        eprintln!(
            "journal-proof-stop event=revoked run={run:?} unix_ns={}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("journal proof clock")
                .as_nanos()
        );
        wait_until(
            "forced synthetic read Stop",
            Duration::from_secs(20),
            || handle.idle(&name),
        );
        wait_until(
            "normal paint after forced Stop",
            Duration::from_secs(20),
            || capture.saw_visible_after_hidden() || capture.normal_closed_paint(),
        );
        if !capture.saw_visible_after_hidden() {
            // Stop can leave the root open or close it. Reopen only when
            // needed, through the real client button path without a reader.
            setup
                .lock()
                .expect("quester live setup lock")
                .open_unowned_journal = true;
            play.wake(&name);
        }
        wait_until(
            "normal visible journal paint after forced Stop",
            Duration::from_secs(20),
            || capture.saw_visible_after_hidden(),
        );
        capture.click(JOURNAL_CLOSE_X, JOURNAL_CLOSE_Y);
        play.wake(&name);
        wait_until(
            "close unowned synthetic journal modal",
            Duration::from_secs(20),
            || capture.saw_closed_after_click(),
        );
        run = handle
            .start_test_script(
                &name,
                Box::new(Quester::new(
                    api::selected::RunKey {
                        slot: 0,
                        run: 0,
                        session: 0,
                    },
                    restart_path,
                    Arc::clone(&quests),
                )),
                Some(Arc::clone(&selected)),
            )
            .expect("reinstall synthetic Quester after forced read cancellation");
        play.wake(&name);
    }

    let seeded = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "seeded Rune Mysteries journal stage",
        |status| {
            status.phase == NativePhase::Working
                && text(status, "stage") == "rm:3"
                && text(status, "rule") == "rm:3"
                && text(status, "journal_lines").contains("Research Package")
                && truth(status, "needs_read") == Truth::False
        },
    );
    assert!(matches!(
        field(&seeded, "varp_hint"),
        StatusValue::Integer(3)
    ));
    match field(&seeded, "progress") {
        StatusValue::Quest(progress) => {
            assert!(
                progress.signals.is_empty(),
                "readable journal must not invent signals"
            );
        }
        other => panic!("progress status is not Quest: {other:?}"),
    }
    println!(
        "synthetic seeded journal status: run={:?} raw={:?} status={seeded:?}",
        seeded.run,
        text(&seeded, "journal_lines")
    );

    let advanced = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "Rune Mysteries branch advance and fresh journal settle",
        |status| {
            status.phase == NativePhase::Working
                && text(status, "stage") == "rm:4"
                && text(status, "rule") == "rm:4"
                && text(status, "journal_lines").contains("delivered it")
                && truth(status, "needs_read") == Truth::False
        },
    );
    assert!(matches!(
        field(&advanced, "varp_hint"),
        StatusValue::Integer(4)
    ));
    match field(&advanced, "progress") {
        StatusValue::Quest(progress) => assert!(progress.signals.is_empty()),
        other => panic!("progress status is not Quest: {other:?}"),
    }
    println!(
        "synthetic advanced journal status: run={:?} raw={:?} status={advanced:?}",
        advanced.run,
        text(&advanced, "journal_lines")
    );

    // Replace the card with the same real journal machine but rules that do
    // not match any Rune Mysteries body. The machine must retain the raw read,
    // publish `unknown`, carry no invented signal ranges, and park.
    assert!(play.script_native_stop(&name, run), "stop synthetic run");
    wait_until("synthetic Quester stop", Duration::from_secs(20), || {
        handle.idle(&name)
    });
    let no_match_run = handle
        .start_test_script(
            &name,
            Box::new(Quester::new(
                api::selected::RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                no_match_path,
                Arc::clone(&quests),
            )),
            Some(selected),
        )
        .expect("install no-match Quester");
    play.wake(&name);
    let parked = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "no-match journal parking",
        |status| {
            status.phase == NativePhase::Blocked
                && text(status, "rule") == "unknown"
                && text(status, "journal_lines") == text(&advanced, "journal_lines")
        },
    );
    assert_eq!(parked.run, no_match_run);
    assert!(matches!(
        field(&parked, "needs_read"),
        StatusValue::Truth(Truth::True)
    ));
    match field(&parked, "progress") {
        StatusValue::Quest(progress) => assert!(progress.signals.is_empty()),
        other => panic!("no-match progress is not Quest: {other:?}"),
    }
    println!(
        "synthetic no-match journal status: run={:?} raw={:?} status={parked:?}",
        parked.run,
        text(&parked, "journal_lines")
    );

    // Read now is a real retry command for a parked run. It must expose a
    // pending native read first, then publish a later no-match proof and park
    // again; an Ok response that leaves the slot Blocked is not sufficient.
    let parked_lines = text(&parked, "journal_lines").to_string();
    let parked_evidence = progress_evidence(&parked, "progress");
    assert!(
        play.script_native_read_journal(&name, no_match_run).is_ok(),
        "ReadJournal must accept the current parked native run"
    );
    let active_read = wait_read_journal_active(&play, &name, no_match_run, &parked_lines);
    assert_eq!(active_read.run, no_match_run);
    let reread = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "no-match ReadJournal re-parking",
        |status| {
            status.phase == NativePhase::Blocked
                && text(status, "rule") == "unknown"
                && text(status, "journal_lines") == parked_lines
                && {
                    let evidence = progress_evidence(status, "progress");
                    evidence.run == parked_evidence.run
                        && (evidence.tick, evidence.sequence)
                            > (parked_evidence.tick, parked_evidence.sequence)
                }
        },
    );
    assert_eq!(reread.run, no_match_run);
    assert_eq!(truth(&reread, "needs_read"), Truth::True);
    match field(&reread, "progress") {
        StatusValue::Quest(progress) => assert!(progress.signals.is_empty()),
        other => panic!("re-read progress is not Quest: {other:?}"),
    }
    println!(
        "synthetic no-match ReadJournal re-read status: run={:?} raw={:?} status={reread:?}",
        reread.run,
        text(&reread, "journal_lines")
    );

    if let Some(capture) = capture.as_ref() {
        wait_until(
            "normal closed journal paint after no-match",
            Duration::from_secs(20),
            || capture.normal_closed_paint(),
        );
        capture.assert_no_flash();
    }
    drop(play);
    drop(home);
    if let Some(capture) = capture {
        capture.finish();
    }
}

struct SyntheticLive {
    home: ThrowawayHome,
    play: super::Play,
    setup: Arc<Mutex<SetupState>>,
    name: String,
    selected: Arc<api::game_data::SelectedGameData>,
    quests: Arc<QuestCatalog>,
    path: Arc<script::quester::compile::CompiledPath>,
    expected_title: String,
}

fn prepare_synthetic_live(label: &str) -> SyntheticLive {
    let (home, play, setup, name, _capture) = launch_live(SetupMode::Synthetic, label);
    wait_relogged(&play, &setup);
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("R289 game data");
    let quests =
        Arc::new(QuestCatalog::from_identity(selected.quest_identity()).expect("quest catalog"));
    let facts = quests
        .quest(SYNTHETIC_QUEST)
        .expect("Rune Mysteries catalog row");
    let expected_display = facts.display.to_string();
    let expected_title = facts
        .journal_title
        .as_deref()
        .expect("Rune Mysteries journal title")
        .to_string();
    arm_journal_capture(&setup, &expected_display, &expected_title);
    wait_until_fast("journal capture hook", Duration::from_secs(5), || {
        journal_probe(&setup).capture_ready
    });
    let mut path = compile_synthetic(&selected, &quests, false);
    // The lifecycle fixture ends at package delivery, not quest completion.
    let delivered = Arc::get_mut(&mut path)
        .expect("uncached lifecycle fixture")
        .sequences
        .iter_mut()
        .find(|sequence| sequence.stage.0.as_ref() == "rm:4")
        .expect("delivery sequence");
    delivered.terminal = true;
    delivered.steps.clear();
    SyntheticLive {
        home,
        play,
        setup,
        name,
        selected,
        quests,
        path,
        expected_title,
    }
}

fn wait_recovered_synthetic(
    play: &super::Play,
    name: &str,
    state: &Arc<Mutex<SetupState>>,
    run: api::selected::RunKey,
    baseline: &JournalProbe,
    expected_title: &str,
    label: &str,
) -> Arc<ScriptStatus> {
    let recovered = wait_status(play, name, SYNTHETIC_TIMEOUT, label, |status| {
        status.phase == NativePhase::Working
            && status.run == run
            && text(status, "stage") == "rm:3"
            && text(status, "rule") == "rm:3"
            && text(status, "journal_lines").contains("Research Package")
            && truth(status, "needs_read") == Truth::False
    });
    let after = journal_probe(state);
    assert_exact_journal_titles(&after, expected_title, label);
    assert!(
        after.closes > baseline.closes,
        "{label} must observe the recovered journal close: before={baseline:?} after={after:?}"
    );
    let click_delta = after.clicks.saturating_sub(baseline.clicks);
    assert!(
        click_delta <= 1,
        "{label} emitted more than one recovery journal click: before={baseline:?} after={after:?}"
    );
    assert_eq!(
        after.opens,
        baseline.opens + click_delta,
        "{label} must either adopt the retained page or freshly open exactly one page"
    );
    assert_eq!(
        after.clicks,
        baseline.clicks + click_delta,
        "{label} click accounting must match adoption/fresh-open accounting"
    );
    recovered
}

#[derive(Default)]
struct JournalDebugLog {
    state: Mutex<Option<Arc<Mutex<SetupState>>>>,
    records: Mutex<Vec<(String, String, Option<i32>)>>,
}

impl api::hostlog::Sink for JournalDebugLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.starts_with("debug ::") {
            let modal = self
                .state
                .lock()
                .expect("debug journal state")
                .as_ref()
                .and_then(|state| {
                    state
                        .lock()
                        .expect("journal capture state")
                        .journal_last_modal
                });
            println!(
                "{} {} modal={modal:?}",
                record.slot.unwrap_or("process"),
                record.message
            );
            self.records.lock().expect("debug journal records").push((
                record.slot.unwrap_or_default().to_string(),
                record.message.to_string(),
                modal,
            ));
        }
    }
}

static JOURNAL_DEBUG_LOG: std::sync::LazyLock<JournalDebugLog> =
    std::sync::LazyLock::new(JournalDebugLog::default);

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_journal_debug_reply_does_not_take_modal_ownership() {
    assert!(live(), "journal DebugPanel proof requires LIVE=1");
    let SyntheticLive {
        home,
        play,
        setup,
        name,
        selected,
        quests,
        path,
        expected_title,
    } = prepare_synthetic_live("journal-debug");
    let log = &*JOURNAL_DEBUG_LOG;
    *log.state.lock().expect("debug journal state") = Some(Arc::clone(&setup));
    assert!(api::hostlog::install_sink(log));
    let handle = play.script_start_handle();
    let run = start_synthetic(&handle, &play, &name, path, &quests, &selected);
    let opened = wait_journal_open(&setup, &expected_title, "DebugPanel during journal read");
    // Use the same host queue and reply tracker as the DebugPanel, not a
    // fixture-side client cheat. getcoord emits chat without replacing a modal.
    play.cheat(&name, "getcoord")
        .expect("queue DebugPanel command");
    wait_until_fast(
        "DebugPanel journal send and chat reply",
        Duration::from_secs(20),
        || {
            let records = log.records.lock().expect("debug journal records");
            records.iter().any(|(slot, message, modal)| {
                slot == &name
                    && message.starts_with("debug ::getcoord: sent;")
                    && *modal == Some(JOURNAL_ROOT_R289)
            }) && records.iter().any(|(slot, message, _)| {
                slot == &name && message.starts_with("debug ::[getcoord]: chat reply candidate:")
            })
        },
    );
    let advanced = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "DebugPanel overlap fresh journal advancement",
        |status| {
            status.run == run
                && text(status, "stage") == "rm:4"
                && text(status, "rule") == "rm:4"
                && text(status, "journal_lines").contains("delivered it")
                && truth(status, "needs_read") == Truth::False
        },
    );
    let after = journal_probe(&setup);
    assert_exact_journal_titles(&after, &expected_title, "DebugPanel overlap");
    assert_eq!(
        after.clicks,
        opened.clicks + 1,
        "one fresh advancement read"
    );
    assert_eq!(
        after.opens,
        opened.opens + 1,
        "reply tracker must not reopen the journal"
    );
    assert_eq!(
        after.closes,
        opened.closes + 2,
        "only the two owned reads close"
    );
    assert!(!text(&advanced, "journal_lines").contains("getcoord"));
    wait_until(
        "DebugPanel overlap completion",
        Duration::from_secs(20),
        || {
            play.script_lifecycle_receipt(&name)
                .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
        },
    );
    println!("DebugPanel overlap journal proof: {after:?}; status={advanced:?}");
    assert!(handle.idle(&name));
    drop(play);
    *log.state.lock().expect("debug journal state") = None;
    drop(home);
}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_journal_stop_start_recovers_stranded_page() {
    assert!(
        live(),
        "live_quester_journal_stop_start_recovers_stranded_page requires LIVE=1"
    );
    let SyntheticLive {
        home,
        play,
        setup,
        name,
        selected,
        quests,
        path,
        expected_title,
    } = prepare_synthetic_live("journal-stop");
    let handle = play.script_start_handle();
    let first_run = start_synthetic(&handle, &play, &name, Arc::clone(&path), &quests, &selected);
    let _opened = wait_journal_open(&setup, &expected_title, "Stop mid-read journal capture");
    assert!(
        play.script_native_stop(&name, first_run),
        "stop the Quester while root 8134 is still open"
    );
    wait_until("Stop mid-read idle", Duration::from_secs(20), || {
        handle.idle(&name)
    });
    let baseline = journal_probe(&setup);
    assert_exact_journal_titles(&baseline, &expected_title, "Stop stranded page");
    println!("Stop mid-read stranded journal probe: {baseline:?}");

    let restarted = start_synthetic(&handle, &play, &name, Arc::clone(&path), &quests, &selected);
    let recovered = wait_recovered_synthetic(
        &play,
        &name,
        &setup,
        restarted,
        &baseline,
        &expected_title,
        "Stop then Start journal recovery",
    );
    let recovered_probe = journal_probe(&setup);
    let advanced = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "Stop recovery Rune Mysteries advancement",
        |status| {
            status.phase == NativePhase::Working
                && status.run == restarted
                && text(status, "stage") == "rm:4"
                && text(status, "rule") == "rm:4"
                && text(status, "journal_lines").contains("delivered it")
                && truth(status, "needs_read") == Truth::False
        },
    );
    wait_until_fast(
        "Stop recovery fresh journal click",
        Duration::from_secs(2),
        || journal_probe(&setup).clicks > recovered_probe.clicks,
    );
    let advanced_probe = journal_probe(&setup);
    assert_exact_journal_titles(
        &advanced_probe,
        &expected_title,
        "Stop recovery advanced journal",
    );
    assert_eq!(
        advanced_probe.clicks,
        recovered_probe.clicks + 1,
        "advancement must perform one fresh journal click after recovery"
    );
    assert!(
        advanced_probe.closes > recovered_probe.closes,
        "advancement must close its fresh journal read"
    );
    assert_eq!(text(&advanced, "stage"), "rm:4");
    println!(
        "Stop recovery advanced journal status: run={:?} raw={:?} status={advanced:?}",
        advanced.run,
        text(&advanced, "journal_lines")
    );
    wait_until("Stop recovery completion", Duration::from_secs(20), || {
        play.script_lifecycle_receipt(&name)
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
    });
    println!(
        "Stop recovery terminal receipt: {:?}",
        play.script_lifecycle_receipt(&name)
            .expect("completed recovery")
    );
    assert!(handle.idle(&name));
    drop(play);
    drop(home);
    let _ = recovered;
}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_journal_pause_resume_recovers_stranded_page() {
    assert!(
        live(),
        "live_quester_journal_pause_resume_recovers_stranded_page requires LIVE=1"
    );
    let SyntheticLive {
        home,
        play,
        setup,
        name,
        selected,
        quests,
        path,
        expected_title,
    } = prepare_synthetic_live("journal-pause");
    let handle = play.script_start_handle();
    let run = start_synthetic(&handle, &play, &name, Arc::clone(&path), &quests, &selected);
    let _opened = wait_journal_open(&setup, &expected_title, "Pause mid-read journal capture");
    assert!(
        play.script_native_pause(&name, run, true),
        "pause the Quester while root 8134 is still open"
    );
    wait_until_fast("Pause mid-read paused", Duration::from_secs(10), || {
        handle.run_state(&name) == script::RunState::Paused
    });
    let baseline = journal_probe(&setup);
    assert_exact_journal_titles(&baseline, &expected_title, "Pause stranded page");
    println!("Pause mid-read stranded journal probe: {baseline:?}");
    assert!(
        play.script_native_pause(&name, run, false),
        "resume the paused Quester"
    );

    let recovered = wait_recovered_synthetic(
        &play,
        &name,
        &setup,
        run,
        &baseline,
        &expected_title,
        "Pause then Resume journal recovery",
    );
    let recovered_probe = journal_probe(&setup);
    let advanced = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "Pause recovery Rune Mysteries advancement",
        |status| {
            status.phase == NativePhase::Working
                && status.run == run
                && text(status, "stage") == "rm:4"
                && text(status, "rule") == "rm:4"
                && text(status, "journal_lines").contains("delivered it")
                && truth(status, "needs_read") == Truth::False
        },
    );
    wait_until_fast(
        "Pause recovery fresh journal click",
        Duration::from_secs(2),
        || journal_probe(&setup).clicks > recovered_probe.clicks,
    );
    let advanced_probe = journal_probe(&setup);
    assert_exact_journal_titles(
        &advanced_probe,
        &expected_title,
        "Pause recovery advanced journal",
    );
    assert_eq!(
        advanced_probe.clicks,
        recovered_probe.clicks + 1,
        "advancement must perform one fresh journal click after recovery"
    );
    assert!(
        advanced_probe.closes > recovered_probe.closes,
        "advancement must close its fresh journal read"
    );
    assert_eq!(text(&advanced, "stage"), "rm:4");
    println!(
        "Pause recovery advanced journal status: run={:?} raw={:?} status={advanced:?}",
        advanced.run,
        text(&advanced, "journal_lines")
    );
    wait_until("Pause recovery completion", Duration::from_secs(20), || {
        play.script_lifecycle_receipt(&name)
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
    });
    println!(
        "Pause recovery terminal receipt: {:?}",
        play.script_lifecycle_receipt(&name)
            .expect("completed recovery")
    );
    assert!(handle.idle(&name));
    drop(play);
    drop(home);
    let _ = recovered;

}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_cook_reaches_complete() {
    assert!(live(), "live_quester_cook_reaches_complete requires LIVE=1");
    let (home, play, setup, name, capture) = launch_live(SetupMode::Cook, "cook");
    wait_relogged(&play, &setup);

    let handle = play.script_start_handle();
    play.script_start(&name, script::CompiledId("Quester"), serde_json::Map::new())
        .expect("start released Cook Quester card");
    wait_test_start(&handle, &name);
    wait_until("fresh Cook's Assistant completion", COOK_TIMEOUT, || {
        play.script_lifecycle_receipt(&name)
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
    });
    let receipt = play
        .script_lifecycle_receipt(&name)
        .expect("completed Cook receipt");
    println!("Cook terminal receipt: {receipt:?}");
    assert_eq!(receipt.state, script::ScriptTerminalState::Completed);
    assert_eq!(handle.run_state(&name), script::RunState::Idle);
    drop(play);
    drop(home);
    if let Some(capture) = capture {
        capture.finish();
    }
}
