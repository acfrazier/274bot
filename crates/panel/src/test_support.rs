use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use frontend_core::scripts::{LoadoutForm, Loadouts, LoadoutsErrorKind};
use host_play::local_state::{LocalStateService, StateRoot};

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

/// The state root a `Session` on this thread opens its service at: this
/// thread's pinned scratch home, never the operator's.
pub(crate) fn thread_state_root() -> StateRoot {
    frontend_core::test_support::IsolatedStores::ensure_thread();
    StateRoot::new(frontend_core::scripts::paths::bot_home().join(".274bot"))
}

/// A process service opened on `dir` itself, a scratch state root: its
/// files (`panel-ui.json`, `loadouts.json`) sit directly in `dir`.
pub(crate) fn scratch_service(dir: &Path) -> Arc<LocalStateService> {
    LocalStateService::open_process(StateRoot::new(dir.to_path_buf()))
}

/// Write a private-state fixture, usually a preferences file, with mode
/// 0600 whatever the process umask. A plain write creates the file 0664
/// under umask 0002, and the product reader refuses a group- or
/// world-writable preferences file. The panel has no `vault` dependency,
/// so the mode is set here, before any read.
pub(crate) fn write_private_fixture(
    path: impl AsRef<Path>,
    bytes: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The preferences of a new service on [`thread_state_root`]: the files a
/// `Session` on this thread uses, before one exists, or as a second writer
/// of them (a peer process, as far as the files go). A `Session` sees a
/// write of this handle after its own preferences re-read
/// (`Preferences::reload`).
pub(crate) fn thread_preferences() -> frontend_core::UiPreferences {
    frontend_core::UiPreferences::of_service_for_test(&LocalStateService::open_process(
        thread_state_root(),
    ))
}

/// Hold the real `panel-ui.json.lock` beside `preferences`' file, as a
/// second program does: each preference write waits the writer's bounded
/// lock wait and then fails busy. Dropping the file releases the lock.
pub(crate) fn hold_preferences_lock(preferences: &frontend_core::UiPreferences) -> std::fs::File {
    let file_path = preferences.path();
    if let Some(parent) = file_path.parent() {
        let mut dirs = std::fs::DirBuilder::new();
        dirs.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dirs.mode(0o700);
        }
        dirs.create(parent).expect("create the state directory");
    }
    let mut path = file_path.into_os_string();
    path.push(".lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open the preferences lock");
    file.lock().expect("hold the preferences lock");
    file
}

/// The preferences file of `preferences` as it is on disk now (empty when
/// it does not exist): what a relaunch would read.
pub(crate) fn preferences_on_disk(
    preferences: &frontend_core::UiPreferences,
) -> serde_json::Map<String, serde_json::Value> {
    let path = preferences.path();
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).expect("the preferences file is JSON"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}
/// Retry a frontend-backed write while it reports the write queued past
/// the frontend's bounded durability wait: under machine load a write
/// stays queued past 5 s and settles later. Any other error returns at
/// once, so refusals and genuine conflicts keep their fast verdict.
pub(crate) fn patiently(mut attempt: impl FnMut() -> Result<(), String>) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error)
                if error.contains("not durable within")
                    && start.elapsed() < std::time::Duration::from_secs(60) =>
            {
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}
/// A preferences write through a scratch service, retried past load-slow
/// durability (see [`patiently`]). The update re-reads and re-applies its
/// change on a conflict, so a retry after a late-landing write converges.
pub(crate) fn set_patiently(
    preferences: &frontend_core::UiPreferences,
    key: &str,
    value: serde_json::Value,
) {
    patiently(|| {
        preferences
            .set(key, value.clone())
            .map_err(|error| error.to_string())
    })
    .unwrap();
}
/// A [`Loadouts`] save through a scratch service, retried
/// past load-slow durability (see [`patiently`]). A write of an earlier
/// attempt that lands late conflicts with its own retry; the save replays
/// the store's latest on a conflict, so a retry converges.
pub(crate) fn save_store_patiently(store: &mut Loadouts) {
    let start = std::time::Instant::now();
    loop {
        match store.save_for_test() {
            Ok(()) => return,
            Err(error)
                if (error.to_string().contains("not durable within")
                    || error.kind() == LoadoutsErrorKind::Conflict)
                    && start.elapsed() < std::time::Duration::from_secs(60) =>
            {
                std::thread::yield_now();
            }
            Err(error) => panic!("loadouts save failed: {error}"),
        }
    }
}
/// [`crate::loadouts::save_draft`], retried past load-slow durability: a
/// failed save keeps the operator's draft, so saving again retries the
/// same edit. A conflict is retried only after a timeout saw the store
/// move under the save; a first-answer conflict (another writer) returns
/// at once for the caller to assert on.
pub(crate) fn save_draft_patiently(session: &mut crate::session::Session) -> bool {
    let start = std::time::Instant::now();
    let mut timed_out_before = false;
    loop {
        let saved = crate::loadouts::save_draft(session);
        if saved {
            return true;
        }
        let status = session.loadouts.status().to_string();
        if status.contains("not durable within") {
            timed_out_before = true;
        } else if !(timed_out_before && status.contains("changed since this list was loaded")) {
            return false;
        }
        if start.elapsed() >= std::time::Duration::from_secs(60) {
            return false;
        }
        std::thread::yield_now();
    }
}
/// Re-read a loadouts form until `settled` holds (up to 60 s): recovery
/// for a product save that stayed queued past its bounded wait, where a
/// retry could double-apply (add/duplicate). Reload errors are transient
/// (the writer is still draining the queued write); the final assert names
/// the missing effect.
pub(crate) fn reload_form_until(
    form: &mut LoadoutForm,
    label: &str,
    mut settled: impl FnMut(&LoadoutForm) -> bool,
) {
    let start = std::time::Instant::now();
    while !settled(form) {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(60),
            "{label} never landed"
        );
        let _ = form.reload_for_test();
        std::thread::sleep(std::time::Duration::from_millis(5));
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

/// The process has one session file: a test that opens, fails or closes it
/// holds this, so another test cannot close it underneath.
pub(crate) fn session_file_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    lock_unpoisoned(&LOCK)
}

/// A Dear ImGui context for a test, with the `.ini` settings file off.
/// Dear ImGui's default is `imgui.ini` in the working directory: every
/// context would load it at its first frame and write it back when dropped,
/// so one test's window, dock and table settings (a Fleet column width at
/// another font scale, for one) changed the layout the next test, and the
/// next run, measured. Tests create contexts here; `crates/panel/clippy.toml`
/// disallows `Context::create()` everywhere else.
pub(crate) fn imgui_context() -> dear_imgui_rs::Context {
    #[expect(clippy::disallowed_methods, reason = "the one sanctioned test context")]
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
