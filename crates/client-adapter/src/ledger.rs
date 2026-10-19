//! Physical resource admission. Logical completion never frees a charge.
//!
//! Bytes here count known Rust-owned payload/storage extents. Native TLS,
//! allocator overhead, kernel socket buffers, and stack RSS are separate
//! measurements, never silently represented by zero or by Vec `len`.

use std::sync::{Arc, Mutex};

pub const MAX_CONNECTIONS: usize = 1024;
pub const MAX_LOCAL_WORK: usize = 1024;
pub const MAX_RECORDS_PER_CONNECTION: usize = 64;
pub const COMMAND_CELLS_PER_CONNECTION: usize = 2;
pub const REPLY_CELLS_PER_CONNECTION: usize = 2;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ResourceRefusal {
    ConnectionsFull,
    LocalWorkFull,
    HandshakeLaneBusy,
    CountOverflow,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ResourceSnapshot {
    /// Includes physically live sockets even after logical terminal proof.
    pub connections: usize,
    /// Fixed client-owned extents only; not an all-process/RSS bound.
    pub known_rust_bytes: usize,
    /// Both command/reply cell kinds remain reserved until connection drop.
    pub command_cells: usize,
    pub reply_cells: usize,
    pub local_work: usize,
    /// A nonreturning native call retains this physical charge indefinitely.
    pub handshake_lane_occupied: bool,
}

#[derive(Debug, Default)]
pub struct ResourceLedger {
    counts: Mutex<ResourceSnapshot>,
}

/// Live socket/connection charge. Keep this in the owner that actually holds
/// the socket (the worker during handshake; reactor after transfer).
#[derive(Debug)]
pub struct ConnectionCharge {
    ledger: Arc<ResourceLedger>,
    known_bytes: usize,
}

/// Exactly one initial handshake can own the fixed lane and worker stack.
#[derive(Debug)]
pub struct HandshakeCharge {
    ledger: Arc<ResourceLedger>,
}

impl ResourceLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Caller supplies the actual statically allocated connection extent,
    /// including both rings, record slots, controls, and reply cells. Dynamic
    /// TLS/WS buffers are measured separately, not guessed as this value.
    pub fn admit_connection(
        self: &Arc<Self>,
        known_bytes: usize,
    ) -> Result<ConnectionCharge, ResourceRefusal> {
        let mut counts = self.counts.lock().unwrap();
        if counts.connections == MAX_CONNECTIONS {
            return Err(ResourceRefusal::ConnectionsFull);
        }
        let bytes = counts
            .known_rust_bytes
            .checked_add(known_bytes)
            .ok_or(ResourceRefusal::CountOverflow)?;
        let command_cells = counts
            .command_cells
            .checked_add(COMMAND_CELLS_PER_CONNECTION)
            .ok_or(ResourceRefusal::CountOverflow)?;
        let reply_cells = counts
            .reply_cells
            .checked_add(REPLY_CELLS_PER_CONNECTION)
            .ok_or(ResourceRefusal::CountOverflow)?;
        counts.connections += 1;
        counts.known_rust_bytes = bytes;
        counts.command_cells = command_cells;
        counts.reply_cells = reply_cells;
        Ok(ConnectionCharge {
            ledger: Arc::clone(self),
            known_bytes,
        })
    }

    pub fn admit_handshake(self: &Arc<Self>) -> Result<HandshakeCharge, ResourceRefusal> {
        let mut counts = self.counts.lock().unwrap();
        if counts.handshake_lane_occupied {
            return Err(ResourceRefusal::HandshakeLaneBusy);
        }
        counts.handshake_lane_occupied = true;
        Ok(HandshakeCharge {
            ledger: Arc::clone(self),
        })
    }

    /// Reactor-only admission into the de-duplicated FIFO. No byte payload
    /// or additional command cell is allocated for a duplicate kick.
    pub fn enqueue_local_work(&self) -> Result<(), ResourceRefusal> {
        let mut counts = self.counts.lock().unwrap();
        if counts.local_work == MAX_LOCAL_WORK {
            return Err(ResourceRefusal::LocalWorkFull);
        }
        counts.local_work += 1;
        Ok(())
    }

    /// Reactor-only removal when a queued connection takes its fair turn or
    /// is closed. A bit in that connection prevents double enqueue/removal.
    pub fn dequeue_local_work(&self) {
        let mut counts = self.counts.lock().unwrap();
        assert!(counts.local_work > 0);
        counts.local_work -= 1;
    }

    pub fn snapshot(&self) -> ResourceSnapshot {
        *self.counts.lock().unwrap()
    }
}

impl Drop for ConnectionCharge {
    fn drop(&mut self) {
        let mut counts = self.ledger.counts.lock().unwrap();
        assert!(counts.connections > 0);
        counts.connections -= 1;
        counts.known_rust_bytes -= self.known_bytes;
        counts.command_cells -= COMMAND_CELLS_PER_CONNECTION;
        counts.reply_cells -= REPLY_CELLS_PER_CONNECTION;
    }
}

impl Drop for HandshakeCharge {
    fn drop(&mut self) {
        let mut counts = self.ledger.counts.lock().unwrap();
        assert!(counts.handshake_lane_occupied);
        counts.handshake_lane_occupied = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_timeout_keeps_socket_and_handshake_charge_until_drop() {
        let ledger = ResourceLedger::new();
        let connection = ledger.admit_connection(40_000).unwrap();
        let lane = ledger.admit_handshake().unwrap();
        assert_eq!(
            ledger.admit_handshake().unwrap_err(),
            ResourceRefusal::HandshakeLaneBusy
        );
        // Publishing a logical terminal is not a resource operation.
        assert_eq!(ledger.snapshot().connections, 1);
        assert!(ledger.snapshot().handshake_lane_occupied);
        assert_eq!(ledger.snapshot().command_cells, 2);
        assert_eq!(ledger.snapshot().reply_cells, 2);
        drop(lane);
        assert!(!ledger.snapshot().handshake_lane_occupied);
        assert_eq!(ledger.snapshot().connections, 1);
        drop(connection);
        assert_eq!(ledger.snapshot().connections, 0);
        assert_eq!(ledger.snapshot().known_rust_bytes, 0);
    }

    #[test]
    fn capacity_refuses_without_charging_and_fifo_is_bounded() {
        let ledger = ResourceLedger::new();
        let mut sockets = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            sockets.push(ledger.admit_connection(1).unwrap());
        }
        assert_eq!(
            ledger.admit_connection(1).unwrap_err(),
            ResourceRefusal::ConnectionsFull
        );
        assert_eq!(ledger.snapshot().command_cells, 2 * MAX_CONNECTIONS);
        for _ in 0..MAX_LOCAL_WORK {
            ledger.enqueue_local_work().unwrap();
        }
        assert_eq!(
            ledger.enqueue_local_work().unwrap_err(),
            ResourceRefusal::LocalWorkFull
        );
        ledger.dequeue_local_work();
        ledger.enqueue_local_work().unwrap();
        drop(sockets);
        assert_eq!(ledger.snapshot().connections, 0);
    }
}
