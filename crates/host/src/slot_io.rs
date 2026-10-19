use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::native_input::{NativeInputAuthority, NativeInputPermit};

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(windows)]
use std::net::TcpStream;
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};

use client::client::{present::pack_rgb, GameShell, APPLET_H, APPLET_W};
use client::render::backend::FrameOutput;

use crate::render::{HeadIdentity, RenderRefusal};

/// OS socket handle used by the idle park (`poll` / `WSAPoll`).
#[cfg(unix)]
pub(crate) type WaitHandle = RawFd;
#[cfg(windows)]
pub(crate) type WaitHandle = RawSocket;

/// Maximum handles one wait/park may observe (control + client socket).
/// Fixed so wait/park stay stack-only — no heap per idle park.
pub(crate) const MAX_WAIT_HANDLES: usize = 2;

/// Wait until any of `handles` is readable (data, hangup, or error), or
/// `timeout` elapses. Only the first `handles.len()` entries of the returned
/// stack array are meaningful. Empty `handles` is a pure sleep (all false).
///
/// Stack-only: fixed `[pollfd|WSAPOLLFD; MAX_WAIT_HANDLES]` + `[bool; N]`.
/// Unix: `poll(2)`. Windows: `WSAPoll` on pointer-width `SOCKET` values —
/// never truncates to 32-bit fds and never busy-spins.
#[allow(unsafe_code)]
pub(crate) fn wait_readable(handles: &[WaitHandle], timeout: Duration) -> [bool; MAX_WAIT_HANDLES] {
    debug_assert!(
        handles.len() <= MAX_WAIT_HANDLES,
        "wait_readable supports at most {MAX_WAIT_HANDLES} handles"
    );
    let n = handles.len().min(MAX_WAIT_HANDLES);
    let mut out = [false; MAX_WAIT_HANDLES];
    if n == 0 {
        std::thread::sleep(timeout);
        return out;
    }
    let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);

    #[cfg(unix)]
    {
        let mut fds = [libc::pollfd {
            fd: -1,
            events: 0,
            revents: 0,
        }; MAX_WAIT_HANDLES];
        for i in 0..n {
            fds[i] = libc::pollfd {
                fd: handles[i],
                events: libc::POLLIN,
                revents: 0,
            };
        }
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), n as libc::nfds_t, ms) };
        if rc > 0 {
            let mask = libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;
            for i in 0..n {
                out[i] = fds[i].revents & mask != 0;
            }
        }
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock::{
            WSAPoll, POLLERR, POLLHUP, POLLIN, POLLNVAL, SOCKET, WSAPOLLFD,
        };
        let mut fds = [WSAPOLLFD {
            fd: SOCKET::MAX, // unused slots ignored; WSAPoll sees only `n`
            events: 0,
            revents: 0,
        }; MAX_WAIT_HANDLES];
        for i in 0..n {
            fds[i] = WSAPOLLFD {
                fd: handles[i] as SOCKET,
                events: POLLIN,
                revents: 0,
            };
        }
        let rc = unsafe { WSAPoll(fds.as_mut_ptr(), n as u32, ms) };
        if rc > 0 {
            let mask = POLLIN | POLLHUP | POLLERR | POLLNVAL;
            for i in 0..n {
                out[i] = fds[i].revents & mask != 0;
            }
        }
    }

    out
}

/// Policy state attached by the producer to the actual journal-fixture paint.
#[cfg(feature = "journal-paint-proof")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalPaintStamp {
    pub modal_root: i32,
    pub journal_paint_hidden: bool,
}

/// One immutable paint witness. GPU pixels are read back before the producer
/// can reuse its render texture; the ordinary product mailbox is unchanged.
#[cfg(feature = "journal-paint-proof")]
pub struct JournalProofFrame {
    pub generation: u64,
    pub pixmap: client::graphics::PixMap,
    pub stamp: Option<JournalPaintStamp>,
    pub gpu: bool,
}

/// Per-slot frame mailbox: the slot's attached render head publishes each
/// rendered [`FrameOutput`] here; the panel hands it to the frame consumer
/// with [`FrameBuf::take`] (one consumer per mailbox — a
/// `FrameOutput::Texture` hands its wgpu view off once, no Clone).
/// [`FrameBuf::snapshot`] stays for the CPU packing path and the tests.
/// Replaces the old packed-pixels byte buffer.
///
/// Publication is identity-checked: the mailbox accepts frames only from
/// the head attached to it (`(SlotKey, head_generation)`), so a retired
/// head can never publish into a reattached head's mailbox.
pub struct FrameBuf {
    inner: Mutex<Mailbox>,
    gen: AtomicU64,
    #[cfg(feature = "journal-paint-proof")]
    proof_frames: Option<parking_lot::Mutex<VecDeque<JournalProofFrame>>>,
}

/// One lock owns the latest frame, the head allowed to publish it and,
/// under `render-diagnostics`, the sidecar that belongs to it. Publish and
/// take share this critical section so a later publish cannot attach a new
/// ROI to an earlier take.
struct Mailbox {
    frame: Option<FrameOutput>,
    /// The attached head whose frames this mailbox accepts: set when the
    /// head attaches, cleared when it retires.
    head: Option<HeadIdentity>,
    /// Why the slot's draw request has no head, while its attach is refused.
    refused: Option<RenderRefusal>,
    #[cfg(feature = "render-diagnostics")]
    stored_roi: Option<client::render::diagnostics::PixelRoiMeta>,
    #[cfg(feature = "render-diagnostics")]
    taken_roi: Option<client::render::diagnostics::PixelRoiMeta>,
}

impl FrameBuf {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(Mailbox {
                frame: None,
                head: None,
                refused: None,
                #[cfg(feature = "render-diagnostics")]
                stored_roi: None,
                #[cfg(feature = "render-diagnostics")]
                taken_roi: None,
            }),
            gen: AtomicU64::new(0),
            #[cfg(feature = "journal-paint-proof")]
            proof_frames: None,
        })
    }
    /// Opt in to lossless capture for one fixture consumer. Unlike `new`,
    /// this retains every paint and never hands out an aliased GPU texture.
    #[cfg(feature = "journal-paint-proof")]
    pub fn new_for_journal_proof() -> std::sync::Arc<Self> {
        let mut mailbox = Self::new();
        std::sync::Arc::get_mut(&mut mailbox)
            .expect("new fixture mailbox has one owner")
            .proof_frames = Some(parking_lot::Mutex::new(VecDeque::new()));
        mailbox
    }

    #[cfg(feature = "journal-paint-proof")]
    pub fn take_journal_proof(&self) -> Option<JournalProofFrame> {
        self.proof_frames.as_ref()?.lock().pop_front()
    }

    #[cfg(feature = "journal-paint-proof")]
    pub(crate) fn publish_journal_paint(
        &self,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: JournalPaintStamp,
    ) -> bool {
        if let Some(queue) = &self.proof_frames {
            self.publish_proof(queue, head, frame, Some(stamp))
        } else {
            self.publish(head, frame)
        }
    }

    /// Publish `head`'s paint into the proof queue, if `head` is attached.
    /// The identity check and the publication share one mailbox critical
    /// section, as in [`FrameBuf::publish`]: `retire_head`/`attach_head`
    /// take the same lock, so a head retired after its check cannot publish
    /// into its slot's reattached generation.
    #[cfg(feature = "journal-paint-proof")]
    fn publish_proof(
        &self,
        queue: &parking_lot::Mutex<VecDeque<JournalProofFrame>>,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: Option<JournalPaintStamp>,
    ) -> bool {
        // Read the pixels back before taking the mailbox lock: a GPU readback
        // waits on the device, and the frame still belongs to this producer.
        let (pixmap, gpu) = match frame {
            FrameOutput::PixMap(pixmap) => (pixmap, false),
            FrameOutput::Texture(handle) => (
                client::graphics::PixMap {
                    width: handle.width as i32,
                    height: handle.height as i32,
                    pixels: handle.read_back(),
                },
                true,
            ),
        };
        let inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            return false;
        }
        #[cfg(test)]
        tests::pause_in_publication_gap();
        let mut queue = queue.lock();
        let generation = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
        queue.push_back(JournalProofFrame {
            generation,
            pixmap,
            stamp,
            gpu,
        });
        drop(queue);
        drop(inner);
        true
    }

    /// Store `head`'s latest frame and bump the generation, if `head` is the
    /// head attached to this mailbox; otherwise drop the frame and return
    /// false. The full [`FrameOutput`] is kept so the panel can bind a
    /// `FrameOutput::Texture` or pack a `PixMap`.
    pub(crate) fn publish(&self, head: HeadIdentity, frame: FrameOutput) -> bool {
        #[cfg(feature = "journal-paint-proof")]
        if let Some(queue) = &self.proof_frames {
            return self.publish_proof(queue, head, frame, None);
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            return false;
        }
        #[cfg(test)]
        tests::pause_in_publication_gap();
        #[cfg(feature = "render-diagnostics")]
        {
            inner.stored_roi = client::render::diagnostics::take_thread_pixel_roi();
        }
        inner.frame = Some(frame);
        drop(inner);
        self.gen.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// `head` attached: from now on its frames are accepted, and a refusal
    /// shown for this slot is over.
    pub(crate) fn attach_head(&self, head: HeadIdentity) {
        let mut inner = self.inner.lock().unwrap();
        inner.head = Some(head);
        inner.refused = None;
    }

    /// `head` retired at a detach: stop accepting its frames and drain the
    /// mailbox (the frame is dropped before the head's backend). One critical
    /// section, so the drain cannot take a frame a later head published.
    pub(crate) fn retire_head(&self, head: HeadIdentity) -> Option<FrameOutput> {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) {
            inner.head = None;
        }
        Self::take_locked(&mut inner)
    }

    /// `head` ended with its slot loop: stop accepting its frames, but keep
    /// the last one for the consumer.
    pub(crate) fn release_head(&self, head: HeadIdentity) {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) {
            inner.head = None;
        }
    }

    /// Record why the slot's draw request has no head, or clear it.
    pub(crate) fn set_refusal(&self, refusal: Option<RenderRefusal>) {
        self.inner.lock().unwrap().refused = refusal;
    }

    /// The attached head whose frames this mailbox accepts, if any.
    pub fn head(&self) -> Option<HeadIdentity> {
        self.inner.lock().unwrap().head
    }

    /// Why this slot draws no picture though its draw policy is on: the
    /// render service refused its head. Shown in the slot's own cell.
    pub fn refusal(&self) -> Option<RenderRefusal> {
        self.inner.lock().unwrap().refused
    }

    /// Move the stored frame out and clear the mailbox: the single
    /// consumer's handoff (a `FrameOutput::Texture` is single-consumer, no
    /// Clone). `None` when nothing was stored since the last take. The
    /// generation is untouched — only [`FrameBuf::publish`] bumps it.
    pub fn take(&self) -> Option<FrameOutput> {
        Self::take_locked(&mut self.inner.lock().unwrap())
    }

    fn take_locked(inner: &mut Mailbox) -> Option<FrameOutput> {
        let frame = inner.frame.take();
        #[cfg(feature = "render-diagnostics")]
        if frame.is_some() {
            inner.taken_roi = inner.stored_roi.take();
        }
        frame
    }

    /// Sidecar that belonged to the last successful [`take`]. Ordinary
    /// builds omit this.
    #[cfg(feature = "render-diagnostics")]
    pub fn take_pixel_roi(&self) -> Option<client::render::diagnostics::PixelRoiMeta> {
        self.inner.lock().unwrap().taken_roi.take()
    }
    /// CPU path: pack the latest `PixMap`'s pixels via `pack_rgb` (765×503,
    /// same shape the panel's texture upload expects). Empty when nothing
    /// was stored yet, the frame is `FrameOutput::Texture`, or the `PixMap`
    /// is not full-applet sized.
    pub fn snapshot(&self) -> Vec<u32> {
        let n = (APPLET_W * APPLET_H) as usize;
        let inner = self.inner.lock().unwrap();
        let mut out = Vec::with_capacity(n);
        if let Some(FrameOutput::PixMap(pix)) = &inner.frame {
            if pix.width == APPLET_W && pix.height == APPLET_H && pix.pixels.len() >= n {
                for src in pix.pixels.iter().take(n) {
                    out.push(pack_rgb(*src));
                }
            }
        }
        out
    }
    /// Bumps on every accepted [`FrameBuf::publish`]. The panel skips uploads while
    /// this stays unchanged (avoids Poll-spin on a frozen blit).
    pub fn generation(&self) -> u64 {
        self.gen.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
pub enum InputEv {
    Move { x: i32, y: i32 },
    Down { button: i32, x: i32, y: i32 },
    Up,
    Key { down: bool, ch: i32 },
}

pub fn map_image_to_applet(
    local_x: f32,
    local_y: f32,
    image_w: f32,
    image_h: f32,
) -> Option<(i32, i32)> {
    if image_w <= 0.0 || image_h <= 0.0 {
        return None;
    }
    if local_x < 0.0 || local_y < 0.0 || local_x > image_w || local_y > image_h {
        return None;
    }
    let x = ((local_x / image_w) * APPLET_W as f32).floor() as i32;
    let y = ((local_y / image_h) * APPLET_H as f32).floor() as i32;
    let x = x.clamp(0, APPLET_W - 1);
    let y = y.clamp(0, APPLET_H - 1);
    Some((x, y))
}

/// Who owns `GameShell` click/held bits. User and Script never share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseOwner {
    None,
    User,
    Script { generation: u64, seq: u64 },
}

