use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use client::client::{present::pack_rgb, GameShell, APPLET_H, APPLET_W};
use client::render::backend::FrameOutput;

/// Per-slot frame mailbox: the slot thread stores each rendered
/// [`FrameOutput`] into [`FrameBuf::store`]; the panel samples the latest
/// `PixMap` (the CPU upload path, [`FrameBuf::snapshot`]) and, in Task 4c,
/// binds the `Texture` variant directly. Replaces the old packed-pixels
/// byte buffer.
pub struct FrameBuf {
    inner: Mutex<Option<FrameOutput>>,
    gen: AtomicU64,
}

impl FrameBuf {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(None),
            gen: AtomicU64::new(0),
        })
    }
    /// Store the latest frame and bump the generation. The full
    /// [`FrameOutput`] is kept so Task 4c can hand a `FrameOutput::Texture`
    /// to the panel; today the backend always yields a `PixMap`.
    pub fn store(&self, frame: FrameOutput) {
        *self.inner.lock().unwrap() = Some(frame);
        self.gen.fetch_add(1, Ordering::Relaxed);
    }
    /// CPU path: pack the latest `PixMap`'s pixels via `pack_rgb` (765×503,
    /// same shape the panel's texture upload expects). Empty when nothing
    /// was stored yet, the frame is `FrameOutput::Texture`, or the `PixMap`
    /// is not full-applet sized.
    pub fn snapshot(&self) -> Vec<u32> {
        let n = (APPLET_W * APPLET_H) as usize;
        let inner = self.inner.lock().unwrap();
        let mut out = Vec::with_capacity(n);
        if let Some(FrameOutput::PixMap(pix)) = &*inner {
            if pix.width == APPLET_W && pix.height == APPLET_H && pix.pixels.len() >= n {
                for src in pix.pixels.iter().take(n) {
                    out.push(pack_rgb(*src));
                }
            }
        }
        out
    }
    /// Bumps on every [`FrameBuf::store`]. The panel skips uploads while
    /// this stays unchanged (dear-app would otherwise Poll-spin).
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

pub struct SlotInput {
    enabled: AtomicBool,
    rx: Mutex<Option<Receiver<InputEv>>>,
}

impl SlotInput {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            enabled: AtomicBool::new(false),
            rx: Mutex::new(None),
        })
    }
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn connect_rx(&self, rx: Receiver<InputEv>) {
        *self.rx.lock().unwrap() = Some(rx);
    }
    pub fn disconnect_rx(&self) {
        *self.rx.lock().unwrap() = None;
    }
    pub fn drain(&self, shell: &mut GameShell) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut g = self.rx.lock().unwrap();
        let Some(rx) = g.as_mut() else {
            return;
        };
        while let Ok(ev) = rx.try_recv() {
            match ev {
                InputEv::Move { x, y } => shell.apply_mouse_move(x, y),
                InputEv::Down { button, x, y } => shell.apply_mouse_down(button, x, y),
                InputEv::Up => shell.apply_mouse_up(),
                InputEv::Key { down, ch } => shell.apply_key(down, 0, ch),
            }
        }
    }
}

/// Per-slot control wake: the panel/host-play side holds [`SlotWake`] ends
/// and `wake()`s a parked slot thread (focus/draw/stop/spawn); the slot
/// thread hands the [`SlotPark`] end to [`Host::run_client`], which polls
/// its fd next to the client socket inside the idle park. A Unix socketpair
/// rather than std `mpsc` because `poll(2)` needs an fd; the payload is an
/// empty kick — the parker re-reads the shared state (`client.draw`,
/// capture, the probe's stop flag) after waking.
#[derive(Clone)]
pub struct SlotWake {
    tx: Arc<UnixStream>,
}

/// The slot-thread end of the control channel. Not shared: one per parked
/// thread, polled by [`Host::run_client`].
pub struct SlotPark {
    rx: UnixStream,
}

/// Create a control-wake channel pair. A [`SlotWake::wake`] writes a byte
/// that fires a `poll(2)` on [`SlotPark::fd`].
pub fn wake_channel() -> (SlotWake, SlotPark) {
    let (tx, rx) = UnixStream::pair().expect("socketpair for slot wake");
    let _ = tx.set_nonblocking(true);
    let _ = rx.set_nonblocking(true);
    (SlotWake { tx: Arc::new(tx) }, SlotPark { rx })
}

