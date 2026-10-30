use std::collections::VecDeque;
use std::io::{Read, Write};
#[cfg(feature = "journal-paint-proof")]
use std::num::NonZeroUsize;
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
pub(crate) use std::os::unix::io::RawFd as WaitHandle;
#[cfg(windows)]
pub(crate) use std::os::windows::io::RawSocket as WaitHandle;

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

/// One immutable paint witness. A GPU proof keeps its process readback permit
/// until the consumer drops these pixels.
#[cfg(feature = "journal-paint-proof")]
pub struct JournalProofFrame {
    pub generation: u64,
    pub pixmap: client::graphics::PixMap,
    pub stamp: Option<JournalPaintStamp>,
    pub gpu: bool,
    frame_stamp: FrameStamp,
    _readback_permit: Option<client::render::backend::ReadbackPermit>,
}

#[cfg(feature = "journal-paint-proof")]
impl JournalProofFrame {
    pub fn frame_stamp(&self) -> &FrameStamp {
        &self.frame_stamp
    }
}

#[cfg(feature = "journal-paint-proof")]
struct JournalProofPolicy {
    record_ceiling: NonZeroUsize,
    readback_timeout: Duration,
}

#[cfg(feature = "journal-paint-proof")]
struct JournalConsumerRegistration {
    head: HeadIdentity,
    registration: client::render::backend::ConsumerRegistration,
}

#[cfg(feature = "journal-paint-proof")]
struct PendingJournalProof {
    readback: client::render::backend::PendingReadback,
    head: HeadIdentity,
    stamp: Option<JournalPaintStamp>,
    token: client::render::backend::PresentToken,
    geometry: client::render::backend::CaptureGeometry,
}

#[cfg(feature = "journal-paint-proof")]
struct JournalProofQueue {
    frames: VecDeque<JournalProofFrame>,
    policy: Option<JournalProofPolicy>,
    registration: Option<JournalConsumerRegistration>,
    pending: Option<PendingJournalProof>,
    resolving_head: Option<HeadIdentity>,
    last_error: Option<client::render::backend::ReadbackError>,
}

#[cfg(feature = "journal-paint-proof")]
impl JournalProofQueue {
    fn new() -> Self {
        Self {
            frames: VecDeque::new(),
            policy: None,
            registration: None,
            pending: None,
            resolving_head: None,
            last_error: None,
        }
    }
}
#[cfg(all(feature = "journal-paint-proof", feature = "render-test-hooks"))]
fn fair_render_enabled() -> bool {
    crate::render_head::fair_enabled()
}

#[cfg(all(feature = "journal-paint-proof", not(feature = "render-test-hooks")))]
fn fair_render_enabled() -> bool {
    false
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
    frame_generation: AtomicU64,
    #[cfg(feature = "journal-paint-proof")]
    proof_frames: Option<parking_lot::Mutex<JournalProofQueue>>,
}

/// Identity and scene authority frozen with one accepted mailbox frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameStamp {
    head: HeadIdentity,
    mailbox_sequence: u64,
    present_token: Option<client::render::backend::PresentToken>,
    has_valid_scene: bool,
}

impl FrameStamp {
    pub const fn head(&self) -> HeadIdentity {
        self.head
    }

    pub const fn mailbox_sequence(&self) -> u64 {
        self.mailbox_sequence
    }

    pub const fn present_token(&self) -> Option<client::render::backend::PresentToken> {
        self.present_token
    }

    pub const fn has_valid_scene(&self) -> bool {
        self.has_valid_scene
    }
}

/// Single-consumer output, authority and diagnostics move as one envelope.
pub struct PresentedFrame {
    output: FrameOutput,
    stamp: FrameStamp,
    #[cfg(feature = "render-diagnostics")]
    roi: Option<client::render::diagnostics::PixelRoiMeta>,
}

impl PresentedFrame {
    pub fn stamp(&self) -> &FrameStamp {
        &self.stamp
    }

    pub fn output(&self) -> &FrameOutput {
        &self.output
    }

    pub fn into_output(self) -> FrameOutput {
        self.output
    }

    #[cfg(feature = "render-diagnostics")]
    pub fn roi(&self) -> Option<&client::render::diagnostics::PixelRoiMeta> {
        self.roi.as_ref()
    }

    #[cfg(feature = "render-diagnostics")]
    pub fn take_roi(&mut self) -> Option<client::render::diagnostics::PixelRoiMeta> {
        self.roi.take()
    }
}

/// One lock owns the latest frame, the head allowed to publish it and,
/// under `render-diagnostics`, the sidecar that belongs to it. Publish and
/// take share this critical section so a later publish cannot attach a new
/// ROI to an earlier take.
struct Mailbox {
    frame: Option<PresentedFrame>,
    /// The attached head whose frames this mailbox accepts: set when the
    /// head attaches, cleared when it retires.
    head: Option<HeadIdentity>,
    /// Why the slot's draw request has no head, while its attach is refused.
    refused: Option<RenderRefusal>,
    render_fault: Option<client::render::backend::RenderFault>,
    #[cfg(feature = "draw-memory-profile")]
    next_render_bytes_request: Option<u64>,
    #[cfg(feature = "draw-memory-profile")]
    render_bytes_request: Option<u64>,
    #[cfg(feature = "draw-memory-profile")]
    render_bytes_sample: Option<client::render::budget::RenderBytes>,
    #[cfg(feature = "render-test-hooks")]
    fair_stats: Option<crate::render_head::HeadFairStats>,
}

/// By-value paint metadata; no payload allocation or framebuffer layout change.
#[derive(Clone, Copy, Default)]
pub(crate) struct FramePaintStatus {
    cpu_has_valid_scene: bool,
    recovered_fault: Option<client::render::backend::RenderFault>,
}

impl FramePaintStatus {
    pub(crate) const fn new(
        cpu_has_valid_scene: bool,
        recovered_fault: Option<client::render::backend::RenderFault>,
    ) -> Self {
        Self {
            cpu_has_valid_scene,
            recovered_fault,
        }
    }
}