/// A winning human left-click's movement intent, not proof of a sent packet.
/// Classified against the displayed client state before mainloop changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ManualMoveIntent {
    Viewport,
    WorldMenu,
    Minimap,
}

impl ManualMoveIntent {
    fn classify(client: &client::client::Client, owner: MouseOwner) -> Option<Self> {
        if owner != MouseOwner::User || !client.ingame || client.shell.mouse_click_button != 1 {
            return None;
        }
        let (x, y) = (client.shell.mouse_click_x, client.shell.mouse_click_y);
        // mouse_loop can dismiss a menu and minimap_loop can still walk
        // from the same click. Main modals do not disable the minimap.
        if client.minimap_state == 0 && client::client::Client::minimap_hit(x, y).is_some() {
            return Some(Self::Minimap);
        }
        if client.main_modal_id != -1 {
            return None;
        }
        if client.is_menu_open {
            return (client.menu_area == 0 && client.menu_option_at(x, y).is_some())
                .then_some(Self::WorldMenu);
        }
        client::client::Client::world_viewport_hit(x, y).then_some(Self::Viewport)
    }
}

struct ScriptMouseEv {
    generation: u64,
    seq: u64,
    down: bool,
    button: i32,
    x: i32,
    y: i32,
}

#[derive(Clone, Copy)]
struct ScriptMouseGesture {
    generation: u64,
    seq: u64,
}

struct MouseState {
    q: VecDeque<ScriptMouseEv>,
    gestures: VecDeque<ScriptMouseGesture>,
    next_seq: u64,
    held: MouseOwner,
    pending: MouseOwner,
    overflow: bool,
    // Fits the existing alignment padding; no gesture queue or per-slot growth.
    manual_move_intent: Option<ManualMoveIntent>,
}

impl MouseState {
    fn new() -> Self {
        Self {
            q: VecDeque::new(),
            gestures: VecDeque::new(),
            next_seq: 0,
            held: MouseOwner::None,
            pending: MouseOwner::None,
            overflow: false,
            manual_move_intent: None,
        }
    }
}

/// Total pending script-mouse events per slot (not per isolate batch).
const SCRIPT_MOUSE_QUEUE_CAP: usize = 32;

struct UserInput {
    rx: Option<Receiver<InputEv>>,
    keyboard_enabled: bool,
    release_pending: bool,
    /// Keys this user channel applied. The release latch must not clear
    /// `key_held` bits set with `GameShell::apply_key` outside `SlotInput`.
    held_keys: [bool; 128],
}

impl UserInput {
    /// A detach survives receiver replacement and disable/re-enable before
    /// the consumer runs. Buffered presses belong to the detached input.
    fn detach(&mut self) {
        self.release_pending = true;
        if let Some(rx) = &self.rx {
            while rx.try_recv().is_ok() {}
        }
    }

    fn note_key(&mut self, ch: i32, down: bool) {
        if ch > 0 && ch < 128 {
            self.held_keys[ch as usize] = down;
        }
    }

    fn release_held_keys(&mut self, shell: &mut GameShell) {
        for (ch, held) in self.held_keys.iter_mut().enumerate().skip(1) {
            if *held {
                shell.apply_key(false, 0, ch as i32);
                *held = false;
            }
        }
    }
}

pub struct SlotInput {
    enabled: AtomicBool,
    /// 50 fps frame-cadence latch: the panel's sidecar-50 pref sets this
    /// so a draw-on wall/grid member stays on the 20 ms loop and paints
    /// every tick (host-side; the old `SlotLoop::full_rate` stub moved
    /// here so the panel can reach it).
    full_rate: AtomicBool,
    /// Per-slot CpuPix3D (else GPU). Applied when the slot lazily builds
    /// its `Renderer` — flipping it on a live slot drops + reattaches the
    /// head (the `Client` and its socket stay up).
    prefer_cpu: AtomicBool,
    /// Serializes detach/replace with applying user input to the shell.
    rx: Mutex<UserInput>,
    /// SlotScript publishes/revokes; consume holds this across latch.
    /// Lock order: authority, then [`Self::rx`], then [`Self::mouse`].
    authority: Arc<NativeInputAuthority>,
    mouse: Mutex<MouseState>,
    /// Guardian / `!up` gate set by the slot thread before consume.
    /// Default true so capture-off catalog slots still consume script mouse.
    host_consume_allowed: AtomicBool,
}

