//! Own test binary: a steady stream of single-member commands through the
//! [`CommandBook`] allocates nothing once the settled history is full (the
//! evicted reports' member lists are reused), so input-rate rows such as
//! `GameInput` can go through the book every frame.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::num::NonZeroU64;

use api::identity::{ProcessIncarnation, RequestIdMinter, Scope, Settlement};
use api::process_condition::ProcessConditions;
use frontend_core::command::{CommandBook, Completion, OperatorCommand, Subject};

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter uses a non-allocating TLS cell.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
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
