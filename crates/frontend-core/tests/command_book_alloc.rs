//! Own test binary: a steady stream of single-member commands through the
//! [`CommandBook`] allocates nothing once the settled history is full (the
//! evicted reports' member lists are reused), so input-rate rows such as
//! `GameInput` can go through the book every frame.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::num::NonZeroU64;

use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, Settlement};
use api::process_condition::ProcessConditions;
use frontend_core::command::{
    CommandBook, Completion, DrawSwitch, OperatorCommand, RasterMode, Subject,
};
use frontend_core::surface::{InputEv, InputSender, SlotAttach, SlotSurface, SpawnProfile};
use frontend_core::{GameInputEvent, MapDesk, Models, OperatorSession, Targets};
use host_play::{InstancePermit, PlayOptions};
use vault::{Profile, ProfileSettings, Vault};

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
    /// Size of the allocations counted separately as input-transport blocks
    /// (0: nothing is exempt).
    static TRANSPORT_SIZE: Cell<usize> = const { Cell::new(0) };
    static TRANSPORT_EVENTS: Cell<usize> = const { Cell::new(0) };
    static LAST_SIZE: Cell<usize> = const { Cell::new(0) };
}

fn bump(size: usize) {
    let _ = LAST_SIZE.try_with(|last| last.set(size));
    let transport = TRANSPORT_SIZE.try_with(Cell::get).unwrap_or(0);
    if size == transport {
        let _ = TRANSPORT_EVENTS.try_with(|events| events.set(events.get() + 1));
    } else {
        let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
    }
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter uses a non-allocating TLS cell.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(layout.size());
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump(layout.size());
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump(new_size);
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn events() -> usize {
    EVENTS.with(Cell::get)
}
#[test]
fn single_member_commands_allocate_nothing_after_warm_up() {
    let conditions = ProcessConditions::new(ProcessIncarnation::from_start_nonce(NonZeroU64::MIN));
    let requests = RequestIdMinter::new();
    let mut book = CommandBook::new();
    let targets = [(Subject::Application, Scope::Process)];
    let run = |book: &mut CommandBook| {
        let request = requests.mint().expect("request id");
        book.open(
            request,
            OperatorCommand::AcknowledgeNotice,
            &targets,
            &conditions,
        )
        .expect("open");
        book.settle(
            request,
            Subject::Application,
            Settlement::Completed(Completion::Done),
            None,
        )
        .expect("settle");
    };
    for _ in 0..256 {
        run(&mut book);
    }
    let before = events();
    for _ in 0..1_000 {
        run(&mut book);
    }
    assert_eq!(events() - before, 0, "steady-state commands allocated");
}

/// A surface that lends one [`MapDesk`] to every command, as the panel does.
struct DeskSurface {
    desk: MapDesk,
}

impl SlotSurface for DeskSurface {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        profile.set_raster(RasterMode::Off);
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> Models<'_> {
        Models::none().with_map(&mut self.desk)
    }

    fn draw_changed(&mut self, _switch: DrawSwitch, _on: bool) {}
}

/// What the panel's input transport costs, measured instead of assumed.
///
/// `InputSender::send` (`surface.rs:64-66`) pushes into a std unbounded mpsc
/// list channel, which allocates one fixed-size block per 31 sends. That is
/// the pre-existing host input transport, not an allocation of the command
/// seam, so the seam test exempts exactly that block size and nothing else:
/// any other allocation, of any size, fails the test. The exemption is
/// exact: the test asserts the number of exempt allocations equals the number
/// of block boundaries the measured sends cross, which a fresh calibration
/// channel reports send by send.
struct Transport {
    /// Layout size of one channel block.
    block_size: usize,
    /// `allocates[n]` whether the channel's send number `n + 1` allocated a block.
    allocates: Vec<bool>,
}