impl SlotInput {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            enabled: AtomicBool::new(false),
            full_rate: AtomicBool::new(false),
            prefer_cpu: AtomicBool::new(false),
            rx: Mutex::new(UserInput {
                rx: None,
                keyboard_enabled: true,
                release_pending: false,
                held_keys: [false; 128],
            }),
            authority: NativeInputAuthority::new(),
            mouse: Mutex::new(MouseState::new()),
            host_consume_allowed: AtomicBool::new(true),
        })
    }

    pub fn authority(&self) -> Arc<NativeInputAuthority> {
        Arc::clone(&self.authority)
    }

    pub fn set_host_consume_allowed(&self, on: bool) {
        self.host_consume_allowed.store(on, Ordering::Release);
    }
    pub fn set_enabled(&self, on: bool) {
        let mut user = self.rx.lock().unwrap();
        if !on || !self.enabled.load(Ordering::Relaxed) {
            user.detach();
        }
        self.enabled.store(on, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Panel/window keyboard focus is independent of mouse click-through.
    /// Losing it detaches outstanding user holds; new mouse input still
    /// works while new keys are gated. No queued synthetic ups are needed.
    /// Re-enable latches that release without discarding undrained click-through.
    pub fn set_keyboard_enabled(&self, on: bool) {
        let mut user = self.rx.lock().unwrap();
        if user.keyboard_enabled != on {
            if on {
                user.release_pending = true;
            } else {
                user.detach();
            }
            user.keyboard_enabled = on;
        }
    }
    pub fn set_full_rate(&self, on: bool) {
        self.full_rate.store(on, Ordering::Relaxed);
    }
    pub fn full_rate(&self) -> bool {
        self.full_rate.load(Ordering::Relaxed)
    }
    pub fn set_prefer_cpu(&self, on: bool) {
        self.prefer_cpu.store(on, Ordering::Relaxed);
    }
    pub fn prefer_cpu(&self) -> bool {
        self.prefer_cpu.load(Ordering::Relaxed)
    }
    /// Replace the user receiver. Releases keys this channel held and does
    /// not re-press a key the operator is still holding.
    pub fn connect_rx(&self, rx: Receiver<InputEv>) {
        let mut user = self.rx.lock().unwrap();
        user.detach();
        user.rx = Some(rx);
    }
    pub fn disconnect_rx(&self) {
        let mut user = self.rx.lock().unwrap();
        user.detach();
        user.rx = None;
    }
    pub fn drain(&self, shell: &mut GameShell) {
        self.drain_user(shell);
    }

    fn drain_user(&self, shell: &mut GameShell) {
        self.apply_user(shell, false);
    }

    fn release_user_mouse(shell: &mut GameShell, mouse: &mut MouseState) {
        if mouse.held == MouseOwner::User {
            shell.apply_mouse_up();
            mouse.held = MouseOwner::None;
        }
    }

    fn apply_user(&self, shell: &mut GameShell, discard: bool) {
        // Hold both locks through apply: a detach cannot race a batch of
        // already-collected downs back into the shell after its release.
        let mut user = self.rx.lock().unwrap();
        let mut mouse = self.mouse.lock().unwrap();
        if user.release_pending {
            user.release_held_keys(shell);
            Self::release_user_mouse(shell, &mut mouse);
            if mouse.pending == MouseOwner::User {
                shell.clear_unlatched_click();
                mouse.pending = MouseOwner::None;
            }
            user.release_pending = false;
        }
        let enabled = !discard && self.enabled.load(Ordering::Relaxed);
        loop {
            let ev = {
                let Some(rx) = user.rx.as_ref() else {
                    return;
                };
                match rx.try_recv() {
                    Ok(ev) => ev,
                    Err(_) => break,
                }
            };
            match ev {
                InputEv::Move { x, y } if enabled => shell.apply_mouse_move(x, y),
                InputEv::Down { button, x, y } if enabled => {
                    shell.apply_mouse_down(button, x, y);
                    mouse.held = MouseOwner::User;
                    mouse.pending = MouseOwner::User;
                }
                InputEv::Up => Self::release_user_mouse(shell, &mut mouse),
                InputEv::Key { down, ch } if !down || (enabled && user.keyboard_enabled) => {
                    shell.apply_key(down, 0, ch);
                    user.note_key(ch, down);
                }
                InputEv::Move { .. } | InputEv::Down { .. } | InputEv::Key { .. } => {}
            }
        }
    }

    /// Drop user input buffered during a login wait/handshake so presses
    /// cannot act on the next title. Apply releases and any pending detach
    /// through the same consumer path as ordinary frames.
    pub fn discard_user(&self, shell: &mut GameShell) {
        self.apply_user(shell, true);
    }

    /// Map frozen/script client coordinates onto applet pixels.
    /// Refuses non-finite, negative, outside, and non-left buttons.
    /// DOM 0/1 → Java button 1. Does not truncate before the bounds check.
    pub fn map_script_mouse(x: f64, y: f64, button: i32) -> Option<(i32, i32, i32)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        if button != 0 && button != 1 {
            return None;
        }
        if x < 0.0 || y < 0.0 || x > f64::from(APPLET_W) || y > f64::from(APPLET_H) {
            return None;
        }
        let ax = x.floor() as i32;
        let ay = y.floor() as i32;
        if !(0..APPLET_W).contains(&ax) || !(0..APPLET_H).contains(&ay) {
            return None;
        }
        Some((ax, ay, 1))
    }

    pub fn enqueue_script_mouse(&self, down: bool, x: f64, y: f64, button: i32) {
        let identity = self.authority.lock().identity();
        self.enqueue_script_mouse_at(identity, down, x, y, button);
    }

    /// Enqueue a mouse event that already carries its production identity.
    /// Does not restamp the live permit onto stale work.
    pub fn enqueue_script_mouse_at(&self, identity: u64, down: bool, x: f64, y: f64, button: i32) {
        let permit = self.authority.lock();
        let Some((ax, ay, java_btn)) = Self::map_script_mouse(x, y, button) else {
            return;
        };
        let mut mouse = self.mouse.lock().unwrap();
        let pair = if down {
            if !permit.eligible() || identity != permit.identity() {
                return;
            }
            if mouse.gestures.len() >= SCRIPT_MOUSE_QUEUE_CAP {
                mouse.overflow = true;
                mouse.q.clear();
                mouse.gestures.clear();
                return;
            }
            let pair = ScriptMouseGesture {
                generation: identity,
                seq: mouse.next_seq,
            };
            mouse.next_seq = mouse.next_seq.wrapping_add(1);
            mouse.gestures.push_back(pair);
            pair
        } else {
            let Some(pos) = mouse
                .gestures
                .iter()
                .position(|pair| pair.generation == identity)
            else {
                return;
            };
            let pair = mouse.gestures.remove(pos).expect("mouse gesture position");
            if !permit.eligible() || identity != permit.identity() {
                return;
            }
            pair
        };
        if mouse.q.len() >= SCRIPT_MOUSE_QUEUE_CAP {
            mouse.overflow = true;
            mouse.q.clear();
            mouse.gestures.clear();
            return;
        }
        mouse.q.push_back(ScriptMouseEv {
            generation: pair.generation,
            seq: pair.seq,
            down,
            button: java_btn,
            x: ax,
            y: ay,
        });
    }

    /// User drain, script consume, latch. Holds the authority mutex across
    /// apply + latch and returns the winning click owner, not the held owner.
    pub fn consume_native_frame(&self, shell: &mut GameShell) -> MouseOwner {
        let permit = self.authority.lock();
        self.drain_user(shell);
        self.consume_script_mouse(shell, &permit);
        shell.latch_click();
        let mut mouse = self.mouse.lock().unwrap();
        let owner = mouse.pending;
        mouse.pending = MouseOwner::None;
        mouse.manual_move_intent = None;
        owner
    }

    pub(crate) fn classify_manual_move_intent(
        &self,
        client: &client::client::Client,
        owner: MouseOwner,
    ) {
        if let Some(intent) = ManualMoveIntent::classify(client, owner) {
            self.mouse.lock().unwrap().manual_move_intent = Some(intent);
        }
    }

    /// Take the pre-mainloop click fact once, before frontend and script follow.
    /// A subsequent native frame replaces an untaken fact rather than replaying it.
    pub fn take_manual_move_intent(&self) -> Option<ManualMoveIntent> {
        self.mouse.lock().unwrap().manual_move_intent.take()
    }

    fn consume_script_mouse(&self, shell: &mut GameShell, permit: &NativeInputPermit) {
        let mut mouse = self.mouse.lock().unwrap();
        let host_ok = self.host_consume_allowed.load(Ordering::Acquire);
        let script_ok = permit.eligible() && host_ok;

        let release_script = |shell: &mut GameShell, mouse: &mut MouseState| {
            if matches!(mouse.held, MouseOwner::Script { .. }) {
                shell.apply_mouse_up();
                mouse.held = MouseOwner::None;
            }
            if matches!(mouse.pending, MouseOwner::Script { .. }) {
                shell.clear_unlatched_click();
                mouse.pending = MouseOwner::None;
            }
        };

        if mouse.overflow {
            release_script(shell, &mut mouse);
            mouse.q.clear();
            mouse.overflow = false;
            if !script_ok {
                return;
            }
        }

        if !script_ok {
            release_script(shell, &mut mouse);
            mouse.q.clear();
            return;
        }

        let live = permit.identity();
        if let MouseOwner::Script { generation, .. } = mouse.held {
            if generation != live {
                release_script(shell, &mut mouse);
            }
        }
        mouse.q.retain(|ev| ev.generation == live);

        let pending: Vec<ScriptMouseEv> = mouse.q.drain(..).collect();
        for ev in pending {
            if ev.down {
                if mouse.pending != MouseOwner::None || matches!(mouse.held, MouseOwner::User) {
                    continue;
                }
                shell.apply_mouse_down(ev.button, ev.x, ev.y);
                let owner = MouseOwner::Script {
                    generation: ev.generation,
                    seq: ev.seq,
                };
                mouse.held = owner;
                mouse.pending = owner;
            } else if matches!(
                mouse.held,
                MouseOwner::Script { generation, seq }
                    if generation == ev.generation && seq == ev.seq
            ) {
                shell.apply_mouse_up();
                mouse.held = MouseOwner::None;
            }
        }
    }

    #[cfg(test)]
    fn script_queue_len(&self) -> usize {
        self.mouse.lock().unwrap().q.len()
    }

    #[cfg(test)]
    fn script_held(&self) -> bool {
        matches!(self.mouse.lock().unwrap().held, MouseOwner::Script { .. })
    }
}

#[cfg(unix)]
type WakeStream = UnixStream;
#[cfg(windows)]
type WakeStream = TcpStream;

/// Per-slot control wake: the panel/host-play side holds [`SlotWake`] ends
/// and `wake()`s a parked slot thread (focus/draw/stop/spawn); the slot
/// thread hands the [`SlotPark`] end to [`Host::run_client`], which polls
/// its wait handle next to the client socket inside the idle park. Unix
/// uses a socketpair; Windows uses an owned nonblocking loopback TCP pair
/// with peer identity checked. The payload is an empty kick — the parker
/// re-reads the shared state (`client.draw`, capture, the probe's stop
/// flag) after waking.
#[derive(Clone)]
pub struct SlotWake {
    tx: Arc<WakeStream>,
}

/// The slot-thread end of the control channel. Not shared: one per parked
/// thread, polled by [`Host::run_client`].
pub struct SlotPark {
    rx: WakeStream,
}

