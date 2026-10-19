//! One physical game connection's slot/reactor shared state. No mutable
//! socket or native TLS object is shared with the slot.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::control::{BindingId, CommandBus, ControlCells};
use super::park::Park;
use super::records::RecordQueue;
use super::ring::{Ring, IN_RING_BYTES, OUT_RING_BYTES};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AppendRefusal {
    Closed,
    StaleBinding,
    Backpressured,
    InvalidLength,
}

/// A successful append has full record custody even if mio Waker fails. The
/// caller must never encode or append it a second time on that error.
#[derive(Debug)]
pub enum AppendResult {
    Accepted,
    AcceptedWakeFailed(io::Error),
    Refused(AppendRefusal),
}

/// Transport-level cause retained before generic `io::Error` erases protocol
/// details. The slot takes this at most once when it classifies session loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamExitReason {
    WebSocketClose { code: Option<u16>, reason: String },
    Tls(String),
    WebSocket(String),
}

#[derive(Debug)]
pub struct SharedConnection {
    pub inbound: Ring<IN_RING_BYTES>,
    pub outbound: Ring<OUT_RING_BYTES>,
    pub records: RecordQueue,
    pub control: Arc<ControlCells>,
    /// Only Phase C publishes a session binding. A reactor Rebind requires
    /// the old binding's committed authority and clears it before new login.
    session_binding: AtomicU64,
    /// The slot disables reads before posting Rebind. Only then may the
    /// reactor consume/dispose old-generation ring bytes.
    rebind_disabled: AtomicBool,
    closed: AtomicBool,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    exit_reason: Mutex<Option<StreamExitReason>>,
}

impl SharedConnection {
    pub fn new(id: u64, park: Arc<Park>) -> Arc<Self> {
        Arc::new(Self {
            inbound: Ring::new(),
            outbound: Ring::new(),
            records: RecordQueue::new(),
            control: ControlCells::new(id, park),
            session_binding: AtomicU64::new(0),
            rebind_disabled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            exit_reason: Mutex::new(None),
        })
    }

    /// Called by the reactor only after initial connect/accepted Rebind.
    pub fn accept_binding(&self, id: BindingId) {
        assert_eq!(self.control.id, id.connection);
        assert_ne!(id.binding, 0);
        self.session_binding.store(0, Ordering::Release);
        self.rebind_disabled.store(false, Ordering::Release);
        self.control.accept_binding(id.binding);
    }

    pub fn binding(&self) -> u64 {
        self.control.binding()
    }

    pub fn commit_session(&self, id: BindingId) -> bool {
        if self.control.id != id.connection || self.binding() != id.binding || self.is_closed() {
            return false;
        }
        self.session_binding.store(id.binding, Ordering::Release);
        true
    }

    pub fn session_binding(&self) -> u64 {
        self.session_binding.load(Ordering::Acquire)
    }

    pub fn disable_for_rebind(&self) {
        self.rebind_disabled.store(true, Ordering::Release);
    }

    pub fn rebind_disabled(&self) -> bool {
        self.rebind_disabled.load(Ordering::Acquire)
    }

    /// Publish the first typed failure before `publish_closed` wakes the
    /// slot. A later close/error cannot overwrite the original exit cause.
    pub(crate) fn publish_exit_reason(&self, reason: StreamExitReason) {
        let mut pending = self.exit_reason.lock().unwrap();
        if pending.is_none() {
            *pending = Some(reason);
        }
    }

    pub(crate) fn take_exit_reason(&self) -> Option<StreamExitReason> {
        self.exit_reason.lock().unwrap().take()
    }

    /// Called by the reactor after socket close/deregister/drop, before its
    /// physical retirement acknowledgement. Rebind never resets ring cursors.
    pub fn publish_closed(&self) {
        self.session_binding.store(0, Ordering::Release);
        self.closed.store(true, Ordering::Release);
        self.control.park.publish();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Publish one complete prepared semantic packet. The login slot may
    /// call this *inside K2*, after terminal/deadline/capacity rechecks, then
    /// schedule the reactor only after K2 unlock. Ring backpressure publishes
    /// neither bytes nor a record boundary.
    pub fn publish_record(&self, id: BindingId, record: &[u8]) -> Result<(), AppendRefusal> {
        if self.is_closed() {
            return Err(AppendRefusal::Closed);
        }
        if self.control.id != id.connection || self.binding() != id.binding {
            return Err(AppendRefusal::StaleBinding);
        }
        if record.is_empty() || record.len() > OUT_RING_BYTES || record.len() > u16::MAX as usize {
            return Err(AppendRefusal::InvalidLength);
        }
        if !self.records.has_capacity() || !self.outbound.push_all(record) {
            return Err(AppendRefusal::Backpressured);
        }
        self.records.push(record.len() as u16);
        self.bytes_out.fetch_add(record.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Schedule only after publication and after all owner locks unlock.
    pub fn kick_record(&self, id: BindingId, bus: &CommandBus) -> io::Result<()> {
        self.control.kick_output(id.binding, bus)
    }

    /// The in-game slot does not Park for output capacity. A committed record
    /// remains committed if a mio Waker fails; never encode it twice.
    pub fn append_record(&self, id: BindingId, record: &[u8], bus: &CommandBus) -> AppendResult {
        match self.publish_record(id, record) {
            Ok(()) => match self.kick_record(id, bus) {
                Ok(()) => AppendResult::Accepted,
                Err(error) => AppendResult::AcceptedWakeFailed(error),
            },
            Err(refusal) => AppendResult::Refused(refusal),
        }
    }

    /// Consume only after the Park fast-path terminal/deadline checks. A
    /// positive read remains byte-exact across TCP/WSS and Seal authority.
    pub fn take_inbound(&self, dest: &mut [u8]) -> usize {
        let read = self.inbound.pop(dest);
        self.bytes_in.fetch_add(read as u64, Ordering::Relaxed);
        read
    }

    pub fn bytes_in(&self) -> u64 {
        self.bytes_in.load(Ordering::Relaxed)
    }

    pub fn bytes_out(&self) -> u64 {
        self.bytes_out.load(Ordering::Relaxed)
    }

    /// This is the known statically allocated connection extent; allocator
    /// header/capacity, TLS, WS, kernel and RSS need separate G5 evidence.
    pub fn known_struct_bytes() -> usize {
        std::mem::size_of::<Self>() + std::mem::size_of::<ControlCells>()
    }
}
