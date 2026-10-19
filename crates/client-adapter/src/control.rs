//! Two reserved connection control cells, distinct replies, and one bounded
//! de-duplicated reactor command FIFO. Close never queues behind a full Seal.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mio::Waker;

use super::cancel::{AttemptRetire, CancelToken};
use super::ledger::MAX_CONNECTIONS;
use super::park::Park;
use super::resolver::ResolvedAddresses;
use client::Transport;

/// Every command and result is fenced by all four immutable identities.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BindingId {
    pub incarnation: u64,
    pub connection: u64,
    pub binding: u64,
    pub attempt: u64,
}

/// Resolved addresses are bounded to eight by G4 before Connect admission.
/// This cell, including its retirement authority, remains occupied until a
/// positive exact-generation reply or explicit reactor rejection.
#[derive(Debug)]
pub struct ConnectCommand {
    pub binding: BindingId,
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub addresses: ResolvedAddresses,
    pub deadline: Option<Instant>,
    pub cancel: CancelToken,
    pub retire: Arc<AttemptRetire>,
    pub queue_release: Option<Arc<crate::handoff::QueueReleaseProof>>,
    /// Exclusive, socket-free WSS lane reservation retained through the
    /// reactor's actual worker submission. Plain TCP never carries a ticket.
    pub tls_ticket: Option<crate::tls_handshake::GameTlsTicket>,
}
#[derive(Debug)]
pub enum PrimaryCommand {
    Connect(ConnectCommand),
    Seal { binding: BindingId },
    Rebind {
        expected: BindingId,
        next: BindingId,
        deadline: Instant,
        cancel: CancelToken,
        retire: Arc<AttemptRetire>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RejectedCommand {
    Closed,
    StaleIdentity,
    Revoked,
    Deadline,
    DirtyReceiveBoundary,
    WorkerOwned,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PrimaryReply {
    Connected { binding: BindingId },
    /// The original OS failure kind remains available to the login Connect stage.
    ConnectFailed { binding: BindingId, kind: io::ErrorKind },
    Sealed { binding: BindingId, confirmed_at: Instant },
    Rebound { binding: BindingId },
    Rejected { binding: BindingId, reason: RejectedCommand },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CloseReason {
    Cancelled,
    Deadline,
    Abort,
    Unavailable,
    GracefulSession { by: Instant },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CloseCommand {
    pub binding: BindingId,
    pub reason: CloseReason,
}

#[derive(Debug)]
pub enum SubmitError {
    PrimaryOccupied,
    CloseOccupied,
    ReplyOccupied,
    StaleIdentity,
    Unavailable,
    Wake(io::Error),
}

/// One connection retains both transaction cells and both reply cells for
/// its whole physical lifetime. No third cell is allocated under pressure.
#[derive(Debug)]
pub struct ControlCells {
    pub id: u64,
    queued: AtomicBool,
    accepted_binding: AtomicU64,
    primary_ready: AtomicBool,
    primary_busy: AtomicBool,
    primary: Mutex<Option<PrimaryCommand>>,
    immediate_close: Mutex<Option<CloseCommand>>,
    primary_reply: Mutex<Option<PrimaryReply>>,
    close_ready: AtomicBool,
    close_reply: Mutex<Option<BindingId>>,
    resume_read: AtomicU64,
    output_kick: AtomicU64,
    pub park: Arc<Park>,
}

impl ControlCells {
    pub fn new(id: u64, park: Arc<Park>) -> Arc<Self> {
        Arc::new(Self {
            id,
            queued: AtomicBool::new(false),
            accepted_binding: AtomicU64::new(0),
            primary_busy: AtomicBool::new(false),
            primary: Mutex::new(None),
            primary_ready: AtomicBool::new(false),
            immediate_close: Mutex::new(None),
            primary_reply: Mutex::new(None),
            close_reply: Mutex::new(None),
            resume_read: AtomicU64::new(0),
            close_ready: AtomicBool::new(false),
            output_kick: AtomicU64::new(0),
            park,
        })
    }

    pub fn post_primary(&self, command: PrimaryCommand, bus: &CommandBus) -> Result<(), SubmitError> {
        if self.primary_busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err(if self.primary_reply.lock().unwrap().is_some() {
                SubmitError::ReplyOccupied
            } else {
                SubmitError::PrimaryOccupied
            });
        }
        *self.primary.lock().unwrap() = Some(command);
        bus.schedule(self.id, &self.queued).map_err(SubmitError::Wake)
    }

    /// Dedicated close cell remains available while Seal is in flight.
    /// A newly accepted Rebind may replace an older stale Close in this same
    /// cell. A new attempt that has not been accepted cannot replace it.
    pub fn post_close(&self, command: CloseCommand, bus: &CommandBus) -> Result<(), SubmitError> {
        if self.close_reply.lock().unwrap().is_some() {
            return Err(SubmitError::CloseOccupied);
        }
        {
            let mut pending = self.immediate_close.lock().unwrap();
            if let Some(previous) = pending.as_ref() {
                let accepted = self.accepted_binding.load(Ordering::Acquire);
                if command.binding.connection != self.id
                    || command.binding.incarnation != previous.binding.incarnation
                    || command.binding.binding != accepted
                    || command.binding.binding <= previous.binding.binding
                {
                    return Err(SubmitError::CloseOccupied);
                }
            }
            *pending = Some(command);
        }
        bus.schedule(self.id, &self.queued).map_err(SubmitError::Wake)
    }

    pub fn binding(&self) -> u64 {
        self.accepted_binding.load(Ordering::Acquire)
    }

    pub fn accept_binding(&self, binding: u64) {
        assert_ne!(binding, 0);
        self.accepted_binding.store(binding, Ordering::Release);
    }

    /// ResumeRead and output kicks are coalesced generation-scoped flags, not
    /// extra commands. A stale flag cannot act on the next binding.
    pub fn resume_read(&self, binding_generation: u64, bus: &CommandBus) -> io::Result<()> {
        assert_ne!(binding_generation, 0);
        self.resume_read.store(binding_generation, Ordering::Release);
        bus.schedule(self.id, &self.queued)
    }

    pub fn kick_output(&self, binding_generation: u64, bus: &CommandBus) -> io::Result<()> {
        assert_ne!(binding_generation, 0);
        self.output_kick.store(binding_generation, Ordering::Release);
        bus.schedule(self.id, &self.queued)
    }

    /// Reactor-only take. Prioritize immediate close before any old primary.
    pub fn take_close(&self) -> Option<CloseCommand> {
        self.immediate_close.lock().unwrap().take()
    }

    pub fn take_primary(&self) -> Option<PrimaryCommand> {
        self.primary.lock().unwrap().take()
    }

    pub fn take_resume_read(&self) -> u64 {
        self.resume_read.swap(0, Ordering::AcqRel)
    }

    pub fn take_output_kick(&self) -> u64 {
        self.output_kick.swap(0, Ordering::AcqRel)
    }

    /// The reactor publishes the reply before the K2 epoch/wake. The slot
    /// consumes each exact-generation reply exactly once.
    pub fn publish_primary(&self, reply: PrimaryReply) {
        {
            let mut cell = self.primary_reply.lock().unwrap();
            assert!(cell.is_none(), "primary reply cell cannot overwrite");
            *cell = Some(reply);
            self.primary_ready.store(true, Ordering::Release);
        }
        self.park.publish();
    }

    /// K2 wait predicates inspect only this publication, not the reply mutex.
    pub fn has_primary_reply(&self) -> bool {
        self.primary_ready.load(Ordering::Acquire)
    }

    pub fn take_primary_reply(&self) -> Option<PrimaryReply> {
        let mut cell = self.primary_reply.lock().unwrap();
        let reply = cell.take();
        if reply.is_some() {
            self.primary_ready.store(false, Ordering::Release);
            self.primary_busy.store(false, Ordering::Release);
        }
        reply
    }

    /// Close acknowledgment is published only *after* actual socket drop or
    /// after the separate post-C committed-detachment disposition.
    pub fn publish_closed(&self, binding: BindingId) {
        {
            let mut cell = self.close_reply.lock().unwrap();
            assert!(cell.is_none(), "close reply cell cannot overwrite");
            *cell = Some(binding);
            self.close_ready.store(true, Ordering::Release);
        }
        self.park.publish();
    }

    pub fn has_close_reply(&self) -> bool {
        self.close_ready.load(Ordering::Acquire)
    }

    pub fn take_closed(&self) -> Option<BindingId> {
        let mut cell = self.close_reply.lock().unwrap();
        let reply = cell.take();
        if reply.is_some() {
            self.close_ready.store(false, Ordering::Release);
        }
        reply
    }
}

/// K3 holds only the bounded FIFO of connection IDs. No per-connection
/// command mutex is held while taking K3 or calling the mio Waker.
#[derive(Debug)]
pub struct CommandBus {
    queue: Mutex<VecDeque<u64>>,
    waker: Arc<Waker>,
}

impl CommandBus {
    pub fn new(waker: Arc<Waker>) -> Self {
        Self { queue: Mutex::new(VecDeque::with_capacity(MAX_CONNECTIONS)), waker }
    }
    /// Wake the owner for shutdown or another service-level transition.
    pub fn wake(&self) -> io::Result<()> {
        self.waker.wake()
    }
    /// Share this reactor wake with its one fixed initial-TLS worker.
    pub fn reactor_waker(&self) -> Arc<Waker> {
        Arc::clone(&self.waker)
    }

    fn schedule(&self, id: u64, queued: &AtomicBool) -> io::Result<()> {
        if !queued.swap(true, Ordering::AcqRel) {
            {
                let mut queue = self.queue.lock().unwrap();
                // Each live connection owns exactly one bit. A bit can enter
                // the FIFO only once until the reactor clears it after pop.
                assert!(queue.len() < MAX_CONNECTIONS, "connection cap protects command FIFO");
                queue.push_back(id);
            }
            self.waker.wake()?;
        }
        Ok(())
    }


    /// Reactor pop by ID when the connection registry is separate. The caller
    /// must clear the corresponding cell's bit *before* taking its commands.
    pub fn pop_id(&self) -> Option<u64> {
        self.queue.lock().unwrap().pop_front()
    }

    pub fn mark_taken(&self, cell: &ControlCells) {
        cell.queued.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mio::{Poll, Token};

    fn bus() -> CommandBus {
        let poll = Poll::new().unwrap();
        CommandBus::new(Arc::new(Waker::new(poll.registry(), Token(0)).unwrap()))
    }

    fn binding(attempt: u64) -> BindingId {
        BindingId { incarnation: 1, connection: 2, binding: attempt, attempt }
    }

    #[test]
    fn close_has_reserved_cell_even_while_seal_and_reply_are_pending() {
        let bus = bus();
        let park = Arc::new(Park::new());
        let cell = ControlCells::new(2, park);
        cell.post_primary(PrimaryCommand::Seal { binding: binding(3) }, &bus).unwrap();
        cell.post_close(
            CloseCommand { binding: binding(3), reason: CloseReason::Abort },
            &bus,
        ).unwrap();
        assert_eq!(bus.pop_id(), Some(2));
        bus.mark_taken(&cell);
        assert_eq!(bus.pop_id(), None, "coalesced commands have one FIFO entry");
        assert_eq!(cell.take_close().unwrap().reason, CloseReason::Abort);
        assert!(matches!(cell.take_primary(), Some(PrimaryCommand::Seal { .. })));
        cell.publish_primary(PrimaryReply::Rejected {
            binding: binding(3),
            reason: RejectedCommand::Closed,
        });
        cell.publish_closed(binding(3));
        assert!(matches!(cell.take_primary_reply(), Some(PrimaryReply::Rejected { .. })));
        assert_eq!(cell.take_closed(), Some(binding(3)));
    }

    #[test]
    fn primary_remains_reserved_after_reactor_takes_it_until_slot_consumes_reply() {
        let bus = bus();
        let cell = ControlCells::new(2, Arc::new(Park::new()));
        cell.post_primary(PrimaryCommand::Seal { binding: binding(3) }, &bus).unwrap();
        assert_eq!(bus.pop_id(), Some(2));
        bus.mark_taken(&cell);
        assert!(matches!(cell.take_primary(), Some(PrimaryCommand::Seal { .. })));
        assert!(matches!(
            cell.post_primary(PrimaryCommand::Seal { binding: binding(4) }, &bus),
            Err(SubmitError::PrimaryOccupied),
        ));
        cell.publish_primary(PrimaryReply::Sealed { binding: binding(3), confirmed_at: Instant::now() });
        assert!(matches!(
            cell.post_primary(PrimaryCommand::Seal { binding: binding(4) }, &bus),
            Err(SubmitError::ReplyOccupied),
        ));
        assert!(matches!(cell.take_primary_reply(), Some(PrimaryReply::Sealed { .. })));
        cell.post_primary(PrimaryCommand::Seal { binding: binding(4) }, &bus).unwrap();
    }

    #[test]
    fn a_producer_after_queue_pop_schedules_a_new_turn() {
        let bus = bus();
        let cell = ControlCells::new(2, Arc::new(Park::new()));
        cell.kick_output(3, &bus).unwrap();
        assert_eq!(bus.pop_id(), Some(2));
        bus.mark_taken(&cell);
        cell.resume_read(4, &bus).unwrap();
        assert_eq!(cell.take_output_kick(), 3);
        assert_eq!(bus.pop_id(), Some(2));
        bus.mark_taken(&cell);
        assert_eq!(cell.take_resume_read(), 4);
    }
}