/// Create a control-wake channel pair. A [`SlotWake::wake`] writes a byte
/// that fires a readability wait on [`SlotPark`].
pub fn wake_channel() -> (SlotWake, SlotPark) {
    #[cfg(unix)]
    {
        let (tx, rx) = UnixStream::pair().expect("socketpair for slot wake");
        let _ = tx.set_nonblocking(true);
        let _ = rx.set_nonblocking(true);
        (SlotWake { tx: Arc::new(tx) }, SlotPark { rx })
    }
    #[cfg(windows)]
    {
        // Fail loudly: a blocking wake pair can stall kick/drain.
        let (tx, rx) = loopback_tcp_pair().expect("loopback TCP pair for slot wake");
        (SlotWake { tx: Arc::new(tx) }, SlotPark { rx })
    }
}

/// Owned nonblocking loopback TCP pair. The accepted peer address must
/// match the connector's local address so a stray connection cannot own
/// the park end. Both ends are nonblocking + TCP_NODELAY (Nagle off) so
/// kick latency is not delayed-ACK bound.
#[cfg(windows)]
fn loopback_tcp_pair() -> std::io::Result<(TcpStream, TcpStream)> {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let client = TcpStream::connect(addr)?;
    let client_local = client.local_addr()?;
    let (server, peer) = listener.accept()?;
    if peer != client_local {
        return Err(std::io::Error::other(format!(
            "wake pair peer mismatch: accepted {peer}, expected {client_local}"
        )));
    }
    if server.peer_addr()? != client_local {
        return Err(std::io::Error::other("wake pair server peer_addr mismatch"));
    }
    // Writer end is `client` (SlotWake); set both ends.
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;
    client.set_nonblocking(true)?;
    server.set_nonblocking(true)?;
    Ok((client, server))
}

impl SlotWake {
    /// Wake a parked slot thread: one byte on the control stream. Nonblocking
    /// and best-effort — a full wake buffer (only possible if the parker
    /// stopped draining) or a dead slot just drops the kick.
    pub fn wake(&self) {
        let _ = (&*self.tx).write(&[1]);
    }
}

impl SlotPark {
    /// The wake end's fd, for the slot thread's `poll(2)`.
    #[cfg(unix)]
    pub fn fd(&self) -> RawFd {
        self.rx.as_raw_fd()
    }

    /// The wake end as a Windows `SOCKET` for `WSAPoll`.
    #[cfg(windows)]
    pub fn raw_socket(&self) -> RawSocket {
        self.rx.as_raw_socket()
    }

    /// Handle used by the host idle park.
    pub(crate) fn wait_handle(&self) -> WaitHandle {
        #[cfg(unix)]
        {
            self.fd()
        }
        #[cfg(windows)]
        {
            self.raw_socket()
        }
    }

    /// Block until a wake byte is readable or `timeout` elapses.
    /// Returns true when the control end is readable (including hangup).
    /// Wraps one stack wait entry — no heap.
    pub fn wait_readable(&self, timeout: Duration) -> bool {
        wait_readable(&[self.wait_handle()], timeout)[0]
    }

    /// Drain any queued wake bytes so a burst of kicks cannot fill the
    /// control-stream buffer.
    pub fn drain(&self) {
        let mut b = [0u8; 64];
        while let Ok(n) = (&self.rx).read(&mut b) {
            if n == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        map_image_to_applet, wait_readable, wake_channel, FrameBuf, InputEv, ManualMoveIntent,
        MouseOwner, SlotInput,
    };
    use crate::render::{HeadIdentity, RenderRefusal, RenderService};
    use api::identity::{Notice, Notifier, SlotKey};
    use client::client::GameShell;
    use client::graphics::PixMap;
    use client::render::backend::FrameOutput;
    use std::cell::RefCell;
    use std::sync::{Arc, Barrier, TryLockError};
    use std::time::{Duration, Instant};

    fn applet_pixmap(pixels: Vec<i32>) -> FrameOutput {
        FrameOutput::PixMap(PixMap {
            width: 765,
            height: 503,
            pixels,
        })
    }

    struct Silent;

    impl Notifier for Silent {
        fn notify(&self, _notice: Notice) {}
    }

    /// Real head identities from one test-local render service: the first
    /// head of `slot`, retired, then its reattached generation. The mailbox
    /// only accepts frames from the head attached to it.
    fn head_generations(slot: u64) -> (HeadIdentity, HeadIdentity) {
        let service = RenderService::new(Duration::from_millis(20)).unwrap();
        let pump = service.register_pump().unwrap();
        let key = SlotKey::from_incarnation(slot).unwrap();
        let first = service
            .attach(&pump, key, Arc::new(Silent))
            .unwrap()
            .identity();
        let second = service
            .attach(&pump, key, Arc::new(Silent))
            .unwrap()
            .identity();
        (first, second)
    }

    fn head_identity(slot: u64) -> HeadIdentity {
        head_generations(slot).0
    }

    /// Publish the way a slot head does, attaching a head first if the
    /// mailbox has none.
    fn store(buf: &FrameBuf, frame: FrameOutput) {
        let head = buf.head().unwrap_or_else(|| {
            let head = head_identity(1);
            buf.attach_head(head);
            head
        });
        assert!(buf.publish(head, frame), "the attached head publishes");
    }

    #[test]
    fn publish_accepts_only_the_attached_head() {
        let buf = FrameBuf::new();
        let (retired, reattached) = head_generations(7);
        assert!(
            !buf.publish(retired, applet_pixmap(vec![1; 765 * 503])),
            "no head attached yet"
        );
        buf.attach_head(retired);
        assert!(buf.publish(retired, applet_pixmap(vec![2; 765 * 503])));
        assert_eq!(buf.generation(), 1);
        assert!(
            buf.retire_head(retired).is_some(),
            "a detach drains the mailbox"
        );
        assert_eq!(reattached.slot(), retired.slot());
        assert_ne!(reattached.generation(), retired.generation());
        buf.attach_head(reattached);
        assert!(
            !buf.publish(retired, applet_pixmap(vec![3; 765 * 503])),
            "a retired head cannot publish into the reattached head's mailbox"
        );
        assert_eq!(buf.generation(), 1, "a refused frame bumps nothing");
        assert!(buf.take().is_none());
        assert!(buf.publish(reattached, applet_pixmap(vec![4; 765 * 503])));
        assert_eq!(buf.generation(), 2);
        assert!(buf.take().is_some());
    }

    #[test]
    fn release_keeps_the_last_frame_and_stops_publication() {
        let buf = FrameBuf::new();
        let head = head_identity(8);
        buf.attach_head(head);
        assert!(buf.publish(head, applet_pixmap(vec![5; 765 * 503])));
        buf.release_head(head);
        assert_eq!(buf.head(), None);
        assert!(!buf.publish(head, applet_pixmap(vec![6; 765 * 503])));
        match buf.take() {
            Some(FrameOutput::PixMap(pix)) => assert_eq!(pix.pixels[0], 5),
            _ => panic!("the released head's last frame stays for the consumer"),
        }
    }

    #[test]
    fn refusal_is_shown_until_a_head_attaches() {
        let buf = FrameBuf::new();
        let refusal = RenderRefusal::HeadCap {
            cap: crate::render::RENDER_HEAD_CAP,
        };
        buf.set_refusal(Some(refusal));
        assert_eq!(buf.refusal(), Some(refusal));
        buf.attach_head(head_identity(9));
        assert_eq!(buf.refusal(), None);
        buf.set_refusal(Some(refusal));
        buf.set_refusal(None);
        assert_eq!(buf.refusal(), None);
    }

    /// A scheduling point between a publication's identity check and its
    /// publication point: the publisher reaches `checked`, then waits at
    /// `resume` until the test has tried to retire and reattach its slot.
    struct PublicationGap {
        checked: Barrier,
        resume: Barrier,
    }

    thread_local! {
        /// Set only on a test's publisher thread, so every other test's
        /// publication runs straight through.
        static PUBLICATION_GAP: RefCell<Option<Arc<PublicationGap>>> =
            const { RefCell::new(None) };
    }

    /// Called by `FrameBuf` right after a publication's identity check.
    pub(super) fn pause_in_publication_gap() {
        if let Some(gap) = PUBLICATION_GAP.with(|gap| gap.borrow().clone()) {
            gap.checked.wait();
            gap.resume.wait();
        }
    }

    /// How a reattach landed against a publication paused in its gap.
    struct Raced {
        /// The paused publication by the retired head returned `true`.
        accepted: bool,
        /// The mailbox let the retire and reattach run inside the gap.
        interleaved: bool,
    }

    fn retire_and_reattach(buf: &FrameBuf, retired: HeadIdentity, reattached: HeadIdentity) {
        let _ = buf.retire_head(retired);
        buf.attach_head(reattached);
    }

    /// Pause `publish` by the slot's first head right after its identity
    /// check, retire that head and attach the slot's next generation, then
    /// let the publication finish. If the mailbox lock is free in the gap,
    /// the retire and reattach run there and then (`interleaved`); if not,
    /// they run on another thread and wait for the publication.
    fn publish_across_a_reattach(
        buf: &Arc<FrameBuf>,
        slot: u64,
        publish: fn(&FrameBuf, HeadIdentity) -> bool,
    ) -> Raced {
        let (retired, reattached) = head_generations(slot);
        buf.attach_head(retired);
        let gap = Arc::new(PublicationGap {
            checked: Barrier::new(2),
            resume: Barrier::new(2),
        });
        let publisher = {
            let buf = Arc::clone(buf);
            let gap = Arc::clone(&gap);
            std::thread::spawn(move || {
                PUBLICATION_GAP.with(|slot| *slot.borrow_mut() = Some(gap));
                publish(&buf, retired)
            })
        };
        gap.checked.wait();
        let interleaved = match buf.inner.try_lock() {
            Ok(_) => true,
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Poisoned(_)) => panic!("mailbox poisoned"),
        };
        let retirer = if interleaved {
            retire_and_reattach(buf, retired, reattached);
            None
        } else {
            let buf = Arc::clone(buf);
            Some(std::thread::spawn(move || {
                retire_and_reattach(&buf, retired, reattached)
            }))
        };
        gap.resume.wait();
        let accepted = publisher.join().unwrap();
        if let Some(retirer) = retirer {
            retirer.join().unwrap();
        }
        assert_eq!(buf.head(), Some(reattached));
        Raced {
            accepted,
            interleaved,
        }
    }

