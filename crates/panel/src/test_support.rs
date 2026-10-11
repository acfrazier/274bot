use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Process- and test-unique scratch directory removed even when a test unwinds.
pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(label: &str) -> Self {
        let serial = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "274bot-panel-{label}-{}-{serial}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A file path that keeps its unique parent scratch directory alive.
pub(crate) struct TestPath {
    _dir: TestDir,
    path: PathBuf,
}

impl TestPath {
    pub(crate) fn new(label: &str, file_name: &str) -> Self {
        let dir = TestDir::new(label);
        let path = dir.join(file_name);
        Self { _dir: dir, path }
    }
}

impl Deref for TestPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            if let Ok(metadata) = std::fs::metadata(&self.path) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o700);
                let _ = std::fs::set_permissions(&self.path, permissions);
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Test guards protect resettable process globals, so a failed assertion does
/// not make the mutex's poison bit a second, unrelated test failure.
pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Dear ImGui's context singleton is dropped before this guard during unwind;
/// recovering poison is therefore safe once the failed test has unwound.
/// Outermost in the test lock order (see `picker::lock_nav_statics`): taking
/// it while holding a nav lock panics instead of deadlocking.
pub(crate) fn imgui_context_guard() -> MutexGuard<'static, ()> {
    assert!(
        !crate::picker::holds_nav_locks(),
        "lock order: imgui_context_guard taken while holding a picker nav lock"
    );
    lock_unpoisoned(&crate::IMGUI_CTX_TEST_GUARD)
}

/// A Dear ImGui context for a test, with the `.ini` settings file off.
/// Dear ImGui's default is `imgui.ini` in the working directory: every
/// context would load it at its first frame and write it back when dropped,
/// so one test's window, dock and table settings (a Fleet column width at
/// another font scale, for one) changed the layout the next test, and the
/// next run, measured. Tests create contexts here, never with
/// `Context::create()` (`no_test_creates_an_ini_backed_context` enforces it).
pub(crate) fn imgui_context() -> dear_imgui_rs::Context {
    let mut ctx = dear_imgui_rs::Context::create();
    ctx.set_ini_filename::<PathBuf>(None)
        .expect("clearing the ini file name cannot fail");
    ctx
}

/// A headless device on this machine's adapter, `None` when there is none
/// (the GPU tests then skip).
pub(crate) fn headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    headless_gpu_with_info().map(|(device, queue, _)| (device, queue))
}

/// [`headless_gpu`] plus the adapter it chose. Backends are tried in the
/// panel's own order ([`crate::window::backend_attempts`]): on Windows
/// Vulkan before D3D12, because enumerating D3D12 adapters can stall on a
/// cold discrete-GPU driver on a hybrid laptop. Asking every backend at
/// once hung the Windows test host's panel lane for hours there, and the
/// stalled driver load held up unrelated tests in the same process.
///
/// `SKIP_GPU=1` (CI's lanes have no adapter; the vendored client reads it
/// the same way) yields `None` before any graphics driver is touched.
///
/// The instance and adapter are made once per process and every call gets
/// its own device and queue from them. Tests run in parallel, and
/// concurrent `wgpu::Instance::new` calls raced inside the Vulkan loader
/// (`libvulkan.so.1`, `vkEnumerateInstanceExtensionProperties`) and
/// crashed the whole test binary with SIGSEGV; the `LazyLock` makes every
/// other test wait for the one initialisation instead.
pub(crate) fn headless_gpu_with_info() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    if std::env::var("SKIP_GPU").as_deref() == Ok("1") {
        return None;
    }
    static ADAPTER: LazyLock<Option<(wgpu::Instance, wgpu::Adapter)>> = LazyLock::new(|| {
        crate::window::backend_attempts()
            .into_iter()
            .find_map(|backends| {
                let instance = crate::window::instance_for(backends);
                let adapter =
                    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    }))
                    .ok()?;
                // A backend whose adapter cannot give a device is skipped
                // here, as it always was, not left to fail every test.
                request_test_device(&adapter)?;
                Some((instance, adapter))
            })
    });
    let (_instance, adapter) = ADAPTER.as_ref()?;
    let (device, queue) = request_test_device(adapter)?;
    Some((device, queue, adapter.get_info()))
}