impl SlotWake {
    /// Wake a parked slot thread: one byte on the socketpair. Nonblocking
    /// and best-effort — a full wake buffer (only possible if the parker
    /// stopped draining) or a dead slot just drops the kick.
    pub fn wake(&self) {
        let _ = (&*self.tx).write(&[1]);
    }
}

impl SlotPark {
    /// The wake end's fd, for the slot thread's `poll(2)`.
    pub fn fd(&self) -> i32 {
        self.rx.as_raw_fd()
    }
    /// Drain any queued wake bytes so a burst of kicks cannot fill the
    /// socketpair buffer.
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
    use super::{map_image_to_applet, wake_channel, FrameBuf, InputEv, SlotInput};
    use client::graphics::PixMap;
    use client::render::backend::{FrameOutput, TextureHandle};

    fn applet_pixmap(pixels: Vec<i32>) -> FrameOutput {
        FrameOutput::PixMap(PixMap {
            width: 765,
            height: 503,
            pixels,
        })
    }

    #[test]
    fn store_packs_pixmap_rgb_and_is_765_by_503() {
        use client::client::present::pack_rgb;
        let buf = FrameBuf::new();
        let mut pix = vec![0i32; 765 * 503];
        pix[0] = 0x00aa_bbcc;
        pix[1] = 0x0011_2233;
        buf.store(applet_pixmap(pix));
        let out = buf.snapshot();
        assert_eq!(out.len(), 765 * 503);
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
        buf.store(FrameOutput::PixMap(PixMap {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3],
        }));
        assert!(buf.snapshot().is_empty());
        // The frame is still stored and the generation bumped; only the
        // CPU packing is shape-gated.
        assert_eq!(buf.generation(), 1);
    }

    #[test]
    fn snapshot_of_texture_is_empty() {
        let buf = FrameBuf::new();
        buf.store(FrameOutput::Texture(TextureHandle));
        assert!(buf.snapshot().is_empty());
    }

    #[test]
    fn store_bumps_generation() {
        let buf = FrameBuf::new();
        assert_eq!(buf.generation(), 0);
        buf.store(applet_pixmap(vec![0i32; 765 * 503]));
        assert_eq!(buf.generation(), 1);
        buf.store(applet_pixmap(vec![1i32; 765 * 503]));
        assert_eq!(buf.generation(), 2);
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

    #[test]
    fn drain_skips_recv_when_disabled_then_applies_when_enabled() {
        use std::sync::mpsc;
        let inp = SlotInput::new();
        let (tx, rx) = mpsc::channel();
        inp.connect_rx(rx);
        tx.send(InputEv::Down {
            button: 1,
            x: 10,
            y: 10,
        })
        .unwrap();
        let mut shell = client::client::GameShell::new();
        inp.set_enabled(false);
        inp.drain(&mut shell);
        shell.latch_click();
        assert_eq!(shell.mouse_click_button, 0);
        inp.set_enabled(true);
        inp.drain(&mut shell);
        shell.latch_click();
        assert_eq!((shell.mouse_click_button, shell.mouse_click_x), (1, 10));
    }

    #[test]
    fn wake_channel_fires_a_polling_parker_and_drains() {
        let (wake, park) = wake_channel();
        let mut fds = [libc::pollfd {
            fd: park.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) };
        assert_eq!(rc, 0, "no kick yet: poll must time out");
        wake.wake();
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 1000) };
        assert_eq!(rc, 1, "a wake byte must fire the poll");
        assert_ne!(fds[0].revents & libc::POLLIN, 0);
        park.drain();
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) };
        assert_eq!(rc, 0, "drained: poll goes quiet again");
    }

    #[test]
    fn wake_channel_clones_share_one_wake() {
        let (wake, park) = wake_channel();
        let wake2 = wake.clone();
        let mut fds = [libc::pollfd {
            fd: park.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        wake2.wake();
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 1000) };
        assert_eq!(rc, 1, "a cloned SlotWake must wake the same park");
    }
}