    #[test]
    fn a_publication_past_its_identity_check_is_ordered_before_a_reattach() {
        let buf = FrameBuf::new();
        let raced = publish_across_a_reattach(&buf, 10, |buf, head| {
            buf.publish(head, applet_pixmap(vec![7; 765 * 503]))
        });
        assert!(
            !(raced.interleaved && raced.accepted),
            "a retired head's frame was published after its slot reattached"
        );
        assert!(raced.accepted, "published while its head was attached");
        assert_eq!(buf.generation(), 1);
        assert!(
            buf.take().is_none(),
            "the retire drained the frame published before it"
        );
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn a_journal_proof_past_its_identity_check_is_ordered_before_a_reattach() {
        let buf = FrameBuf::new_for_journal_proof();
        let raced = publish_across_a_reattach(&buf, 11, |buf, head| {
            buf.publish_journal_paint(
                head,
                FrameOutput::PixMap(PixMap {
                    width: 1,
                    height: 1,
                    pixels: vec![7],
                }),
                super::JournalPaintStamp {
                    modal_root: 1,
                    journal_paint_hidden: false,
                },
            )
        });
        let proof = buf.take_journal_proof();
        assert!(
            !(raced.interleaved && (raced.accepted || proof.is_some())),
            "a retired head's journal proof was published after its slot reattached \
             (accepted={}, queued={})",
            raced.accepted,
            proof.is_some()
        );
        assert!(raced.accepted, "published while its head was attached");
        assert_eq!(proof.map(|proof| proof.generation), Some(1));
        assert!(buf.take_journal_proof().is_none());
    }

    /// Release micro-bench of the work T4.2 adds around a CPU head's paint
    /// (not a gate): the head dispatch inside the raster timer, the
    /// identity-checked publish against the pre-T4.2 unchecked store, and
    /// one render-pump call with 16 attached CPU heads (the M3 consumer
    /// pumps every 16 ms; a CPU head never touches the service per paint).
    #[test]
    #[ignore = "bench: cargo test -p host --release --lib cpu_head_paint_path_added_work -- --ignored --nocapture"]
    fn cpu_head_paint_path_added_work() {
        use client::render::backend::BackendKind;
        use std::hint::black_box;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Mutex;

        const ROUNDS: usize = 5;
        const ITERS: u32 = 2_000_000;
        fn per_iter(mut body: impl FnMut()) -> f64 {
            let mut best = f64::MAX;
            for _ in 0..ROUNDS {
                let start = Instant::now();
                for _ in 0..ITERS {
                    body();
                }
                best = best.min(start.elapsed().as_nanos() as f64 / f64::from(ITERS));
            }
            best
        }

        // Inside the raster timer: `slot.head` is checked and `Head::paint`
        // asks the backend kind before the unchanged `mainredraw`.
        let renderer = client::render::Renderer::new_prefer(true, false);
        let head_slot = Some(&renderer);
        let dispatch = per_iter(|| {
            let head = black_box(head_slot);
            black_box(head.is_none());
            black_box(head.is_some_and(|renderer| renderer.backend_kind() != BackendKind::Gpu));
        });

        // Publication of one full-applet CPU frame and the consumer's take,
        // reusing the frame so no allocation is timed: T4.2 `publish`
        // against the pre-T4.2 `store` shape (lock, store, unlock, bump).
        struct Unchecked {
            frame: Mutex<Option<FrameOutput>>,
            gen: AtomicU64,
        }
        let old = Unchecked {
            frame: Mutex::new(None),
            gen: AtomicU64::new(0),
        };
        let mut spare = Some(applet_pixmap(vec![1; 765 * 503]));
        let unchecked = per_iter(|| {
            *old.frame.lock().unwrap() = spare.take();
            old.gen.fetch_add(1, Ordering::Relaxed);
            spare = black_box(old.frame.lock().unwrap().take());
        });
        let buf = FrameBuf::new();
        let head = head_identity(12);
        buf.attach_head(head);
        let mut spare = Some(applet_pixmap(vec![1; 765 * 503]));
        let checked = per_iter(|| {
            black_box(buf.publish(head, spare.take().unwrap()));
            spare = black_box(buf.take());
        });

        // One pump call with 16 attached CPU heads and no reservations.
        let service = RenderService::new(Duration::from_millis(100)).unwrap();
        let pump = service.register_pump().unwrap();
        let heads: Vec<_> = (0..16)
            .map(|slot| {
                service
                    .attach(
                        &pump,
                        SlotKey::from_incarnation(100 + slot).unwrap(),
                        Arc::new(Silent),
                    )
                    .unwrap()
            })
            .collect();
        let pumped = per_iter(|| {
            black_box(service.pump(&pump, Instant::now()).unwrap());
        });
        drop(heads);

        // M3 cpu-n16-full: 16 heads at about 50 paints/s, one pump per 16 ms.
        let pump_per_paint = pumped * (1000.0 / 16.0) / (16.0 * 50.0);
        let added = dispatch + (checked - unchecked) + pump_per_paint;
        println!("cpu head paint path, best of {ROUNDS} x {ITERS} iterations, ns:");
        println!("  head dispatch (is_none + backend_kind)       {dispatch:8.2}");
        println!("  publish+take, pre-T4.2 unchecked store shape {unchecked:8.2}");
        println!("  publish+take, T4.2 identity-checked          {checked:8.2}");
        println!(
            "  identity check (difference)                  {:8.2}",
            checked - unchecked
        );
        println!("  render pump call, 16 CPU heads               {pumped:8.2}");
        println!("  pump share per paint at cpu-n16-full rates   {pump_per_paint:8.2}");
        println!("  added per paint, total                       {added:8.2}");
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn journal_proof_retains_every_generation_with_its_own_pixels_and_stamp() {
        use super::JournalPaintStamp;
        let buf = FrameBuf::new_for_journal_proof();
        let witnesses = [
            (
                0x0011_2233,
                Some(JournalPaintStamp {
                    modal_root: 8134,
                    journal_paint_hidden: true,
                }),
            ),
            (
                0x0044_5566,
                Some(JournalPaintStamp {
                    modal_root: -1,
                    journal_paint_hidden: false,
                }),
            ),
            (0x0077_8899, None),
        ];
        // All paints precede the consumer: the ordinary latest-only mailbox
        // would discard the owned paint and conflate its state with release.
        let head = head_identity(2);
        buf.attach_head(head);
        for (pixel, stamp) in witnesses {
            let frame = FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![pixel],
            });
            if let Some(stamp) = stamp {
                assert!(buf.publish_journal_paint(head, frame, stamp));
            } else {
                assert!(buf.publish(head, frame));
            }
        }
        for (index, (pixel, stamp)) in witnesses.into_iter().enumerate() {
            let frame = buf.take_journal_proof().expect("no discarded paint");
            assert_eq!(frame.generation, index as u64 + 1);
            assert_eq!(frame.pixmap.pixels, [pixel]);
            assert_eq!(frame.stamp, stamp);
        }
        assert!(buf.take_journal_proof().is_none());
        assert_eq!(buf.generation(), 3);
    }

    #[test]
    fn store_packs_pixmap_rgb_and_is_765_by_503() {
        use client::client::present::pack_rgb;
        let buf = FrameBuf::new();
        let mut pix = vec![0i32; (client::client::APPLET_W * client::client::APPLET_H) as usize];
        pix[0] = 0x00aa_bbcc;
        pix[1] = 0x0011_2233;
        store(&buf, applet_pixmap(pix));
        let out = buf.snapshot();
        assert_eq!(
            out.len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
        assert_eq!(out[0], pack_rgb(0x00aa_bbcc));
        assert_eq!(out[1], pack_rgb(0x0011_2233));
    }

    #[test]
    fn snapshot_is_empty_until_a_pixmap_lands() {
        let buf = FrameBuf::new();
        assert!(buf.snapshot().is_empty());
        assert_eq!(buf.generation(), 0);
    }

    #[test]
    fn snapshot_of_wrong_shape_pixmap_is_empty() {
        let buf = FrameBuf::new();
        store(
            &buf,
            FrameOutput::PixMap(PixMap {
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3],
            }),
        );
        assert!(buf.snapshot().is_empty());
        // The frame is still stored and the generation bumped; only the
        // CPU packing is shape-gated.
        assert_eq!(buf.generation(), 1);
    }

    #[test]
    fn snapshot_of_texture_frame_is_empty_and_take_hands_it_off() {
        // A real `TextureHandle` needs a wgpu device (client-side); the
        // panel's `frame_pixels` read-back test covers the texture pixels.
        // Here the mailbox contract: a stored frame of any variant is taken
        // out whole, and `snapshot` (the CPU packing view) never sees it.
        let buf = FrameBuf::new();
        store(&buf, applet_pixmap(vec![0i32; 765 * 503]));
        assert!(!buf.snapshot().is_empty());
        let frame = buf.take();
        assert!(matches!(frame, Some(FrameOutput::PixMap(_))));
        assert!(buf.take().is_none(), "a second take must be empty");
        assert!(buf.snapshot().is_empty(), "the stored frame was moved out");
        assert_eq!(
            buf.generation(),
            1,
            "take must not bump the generation (only store does)"
        );
    }

    #[test]
    fn take_is_empty_until_a_store_lands() {
        let buf = FrameBuf::new();
        assert!(buf.take().is_none());
        assert_eq!(buf.generation(), 0);
        store(&buf, applet_pixmap(vec![0i32; 765 * 503]));
        assert!(buf.take().is_some());
        assert!(buf.take().is_none());
        assert_eq!(buf.generation(), 1);
    }

