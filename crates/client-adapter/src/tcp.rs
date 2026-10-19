//! Plain game TCP read/write in finite reactor turns. Socket payload bytes
//! travel directly between the registered mio adapter and fixed SPSC rings.

use std::io::{self, Read, Write};

use super::budget::{Budgeted, StopReason, TURN_IO_ATTEMPTS};
use super::park::{Park, WriterBinding};
use super::records::RecordQueue;
use super::ring::{Ring, IN_RING_BYTES, OUT_RING_BYTES};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TcpStop {
    /// Both directions are waiting for fresh kernel readiness or output.
    Quiescent,
    /// A synthetic budget/returned progress needs another fair FIFO turn.
    LocalWork,
    /// Inbound ring full: stop reads until a generation-scoped ResumeRead.
    HighWater,
    /// A real EOF. The reactor closes after returning to its owner loop.
    PeerClosed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TcpTurnResult {
    pub stop: TcpStop,
    pub read_bytes: usize,
    pub written_bytes: usize,
    pub read_ready: bool,
    pub write_ready: bool,
}

/// Reactor-owned state; one record may span multiple partial socket writes.
#[derive(Debug, Default)]
pub struct TcpTurn {
    record_remaining: usize,
    pub read_ready: bool,
    pub write_ready: bool,
}

impl TcpTurn {
    pub fn on_readable(&mut self) {
        self.read_ready = true;
    }

    pub fn on_writable(&mut self) {
        self.write_ready = true;
    }

    pub fn resume_read(&mut self) {
        self.read_ready = true;
    }

    pub fn kick_output(&mut self) {
        self.write_ready = true;
    }

    /// Rebind transfers read ownership to the reactor's old-generation
    /// discard turn. Pending old output may still drain, but no new payload
    /// enters the slot ring before the clean-boundary check.
    pub fn suppress_read_for_rebind(&mut self) {
        self.read_ready = false;
    }

    /// `io` has been reset once at the start of this fair connection turn.
    /// Read/write alternate to prevent continuous inbound data from starving
    /// already-published game records. The caller requeues LocalWork once.
    pub fn drive<S: Read + Write>(
        &mut self,
        io: &mut Budgeted<S>,
        inbound: &Ring<IN_RING_BYTES>,
        outbound: &Ring<OUT_RING_BYTES>,
        records: &RecordQueue,
        park: &Park,
        binding: WriterBinding,
    ) -> io::Result<TcpTurnResult> {
        let mut closed = false;
        let mut high_water = false;
        let mut read_progress = 0;
        let mut write_progress = 0;
        let mut next_is_read = true;
        let mut turn_read_allowed = true;
        let mut turn_write_allowed = true;
        while io.budget().attempts() < TURN_IO_ATTEMPTS {
            let can_read = self.read_ready && turn_read_allowed && inbound.free() > 0;
            let can_write = self.write_ready
                && turn_write_allowed
                && (self.record_remaining != 0 || records.front().is_some());
            if self.read_ready && inbound.free() == 0 {
                high_water = true;
                self.read_ready = false;
            }
            if !can_read && !can_write {
                break;
            }
            if can_read && (next_is_read || !can_write) {
                match inbound.produce_with(|buffer| io.read(buffer)) {
                    Ok(0) => {
                        closed = true;
                        self.read_ready = false;
                        break;
                    }
                    Ok(n) => {
                        read_progress += n;
                        next_is_read = false;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if io.budget().read_stop() == StopReason::SocketWouldBlock {
                            self.read_ready = false;
                        } else {
                            turn_read_allowed = false;
                        }
                        next_is_read = false;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                if self.record_remaining == 0 {
                    self.record_remaining = usize::from(records.front().expect("eligible record"));
                }
                let remaining = self.record_remaining;
                match outbound
                    .consume_with(|buffer| io.write(&buffer[..buffer.len().min(remaining)]))
                {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => {
                        write_progress += n;
                        self.record_remaining -= n;
                        park.publish_freed_space(binding);
                        if self.record_remaining == 0 {
                            records.pop_front();
                        }
                        next_is_read = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if io.budget().write_stop() == StopReason::SocketWouldBlock {
                            self.write_ready = false;
                        } else {
                            turn_write_allowed = false;
                        }
                        next_is_read = true;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let pending_write =
            self.write_ready && (self.record_remaining != 0 || records.front().is_some());
        let stop = if closed {
            TcpStop::PeerClosed
        } else {
            if high_water || (self.read_ready && inbound.free() == 0) {
                self.read_ready = false;
                high_water = true;
            }
            if (self.read_ready && inbound.free() > 0) || pending_write {
                TcpStop::LocalWork
            } else if high_water {
                TcpStop::HighWater
            } else {
                TcpStop::Quiescent
            }
        };
        Ok(TcpTurnResult {
            stop,
            read_bytes: read_progress,
            written_bytes: write_progress,
            read_ready: self.read_ready,
            write_ready: self.write_ready,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Socket {
        incoming: Vec<u8>,
        sent: Vec<u8>,
        read_at: usize,
        writes_blocked: bool,
        max_write: usize,
    }

    impl Read for Socket {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.read_at == self.incoming.len() {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = bytes.len().min(self.incoming.len() - self.read_at);
            bytes[..n].copy_from_slice(&self.incoming[self.read_at..self.read_at + n]);
            self.read_at += n;
            Ok(n)
        }
    }

    impl Write for Socket {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.writes_blocked {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = bytes.len().min(self.max_write);
            self.sent.extend_from_slice(&bytes[..n]);
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_socket_writes_keep_record_cursor_and_order() {
        let inbound = Ring::new();
        let outbound = Ring::new();
        let records = RecordQueue::new();
        let park = Arc::new(Park::new());
        let binding = WriterBinding {
            connection: 4,
            binding: 7,
        };
        let mut io = Budgeted::new(Socket {
            max_write: 2,
            ..Socket::default()
        });
        let mut turn = TcpTurn::default();
        assert!(outbound.push_all(b"abcde"));
        records.push(5);
        assert!(outbound.push_all(b"FG"));
        records.push(2);
        turn.on_writable();
        io.inner_mut().writes_blocked = true;
        assert_eq!(
            turn.drive(&mut io, &inbound, &outbound, &records, &park, binding)
                .unwrap()
                .stop,
            TcpStop::Quiescent
        );
        assert_eq!(records.len(), 2);
        io.inner_mut().writes_blocked = false;
        io.reset_turn();
        turn.on_writable();
        turn.drive(&mut io, &inbound, &outbound, &records, &park, binding)
            .unwrap();
        assert_eq!(io.inner().sent, b"abcdeFG");
        assert_eq!(records.len(), 0);
        assert_eq!(outbound.len(), 0);
    }

    #[test]
    fn a_real_read_block_clears_only_read_eligibility() {
        let inbound = Ring::new();
        let outbound = Ring::new();
        let records = RecordQueue::new();
        let park = Park::new();
        let mut io = Budgeted::new(Socket {
            max_write: 10,
            ..Socket::default()
        });
        let mut turn = TcpTurn::default();
        assert!(outbound.push_all(b"xyz"));
        records.push(3);
        turn.on_readable();
        turn.on_writable();
        let result = turn
            .drive(
                &mut io,
                &inbound,
                &outbound,
                &records,
                &park,
                WriterBinding {
                    connection: 1,
                    binding: 1,
                },
            )
            .unwrap();
        assert!(!result.read_ready);
        assert_eq!(io.inner().sent, b"xyz");
    }

    #[test]
    fn inbound_high_water_does_not_strand_pending_output() {
        let inbound = Ring::new();
        let outbound = Ring::new();
        let records = RecordQueue::new();
        let park = Park::new();
        let binding = WriterBinding {
            connection: 9,
            binding: 2,
        };
        let mut io = Budgeted::new(Socket {
            max_write: 1,
            ..Socket::default()
        });
        let mut turn = TcpTurn::default();
        assert!(inbound.push_all(&[7; IN_RING_BYTES]));
        assert!(outbound.push_all(&[5; 64]));
        records.push(64);
        turn.on_readable();
        turn.on_writable();

        let result = turn
            .drive(&mut io, &inbound, &outbound, &records, &park, binding)
            .unwrap();
        assert_eq!(result.stop, TcpStop::LocalWork);
        assert!(!result.read_ready);
        assert!(result.write_ready);
        assert!(!outbound.is_empty());
    }
}
