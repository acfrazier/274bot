use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// Per-slot frame mailbox: the slot thread stores each rendered
/// [`FrameOutput`] into [`FrameBuf::store`]; the panel hands it to the
/// frame consumer with [`FrameBuf::take`] (one consumer per mailbox — a
/// `FrameOutput::Texture` hands its wgpu view off once, no Clone).
/// [`FrameBuf::snapshot`] stays for the CPU packing path and the tests.
/// Replaces the old packed-pixels byte buffer.
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
    /// [`FrameOutput`] is kept so the panel can bind a
    /// `FrameOutput::Texture` or pack a `PixMap`.
    pub fn store(&self, frame: FrameOutput) {
        *self.inner.lock().unwrap() = Some(frame);
        self.gen.fetch_add(1, Ordering::Relaxed);
    }
    /// Move the stored frame out and clear the mailbox: the single
    /// consumer's handoff (a `FrameOutput::Texture` is single-consumer, no
    /// Clone). `None` when nothing was stored since the last take. The
    /// generation is untouched — only [`FrameBuf::store`] bumps it.
    pub fn take(&self) -> Option<FrameOutput> {
        self.inner.lock().unwrap().take()
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
    rx: Mutex<Option<Receiver<InputEv>>>,
}

impl SlotInput {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            enabled: AtomicBool::new(false),
            full_rate: AtomicBool::new(false),
            prefer_cpu: AtomicBool::new(false),
            rx: Mutex::new(None),
        })
    }
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
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
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("wake pair peer mismatch: accepted {peer}, expected {client_local}"),
        ));
    }
    if server.peer_addr()? != client_local {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "wake pair server peer_addr mismatch",
        ));
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
    use super::{map_image_to_applet, wait_readable, wake_channel, FrameBuf, InputEv, SlotInput};
    use client::graphics::PixMap;
    use client::render::backend::FrameOutput;
    use std::time::{Duration, Instant};

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
    fn snapshot_of_texture_frame_is_empty_and_take_hands_it_off() {
        // A real `TextureHandle` needs a wgpu device (client-side); the
        // panel's `frame_pixels` read-back test covers the texture pixels.
        // Here the mailbox contract: a stored frame of any variant is taken
        // out whole, and `snapshot` (the CPU packing view) never sees it.
        let buf = FrameBuf::new();
        buf.store(applet_pixmap(vec![0i32; 765 * 503]));
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
        buf.store(applet_pixmap(vec![0i32; 765 * 503]));
        assert!(buf.take().is_some());
        assert!(buf.take().is_none());
        assert_eq!(buf.generation(), 1);
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