    #[test]
    fn store_bumps_generation() {
        let buf = FrameBuf::new();
        assert_eq!(buf.generation(), 0);
        store(&buf, applet_pixmap(vec![0i32; 765 * 503]));
        assert_eq!(buf.generation(), 1);
        store(&buf, applet_pixmap(vec![1i32; 765 * 503]));
        assert_eq!(buf.generation(), 2);
    }

    #[cfg(feature = "render-diagnostics")]
    #[test]
    fn take_keeps_production_roi_after_later_live_stamp() {
        use client::render::diagnostics::{
            stamp_thread_pixel_roi, PackedHist, PixelRoiCam, PixelRoiMeta,
        };
        let first = PixelRoiMeta::produced(
            4,
            PixelRoiCam {
                cycle: 10,
                eye_x: 1,
                eye_y: 0,
                eye_z: 0,
                yaw: 0,
                pitch: 0,
                origin_x: 0,
                origin_z: 0,
                trace_frame: 1,
            },
            PackedHist {
                n: 1,
                ..PackedHist::EMPTY
            },
            PackedHist::EMPTY,
        );
        stamp_thread_pixel_roi(first.clone());
        let buf = FrameBuf::new();
        store(&buf, applet_pixmap(vec![0i32; 765 * 503]));
        let later = PixelRoiMeta::produced(
            9,
            PixelRoiCam {
                cycle: 99,
                eye_x: 8,
                eye_y: 0,
                eye_z: 0,
                yaw: 0,
                pitch: 0,
                origin_x: 0,
                origin_z: 0,
                trace_frame: 2,
            },
            PackedHist::EMPTY,
            PackedHist::EMPTY,
        );
        stamp_thread_pixel_roi(later);
        assert!(buf.take().is_some());
        let roi = buf.take_pixel_roi().expect("sidecar from the stored frame");
        assert_eq!(roi.frame_id, 4);
        assert_eq!(roi.cam.cycle, 10);
        assert_ne!(roi.frame_id, 9);
    }

    #[cfg(feature = "render-diagnostics")]
    #[test]
    fn take_keeps_frame_a_roi_when_frame_b_is_stored_before_take_pixel_roi() {
        use client::render::backend::FrameOutput;
        use client::render::diagnostics::{
            stamp_thread_pixel_roi, PackedHist, PixelRoiCam, PixelRoiMeta,
        };
        fn tagged(id: u64, cycle: i32) -> PixelRoiMeta {
            PixelRoiMeta::produced(
                id,
                PixelRoiCam {
                    cycle,
                    eye_x: id as i32,
                    eye_y: 0,
                    eye_z: 0,
                    yaw: 0,
                    pitch: 0,
                    origin_x: 0,
                    origin_z: 0,
                    trace_frame: id as u32,
                },
                PackedHist {
                    n: 1,
                    ..PackedHist::EMPTY
                },
                PackedHist::EMPTY,
            )
        }
        fn pix(tag: i32) -> FrameOutput {
            let mut pixels = vec![0i32; 765 * 503];
            pixels[0] = tag;
            applet_pixmap(pixels)
        }
        let buf = FrameBuf::new();
        stamp_thread_pixel_roi(tagged(4, 10));
        store(&buf, pix(0x00aa));
        let frame_a = buf.take().expect("frame A");
        stamp_thread_pixel_roi(tagged(9, 99));
        store(&buf, pix(0x00bb));
        let roi = buf.take_pixel_roi().expect("roi taken with frame A");
        match frame_a {
            FrameOutput::PixMap(pix) => assert_eq!(pix.pixels[0], 0x00aa, "took frame A pixels"),
            _ => panic!("expected pixmap A"),
        }
        assert_eq!(
            roi.frame_id, 4,
            "roi must stay with frame A, not the interleaved store"
        );
        assert_eq!(roi.cam.cycle, 10);
        let frame_b = buf.take().expect("frame B");
        let roi_b = buf.take_pixel_roi().expect("roi B");
        match frame_b {
            FrameOutput::PixMap(pix) => assert_eq!(pix.pixels[0], 0x00bb),
            _ => panic!("expected pixmap B"),
        }
        assert_eq!(roi_b.frame_id, 9);
    }