impl FrameBuf {
    /// Fixed per-loaded-bot Arc allocation, including the inline mailbox and
    /// optional proof-queue descriptors. Pixel and queue payloads are separate
    /// owners. This storage exists even when no render head is attached.
    #[cfg(any(test, feature = "render-test-hooks"))]
    pub fn fixed_storage_bytes() -> usize {
        let header = std::alloc::Layout::new::<(
            std::sync::atomic::AtomicUsize,
            std::sync::atomic::AtomicUsize,
        )>();
        header
            .extend(std::alloc::Layout::new::<Self>())
            .expect("the fixed mailbox Arc layout fits usize")
            .0
            .pad_to_align()
            .size()
    }

    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(Mailbox {
                frame: None,
                head: None,
                refused: None,
                render_fault: None,
                #[cfg(feature = "draw-memory-profile")]
                next_render_bytes_request: Some(1),
                #[cfg(feature = "draw-memory-profile")]
                render_bytes_request: None,
                #[cfg(feature = "draw-memory-profile")]
                render_bytes_sample: None,
                #[cfg(feature = "render-test-hooks")]
                fair_stats: None,
            }),
            frame_generation: AtomicU64::new(0),
            #[cfg(feature = "journal-paint-proof")]
            proof_frames: None,
        })
    }
    /// Create a fixture mailbox that retains every legacy paint when fair
    /// render is off. Fair render requires explicit bounded policy setup.
    #[cfg(feature = "journal-paint-proof")]
    pub fn new_for_journal_proof() -> std::sync::Arc<Self> {
        let mut mailbox = Self::new();
        std::sync::Arc::get_mut(&mut mailbox)
            .expect("new fixture mailbox has one owner")
            .proof_frames = Some(parking_lot::Mutex::new(JournalProofQueue::new()));
        mailbox
    }

    /// Configure finite fair-gate journal capture before the first proof.
    /// The caller supplies both the proof-record ceiling and readback timeout.
    #[cfg(feature = "journal-paint-proof")]
    pub fn configure_journal_proof_policy(
        &self,
        record_ceiling: NonZeroUsize,
        readback_timeout: Duration,
    ) -> Result<(), client::render::backend::ReadbackError> {
        let queue = self
            .proof_frames
            .as_ref()
            .ok_or(client::render::backend::ReadbackError::Ungated)?;
        let mut queue = queue.lock();
        if queue.policy.is_some()
            || !queue.frames.is_empty()
            || queue.registration.is_some()
            || queue.pending.is_some()
            || queue.resolving_head.is_some()
        {
            return Err(client::render::backend::ReadbackError::Limit);
        }
        let mut frames = VecDeque::new();
        frames
            .try_reserve_exact(record_ceiling.get())
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        queue.frames = frames;
        queue.policy = Some(JournalProofPolicy {
            record_ceiling,
            readback_timeout,
        });
        queue.last_error = None;
        Ok(())
    }

    /// Return the last typed fair-gate capture refusal, if any.
    #[cfg(feature = "journal-paint-proof")]
    pub fn journal_proof_error(&self) -> Option<client::render::backend::ReadbackError> {
        self.proof_frames.as_ref()?.lock().last_error
    }

    #[cfg(feature = "journal-paint-proof")]
    pub fn take_journal_proof_result(
        &self,
    ) -> Result<Option<JournalProofFrame>, client::render::backend::ReadbackError> {
        let Some(queue) = &self.proof_frames else {
            return Ok(None);
        };
        let progress = if fair_render_enabled() {
            self.poll_journal_readback(queue)
        } else {
            Ok(())
        };
        let mut queue = queue.lock();
        if let Some(frame) = queue.frames.pop_front() {
            return Ok(Some(frame));
        }
        progress?;
        match queue.last_error.take() {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    #[cfg(feature = "journal-paint-proof")]
    pub(crate) fn publish_journal_paint(
        &self,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: JournalPaintStamp,
        status: FramePaintStatus,
    ) -> bool {
        if let Some(queue) = &self.proof_frames {
            self.publish_proof(queue, head, frame, Some(stamp), status)
        } else {
            self.publish_with_status(head, frame, status)
        }
    }

    /// Publish `head`'s paint into the proof queue, if `head` is attached.
    /// The identity check and the publication share one mailbox critical
    /// section, as in [`FrameBuf::publish_with_status`]: `retire_head`/`attach_head`
    /// take the same lock, so a head retired after its check cannot publish
    /// into its slot's reattached generation.
    #[cfg(feature = "journal-paint-proof")]
    fn publish_proof(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: Option<JournalPaintStamp>,
        status: FramePaintStatus,
    ) -> bool {
        if !frame.is_fresh() {
            self.update_render_fault(head, frame.render_fault());
            return false;
        }
        if fair_render_enabled() {
            self.publish_bounded_proof(queue, head, frame, stamp, status)
        } else {
            self.publish_legacy_proof(queue, head, frame, stamp, status)
        }
    }

    #[cfg(feature = "journal-paint-proof")]
    fn update_render_fault(
        &self,
        head: HeadIdentity,
        fault: Option<client::render::backend::RenderFault>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) {
            inner.render_fault = fault;
        }
    }

    #[cfg(feature = "journal-paint-proof")]
    fn publish_legacy_proof(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: Option<JournalPaintStamp>,
        status: FramePaintStatus,
    ) -> bool {
        let (present_token, has_valid_scene) = match &frame {
            FrameOutput::PixMap(_) => (None, status.cpu_has_valid_scene),
            FrameOutput::Texture(handle) => (handle.present_token(), handle.has_valid_scene()),
            FrameOutput::Refused(fault) => {
                self.update_render_fault(head, Some(*fault));
                return false;
            }
        };
        let (pixmap, gpu) = match frame {
            FrameOutput::PixMap(pixmap) => (pixmap, false),
            FrameOutput::Texture(handle) => (
                client::graphics::PixMap {
                    width: handle.width() as i32,
                    height: handle.height() as i32,
                    pixels: handle.read_back(),
                },
                true,
            ),
            FrameOutput::Refused(fault) => {
                self.update_render_fault(head, Some(fault));
                return false;
            }
        };
        let mut inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            return false;
        }
        #[cfg(test)]
        tests::pause_in_publication_gap();
        let mut queue = queue.lock();
        let Some(generation) = self.frame_generation.load(Ordering::Relaxed).checked_add(1) else {
            queue.last_error = Some(client::render::backend::ReadbackError::IdentityExhausted);
            return false;
        };
        inner.render_fault = status.recovered_fault;
        queue.frames.push_back(JournalProofFrame {
            generation,
            pixmap,
            stamp,
            gpu,
            frame_stamp: FrameStamp {
                head,
                mailbox_sequence: generation,
                present_token,
                has_valid_scene,
            },
            _readback_permit: None,
        });
        queue.last_error = None;
        self.frame_generation.store(generation, Ordering::Relaxed);
        true
    }

    #[cfg(feature = "journal-paint-proof")]
    fn publish_bounded_proof(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        frame: FrameOutput,
        stamp: Option<JournalPaintStamp>,
        status: FramePaintStatus,
    ) -> bool {
        if let Err(error) = self.poll_journal_readback(queue) {
            Self::record_journal_error(queue, error);
        }
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.head != Some(head) {
                drop(inner);
                return Self::refuse_journal_proof(
                    queue,
                    client::render::backend::ReadbackError::Retired,
                );
            }
            if matches!(&frame, FrameOutput::Texture(_)) {
                inner.render_fault = status.recovered_fault;
            }
        }
        match frame {
            FrameOutput::PixMap(pixmap) => {
                self.publish_bounded_cpu_proof(queue, head, pixmap, stamp, status)
            }
            FrameOutput::Texture(handle) => {
                self.publish_bounded_gpu_proof(queue, head, handle, stamp)
            }
            FrameOutput::Refused(fault) => {
                self.update_render_fault(head, Some(fault));
                false
            }
        }
    }

    #[cfg(feature = "journal-paint-proof")]
    fn publish_bounded_cpu_proof(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        pixmap: client::graphics::PixMap,
        stamp: Option<JournalPaintStamp>,
        status: FramePaintStatus,
    ) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            drop(inner);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::Retired,
            );
        }
        let mut queue = queue.lock();
        let Some(policy) = queue.policy.as_ref() else {
            queue.last_error = Some(client::render::backend::ReadbackError::Ungated);
            return false;
        };
        let occupied = queue
            .frames
            .len()
            .saturating_add(if queue.pending.is_some() { 1 } else { 0 })
            .saturating_add(if queue.resolving_head.is_some() { 1 } else { 0 });
        if occupied >= policy.record_ceiling.get() {
            queue.last_error = Some(client::render::backend::ReadbackError::Limit);
            return false;
        }
        if queue.frames.capacity() < policy.record_ceiling.get() {
            queue.last_error = Some(client::render::backend::ReadbackError::Limit);
            return false;
        }
        #[cfg(test)]
        tests::pause_in_publication_gap();
        let Some(generation) = self.frame_generation.load(Ordering::Relaxed).checked_add(1) else {
            queue.last_error = Some(client::render::backend::ReadbackError::IdentityExhausted);
            return false;
        };
        inner.render_fault = status.recovered_fault;
        queue.frames.push_back(JournalProofFrame {
            generation,
            pixmap,
            stamp,
            gpu: false,
            frame_stamp: FrameStamp {
                head,
                mailbox_sequence: generation,
                present_token: None,
                has_valid_scene: status.cpu_has_valid_scene,
            },
            _readback_permit: None,
        });
        queue.last_error = None;
        self.frame_generation.store(generation, Ordering::Relaxed);
        true
    }

    #[cfg(feature = "journal-paint-proof")]
    fn publish_bounded_gpu_proof(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        handle: client::render::backend::TextureHandle,
        stamp: Option<JournalPaintStamp>,
    ) -> bool {
        let (timeout, needs_registration, old_registration) = {
            let mut queue = queue.lock();
            let Some(policy) = queue.policy.as_ref() else {
                queue.last_error = Some(client::render::backend::ReadbackError::Ungated);
                return false;
            };
            let occupied = queue
                .frames
                .len()
                .saturating_add(if queue.pending.is_some() { 1 } else { 0 })
                .saturating_add(if queue.resolving_head.is_some() { 1 } else { 0 });
            if occupied >= policy.record_ceiling.get() {
                queue.last_error = Some(client::render::backend::ReadbackError::Limit);
                return false;
            }
            let timeout = policy.readback_timeout;
            if queue.pending.is_some() || queue.resolving_head.is_some() {
                queue.last_error = Some(client::render::backend::ReadbackError::Busy);
                return false;
            }
            let old_registration = if queue
                .registration
                .as_ref()
                .is_some_and(|registration| registration.head != head)
            {
                queue.registration.take()
            } else {
                None
            };
            (timeout, queue.registration.is_none(), old_registration)
        };
        drop(old_registration);

        if self.inner.lock().unwrap().head != Some(head) {
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::Retired,
            );
        }

        if needs_registration {
            let registration = match handle.try_register_consumer() {
                Ok(registration) => registration,
                Err(error) => return Self::refuse_journal_proof(queue, error),
            };
            let inner = self.inner.lock().unwrap();
            if inner.head != Some(head) {
                drop(inner);
                drop(registration);
                return Self::refuse_journal_proof(
                    queue,
                    client::render::backend::ReadbackError::Retired,
                );
            }
            let mut queue_state = queue.lock();
            if queue_state.registration.is_some() {
                queue_state.last_error = Some(client::render::backend::ReadbackError::Busy);
                drop(queue_state);
                drop(inner);
                drop(registration);
                return false;
            }
            queue_state.registration = Some(JournalConsumerRegistration { head, registration });
            drop(queue_state);
            drop(inner);
        }

        let consumer = {
            let queue_state = queue.lock();
            let Some(registration) = queue_state
                .registration
                .as_ref()
                .filter(|registration| registration.head == head)
            else {
                return Self::refuse_journal_proof(
                    queue,
                    client::render::backend::ReadbackError::Retired,
                );
            };
            match registration.registration.try_take_frame() {
                Ok(consumer) => consumer,
                Err(error) => return Self::refuse_journal_proof(queue, error),
            }
        };
        let Some(consumer) = consumer else {
            return false;
        };
        let Some(token) = handle.present_token() else {
            drop(consumer);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::Ungated,
            );
        };
        if consumer.present_token() != token {
            drop(consumer);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::StalePresent,
            );
        }
        if !consumer.has_valid_scene() {
            drop(consumer);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::InvalidScene,
            );
        }
        let geometry =
            client::render::backend::CaptureGeometry::new(handle.width(), handle.height());
        let Some(deadline) = std::time::Instant::now().checked_add(timeout) else {
            drop(consumer);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::Timeout,
            );
        };
        let permit = match client::render::backend::ReadbackBudget::process()
            .try_acquire(geometry, deadline)
        {
            Ok(permit) => permit,
            Err(error) => {
                drop(consumer);
                return Self::refuse_journal_proof(queue, error);
            }
        };
        let pending = match consumer
            .try_begin_readback(client::render::backend::ReadbackKind::ValidScene, permit)
        {
            Ok(pending) => pending,
            Err((error, consumer, permit)) => {
                drop(consumer);
                drop(permit);
                return Self::refuse_journal_proof(queue, error);
            }
        };
        let pending = PendingJournalProof {
            readback: pending,
            head,
            stamp,
            token,
            geometry,
        };
        let inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            drop(inner);
            drop(pending);
            return Self::refuse_journal_proof(
                queue,
                client::render::backend::ReadbackError::Retired,
            );
        }
        let mut queue_state = queue.lock();
        let Some(policy) = queue_state.policy.as_ref() else {
            queue_state.last_error = Some(client::render::backend::ReadbackError::Ungated);
            drop(queue_state);
            drop(inner);
            drop(pending);
            return false;
        };
        let occupied = queue_state
            .frames
            .len()
            .saturating_add(if queue_state.pending.is_some() { 1 } else { 0 })
            .saturating_add(if queue_state.resolving_head.is_some() {
                1
            } else {
                0
            });
        if occupied >= policy.record_ceiling.get()
            || queue_state.pending.is_some()
            || queue_state.resolving_head.is_some()
        {
            queue_state.last_error = Some(client::render::backend::ReadbackError::Busy);
            drop(queue_state);
            drop(inner);
            drop(pending);
            return false;
        }
        queue_state.pending = Some(pending);
        queue_state.last_error = None;
        true
    }

    #[cfg(feature = "journal-paint-proof")]
    fn poll_journal_readback(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
    ) -> Result<(), client::render::backend::ReadbackError> {
        let mut pending = {
            let Some(mut queue) = queue.try_lock() else {
                return Err(client::render::backend::ReadbackError::Busy);
            };
            let Some(pending) = queue.pending.take() else {
                return Ok(());
            };
            queue.resolving_head = Some(pending.head);
            pending
        };
        match pending.readback.poll(std::time::Instant::now()) {
            Ok(None) => {
                let inner = self.inner.lock().unwrap();
                let mut queue_state = queue.lock();
                if inner.head == Some(pending.head)
                    && queue_state.resolving_head == Some(pending.head)
                    && queue_state.pending.is_none()
                {
                    queue_state.pending = Some(pending);
                    queue_state.resolving_head = None;
                    Ok(())
                } else {
                    queue_state.resolving_head = None;
                    queue_state.last_error = Some(client::render::backend::ReadbackError::Retired);
                    drop(queue_state);
                    drop(inner);
                    drop(pending);
                    Err(client::render::backend::ReadbackError::Retired)
                }
            }
            Ok(Some(completed)) => self.complete_journal_readback(queue, pending, completed),
            Err(error) => Err(self.finish_journal_readback_error(queue, pending.head, error)),
        }
    }

    #[cfg(feature = "journal-paint-proof")]
    fn complete_journal_readback(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        pending: PendingJournalProof,
        completed: client::render::backend::CompletedReadback,
    ) -> Result<(), client::render::backend::ReadbackError> {
        let (rgba, token, has_valid_scene, geometry, permit) = completed.into_parts();
        if token != pending.token {
            drop(rgba);
            drop(permit);
            return Err(self.finish_journal_readback_error(
                queue,
                pending.head,
                client::render::backend::ReadbackError::StalePresent,
            ));
        }
        if !has_valid_scene {
            drop(rgba);
            drop(permit);
            return Err(self.finish_journal_readback_error(
                queue,
                pending.head,
                client::render::backend::ReadbackError::InvalidScene,
            ));
        }
        if geometry != pending.geometry {
            drop(rgba);
            drop(permit);
            return Err(self.finish_journal_readback_error(
                queue,
                pending.head,
                client::render::backend::ReadbackError::Limit,
            ));
        }
        if self.inner.lock().unwrap().head != Some(pending.head) {
            drop(rgba);
            drop(permit);
            return Err(self.finish_journal_readback_error(
                queue,
                pending.head,
                client::render::backend::ReadbackError::Retired,
            ));
        }
        let (pixmap, permit) = match Self::journal_pixmap_from_rgba(rgba, geometry, permit) {
            Ok(converted) => converted,
            Err(error) => {
                return Err(self.finish_journal_readback_error(queue, pending.head, error));
            }
        };
        let mut proof = JournalProofFrame {
            generation: 0,
            pixmap,
            stamp: pending.stamp,
            gpu: true,
            frame_stamp: FrameStamp {
                head: pending.head,
                mailbox_sequence: 0,
                present_token: Some(token),
                has_valid_scene,
            },
            _readback_permit: Some(permit),
        };
        let mut inner = self.inner.lock().unwrap();
        let mut queue_state = queue.lock();
        let error = if inner.head != Some(pending.head)
            || queue_state.resolving_head != Some(pending.head)
        {
            Some(client::render::backend::ReadbackError::Retired)
        } else if queue_state.policy.is_none() {
            Some(client::render::backend::ReadbackError::Ungated)
        } else if queue_state.frames.len()
            >= queue_state
                .policy
                .as_ref()
                .expect("policy checked above")
                .record_ceiling
                .get()
            || queue_state.frames.capacity()
                < queue_state
                    .policy
                    .as_ref()
                    .expect("policy checked above")
                    .record_ceiling
                    .get()
        {
            Some(client::render::backend::ReadbackError::Limit)
        } else {
            None
        };
        if let Some(error) = error {
            queue_state.resolving_head = None;
            queue_state.last_error = Some(error);
            drop(queue_state);
            drop(inner);
            drop(proof);
            return Err(error);
        }
        let Some(generation) = self.frame_generation.load(Ordering::Relaxed).checked_add(1) else {
            queue_state.resolving_head = None;
            queue_state.last_error =
                Some(client::render::backend::ReadbackError::IdentityExhausted);
            drop(queue_state);
            drop(inner);
            drop(proof);
            return Err(client::render::backend::ReadbackError::IdentityExhausted);
        };
        #[cfg(test)]
        tests::pause_in_publication_gap();
        proof.generation = generation;
        proof.frame_stamp.mailbox_sequence = generation;
        inner.render_fault = None;
        queue_state.frames.push_back(proof);
        queue_state.resolving_head = None;
        queue_state.last_error = None;
        self.frame_generation.store(generation, Ordering::Relaxed);
        Ok(())
    }

    #[cfg(feature = "journal-paint-proof")]
    fn journal_pixmap_from_rgba(
        rgba: Vec<u8>,
        geometry: client::render::backend::CaptureGeometry,
        permit: client::render::backend::ReadbackPermit,
    ) -> Result<
        (
            client::graphics::PixMap,
            client::render::backend::ReadbackPermit,
        ),
        client::render::backend::ReadbackError,
    > {
        let width = usize::try_from(geometry.width())
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        let height = usize::try_from(geometry.height())
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        let pixels_len = width
            .checked_mul(height)
            .ok_or(client::render::backend::ReadbackError::Limit)?;
        let raw_len = pixels_len
            .checked_mul(4)
            .ok_or(client::render::backend::ReadbackError::Limit)?;
        if rgba.len() != raw_len || permit.raw_bytes() != raw_len {
            return Err(client::render::backend::ReadbackError::Device);
        }
        let envelope = permit
            .stage_bytes()
            .checked_add(permit.raw_bytes())
            .ok_or(client::render::backend::ReadbackError::Limit)?;
        let raw_and_pixels = rgba
            .capacity()
            .checked_add(
                pixels_len
                    .checked_mul(std::mem::size_of::<i32>())
                    .ok_or(client::render::backend::ReadbackError::Limit)?,
            )
            .ok_or(client::render::backend::ReadbackError::Limit)?;
        if raw_and_pixels > envelope {
            return Err(client::render::backend::ReadbackError::Limit);
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(pixels_len)
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        let converted_capacity = pixels
            .capacity()
            .checked_mul(std::mem::size_of::<i32>())
            .ok_or(client::render::backend::ReadbackError::Limit)?;
        if rgba
            .capacity()
            .checked_add(converted_capacity)
            .ok_or(client::render::backend::ReadbackError::Limit)?
            > envelope
        {
            return Err(client::render::backend::ReadbackError::Limit);
        }
        for rgba_pixel in rgba.as_chunks::<4>().0 {
            pixels.push(
                ((rgba_pixel[0] as i32) << 16)
                    | ((rgba_pixel[1] as i32) << 8)
                    | rgba_pixel[2] as i32,
            );
        }
        if pixels.len() != pixels_len {
            return Err(client::render::backend::ReadbackError::Device);
        }
        drop(rgba);
        let width = i32::try_from(geometry.width())
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        let height = i32::try_from(geometry.height())
            .map_err(|_| client::render::backend::ReadbackError::Limit)?;
        Ok((
            client::graphics::PixMap {
                width,
                height,
                pixels,
            },
            permit,
        ))
    }

    #[cfg(feature = "journal-paint-proof")]
    fn finish_journal_readback_error(
        &self,
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: HeadIdentity,
        error: client::render::backend::ReadbackError,
    ) -> client::render::backend::ReadbackError {
        let inner = self.inner.lock().unwrap();
        let mut queue_state = queue.lock();
        let error = if inner.head == Some(head) {
            error
        } else {
            client::render::backend::ReadbackError::Retired
        };
        if queue_state.resolving_head == Some(head) {
            queue_state.resolving_head = None;
        }
        queue_state.last_error = Some(error);
        error
    }

    #[cfg(feature = "journal-paint-proof")]
    fn record_journal_error(
        queue: &parking_lot::Mutex<JournalProofQueue>,
        error: client::render::backend::ReadbackError,
    ) {
        queue.lock().last_error = Some(error);
    }

    #[cfg(feature = "journal-paint-proof")]
    fn refuse_journal_proof(
        queue: &parking_lot::Mutex<JournalProofQueue>,
        error: client::render::backend::ReadbackError,
    ) -> bool {
        Self::record_journal_error(queue, error);
        false
    }

    #[cfg(feature = "journal-paint-proof")]
    fn cancel_journal_resources(
        queue: &parking_lot::Mutex<JournalProofQueue>,
        head: Option<HeadIdentity>,
    ) {
        let (pending, registration) = {
            let mut queue = queue.lock();
            let matches_head = |candidate: HeadIdentity| head.is_none_or(|head| candidate == head);
            let pending = if queue
                .pending
                .as_ref()
                .is_some_and(|pending| matches_head(pending.head))
            {
                queue.pending.take()
            } else {
                None
            };
            let registration = if queue
                .registration
                .as_ref()
                .is_some_and(|registration| matches_head(registration.head))
            {
                queue.registration.take()
            } else {
                None
            };
            (pending, registration)
        };
        drop(pending);
        drop(registration);
    }

    /// Store `head`'s latest frame and bump the generation, if `head` is the
    /// head attached to this mailbox; otherwise drop the frame and return
    /// false. The full [`FrameOutput`] is kept so the panel can bind a
    /// `FrameOutput::Texture` or pack a `PixMap`.
    #[cfg(test)]
    pub(crate) fn publish(&self, head: HeadIdentity, frame: FrameOutput) -> bool {
        self.publish_with_status(head, frame, FramePaintStatus::default())
    }

    /// Publish scene validity and a recovered fault with the exact output.
    /// GPU authority is always read from its submitted present token.
    pub(crate) fn publish_with_status(
        &self,
        head: HeadIdentity,
        frame: FrameOutput,
        status: FramePaintStatus,
    ) -> bool {
        #[cfg(feature = "journal-paint-proof")]
        if let Some(queue) = &self.proof_frames {
            return self.publish_proof(queue, head, frame, None, status);
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.head != Some(head) {
            return false;
        }
        if !frame.is_fresh() {
            inner.render_fault = frame.render_fault();
            return false;
        }
        let Some(generation) = self.frame_generation.load(Ordering::Relaxed).checked_add(1) else {
            inner.render_fault =
                Some(client::render::backend::RenderFault::MailboxSequenceExhausted);
            return false;
        };
        #[cfg(test)]
        tests::pause_in_publication_gap();
        let (present_token, has_valid_scene) = match &frame {
            FrameOutput::Texture(handle) => (handle.present_token(), handle.has_valid_scene()),
            FrameOutput::PixMap(_) => (None, status.cpu_has_valid_scene),
            FrameOutput::Refused(_) => return false,
        };
        let presented = PresentedFrame {
            output: frame,
            stamp: FrameStamp {
                head,
                mailbox_sequence: generation,
                present_token,
                has_valid_scene,
            },
            #[cfg(feature = "render-diagnostics")]
            roi: client::render::diagnostics::take_thread_pixel_roi(),
        };
        inner.render_fault = status.recovered_fault;
        inner.frame = Some(presented);
        self.frame_generation.store(generation, Ordering::Relaxed);
        true
    }

    /// `head` attached: from now on its frames are accepted, and a refusal
    /// shown for this slot is over.
    pub(crate) fn attach_head(&self, head: HeadIdentity) {
        let mut inner = self.inner.lock().unwrap();
        #[cfg(feature = "journal-paint-proof")]
        if inner.head != Some(head) {
            if let Some(queue) = &self.proof_frames {
                Self::cancel_journal_resources(queue, inner.head);
            }
        }
        inner.head = Some(head);
        inner.refused = None;
        inner.render_fault = None;
        #[cfg(feature = "draw-memory-profile")]
        {
            inner.render_bytes_request = None;
            inner.render_bytes_sample = None;
        }
        #[cfg(feature = "render-test-hooks")]
        {
            inner.fair_stats = None;
        }
    }

    /// `head` retired at a detach: stop accepting its frames and drain the
    /// mailbox (the frame is dropped before the head's backend). One critical
    /// section, so the drain cannot take a frame a later head published.
    pub(crate) fn retire_head(&self, head: HeadIdentity) -> Option<FrameOutput> {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) {
            #[cfg(feature = "journal-paint-proof")]
            if let Some(queue) = &self.proof_frames {
                Self::cancel_journal_resources(queue, Some(head));
            }
            inner.head = None;
            inner.render_fault = None;
            #[cfg(feature = "draw-memory-profile")]
            {
                inner.render_bytes_request = None;
                inner.render_bytes_sample = Some(Default::default());
            }
        }
        Self::take_locked(&mut inner)
    }

    /// `head` ended with its slot loop: stop accepting its frames, but keep
    /// the last one for the consumer.
    pub(crate) fn release_head(&self, head: HeadIdentity) {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) {
            #[cfg(feature = "journal-paint-proof")]
            if let Some(queue) = &self.proof_frames {
                Self::cancel_journal_resources(queue, Some(head));
            }
            inner.head = None;
            inner.render_fault = None;
            #[cfg(feature = "draw-memory-profile")]
            {
                inner.render_bytes_request = None;
                inner.render_bytes_sample = Some(Default::default());
            }
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

    /// A failed paint leaves the previous picture and its generation intact.
    pub fn render_fault(&self) -> Option<client::render::backend::RenderFault> {
        self.inner.lock().unwrap().render_fault
    }

    /// Request an allocation-free owner census on the slot thread. A request
    /// supersedes older in-flight samples; a detached slot owns no renderer.
    #[cfg(feature = "draw-memory-profile")]
    pub fn request_render_bytes_sample(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.render_bytes_sample = None;
        inner.render_bytes_request = None;
        let Some(request) = inner.next_render_bytes_request else {
            return;
        };
        inner.next_render_bytes_request = request.checked_add(1);
        if inner.head.is_some() {
            inner.render_bytes_request = Some(request);
        } else {
            inner.render_bytes_sample = Some(Default::default());
        }
    }

    #[cfg(feature = "draw-memory-profile")]
    pub fn render_bytes_sample(&self) -> Option<client::render::budget::RenderBytes> {
        self.inner.lock().unwrap().render_bytes_sample
    }

    #[cfg(feature = "draw-memory-profile")]
    pub(crate) fn render_output_storage_bytes(&self) -> usize {
        match self.inner.lock().unwrap().frame.as_ref() {
            Some(PresentedFrame {
                output: FrameOutput::PixMap(pixmap),
                ..
            }) => pixmap.pixels.capacity() * std::mem::size_of::<i32>(),
            _ => 0,
        }
    }

    #[cfg(feature = "draw-memory-profile")]
    pub(crate) fn render_bytes_request(&self, head: HeadIdentity) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        (inner.head == Some(head))
            .then_some(inner.render_bytes_request)
            .flatten()
    }

    #[cfg(feature = "draw-memory-profile")]
    pub(crate) fn publish_render_bytes_sample(
        &self,
        head: HeadIdentity,
        request: u64,
        sample: client::render::budget::RenderBytes,
    ) {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(head) && inner.render_bytes_request == Some(request) {
            inner.render_bytes_request = None;
            inner.render_bytes_sample = Some(sample);
        }
    }

    /// The latest fixed-size sample for this mailbox's render generation.
    #[cfg(feature = "render-test-hooks")]
    pub fn fair_stats(&self) -> Option<crate::render_head::HeadFairStats> {
        self.inner.lock().unwrap().fair_stats
    }

    /// Start diagnostic gap accounting without changing the actual consume timestamp.
    #[cfg(feature = "render-test-hooks")]
    pub fn begin_scene_gap_window(&self, start: std::time::Instant) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(sample) = inner.fair_stats.as_mut() else {
            return false;
        };
        sample.begin_scene_gap_window(start);
        true
    }

    #[cfg(feature = "render-test-hooks")]
    pub(crate) fn publish_fair_stats(&self, mut sample: crate::render_head::HeadFairStats) {
        let mut inner = self.inner.lock().unwrap();
        if inner.head == Some(sample.identity()) {
            if let Some(previous) = inner.fair_stats {
                sample.inherit_scene_gap_window(previous);
            }
            inner.fair_stats = Some(sample);
        }
    }

    /// Move output, stamp and ROI together. A subsequent publish cannot
    /// replace any part of this accepted frame.
    pub fn take_presented(&self) -> Option<PresentedFrame> {
        self.inner.lock().unwrap().frame.take()
    }

    /// Output-only convenience for callers that do not claim scene-proof
    /// authority. The envelope's diagnostics leave with the output.
    pub fn take(&self) -> Option<FrameOutput> {
        self.take_presented().map(PresentedFrame::into_output)
    }

    fn take_locked(inner: &mut Mailbox) -> Option<FrameOutput> {
        inner.frame.take().map(PresentedFrame::into_output)
    }
    /// CPU path: pack the latest `PixMap`'s pixels via `pack_rgb` (765×503,
    /// same shape the panel's texture upload expects). Empty when nothing
    /// was stored yet, the frame is `FrameOutput::Texture`, or the `PixMap`
    /// is not full-applet sized.
    pub fn snapshot(&self) -> Vec<u32> {
        let n = (APPLET_W * APPLET_H) as usize;
        let inner = self.inner.lock().unwrap();
        let mut out = Vec::with_capacity(n);
        if let Some(PresentedFrame {
            output: FrameOutput::PixMap(pix),
            ..
        }) = &inner.frame
        {
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
        self.frame_generation.load(Ordering::Relaxed)
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
        use client::client::MiniMenuAction;
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
            if client.menu_area != 0 {
                return None;
            }
            let row = client.menu_option_at(x, y)?;
            let mut action = *client.menu_action.get(row as usize)?;
            if action >= MiniMenuAction::_PRIORITY {
                action -= MiniMenuAction::_PRIORITY;
            }
            // Cancel and Examine rows never take over: they act in place or
            // do nothing. Every other chosen world-menu row still qualifies.
            if matches!(
                action,
                MiniMenuAction::CANCEL
                    | MiniMenuAction::OP_LOC6
                    | MiniMenuAction::OP_NPC6
                    | MiniMenuAction::OP_OBJ6
            ) {
                return None;
            }
            return Some(Self::WorldMenu);
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
    render_focused: AtomicBool,
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
    pub(crate) input_observation: crate::input_observation::InputObservation,
}

impl SlotInput {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            enabled: AtomicBool::new(false),
            full_rate: AtomicBool::new(false),
            render_focused: AtomicBool::new(false),
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
            input_observation: Default::default(),
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
    /// The actual Game-pane focus, independent of render cadence.
    pub fn set_render_focused(&self, on: bool) {
        self.render_focused.store(on, Ordering::Relaxed);
    }
    pub fn render_focused(&self) -> bool {
        self.render_focused.load(Ordering::Relaxed)
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
                    self.foreign_local_input();
                    shell.apply_mouse_down(button, x, y);
                    mouse.held = MouseOwner::User;
                    mouse.pending = MouseOwner::User;
                }
                InputEv::Up => Self::release_user_mouse(shell, &mut mouse),
                InputEv::Key { down, ch } if !down || (enabled && user.keyboard_enabled) => {
                    if down {
                        self.foreign_local_input();
                    }
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
                self.foreign_local_input();
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

impl api::identity::Notifier for SlotWake {
    fn notify(&self, _: api::identity::Notice) {
        self.wake();
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
        map_image_to_applet, wait_readable, wake_channel, FrameBuf, FramePaintStatus, InputEv,
        ManualMoveIntent, MouseOwner, SlotInput,
    };
    use crate::render::{HeadIdentity, RenderRefusal, RenderService};
    use api::identity::{Notice, Notifier, SlotKey};
    use client::client::{Client, ClientConfig, ClientRevision, GameShell, MiniMenuAction};
    use client::graphics::PixMap;
    use client::render::backend::{FrameOutput, RenderFault};
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

    #[cfg(feature = "render-test-hooks")]
    struct FairGpuControl {
        now: std::sync::Mutex<Instant>,
    }

    #[cfg(feature = "render-test-hooks")]
    impl FairGpuControl {
        fn advance(&self, elapsed: Duration) {
            let mut now = self.now.lock().unwrap();
            *now = now.checked_add(elapsed).unwrap();
        }

        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
    }

    #[cfg(feature = "render-test-hooks")]
    impl crate::render_head::test_hooks::RenderTestControl for FairGpuControl {
        fn now(&self) -> Instant {
            FairGpuControl::now(self)
        }

        fn at(&self, _event: crate::render_head::test_hooks::RenderTestEvent) {}
    }

    #[cfg(feature = "render-test-hooks")]
    fn configure_fair_gpu_process() {
        use std::num::NonZeroUsize;

        client::render::backend::configure_retained_present_budget(NonZeroUsize::new(1).unwrap())
            .expect("the isolated process configures one retained GPU image");
        crate::render_head::test_hooks::configure_process(
            crate::render_head::test_hooks::ProcessFixture {
                grant_ttl: Duration::from_millis(20),
                quantum: Duration::from_millis(40),
            },
        )
        .expect("the isolated process configures its explicit fair fixture");
        crate::render_head::pump(Instant::now());
    }

    #[cfg(feature = "render-test-hooks")]
    fn fair_gpu_client() -> client::client::Client {
        use client::client::{Client, ClientConfig, ClientRevision};
        use client::dash3d::TerrainOverlayShape;

        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/nonexistent-retained-present-fixture".into(),
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
        client.cam_x = 16 * 128;
        client.cam_z = 4 * 128;
        client.cam_y = -600;
        client.cam_pitch = 200;
        client.cam_yaw = 0;
        client
    }

    #[cfg(feature = "render-test-hooks")]
    fn publish_head_frame(head: &crate::render_head::Head, frame: FrameOutput) -> bool {
        #[cfg(not(feature = "journal-paint-proof"))]
        {
            head.publish(frame)
        }
        #[cfg(feature = "journal-paint-proof")]
        {
            head.publish_journal_paint(
                frame,
                super::JournalPaintStamp {
                    modal_root: -1,
                    journal_paint_hidden: false,
                },
            )
        }
    }

    #[cfg(feature = "render-test-hooks")]
    fn presented_gpu_snapshot(mailbox: &FrameBuf) -> (super::FrameStamp, Vec<i32>) {
        let inner = mailbox.inner.lock().unwrap();
        let frame = inner
            .frame
            .as_ref()
            .expect("the prior mailbox frame remains present");
        let pixels = match frame.output() {
            FrameOutput::Texture(texture) => texture.read_back(),
            _ => panic!("the required GPU fixture publishes a texture"),
        };
        (*frame.stamp(), pixels)
    }

    #[cfg(feature = "render-test-hooks")]
    struct PresentGateHolder {
        release: Option<std::sync::mpsc::Sender<()>>,
        worker: Option<std::thread::JoinHandle<bool>>,
    }

    #[cfg(feature = "render-test-hooks")]
    impl PresentGateHolder {
        fn start(handle: client::render::backend::TextureHandle) -> Self {
            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                client::render::backend::gpu::test_hooks::with_present_gate_locked(&handle, || {
                    locked_tx
                        .send(())
                        .expect("the paint test still waits for gate acquisition");
                    release_rx.recv_timeout(Duration::from_secs(15)).is_ok()
                })
            });
            locked_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("the fixture locks the real tracked present gate");
            Self {
                release: Some(release_tx),
                worker: Some(worker),
            }
        }

        fn release_and_join(mut self) -> bool {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.worker
                .take()
                .expect("the gate holder owns its worker")
                .join()
                .expect("the gate holder worker exits")
        }
    }

    #[cfg(feature = "render-test-hooks")]
    impl Drop for PresentGateHolder {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    #[cfg(feature = "render-test-hooks")]
    fn presented_gpu_stamp(mailbox: &FrameBuf) -> super::FrameStamp {
        let inner = mailbox.inner.lock().unwrap();
        *inner
            .frame
            .as_ref()
            .expect("the prior mailbox frame remains present")
            .stamp()
    }

    #[cfg(feature = "render-test-hooks")]
    #[test]
    #[ignore = "requires an explicitly configured GPU adapter"]
    fn read_submission_gate_never_waits_on_simulation() {
        const CHILD: &str = "R274_PRESENT_GATE_NONBLOCKING_CHILD";
        const TEST: &str = "slot_io::tests::read_submission_gate_never_waits_on_simulation";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--ignored", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .expect("spawn an isolated GPU present-gate fixture");
            assert!(
                output.status.success(),
                "present-gate child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        configure_fair_gpu_process();
        let control = Arc::new(FairGpuControl {
            now: std::sync::Mutex::new(Instant::now()),
        });
        let mailbox = FrameBuf::new();
        let (wake, park) = wake_channel();
        let render = crate::render_head::SlotRender::new(
            SlotKey::from_incarnation(91).unwrap(),
            Arc::clone(&mailbox),
            Arc::new(wake),
        )
        .with_test_control(control.clone());
        let mut head = crate::render_head::Head::attach(&render, false, false)
            .expect("the explicit fixture attaches a real renderer");
        assert_eq!(
            head.backend_kind(),
            client::render::backend::BackendKind::Gpu
        );
        let mut client = fair_gpu_client();

        head.note_due(control.now()).unwrap();
        let initial = head.paint(&mut client);
        assert!(initial.is_fresh(), "the real GPU head creates N");
        assert!(publish_head_frame(&head, initial));
        let original_stamp = presented_gpu_stamp(&mailbox);
        assert!(original_stamp.has_valid_scene());
        assert_eq!(original_stamp.head(), head.identity());

        let witness = head.paint(&mut client);
        assert!(witness.is_fresh(), "the retained head advances its display");
        let FrameOutput::Texture(witness) = witness else {
            panic!("the real GPU frame carries the present-gate witness");
        };
        let before_pixels = presented_gpu_snapshot(&mailbox).1;
        let holder = PresentGateHolder::start(witness);

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let contended = head.paint(&mut client);
        assert!(
            !contended.is_fresh(),
            "present contention returns a nonfresh frame"
        );
        assert_eq!(
            contended.render_fault(),
            Some(client::render::backend::RenderFault::PresentContended)
        );
        assert!(!publish_head_frame(&head, contended));
        assert_eq!(
            mailbox.generation(),
            1,
            "contention does not publish a sequence"
        );
        assert_eq!(presented_gpu_stamp(&mailbox), original_stamp);
        assert_eq!(
            presented_gpu_snapshot(&mailbox).1,
            before_pixels,
            "the held present gate prevents a partial display update"
        );
        assert_eq!(
            mailbox.render_fault(),
            Some(client::render::backend::RenderFault::PresentContended)
        );
        assert!(
            holder.release_and_join(),
            "the real gate stays held through paint"
        );

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let recovered = head.paint(&mut client);
        assert!(
            recovered.is_fresh(),
            "the producer progresses after gate release"
        );
        assert!(publish_head_frame(&head, recovered));
        assert_eq!(mailbox.generation(), 2);
        let recovered_stamp = presented_gpu_stamp(&mailbox);
        assert_eq!(recovered_stamp.head(), original_stamp.head());
        assert_eq!(recovered_stamp.mailbox_sequence(), 2);
        assert_ne!(
            recovered_stamp.present_token(),
            original_stamp.present_token()
        );
        assert!(recovered_stamp.has_valid_scene());
        assert_eq!(mailbox.render_fault(), None);

        head.detach();
        drop(park);
    }

    #[cfg(feature = "render-test-hooks")]
    #[test]
    #[ignore = "requires an explicitly configured GPU adapter"]
    fn present_submission_and_mailbox_publication_gap_cannot_forge_valid_scene() {
        use client::render::backend::{CaptureGeometry, ReadbackBudget, ReadbackKind};
        const CHILD: &str = "R274_PRESENT_STAMP_GAP_CHILD";
        const TEST: &str = "slot_io::tests::present_submission_and_mailbox_publication_gap_cannot_forge_valid_scene";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--ignored", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .expect("spawn an isolated GPU stamp fixture");
            assert!(
                output.status.success(),
                "present-stamp child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        configure_fair_gpu_process();
        let control = Arc::new(FairGpuControl {
            now: std::sync::Mutex::new(Instant::now()),
        });
        let mailbox = FrameBuf::new();
        let (wake, park) = wake_channel();
        let render = crate::render_head::SlotRender::new(
            SlotKey::from_incarnation(92).unwrap(),
            Arc::clone(&mailbox),
            Arc::new(wake),
        )
        .with_test_control(control.clone());
        let mut head = crate::render_head::Head::attach(&render, false, false)
            .expect("the explicit fixture attaches a real renderer");
        assert_eq!(
            head.backend_kind(),
            client::render::backend::BackendKind::Gpu
        );
        let mut client = fair_gpu_client();

        head.note_due(control.now()).unwrap();
        let bootstrap = head.paint(&mut client);
        assert!(
            bootstrap.is_fresh(),
            "the real GPU head creates a bootstrap frame"
        );
        let registration = match &bootstrap {
            FrameOutput::Texture(texture) => texture
                .try_register_consumer()
                .expect("the explicit head registers its real present"),
            _ => panic!("the required GPU fixture publishes a texture"),
        };
        drop(bootstrap);

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let valid_n = head.paint(&mut client);
        assert!(valid_n.is_fresh(), "the registered head creates frame N");
        let token_n = match &valid_n {
            FrameOutput::Texture(texture) => {
                assert!(texture.has_valid_scene());
                texture.present_token().expect("valid frame N has a token")
            }
            _ => panic!("the required GPU fixture publishes a texture"),
        };
        let held_n = registration
            .try_take_frame()
            .expect("consumer admission remains live")
            .expect("the next successful present offers retained N");
        assert_eq!(held_n.present_token(), token_n);
        assert!(held_n.has_valid_scene());
        assert!(publish_head_frame(&head, valid_n));
        let stamp_n = presented_gpu_stamp(&mailbox);
        assert!(stamp_n.has_valid_scene());
        assert_eq!(stamp_n.present_token(), Some(token_n));
        let expected_n_pixels = presented_gpu_snapshot(&mailbox).1;

        client.scene_state = 0;
        let invalid_n1 = head.paint(&mut client);
        assert!(
            invalid_n1.is_fresh(),
            "the invalid scene still has a usable GPU frame"
        );
        let FrameOutput::Texture(texture_n1) = &invalid_n1 else {
            panic!("the invalid GPU frame carries its by-value token and validity");
        };
        let token_n1 = texture_n1
            .present_token()
            .expect("the invalid submitted frame has its own token");
        assert_ne!(token_n1, token_n);
        assert!(!texture_n1.has_valid_scene());

        assert_eq!(
            mailbox.generation(),
            1,
            "N+1 remains unpublished while its present already exists"
        );
        assert_eq!(presented_gpu_stamp(&mailbox), stamp_n);

        assert!(publish_head_frame(&head, invalid_n1));
        assert_eq!(mailbox.generation(), 2);
        let stamp_n1 = presented_gpu_stamp(&mailbox);
        assert_eq!(stamp_n1.head(), stamp_n.head());
        assert_eq!(stamp_n1.mailbox_sequence(), 2);
        assert_eq!(stamp_n1.present_token(), Some(token_n1));
        assert!(
            !stamp_n1.has_valid_scene(),
            "the invalid N+1 cannot borrow N's validity"
        );
        assert_eq!(stamp_n.present_token(), Some(token_n));
        assert!(
            stamp_n.has_valid_scene(),
            "the accepted N stamp remains immutable"
        );

        let permit = ReadbackBudget::process()
            .try_acquire(
                CaptureGeometry::new(765, 503),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("the isolated fixture owns the readback permit");
        let mut pending = match held_n.try_begin_readback(ReadbackKind::ValidScene, permit) {
            Ok(pending) => pending,
            Err((error, frame, permit)) => {
                drop(frame);
                drop(permit);
                panic!("held valid N readback was refused: {error:?}");
            }
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let completed = loop {
            match pending.poll(Instant::now()) {
                Ok(Some(completed)) => break completed,
                Ok(None) => {
                    assert!(Instant::now() < deadline, "held N readback completes");
                    std::thread::yield_now();
                }
                Err(error) => panic!("held N readback failed: {error:?}"),
            }
        };
        let (rgba, token, valid, geometry, permit) = completed.into_parts();
        assert_eq!(geometry, CaptureGeometry::new(765, 503));
        assert_eq!(token, token_n);
        assert!(valid, "later invalid N+1 cannot change N's scene validity");
        assert_eq!(rgba.len(), expected_n_pixels.len() * 4);
        let pixels = rgba.as_chunks::<4>().0;
        for (index, (pixel, expected)) in pixels.iter().zip(&expected_n_pixels).enumerate() {
            let expected = *expected as u32;
            assert_eq!(pixel[0], (expected >> 16) as u8, "N red channel at {index}");
            assert_eq!(
                pixel[1],
                (expected >> 8) as u8,
                "N green channel at {index}"
            );
            assert_eq!(pixel[2], expected as u8, "N blue channel at {index}");
        }
        drop(permit);
        drop(registration);
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            0,
            "the isolated retained frame is released before detach"
        );
        head.detach();
        drop(park);
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
    fn refused_paint_preserves_previous_frame_and_fresh_paint_clears_fault() {
        let buf = FrameBuf::new();
        let head = head_identity(70);
        buf.attach_head(head);
        assert!(buf.publish(head, applet_pixmap(vec![11; 765 * 503])));
        assert_eq!(buf.generation(), 1);

        let fault = RenderFault::PresentContended;
        assert!(!buf.publish(head, FrameOutput::Refused(fault)));
        assert_eq!(buf.generation(), 1);
        assert_eq!(buf.render_fault(), Some(fault));
        match buf.take() {
            Some(FrameOutput::PixMap(pixmap)) => assert_eq!(pixmap.pixels[0], 11),
            _ => panic!("a refused output must preserve the previous frame"),
        }

        assert!(buf.publish(head, applet_pixmap(vec![22; 765 * 503])));
        assert_eq!(buf.generation(), 2);
        assert_eq!(buf.render_fault(), None);
        match buf.take() {
            Some(FrameOutput::PixMap(pixmap)) => assert_eq!(pixmap.pixels[0], 22),
            _ => panic!("a fresh output must replace the previous frame"),
        }
    }

    #[test]
    fn recovered_cpu_paint_publishes_new_pixels_and_retains_its_typed_fault() {
        let buf = FrameBuf::new();
        let head = head_identity(76);
        buf.attach_head(head);
        assert!(buf.publish(head, applet_pixmap(vec![11; 765 * 503])));
        let fault = RenderFault::Budget(client::render::budget::RenderLimit::new(
            client::render::budget::RenderResource::HeadMisc,
            2,
            1,
        ));
        assert!(buf.publish_with_status(
            head,
            applet_pixmap(vec![22; 765 * 503]),
            FramePaintStatus::new(true, Some(fault)),
        ));
        assert_eq!(buf.generation(), 2);
        assert_eq!(buf.render_fault(), Some(fault));
        let recovered = buf.take_presented().expect("the recovered paint is usable");
        assert!(recovered.stamp().has_valid_scene());
        match recovered.into_output() {
            FrameOutput::PixMap(frame) => assert_eq!(frame.pixels[0], 22),
            _ => panic!("the recovered CPU paint must publish its complete pixels"),
        }
        assert!(buf.publish(head, applet_pixmap(vec![33; 765 * 503])));
        assert_eq!(buf.generation(), 3);
        assert_eq!(buf.render_fault(), None);
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn recovered_cpu_proof_publishes_pixels_and_typed_fault_together() {
        use std::num::NonZeroUsize;

        let buf = FrameBuf::new_for_journal_proof();
        let head = head_identity(78);
        buf.attach_head(head);
        buf.configure_journal_proof_policy(
            NonZeroUsize::new(1).unwrap(),
            Duration::from_millis(100),
        )
        .unwrap();
        let fault = RenderFault::Budget(client::render::budget::RenderLimit::new(
            client::render::budget::RenderResource::NavInput,
            2,
            1,
        ));
        let queue = buf.proof_frames.as_ref().unwrap();
        assert!(buf.publish_bounded_proof(
            queue,
            head,
            applet_pixmap(vec![22; 765 * 503]),
            None,
            FramePaintStatus::new(true, Some(fault)),
        ));
        assert_eq!(buf.render_fault(), Some(fault));
        assert_eq!(buf.generation(), 1);
        let proof = buf.take_journal_proof_result().unwrap().unwrap();
        assert_eq!(proof.pixmap.pixels[0], 22);
        assert!(proof.frame_stamp().has_valid_scene());
    }

    #[test]
    fn retired_head_cannot_fault_or_replace_its_successor() {
        let buf = FrameBuf::new();
        let (retired, successor) = head_generations(71);
        buf.attach_head(retired);
        assert!(buf.publish(retired, applet_pixmap(vec![31; 765 * 503])));
        assert!(buf.retire_head(retired).is_some());
        buf.attach_head(successor);
        assert!(buf.publish(successor, applet_pixmap(vec![32; 765 * 503])));
        assert_eq!(buf.generation(), 2);

        assert!(!buf.publish(retired, FrameOutput::Refused(RenderFault::PresentContended)));
        assert_eq!(buf.render_fault(), None);
        assert_eq!(buf.generation(), 2);
        match buf.take() {
            Some(FrameOutput::PixMap(pixmap)) => assert_eq!(pixmap.pixels[0], 32),
            _ => panic!("a retired head must not replace its successor's frame"),
        }
    }

    #[cfg(all(feature = "journal-paint-proof", feature = "render-test-hooks"))]
    #[test]
    #[ignore = "requires a wgpu adapter and an isolated process-global fixture"]
    fn journal_proofs_are_process_bounded_and_do_not_wait_on_slot() {
        use std::num::NonZeroUsize;
        use std::sync::{Arc, Mutex};

        use client::client::{Client, ClientConfig, ClientRevision};
        use client::dash3d::TerrainOverlayShape;
        use client::render::backend::gpu::test_hooks::SceneTestPoint;
        use client::render::backend::{
            configure_retained_present_budget, BackendKind, CaptureGeometry, FrameOutput,
            ReadbackBudget, ReadbackError, RenderFault,
        };
        use client::render::Renderer;

        use crate::render_head::test_hooks::{ProcessFixture, RenderTestControl, RenderTestEvent};
        use crate::render_head::{Head, SlotRender};

        struct FixtureControl {
            now: Mutex<Instant>,
            failure: Mutex<Option<SceneTestPoint>>,
        }

        impl FixtureControl {
            fn advance(&self, elapsed: Duration) {
                let mut now = self.now.lock().unwrap();
                *now = now.checked_add(elapsed).unwrap();
            }
        }

        impl RenderTestControl for FixtureControl {
            fn now(&self) -> Instant {
                *self.now.lock().unwrap()
            }

            fn resource_failure(&self) -> Option<SceneTestPoint> {
                self.failure.lock().unwrap().take()
            }

            fn at(&self, _event: RenderTestEvent) {}
        }

        let scene_client = || {
            let mut client = Client::new_with_revision(
                ClientConfig {
                    host: "127.0.0.1".into(),
                    port: 43594,
                    cache_dir: "/nonexistent-journal-proof-fixture".into(),
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
            client.cam_x = 16 * 128;
            client.cam_z = 4 * 128;
            client.cam_y = -600;
            client.cam_pitch = 200;
            client.cam_yaw = 0;
            client
        };
        let stamp = || super::JournalPaintStamp {
            modal_root: 1,
            journal_paint_hidden: false,
        };

        {
            // Before process fixture initialization, preserve the exact legacy
            // gate-off behavior, including the blocking TextureHandle readback.
            assert!(!crate::render_head::fair_enabled());
            let mut legacy_renderer = Renderer::new_prefer(false, true);
            assert_eq!(legacy_renderer.backend_kind(), BackendKind::Gpu);
            let mut legacy_client = scene_client();
            let legacy_frame = legacy_renderer.mainredraw(&mut legacy_client);
            assert!(legacy_frame.is_fresh());
            let legacy_mailbox = FrameBuf::new_for_journal_proof();
            let legacy_head = head_identity(77);
            legacy_mailbox.attach_head(legacy_head);
            assert!(legacy_mailbox.publish_journal_paint(
                legacy_head,
                legacy_frame,
                stamp(),
                FramePaintStatus::default(),
            ));
            let legacy_proof = legacy_mailbox
                .take_journal_proof_result()
                .expect("legacy GPU capture succeeds")
                .expect("legacy GPU proof retains its real texture pixels");
            assert!(legacy_proof.gpu);
            assert_eq!(
                (legacy_proof.pixmap.width, legacy_proof.pixmap.height),
                (765, 503)
            );
            assert_eq!(legacy_proof.pixmap.pixels.len(), 765 * 503);
        }

        // These fixture-owned limits are fixed before any retained consumer
        // registration; fair mode remains explicitly process-fixture gated.
        configure_retained_present_budget(NonZeroUsize::new(1).unwrap()).unwrap();
        crate::render_head::test_hooks::configure_process(ProcessFixture {
            grant_ttl: Duration::from_millis(20),
            quantum: Duration::from_millis(40),
        })
        .unwrap();
        crate::render_head::pump(Instant::now());
        assert!(crate::render_head::fair_enabled());

        let mailbox = FrameBuf::new();
        let (wake, _park) = crate::wake_channel();
        let control = Arc::new(FixtureControl {
            now: Mutex::new(Instant::now()),
            failure: Mutex::new(Some(SceneTestPoint::AfterAdmission)),
        });
        let render = SlotRender::new(
            SlotKey::from_incarnation(78).unwrap(),
            Arc::clone(&mailbox),
            Arc::new(wake),
        )
        .with_test_control(control.clone());
        let mut head = Head::attach(&render, false, false).unwrap();
        assert_eq!(head.backend_kind(), BackendKind::Gpu);
        let identity = head.identity();
        *control.now.lock().unwrap() = Instant::now();
        let mut client = scene_client();

        assert!(mailbox.publish(identity, applet_pixmap(vec![41; 765 * 503])));
        head.note_due(control.now()).unwrap();
        let nonfresh = head.paint(&mut client);
        assert!(!nonfresh.is_fresh());
        assert_eq!(
            nonfresh.render_fault(),
            Some(RenderFault::InjectedResourceLimit)
        );
        assert!(!head.publish_journal_paint(nonfresh, stamp()));
        assert_eq!(mailbox.generation(), 1);
        assert_eq!(
            mailbox.render_fault(),
            Some(RenderFault::InjectedResourceLimit)
        );
        match mailbox.take() {
            Some(FrameOutput::PixMap(pixmap)) => assert_eq!(pixmap.pixels[0], 41),
            _ => panic!("a nonfresh GPU frame must preserve the previous mailbox frame"),
        }

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let fresh = head.paint(&mut client);
        assert!(fresh.is_fresh());
        assert!(head.publish_journal_paint(fresh, stamp()));
        assert_eq!(mailbox.generation(), 2);
        assert_eq!(mailbox.render_fault(), None);
        assert!(matches!(mailbox.take(), Some(FrameOutput::Texture(_))));

        let proofs = FrameBuf::new_for_journal_proof();
        proofs.attach_head(identity);
        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let unconfigured = head.paint(&mut client);
        assert!(unconfigured.is_fresh());
        assert!(!proofs.publish_journal_paint(
            identity,
            unconfigured,
            stamp(),
            FramePaintStatus::default()
        ));
        assert_eq!(proofs.generation(), 0);
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            0
        );
        assert!(matches!(
            proofs.take_journal_proof_result(),
            Err(ReadbackError::Ungated)
        ));

        proofs
            .configure_journal_proof_policy(NonZeroUsize::new(1).unwrap(), Duration::from_secs(10))
            .unwrap();
        assert!(!proofs.publish_journal_paint(
            identity,
            FrameOutput::Refused(RenderFault::PresentContended),
            stamp(),
            FramePaintStatus::default(),
        ));
        assert_eq!(proofs.generation(), 0);
        assert_eq!(proofs.render_fault(), Some(RenderFault::PresentContended));
        assert!(proofs.take_journal_proof_result().unwrap().is_none());
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            0
        );

        *control.failure.lock().unwrap() = Some(SceneTestPoint::AfterAdmission);
        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let nonfresh_proof = head.paint(&mut client);
        assert!(!nonfresh_proof.is_fresh());
        assert!(!proofs.publish_journal_paint(
            identity,
            nonfresh_proof,
            stamp(),
            FramePaintStatus::default()
        ));
        assert_eq!(proofs.generation(), 0);
        assert!(proofs.take_journal_proof_result().unwrap().is_none());
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            0
        );

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let registration_frame = head.paint(&mut client);
        assert!(registration_frame.is_fresh());
        assert!(!proofs.publish_journal_paint(
            identity,
            registration_frame,
            stamp(),
            FramePaintStatus::default()
        ));
        assert_eq!(proofs.generation(), 0);
        assert!(
            proofs.take_journal_proof_result().unwrap().is_none(),
            "registration cannot retain an already painted frame"
        );

        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let fresh_proof = head.paint(&mut client);
        assert!(fresh_proof.is_fresh());
        let proof_token = match &fresh_proof {
            FrameOutput::Texture(frame) => {
                assert!(frame.has_valid_scene());
                frame
                    .present_token()
                    .expect("the fresh GPU proof has a token")
            }
            _ => panic!("a required GPU proof carries a tracked frame"),
        };
        assert!(proofs.publish_journal_paint(
            identity,
            fresh_proof,
            stamp(),
            FramePaintStatus::default()
        ));
        assert_eq!(
            proofs.generation(),
            0,
            "pending GPU readback is not a proof"
        );
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            1
        );

        // The copy is already ordered into process staging; a due paint on
        // this same slot must progress without waiting for journal mapping.
        control.advance(Duration::from_millis(41));
        head.note_due(control.now()).unwrap();
        let during_pending_copy = head.paint(&mut client);
        assert!(during_pending_copy.is_fresh());
        drop(during_pending_copy);
        assert_eq!(proofs.generation(), 0, "the later paint is not proof N");
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            1,
            "the registered retained texture is reused while N maps"
        );

        {
            let deadline = Instant::now() + Duration::from_secs(10);
            let proof = loop {
                match proofs.take_journal_proof_result() {
                    Ok(Some(proof)) => break proof,
                    Ok(None) | Err(ReadbackError::Busy) => {
                        assert!(Instant::now() < deadline, "GPU proof readback timed out");
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("GPU proof readback refused: {error:?}"),
                }
            };
            assert_eq!(proof.generation, 1);
            assert_eq!(proof.frame_stamp().present_token(), Some(proof_token));
            assert!(proof.frame_stamp().has_valid_scene());
            assert!(proof.gpu);
            assert_eq!((proof.pixmap.width, proof.pixmap.height), (765, 503));
            assert_eq!(proof.pixmap.pixels.len(), 765 * 503);
            assert_eq!(proofs.render_fault(), None);
            assert!(matches!(
                ReadbackBudget::process().try_acquire(
                    CaptureGeometry::new(1, 1),
                    Instant::now() + Duration::from_secs(1),
                ),
                Err(ReadbackError::Busy)
            ));
        }
        {
            let _released = ReadbackBudget::process()
                .try_acquire(
                    CaptureGeometry::new(1, 1),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect("dropping the proof releases its process permit");
        }

        assert!(proofs.retire_head(identity).is_none());
        head.detach();
        assert_eq!(
            client::render::backend::present::process_retained_present_resource_count(),
            0
        );
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

    #[cfg(feature = "draw-memory-profile")]
    #[test]
    fn render_bytes_sample_ignores_stale_sequence_and_head() {
        let buf = FrameBuf::new();
        let (retired, current) = head_generations(72);
        buf.attach_head(retired);
        buf.request_render_bytes_sample();
        let retired_request = buf.render_bytes_request(retired).unwrap();

        buf.attach_head(current);
        assert_eq!(buf.render_bytes_sample(), None);
        buf.publish_render_bytes_sample(retired, retired_request, Default::default());
        assert_eq!(buf.render_bytes_sample(), None);

        buf.request_render_bytes_sample();
        let current_request = buf.render_bytes_request(current).unwrap();
        assert_ne!(current_request, retired_request);
        buf.publish_render_bytes_sample(current, retired_request, Default::default());
        assert_eq!(buf.render_bytes_request(current), Some(current_request));
        assert_eq!(buf.render_bytes_sample(), None);
        buf.publish_render_bytes_sample(current, current_request, Default::default());
        assert_eq!(buf.render_bytes_sample(), Some(Default::default()));
    }

    #[cfg(feature = "draw-memory-profile")]
    #[test]
    fn detached_render_bytes_request_publishes_zero_sample() {
        let buf = FrameBuf::new();
        let head = head_identity(73);
        buf.attach_head(head);
        buf.request_render_bytes_sample();
        let request = buf.render_bytes_request(head).unwrap();
        assert_eq!(buf.render_bytes_sample(), None);

        assert!(buf.retire_head(head).is_none());
        assert_eq!(buf.render_bytes_request(head), None);
        assert_eq!(buf.render_bytes_sample(), Some(Default::default()));
        buf.publish_render_bytes_sample(head, request, Default::default());
        assert_eq!(buf.render_bytes_sample(), Some(Default::default()));
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
                FramePaintStatus::default(),
            )
        });
        let proof = buf.take_journal_proof_result().unwrap();
        assert!(
            !(raced.interleaved && (raced.accepted || proof.is_some())),
            "a retired head's journal proof was published after its slot reattached \
             (accepted={}, queued={})",
            raced.accepted,
            proof.is_some()
        );
        assert!(raced.accepted, "published while its head was attached");
        assert_eq!(proof.map(|proof| proof.generation), Some(1));
        assert!(buf.take_journal_proof_result().unwrap().is_none());
    }

    #[cfg(feature = "journal-paint-proof")]
    fn publish_bounded_journal_paint_for_race(buf: &FrameBuf, head: HeadIdentity) -> bool {
        let queue = buf.proof_frames.as_ref().unwrap();
        buf.publish_bounded_proof(
            queue,
            head,
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![7],
            }),
            Some(super::JournalPaintStamp {
                modal_root: 1,
                journal_paint_hidden: false,
            }),
            FramePaintStatus::default(),
        )
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn bounded_journal_policy_requires_setup_and_refuses_overflow() {
        use std::num::NonZeroUsize;

        let buf = FrameBuf::new_for_journal_proof();
        let head = head_identity(74);
        buf.attach_head(head);
        let queue = buf.proof_frames.as_ref().unwrap();
        let paint = || {
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![9],
            })
        };

        assert!(!buf.publish_bounded_proof(
            queue,
            head,
            paint(),
            None,
            FramePaintStatus::default()
        ));
        assert_eq!(
            buf.journal_proof_error(),
            Some(client::render::backend::ReadbackError::Ungated)
        );
        assert_eq!(buf.generation(), 0);

        buf.configure_journal_proof_policy(
            NonZeroUsize::new(1).unwrap(),
            Duration::from_millis(100),
        )
        .unwrap();
        let reserved_capacity = queue.lock().frames.capacity();
        assert!(buf.publish_bounded_proof(queue, head, paint(), None, FramePaintStatus::default()));
        assert!(!buf.publish_bounded_proof(
            queue,
            head,
            paint(),
            None,
            FramePaintStatus::default()
        ));
        assert_eq!(buf.generation(), 1);
        assert_eq!(
            buf.journal_proof_error(),
            Some(client::render::backend::ReadbackError::Limit)
        );
        assert_eq!(queue.lock().frames.len(), 1);
        assert_eq!(queue.lock().frames.capacity(), reserved_capacity);

        let proof = buf.take_journal_proof_result().unwrap().unwrap();
        assert_eq!(proof.pixmap.pixels, [9]);
        assert_eq!(proof.generation, 1);
        assert!(matches!(
            buf.take_journal_proof_result(),
            Err(client::render::backend::ReadbackError::Limit)
        ));
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn bounded_journal_proof_is_ordered_before_a_reattach() {
        use std::num::NonZeroUsize;

        let buf = FrameBuf::new_for_journal_proof();
        buf.configure_journal_proof_policy(
            NonZeroUsize::new(1).unwrap(),
            Duration::from_millis(100),
        )
        .unwrap();
        let raced = publish_across_a_reattach(&buf, 75, publish_bounded_journal_paint_for_race);
        let proof = buf.take_journal_proof_result().unwrap();
        assert!(
            !(raced.interleaved && (raced.accepted || proof.is_some())),
            "a bounded proof from a retired head entered its successor"
        );
        assert!(raced.accepted);
        assert_eq!(proof.map(|proof| proof.generation), Some(1));
        assert_eq!(buf.generation(), 1);
        let (retired, _) = head_generations(75);
        let queue = buf.proof_frames.as_ref().unwrap();
        assert!(!buf.publish_bounded_proof(
            queue,
            retired,
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![8],
            }),
            None,
            FramePaintStatus::default(),
        ));
        assert_eq!(
            buf.journal_proof_error(),
            Some(client::render::backend::ReadbackError::Retired)
        );
        assert_eq!(buf.generation(), 1);
    }

    #[test]
    fn mailbox_sequence_exhaustion_never_reuses_stamp() {
        let buf = FrameBuf::new();
        let head = head_identity(75);
        buf.attach_head(head);
        let pixmap = |pixel| {
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![pixel],
            })
        };
        assert!(buf.publish_with_status(head, pixmap(3), FramePaintStatus::new(true, None)));
        buf.frame_generation
            .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        assert!(!buf.publish_with_status(head, pixmap(7), FramePaintStatus::new(true, None)));
        assert_eq!(buf.generation(), u64::MAX);
        assert_eq!(
            buf.render_fault(),
            Some(client::render::backend::RenderFault::MailboxSequenceExhausted),
        );
        let accepted = buf.take_presented().expect("last accepted frame");
        assert_eq!(accepted.stamp().head(), head);
        assert_eq!(accepted.stamp().mailbox_sequence(), 1);
        assert!(accepted.stamp().has_valid_scene());
        match accepted.into_output() {
            FrameOutput::PixMap(pixmap) => assert_eq!(pixmap.pixels, [3]),
            _ => panic!("sequence exhaustion must preserve the accepted pixels"),
        }
        assert!(!buf.publish_with_status(head, pixmap(9), FramePaintStatus::default()));
        assert!(buf.take_presented().is_none());
    }

    #[test]
    fn accepted_envelope_keeps_exact_head_sequence_and_scene_validity() {
        let buf = FrameBuf::new();
        let head = head_identity(74);
        buf.attach_head(head);
        let pixmap = |pixel| {
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![pixel],
            })
        };
        assert!(buf.publish_with_status(head, pixmap(4), FramePaintStatus::new(true, None)));
        let first = buf.take_presented().expect("first frame");
        let first_stamp = *first.stamp();
        assert!(buf.publish_with_status(head, pixmap(9), FramePaintStatus::default()));
        let second = buf.take_presented().expect("second frame");
        assert_eq!(first.stamp(), &first_stamp);
        assert_eq!(first_stamp.head(), head);
        assert_eq!(first_stamp.mailbox_sequence(), 1);
        assert_eq!(first_stamp.present_token(), None);
        assert!(first_stamp.has_valid_scene());
        assert_eq!(second.stamp().head(), head);
        assert_eq!(second.stamp().mailbox_sequence(), 2);
        assert!(!second.stamp().has_valid_scene());
        match (first.into_output(), second.into_output()) {
            (FrameOutput::PixMap(first), FrameOutput::PixMap(second)) => {
                assert_eq!(first.pixels, [4]);
                assert_eq!(second.pixels, [9]);
            }
            _ => panic!("CPU envelopes retain their own payload"),
        }
    }

    #[test]
    fn completed_capture_rejects_reattach_and_cpu_title() {
        let (old_head, new_head) = head_generations(79);
        let cache_dir = format!(
            "{}/../../target/completed-capture-absent-{}",
            env!("CARGO_MANIFEST_DIR"),
            std::process::id()
        );
        assert!(!std::path::Path::new(&cache_dir).exists());
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir,
                members: true,
                lowmem: false,
            },
            ClientRevision::R289,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.local_player = Some(client::dash3d::ClientPlayer::at(50, 50));
        client.cinema_cam = true;
        client.cam_x = 50 * 128 + 64;
        client.cam_z = 50 * 128 + 64;
        client.cam_y = 0;
        client.cam_pitch = 512;
        client.cam_yaw = 0;
        client.world = client::core::World::new(vec![vec![vec![2000; 105]; 105]; 4], 104, 4, 104);
        client.set_draw(true);
        client.world.fill_base_level(0);
        for x in 49..=51 {
            for z in 49..=51 {
                client.world.set_ground(
                    0,
                    x,
                    z,
                    client::dash3d::TerrainOverlayShape::PLAIN,
                    0,
                    -1,
                    2000,
                    2000,
                    2000,
                    2000,
                    25_700,
                    25_700,
                    25_700,
                    25_700,
                    25_700,
                    25_700,
                    25_700,
                    25_700,
                    0,
                    0,
                );
            }
        }
        let mut renderer = client::render::Renderer::new(false);
        let valid_output = renderer.game_draw(&mut client);
        assert!(renderer.has_valid_scene(), "the live CPU paint completed");
        assert_eq!(renderer.retained_storage_allocations(), 0);

        let buf = FrameBuf::new();
        buf.attach_head(old_head);
        assert!(buf.publish_with_status(
            old_head,
            valid_output,
            FramePaintStatus::new(renderer.has_valid_scene(), None),
        ));
        let completed = buf
            .take_presented()
            .expect("the completed CPU capture carries a stamp");
        let completed_stamp = *completed.stamp();
        assert_eq!(completed_stamp.head(), old_head);
        assert!(completed_stamp.has_valid_scene());
        let completed_output = completed.into_output();

        let title_output = renderer.title_screen_draw(&mut client);
        assert!(
            !renderer.has_valid_scene(),
            "CPU Title invalidates scene authority without activating gated retention"
        );
        assert_eq!(renderer.retained_storage_allocations(), 0);
        assert!(buf.publish_with_status(
            old_head,
            title_output,
            FramePaintStatus::new(renderer.has_valid_scene(), None),
        ));
        let title_capture = buf
            .take_presented()
            .expect("the title paint remains a completed frame");
        assert!(!title_capture.stamp().has_valid_scene());
        assert_eq!(title_capture.stamp().mailbox_sequence(), 2);

        assert!(buf.retire_head(old_head).is_none());
        buf.attach_head(new_head);
        assert_eq!(buf.head(), Some(new_head));
        assert_ne!(completed_stamp.head(), buf.head().unwrap());
        assert!(
            !buf.publish_with_status(
                completed_stamp.head(),
                completed_output,
                FramePaintStatus::new(completed_stamp.has_valid_scene(), None),
            ),
            "a completed old-head capture cannot publish after reattach"
        );
        assert_eq!(
            buf.generation(),
            2,
            "stale proof publication advances nothing"
        );
        assert!(buf.take_presented().is_none());
    }

    #[cfg(feature = "journal-paint-proof")]
    #[test]
    fn journal_mailbox_sequence_exhaustion_never_reuses_a_stamp() {
        let buf = FrameBuf::new_for_journal_proof();
        let head = head_identity(76);
        buf.attach_head(head);
        buf.frame_generation
            .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);

        assert!(!buf.publish(
            head,
            FrameOutput::PixMap(PixMap {
                width: 1,
                height: 1,
                pixels: vec![1],
            })
        ));
        assert_eq!(buf.generation(), u64::MAX);
        assert!(matches!(
            buf.take_journal_proof_result(),
            Err(client::render::backend::ReadbackError::IdentityExhausted)
        ));
        assert!(buf.take_journal_proof_result().unwrap().is_none());
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
            frame_generation: AtomicU64,
        }
        let old = Unchecked {
            frame: Mutex::new(None),
            frame_generation: AtomicU64::new(0),
        };
        let mut spare = Some(applet_pixmap(vec![1; 765 * 503]));
        let unchecked = per_iter(|| {
            *old.frame.lock().unwrap() = spare.take();
            old.frame_generation.fetch_add(1, Ordering::Relaxed);
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
                assert!(buf.publish_journal_paint(head, frame, stamp, FramePaintStatus::default()));
            } else {
                assert!(buf.publish(head, frame));
            }
        }
        for (index, (pixel, stamp)) in witnesses.into_iter().enumerate() {
            let frame = buf
                .take_journal_proof_result()
                .unwrap()
                .expect("no discarded paint");
            assert_eq!(frame.generation, index as u64 + 1);
            assert_eq!(frame.pixmap.pixels, [pixel]);
            assert_eq!(frame.stamp, stamp);
        }
        assert!(buf.take_journal_proof_result().unwrap().is_none());
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
        let mut frame = buf.take_presented().expect("stored frame");
        let roi = frame.take_roi().expect("sidecar from the stored frame");
        assert_eq!(roi.frame_id, 4);
        assert_eq!(roi.cam.cycle, 10);
        assert_ne!(roi.frame_id, 9);
    }

    #[cfg(feature = "render-diagnostics")]
    #[test]
    fn accepted_envelope_keeps_frame_a_roi_when_frame_b_is_published() {
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
        let mut frame_a = buf.take_presented().expect("frame A");
        stamp_thread_pixel_roi(tagged(9, 99));
        store(&buf, pix(0x00bb));
        let roi = frame_a.take_roi().expect("roi taken with frame A");
        assert_eq!(frame_a.stamp().mailbox_sequence(), 1);
        match frame_a.into_output() {
            FrameOutput::PixMap(pix) => assert_eq!(pix.pixels[0], 0x00aa, "took frame A pixels"),
            _ => panic!("expected pixmap A"),
        }
        assert_eq!(
            roi.frame_id, 4,
            "roi must stay with frame A, not the interleaved store"
        );
        assert_eq!(roi.cam.cycle, 10);
        let mut frame_b = buf.take_presented().expect("frame B");
        let roi_b = frame_b.take_roi().expect("roi B");
        assert_eq!(frame_b.stamp().mailbox_sequence(), 2);
        match frame_b.into_output() {
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
        let (first_published, first_publication) = std::sync::mpsc::sync_channel(0);
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
                    if n == 1 && first_published.send(()).is_err() {
                        return;
                    }
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
                first_publication
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect("producer publishes before the consumer starts polling");
                for _ in 0..8_000 {
                    if let Some(mut frame) = buf.take_presented() {
                        let roi = frame.take_roi();
                        if let FrameOutput::PixMap(pix) = frame.into_output() {
                            let tag = pix.pixels[0] as u64;
                            match roi {
                                Some(roi) if roi.frame_id == tag => {
                                    seen.fetch_add(1, Ordering::Relaxed);
                                }
                                _ => {
                                    mismatches.fetch_add(1, Ordering::Relaxed);
                                }
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
        client.menu_action[0] = MiniMenuAction::WALK;
        // A real action row qualifies; an off-row world click is only menu
        // dismissal, not raw viewport intent. Cancel and Examine rows never
        // qualify (see world_menu_examine_and_cancel_are_not_takeovers).
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
    fn world_menu_examine_and_cancel_are_not_takeovers() {
        let mut client = intent_client();
        client.shell.mouse_click_button = 1;
        client.shell.mouse_click_x = 30;
        client.shell.mouse_click_y = 51;
        client.is_menu_open = true;
        client.menu_area = 0;
        client.menu_x = 20;
        client.menu_y = 20;
        client.menu_width = 100;
        client.menu_num_entries = 1;
        // Cancel and every Examine row stay in place, with and without the
        // _PRIORITY offset a highlighted row carries. Walk and Talk-to
        // still take over.
        for action in [
            MiniMenuAction::CANCEL,
            MiniMenuAction::OP_LOC6,
            MiniMenuAction::OP_NPC6,
            MiniMenuAction::OP_OBJ6,
        ] {
            for offset in [0, MiniMenuAction::_PRIORITY] {
                client.menu_action[0] = offset + action;
                assert_eq!(
                    ManualMoveIntent::classify(&client, MouseOwner::User),
                    None,
                    "action {action} offset {offset}"
                );
            }
        }
        for action in [MiniMenuAction::WALK, MiniMenuAction::OP_NPC2] {
            client.menu_action[0] = action;
            assert_eq!(
                ManualMoveIntent::classify(&client, MouseOwner::User),
                Some(ManualMoveIntent::WorldMenu),
                "action {action}"
            );
        }
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