fn request_test_device(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("274 panel test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .ok()
}

/// An OS window frame for the M-003 fit glue. Size and position writes land
/// the way a window manager applies them (outer = inner + chrome), and each
/// kind of write is counted. `drag_to` / `user_resize` are the operator.
pub(crate) struct FakeFrame {
    scale: f64,
    chrome: (u32, u32),
    state: parking_lot::Mutex<FrameState>,
}

struct FrameState {
    inner: (u32, u32),
    pos: (i32, i32),
    resizes: u32,
    moves: u32,
}

impl FakeFrame {
    pub(crate) fn new(scale: f64, chrome: (u32, u32), inner: (u32, u32), pos: (i32, i32)) -> Self {
        Self {
            scale,
            chrome,
            state: parking_lot::Mutex::new(FrameState {
                inner,
                pos,
                resizes: 0,
                moves: 0,
            }),
        }
    }

    /// ImGui `display_size` for this frame: the logical inner size.
    pub(crate) fn display_size(&self) -> [f32; 2] {
        let (w, h) = self.inner();
        [
            (f64::from(w) / self.scale) as f32,
            (f64::from(h) / self.scale) as f32,
        ]
    }

    pub(crate) fn inner(&self) -> (u32, u32) {
        self.state.lock().inner
    }

    pub(crate) fn resizes(&self) -> u32 {
        self.state.lock().resizes
    }

    pub(crate) fn moves(&self) -> u32 {
        self.state.lock().moves
    }

    /// Outer frame as `(left, top, right, bottom)` in physical pixels.
    pub(crate) fn outer_rect(&self) -> (i32, i32, i32, i32) {
        let s = self.state.lock();
        let ((x, y), (w, h)) = (s.pos, s.inner);
        (
            x,
            y,
            x + (w + self.chrome.0) as i32,
            y + (h + self.chrome.1) as i32,
        )
    }

    pub(crate) fn drag_to(&self, pos: (i32, i32)) {
        self.state.lock().pos = pos;
    }

    pub(crate) fn user_resize(&self, inner: (u32, u32)) {
        self.state.lock().inner = inner;
    }
}

impl crate::window::FitTarget for FakeFrame {
    fn scale_factor(&self) -> f64 {
        self.scale
    }
    fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        let (w, h) = self.inner();
        winit::dpi::PhysicalSize::new(w, h)
    }
    fn outer_size(&self) -> winit::dpi::PhysicalSize<u32> {
        let (w, h) = self.inner();
        winit::dpi::PhysicalSize::new(w + self.chrome.0, h + self.chrome.1)
    }
    fn outer_position(&self) -> Option<winit::dpi::PhysicalPosition<i32>> {
        let (x, y) = self.state.lock().pos;
        Some(winit::dpi::PhysicalPosition::new(x, y))
    }
    fn request_inner_size(&self, size: winit::dpi::Size) {
        let size = size.to_physical::<u32>(self.scale);
        let mut s = self.state.lock();
        s.inner = (size.width, size.height);
        s.resizes += 1;
    }
    fn set_outer_position(&self, pos: winit::dpi::PhysicalPosition<i32>) {
        let mut s = self.state.lock();
        s.pos = (pos.x, pos.y);
        s.moves += 1;
    }
}

#[test]
fn test_contexts_have_no_ini_file() {
    let _guard = imgui_context_guard();
    let ctx = imgui_context();
    assert!(
        ctx.io().ini_filename().is_none(),
        "a test context must not read or write imgui.ini"
    );
}

/// One test's persisted Fleet table widths (saved at another font scale)
/// moved the last column of the next test's table a pixel past its window.
/// No test source creates a context that would read or write `imgui.ini`:
/// they go through [`imgui_context`]. `window.rs` (the product) sets its own
/// ini policy; this file is the helper.
#[test]
fn no_test_creates_an_ini_backed_context() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&src).expect("read the panel source directory") {
        let path = entry.expect("source directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.extension().is_none_or(|e| e != "rs")
            || name == "window.rs"
            || name == "test_support.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a panel source file");
        for (index, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("Context::create(") {
                offenders.push(format!("{name}:{}", index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use test_support::imgui_context() instead of Context::create(): {offenders:?}"
    );
}