impl Transport {
    /// Calibrate on fresh channels with `sends` sends, draining after each as
    /// the session test does.
    fn calibrate(sends: usize) -> Self {
        TRANSPORT_SIZE.with(|size| size.set(0));
        let (first, probe) = InputSender::probe();
        assert!(first.send(InputEv::Up));
        let block_size = LAST_SIZE.with(Cell::get);
        assert!(
            block_size >= 31,
            "the first send allocates the channel block"
        );
        while probe.try_recv().is_ok() {}
        TRANSPORT_SIZE.with(|size| size.set(block_size));
        let (sender, probe) = InputSender::probe();
        let mut allocates = Vec::with_capacity(sends);
        for _ in 0..sends {
            let blocks = TRANSPORT_EVENTS.with(Cell::get);
            let others = events();
            assert!(sender.send(InputEv::Up));
            assert_eq!(events(), others, "a send allocated more than a block");
            allocates.push(TRANSPORT_EVENTS.with(Cell::get) > blocks);
            while probe.try_recv().is_ok() {}
        }
        Self {
            block_size,
            allocates,
        }
    }

    /// Block allocations among the channel's sends `after + 1 ..= through`.
    fn blocks_between(&self, after: usize, through: usize) -> usize {
        self.allocates[after..through]
            .iter()
            .filter(|allocated| **allocated)
            .count()
    }
}

/// The panel's per-frame row: stage the frame's captured input, then submit
/// one `GameInput` for the selected profile through the command seam. Every
/// layer on that path (target resolution, the report book, the map dispatch,
/// the member settle) must reuse its buffers.
#[test]
fn game_input_submitted_through_the_session_allocates_nothing_after_warm_up() {
    const WARM_UP: usize = 200;
    const MEASURED: usize = 1_000;
    let transport = Transport::calibrate(WARM_UP + MEASURED);
    assert!(
        transport.blocks_between(WARM_UP, WARM_UP + MEASURED) >= 1,
        "the measured sends cross block boundaries"
    );
    let dir = std::env::temp_dir().join(format!("274bot-command-alloc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test directory");
    // Hermetic: no world pack from the operator's home (see memory_notice.rs).
    std::env::set_var("NAV_PACK", dir.join("empty.navpack"));
    let play = host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port: 43594,
            cache_dir: dir.join("cache").to_string_lossy().into_owned(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let mut vault = Vault::create(&dir.join("vault"), "test-passphrase-01").expect("vault");
    vault
        .upsert(Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 7,
            settings: ProfileSettings::default(),
        })
        .expect("profile");
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start_for_test(vault, play);
    let mut surface = DeskSurface {
        desk: MapDesk::default(),
    };
    session
        .submit(
            OperatorCommand::Load,
            Targets::Profile("alice"),
            &mut surface,
        )
        .expect("load");
    session
        .submit(
            OperatorCommand::Select,
            Targets::Profile("alice"),
            &mut surface,
        )
        .expect("select");
    // A fresh channel, so its send numbers match the calibration's.
    let (sender, probe) = InputSender::probe();
    surface.desk.set_input_sender(Some(sender));

    let frame = |session: &mut OperatorSession<()>, surface: &mut DeskSurface| {
        assert!(surface
            .desk
            .stage_game_input(GameInputEvent::Captured(InputEv::Up)));
        let request = session
            .submit(OperatorCommand::GameInput, Targets::Selected, surface)
            .expect("submit")
            .request();
        assert!(
            !surface.desk.has_staged_game_input(),
            "the dispatch drained the staged input"
        );
        let report = session.commands().report(request).expect("report");
        assert!(
            report.members().iter().all(|member| matches!(
                member.terminal(),
                Some(Settlement::Completed(Completion::Done))
            )),
            "the input was delivered"
        );
        // The probe's channel is the only receiver; drain so it stays bounded.
        while probe.try_recv().is_ok() {}
    };
    for _ in 0..WARM_UP {
        frame(&mut session, &mut surface);
    }
    let others = events();
    let blocks = TRANSPORT_EVENTS.with(Cell::get);
    for _ in 0..MEASURED {
        frame(&mut session, &mut surface);
    }
    assert_eq!(
        events() - others,
        0,
        "steady-state GameInput submits allocated (outside the {}-byte transport block)",
        transport.block_size
    );
    assert_eq!(
        TRANSPORT_EVENTS.with(Cell::get) - blocks,
        transport.blocks_between(WARM_UP, WARM_UP + MEASURED),
        "the transport blocks allocated are exactly the block boundaries the sends crossed"
    );
}