    #[cfg(feature = "render-diagnostics")]
    #[test]
    fn concurrent_store_take_never_pairs_a_frame_with_another_roi() {
        use client::render::backend::FrameOutput;
        use client::render::diagnostics::{
            stamp_thread_pixel_roi, PackedHist, PixelRoiCam, PixelRoiMeta,
        };
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Arc;
        use std::thread;
        fn tagged(id: u64) -> PixelRoiMeta {
            PixelRoiMeta::produced(
                id,
                PixelRoiCam {
                    cycle: id as i32,
                    eye_x: id as i32,
                    eye_y: 0,
                    eye_z: 0,
                    yaw: 0,
                    pitch: 0,
                    origin_x: 0,
                    origin_z: 0,
                    trace_frame: id as u32,
                },
                PackedHist {
                    n: 1,
                    ..PackedHist::EMPTY
                },
                PackedHist::EMPTY,
            )
        }
        let buf = FrameBuf::new();
        let stop = Arc::new(AtomicBool::new(false));
        let mismatches = Arc::new(AtomicU64::new(0));
        let seen = Arc::new(AtomicU64::new(0));
        let producer = {
            let buf = Arc::clone(&buf);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut n = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    stamp_thread_pixel_roi(tagged(n));
                    let mut pixels = vec![0i32; 765 * 503];
                    pixels[0] = n as i32;
                    store(&buf, applet_pixmap(pixels));
                    n = n.wrapping_add(1);
                    if n == 0 {
                        n = 1;
                    }
                }
            })
        };
        let consumer = {
            let buf = Arc::clone(&buf);
            let stop = Arc::clone(&stop);
            let mismatches = Arc::clone(&mismatches);
            let seen = Arc::clone(&seen);
            thread::spawn(move || {
                for _ in 0..8_000 {
                    if let Some(FrameOutput::PixMap(pix)) = buf.take() {
                        let tag = pix.pixels[0] as u64;
                        match buf.take_pixel_roi() {
                            Some(roi) if roi.frame_id == tag => {
                                seen.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {
                                mismatches.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                stop.store(true, Ordering::Relaxed);
            })
        };
        consumer.join().expect("consumer");
        producer.join().expect("producer");
        assert_eq!(
            mismatches.load(Ordering::Relaxed),
            0,
            "a taken pixmap tag must equal the sidecar frame_id"
        );
        assert!(
            seen.load(Ordering::Relaxed) > 0,
            "consumer must observe stores"
        );
    }

    #[test]
    fn map_image_to_applet_scales_and_rejects_outside() {
        assert_eq!(map_image_to_applet(0.0, 0.0, 1530.0, 1006.0), Some((0, 0)));
        assert_eq!(
            map_image_to_applet(1530.0, 1006.0, 1530.0, 1006.0),
            Some((764, 502))
        );
        assert_eq!(map_image_to_applet(-1.0, 10.0, 765.0, 503.0), None);
    }

    fn live_input() -> std::sync::Arc<SlotInput> {
        let inp = SlotInput::new();
        inp.authority().publish_live();
        inp
    }

    #[test]
    fn user_detach_releases_all_holds_without_queued_ups() {
        enum Detach {
            Disable,
            Disconnect,
            Replace,
        }
        for detach in [Detach::Disable, Detach::Disconnect, Detach::Replace] {
            for button in [1, 2] {
                let input = live_input();
                let (tx, rx) = std::sync::mpsc::channel();
                input.connect_rx(rx);
                input.set_enabled(true);
                for ch in [1, 58, 65] {
                    tx.send(InputEv::Key { down: true, ch }).unwrap();
                }
                tx.send(InputEv::Down {
                    button,
                    x: 20,
                    y: 30,
                })
                .unwrap();
                let mut shell = GameShell::new();
                input.drain(&mut shell);
                assert_eq!(shell.mouse_button, button);
                assert_eq!(
                    (shell.key_held[1], shell.key_held[58], shell.key_held[65]),
                    (1, 1, 1)
                );
                match detach {
                    Detach::Disable => input.set_enabled(false),
                    Detach::Disconnect => input.disconnect_rx(),
                    Detach::Replace => {
                        let (_tx, rx) = std::sync::mpsc::channel();
                        input.connect_rx(rx);
                    }
                }
                // The release must also run outside ordinary frames and
                // without a receiver (title/login discard path).
                input.discard_user(&mut shell);
                assert_eq!(shell.key_held, [0; 128]);
                assert_eq!(shell.mouse_button, 0);
                shell.latch_click();
                assert_eq!(shell.mouse_click_button, 0);
            }
        }
    }

    #[test]
    fn user_detach_survives_reattach_before_consumer_and_preserves_new_downs() {
        for replace in [false, true] {
            let input = live_input();
            let (mut tx, rx) = std::sync::mpsc::channel();
            input.connect_rx(rx);
            input.set_enabled(true);
            tx.send(InputEv::Key { down: true, ch: 1 }).unwrap();
            tx.send(InputEv::Down {
                button: 1,
                x: 10,
                y: 20,
            })
            .unwrap();
            let mut shell = GameShell::new();
            input.consume_native_frame(&mut shell);
            assert_eq!((shell.key_held[1], shell.mouse_button), (1, 1));
            // Buffered attached and disabled presses must not replay.
            tx.send(InputEv::Key { down: true, ch: 58 }).unwrap();
            input.set_enabled(false);
            tx.send(InputEv::Key { down: true, ch: 65 }).unwrap();
            tx.send(InputEv::Down {
                button: 2,
                x: 30,
                y: 40,
            })
            .unwrap();
            if replace {
                let (new_tx, rx) = std::sync::mpsc::channel();
                input.connect_rx(rx);
                tx = new_tx;
            }
            input.set_enabled(true);
            tx.send(InputEv::Key { down: true, ch: 2 }).unwrap();
            tx.send(InputEv::Down {
                button: 2,
                x: 50,
                y: 60,
            })
            .unwrap();
            assert_eq!(input.consume_native_frame(&mut shell), MouseOwner::User);
            let mut expected = [0; 128];
            expected[2] = 1;
            assert_eq!(shell.key_held, expected);
            assert_eq!(shell.poll_key(), -1, "detached text must not replay");
            assert_eq!(
                (shell.mouse_button, shell.mouse_click_x, shell.mouse_click_y),
                (2, 50, 60)
            );
            tx.send(InputEv::Key { down: false, ch: 2 }).unwrap();
            tx.send(InputEv::Up).unwrap();
            input.consume_native_frame(&mut shell);
            assert_eq!(shell.key_held, [0; 128]);
            assert_eq!(shell.mouse_button, 0);
        }
    }

    #[test]
    fn keyboard_detach_gates_keys_but_keeps_new_mouse_click_through() {
        let input = live_input();
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        input.set_enabled(true);
        tx.send(InputEv::Key { down: true, ch: 1 }).unwrap();
        let mut shell = GameShell::new();
        input.drain(&mut shell);
        input.set_keyboard_enabled(false);
        tx.send(InputEv::Key { down: true, ch: 58 }).unwrap();
        tx.send(InputEv::Down {
            button: 2,
            x: 20,
            y: 30,
        })
        .unwrap();
        assert_eq!(input.consume_native_frame(&mut shell), MouseOwner::User);
        assert_eq!(shell.key_held, [0; 128]);
        assert_eq!(shell.poll_key(), -1);
        assert_eq!((shell.mouse_button, shell.mouse_click_button), (2, 2));
        input.set_keyboard_enabled(true);
        tx.send(InputEv::Key { down: true, ch: 65 }).unwrap();
        input.drain(&mut shell);
        assert_eq!(shell.key_held[65], 1);
        assert_eq!(shell.poll_key(), 65);
        assert_eq!(shell.mouse_button, 0);
    }

    #[test]
    fn keyboard_reenable_keeps_undrained_click() {
        let input = live_input();
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        input.set_enabled(true);
        let mut shell = GameShell::new();
        input.set_keyboard_enabled(false);
        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 30,
        })
        .unwrap();
        input.set_keyboard_enabled(true);
        assert_eq!(input.consume_native_frame(&mut shell), MouseOwner::User);
        assert_eq!(shell.mouse_button, 1);
        assert_eq!(
            (
                shell.mouse_click_button,
                shell.mouse_click_x,
                shell.mouse_click_y
            ),
            (1, 20, 30)
        );
    }

    #[test]
    fn release_latch_preserves_non_user_keys() {
        let input = live_input();
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        input.set_enabled(true);
        let mut shell = GameShell::new();
        input.drain(&mut shell);
        shell.apply_key(true, 0, 65);
        tx.send(InputEv::Key { down: true, ch: 1 }).unwrap();
        input.drain(&mut shell);
        assert_eq!((shell.key_held[1], shell.key_held[65]), (1, 1));
        input.set_enabled(false);
        input.drain(&mut shell);
        assert_eq!(shell.key_held[1], 0);
        assert_eq!(shell.key_held[65], 1);
        assert_eq!(shell.poll_key(), 65);
        assert_eq!(
            shell.poll_key(),
            -1,
            "latch up must not enter the chat ring"
        );
    }

    #[test]
    fn same_slot_connect_rx_releases_without_repress() {
        let input = live_input();
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        input.set_enabled(true);
        tx.send(InputEv::Key { down: true, ch: 65 }).unwrap();
        let mut shell = GameShell::new();
        input.drain(&mut shell);
        assert_eq!(shell.key_held[65], 1);
        assert_eq!(shell.poll_key(), 65);
        let (tx2, rx2) = std::sync::mpsc::channel();
        input.connect_rx(rx2);
        input.drain(&mut shell);
        assert_eq!(shell.key_held[65], 0);
        assert_eq!(shell.poll_key(), -1, "same-slot reattach must not re-press");
        tx2.send(InputEv::Key { down: true, ch: 2 }).unwrap();
        input.drain(&mut shell);
        assert_eq!(shell.key_held[2], 1);
        assert_eq!(shell.key_held[65], 0);
    }

    #[test]
    fn user_detach_and_stale_mouse_up_do_not_release_script_hold() {
        let input = live_input();
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        input.set_enabled(true);
        let mut shell = GameShell::new();
        input.enqueue_script_mouse(true, 20.0, 30.0, 0);
        assert!(matches!(
            input.consume_native_frame(&mut shell),
            MouseOwner::Script { .. }
        ));
        input.set_enabled(false);
        tx.send(InputEv::Up).unwrap();
        input.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        assert!(input.script_held());
        input.enqueue_script_mouse(false, 20.0, 30.0, 0);
        input.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
    }

    fn intent_client() -> client::client::Client {
        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        });
        client.ingame = true;
        client
    }

    #[test]
    fn manual_intent_requires_winning_user_click_not_matching_coordinates_or_hold() {
        let input = live_input();
        input.set_enabled(true);
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        let mut client = intent_client();
        input.enqueue_script_mouse(true, 20.0, 20.0, 0);
        let owner = input.consume_native_frame(&mut client.shell);
        assert!(matches!(owner, MouseOwner::Script { .. }));
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(input.take_manual_move_intent(), None);

        // Same coordinates, competing script down, and a released user
        // button still produce one winning User click.
        input.enqueue_script_mouse(true, 20.0, 20.0, 0);
        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 20,
        })
        .unwrap();
        tx.send(InputEv::Up).unwrap();
        let owner = input.consume_native_frame(&mut client.shell);
        assert_eq!(owner, MouseOwner::User);
        assert_eq!(client.shell.mouse_button, 0);
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(
            input.take_manual_move_intent(),
            Some(ManualMoveIntent::Viewport)
        );
        assert_eq!(input.take_manual_move_intent(), None);

        // Held buttons and old click coordinates never replay a fact.
        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 20,
        })
        .unwrap();
        let owner = input.consume_native_frame(&mut client.shell);
        input.classify_manual_move_intent(&client, owner);
        let owner = input.consume_native_frame(&mut client.shell);
        assert_eq!(owner, MouseOwner::None);
        assert_eq!(client.shell.mouse_button, 1);
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(input.take_manual_move_intent(), None);
    }

    #[test]
    fn manual_intent_uses_latched_winner_and_excludes_discarded_input() {
        let input = live_input();
        input.set_enabled(true);
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        let mut client = intent_client();
        for (button, x, y) in [(1, 20, 20), (2, 30, 30)] {
            tx.send(InputEv::Down { button, x, y }).unwrap();
        }
        let owner = input.consume_native_frame(&mut client.shell);
        assert_eq!(
            (client.shell.mouse_click_button, client.shell.mouse_click_x),
            (2, 30)
        );
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(input.take_manual_move_intent(), None);

        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 20,
        })
        .unwrap();
        tx.send(InputEv::Up).unwrap();
        input.discard_user(&mut client.shell);
        let owner = input.consume_native_frame(&mut client.shell);
        assert_eq!(owner, MouseOwner::None);
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(input.take_manual_move_intent(), None);

        input.set_enabled(false);
        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 20,
        })
        .unwrap();
        let owner = input.consume_native_frame(&mut client.shell);
        assert_eq!(owner, MouseOwner::None);
        input.classify_manual_move_intent(&client, owner);
        assert_eq!(input.take_manual_move_intent(), None);
    }

    #[test]
    fn manual_intent_modal_menu_and_minimap_policy() {
        let mut client = intent_client();
        client.shell.mouse_click_button = 1;
        for (x, y, expected) in [
            (5, 5, Some(ManualMoveIntent::Viewport)),
            (515, 337, Some(ManualMoveIntent::Viewport)),
            (4, 5, None),
            (516, 5, None),
            (5, 4, None),
            (5, 338, None),
            (560, 220, None),
            (30, 400, None),
            (550, 180, None),
            (575, 8, Some(ManualMoveIntent::Minimap)),
            (720, 158, Some(ManualMoveIntent::Minimap)),
            (574, 83, None),
            (721, 83, None),
            (648, 7, None),
            (648, 159, None),
        ] {
            client.shell.mouse_click_x = x;
            client.shell.mouse_click_y = y;
            assert_eq!(
                ManualMoveIntent::classify(&client, MouseOwner::User),
                expected,
                "({x},{y})"
            );
        }
        client.shell.mouse_click_x = 30;
        client.shell.mouse_click_y = 51;
        client.main_modal_id = 100;
        assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
        client.main_modal_id = -1;
        client.is_menu_open = true;
        client.menu_area = 0;
        client.menu_x = 20;
        client.menu_y = 20;
        client.menu_width = 100;
        client.menu_num_entries = 1;
        // A real row (including Cancel/Examine) qualifies; an off-row
        // world click is only menu dismissal, not raw viewport intent.
        assert_eq!(
            ManualMoveIntent::classify(&client, MouseOwner::User),
            Some(ManualMoveIntent::WorldMenu)
        );
        client.shell.mouse_click_y = 20;
        assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
        for (area, x, y) in [(1, 583, 252), (2, 47, 404)] {
            client.menu_area = area;
            client.shell.mouse_click_x = x;
            client.shell.mouse_click_y = y;
            assert!(client.menu_option_at(x, y).is_some());
            assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
        }
        client.menu_area = 0;
        client.main_modal_id = 100;
        client.shell.mouse_click_x = 648;
        client.shell.mouse_click_y = 83;
        assert_eq!(
            ManualMoveIntent::classify(&client, MouseOwner::User),
            Some(ManualMoveIntent::Minimap)
        );
        client.minimap_state = 1;
        assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
        client.minimap_state = 0;
        client.shell.mouse_click_button = 2;
        assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
        client.shell.mouse_click_button = 1;
        client.ingame = false;
        assert_eq!(ManualMoveIntent::classify(&client, MouseOwner::User), None);
    }

    #[test]
    fn map_script_mouse_keeps_fractional_center_and_refuses_negative() {
        assert_eq!(
            SlotInput::map_script_mouse(382.5, 251.5, 0),
            Some((382, 251, 1))
        );
        assert_eq!(SlotInput::map_script_mouse(-0.25, 10.0, 0), None);
        assert_eq!(SlotInput::map_script_mouse(f64::NAN, 10.0, 0), None);
        assert_eq!(SlotInput::map_script_mouse(10.0, f64::INFINITY, 0), None);
        assert_eq!(SlotInput::map_script_mouse(10.0, 10.0, 2), None);
        assert_eq!(SlotInput::map_script_mouse(765.0, 0.0, 0), None);
        assert_eq!(SlotInput::map_script_mouse(0.0, 0.0, 0), Some((0, 0, 1)));
    }

    #[test]
    fn capture_off_consumes_script_center_down_then_up() {
        let inp = live_input();
        inp.set_enabled(false);
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            (
                shell.mouse_click_button,
                shell.mouse_click_x,
                shell.mouse_click_y
            ),
            (1, 382, 251)
        );
        assert_eq!(shell.mouse_button, 1);
        assert_eq!(shell.mouse_x, 382);
        assert_eq!(shell.mouse_y, 251);
        inp.enqueue_script_mouse(false, 382.5, 251.5, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
        assert_eq!(shell.mouse_click_button, 0);
    }

    #[test]
    fn revoke_before_consume_drops_queued_down() {
        let inp = live_input();
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        assert_eq!(inp.script_queue_len(), 1);
        inp.authority().revoke();
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_click_button, 0);
        assert_eq!(shell.mouse_button, 0);
        assert_eq!(inp.script_queue_len(), 0);
    }

    #[test]
    fn stop_after_consumed_down_releases_held_without_new_click() {
        let inp = live_input();
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_click_button, 1);
        assert!(inp.script_held());
        inp.authority().revoke();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
        assert_eq!(shell.mouse_click_button, 0);
        assert!(!inp.script_held());
    }

    #[test]
    fn produced_identity_mismatch_does_not_restamp() {
        let inp = live_input();
        let live = inp.authority().lock().identity();
        inp.enqueue_script_mouse_at(live.wrapping_add(1), true, 100.0, 100.0, 0);
        assert_eq!(inp.script_queue_len(), 0);
        inp.enqueue_script_mouse_at(0, true, 100.0, 100.0, 0);
        assert_eq!(inp.script_queue_len(), 0);
        inp.enqueue_script_mouse_at(live, true, 100.0, 100.0, 0);
        assert_eq!(inp.script_queue_len(), 1);
    }

    #[test]
    fn older_script_up_does_not_release_newer_same_identity_hold() {
        let inp = live_input();
        let mut shell = client::client::GameShell::new();

        inp.enqueue_script_mouse(true, 10.0, 10.0, 0);
        inp.consume_native_frame(&mut shell);
        inp.enqueue_script_mouse(true, 20.0, 20.0, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        assert_eq!(shell.mouse_click_x, 20);

        inp.enqueue_script_mouse(false, 10.0, 10.0, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            shell.mouse_button, 1,
            "the older gesture's up must not release the newer hold"
        );

        inp.enqueue_script_mouse(false, 20.0, 20.0, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
    }

    #[test]
    fn root_mouse_revoked_hold_must_not_survive_resume_before_frame() {
        let inp = live_input();
        let mut shell = client::client::GameShell::new();
        inp.enqueue_script_mouse(true, 100.0, 100.0, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        inp.authority().revoke();
        inp.authority().resume();
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            shell.mouse_button, 0,
            "revoked old hold survives Resume before next frame"
        );
    }

    #[test]
    fn resume_before_frame_preserves_user_hold() {
        use std::sync::mpsc;
        let inp = live_input();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        inp.set_enabled(true);
        tx.send(InputEv::Down {
            button: 1,
            x: 5,
            y: 6,
        })
        .unwrap();
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        inp.authority().revoke();
        inp.authority().resume();
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            shell.mouse_button, 1,
            "user hold must survive script revoke+resume"
        );
        assert_eq!(shell.mouse_click_x, 5);
    }

    #[test]
    fn user_down_wins_same_frame_and_delayed_script_up_does_not_release() {
        use std::sync::mpsc;
        let inp = live_input();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        inp.set_enabled(true);
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        tx.send(InputEv::Down {
            button: 1,
            x: 10,
            y: 20,
        })
        .unwrap();
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            (
                shell.mouse_click_button,
                shell.mouse_click_x,
                shell.mouse_click_y
            ),
            (1, 10, 20)
        );
        assert_eq!(shell.mouse_button, 1);
        inp.enqueue_script_mouse(false, 382.5, 251.5, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            shell.mouse_button, 1,
            "delayed script up must not release user hold"
        );
        tx.send(InputEv::Up).unwrap();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
    }

    #[test]
    fn script_click_works_after_consumed_user_click() {
        use std::sync::mpsc;
        let inp = live_input();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        inp.set_enabled(true);
        tx.send(InputEv::Down {
            button: 1,
            x: 11,
            y: 12,
        })
        .unwrap();
        tx.send(InputEv::Up).unwrap();
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_click_button, 1);
        assert_eq!(shell.mouse_button, 0);
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            (
                shell.mouse_click_button,
                shell.mouse_click_x,
                shell.mouse_click_y
            ),
            (1, 382, 251)
        );
    }

    #[test]
    fn two_slots_stay_isolated() {
        let a = live_input();
        let b = live_input();
        a.enqueue_script_mouse(true, 382.5, 251.5, 0);
        let mut shell_a = client::client::GameShell::new();
        let mut shell_b = client::client::GameShell::new();
        a.consume_native_frame(&mut shell_a);
        b.consume_native_frame(&mut shell_b);
        assert_eq!(shell_a.mouse_button, 1);
        assert_eq!(shell_b.mouse_button, 0);
        assert_eq!(shell_b.mouse_click_button, 0);
        assert_eq!(shell_b.mouse_x, -1);
    }

    #[test]
    fn queue_overflow_releases_script_hold() {
        let inp = live_input();
        inp.enqueue_script_mouse(true, 10.0, 10.0, 0);
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        for _ in 0..33 {
            inp.enqueue_script_mouse(true, 10.0, 10.0, 0);
        }
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 0);
        assert_eq!(inp.script_queue_len(), 0);
    }

    #[test]
    fn same_frame_down_up_latches_once_and_releases() {
        let inp = live_input();
        inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
        inp.enqueue_script_mouse(false, 382.5, 251.5, 0);
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_click_button, 1);
        assert_eq!(shell.mouse_button, 0);
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_click_button, 0);
    }

    #[test]
    fn host_hold_revokes_script_without_clearing_user() {
        use std::sync::mpsc;
        let inp = live_input();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        inp.set_enabled(true);
        tx.send(InputEv::Down {
            button: 1,
            x: 5,
            y: 6,
        })
        .unwrap();
        let mut shell = client::client::GameShell::new();
        inp.consume_native_frame(&mut shell);
        assert_eq!(shell.mouse_button, 1);
        inp.enqueue_script_mouse(true, 100.0, 100.0, 0);
        inp.set_host_consume_allowed(false);
        inp.consume_native_frame(&mut shell);
        assert_eq!(
            shell.mouse_button, 1,
            "user hold must survive script revoke"
        );
        assert_eq!(shell.mouse_click_x, 5);
    }

    #[test]
    fn wake_channel_fires_a_polling_parker_and_drains() {
        let (wake, park) = wake_channel();
        assert!(
            !park.wait_readable(Duration::from_millis(0)),
            "no kick yet: wait must time out"
        );
        wake.wake();
        assert!(
            park.wait_readable(Duration::from_millis(1000)),
            "a wake byte must fire the wait"
        );
        park.drain();
        assert!(
            !park.wait_readable(Duration::from_millis(0)),
            "drained: wait goes quiet again"
        );
    }

    #[test]
    fn wake_channel_clones_share_one_wake() {
        let (wake, park) = wake_channel();
        let wake2 = wake.clone();
        wake2.wake();
        assert!(
            park.wait_readable(Duration::from_millis(1000)),
            "a cloned SlotWake must wake the same park"
        );
    }

    #[test]
    fn wake_channel_queued_kicks_drain_without_residual() {
        let (wake, park) = wake_channel();
        for _ in 0..8 {
            wake.wake();
        }
        assert!(park.wait_readable(Duration::from_millis(1000)));
        park.drain();
        assert!(
            !park.wait_readable(Duration::from_millis(0)),
            "drain must clear coalesced kicks with no residual readability"
        );
    }

    #[test]
    fn wait_readable_reports_socket_data_and_close() {
        use std::io::Write;
        use std::net::{Shutdown, TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        #[cfg(unix)]
        let handle = {
            use std::os::unix::io::AsRawFd;
            client.as_raw_fd()
        };
        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::AsRawSocket;
            client.as_raw_socket()
        };

        assert!(
            !wait_readable(&[handle], Duration::from_millis(0))[0],
            "idle TCP pair must not be readable"
        );
        server.write_all(&[7]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if wait_readable(&[handle], Duration::from_millis(0))[0] {
                break;
            }
            assert!(Instant::now() < deadline, "socket never became readable");
            std::thread::sleep(Duration::from_millis(5));
        }
        // Consume the byte so only close remains to wake the next wait.
        {
            use std::io::Read;
            let mut b = [0u8; 1];
            let _ = (&client).read(&mut b);
        }
        server.shutdown(Shutdown::Both).unwrap();
        assert!(
            wait_readable(&[handle], Duration::from_millis(1000))[0],
            "peer close must wake readability"
        );
    }

    #[test]
    fn wait_readable_honors_bounded_timeout() {
        let (_wake, park) = wake_channel();
        let start = Instant::now();
        let fired = wait_readable(&[park.wait_handle()], Duration::from_millis(80));
        assert!(!fired[0]);
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(60),
            "timeout returned too early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "timeout waited too long: {elapsed:?}"
        );
    }
}
