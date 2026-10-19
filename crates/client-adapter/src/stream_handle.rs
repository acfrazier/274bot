//! Slot-side game stream access. No socket, WS parser or TLS object crosses
//! this boundary; accepted login I/O uses K2 terminal-first Park checks.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cancel::CancelToken;
use super::control::{BindingId, CloseCommand, CloseReason, CommandBus};
use super::health::ReactorHealth;
use super::park::{Park, ParkOutcome, WriterBinding};
use super::shared::{AppendRefusal, AppendResult, SharedConnection, StreamExitReason};

pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ReadResult {
    Bytes(usize),
    End,
}

fn session_append_result(result: AppendResult) -> io::Result<()> {
    match result {
        AppendResult::Accepted => Ok(()),
        // The record already belongs to the reactor. Poll wakes it within
        // 100 ms; health maintenance reports lasting loss.
        AppendResult::AcceptedWakeFailed(_error) => Ok(()),
        AppendResult::Refused(AppendRefusal::Backpressured) => {
            Err(io::ErrorKind::WouldBlock.into())
        }
        AppendResult::Refused(reason) => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            format!("record refused: {reason:?}"),
        )),
    }
}

#[derive(Debug)]
pub struct ConnectionHandle {
    shared: Arc<SharedConnection>,
    bus: Arc<CommandBus>,
    health: Arc<ReactorHealth>,
    binding: BindingId,
    deadline: Option<Instant>,
    token: Option<CancelToken>,
    stronger_terminal: Option<Arc<AtomicBool>>,
    enabled: bool,
    session_authority: bool,
    closed_locally: bool,
}

impl ConnectionHandle {
    pub fn new(
        shared: Arc<SharedConnection>,
        bus: Arc<CommandBus>,
        health: Arc<ReactorHealth>,
        binding: BindingId,
        deadline: Option<Instant>,
        token: Option<CancelToken>,
        stronger_terminal: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            shared,
            bus,
            health,
            binding,
            deadline,
            token,
            stronger_terminal,
            enabled: false,
            session_authority: false,
            closed_locally: false,
        }
    }

    pub fn park(&self) -> Arc<Park> {
        Arc::clone(&self.shared.control.park)
    }

    pub fn identity(&self) -> BindingId {
        self.binding
    }

    /// Positive matching Connect/Rebind receipt alone enables login I/O.
    pub fn confirm_binding(&mut self, identity: BindingId) -> io::Result<()> {
        if identity != self.binding
            || self.shared.binding() != identity.binding
            || self.shared.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stale connection binding",
            ));
        }
        self.enabled = true;
        Ok(())
    }

    /// Disable old/new ring access while a Rebind transaction is pending.
    pub fn disable_for_rebind(&mut self) {
        self.enabled = false;
        self.shared.disable_for_rebind();
    }

    /// Apply only a positive exact-generation Rebind reply. The reactor has
    /// already drained old output and established a clean receive boundary.
    pub fn confirm_rebind(
        &mut self,
        next: BindingId,
        deadline: Instant,
        token: CancelToken,
        stronger_terminal: Option<Arc<AtomicBool>>,
    ) -> io::Result<()> {
        if self.enabled
            || next.connection != self.binding.connection
            || self.shared.binding() != next.binding
            || self.shared.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rebind not confirmed",
            ));
        }
        self.binding = next;
        self.deadline = Some(deadline);
        self.token = Some(token);
        self.stronger_terminal = stronger_terminal;
        self.session_authority = false;
        self.enabled = true;
        Ok(())
    }

    /// A Seal receipt alone is not session authority; host Phase C invokes
    /// this only after its K4 commit and deadline detachment.
    pub fn commit_session(&mut self, identity: BindingId) -> io::Result<()> {
        if identity != self.binding || !self.enabled || !self.shared.commit_session(identity) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session commit rejected",
            ));
        }
        self.deadline = None;
        self.token = None;
        self.stronger_terminal = None;
        self.session_authority = true;
        Ok(())
    }

    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        // Clearing the deadline is a committed session action. A returned
        // candidate cannot clear it merely because Client::login returned.
        if deadline.is_some() || self.session_authority {
            self.deadline = deadline;
        }
    }

    pub fn bytes_in(&self) -> u64 {
        self.shared.bytes_in()
    }

    pub fn bytes_out(&self) -> u64 {
        self.shared.bytes_out()
    }

    pub(crate) fn take_exit_reason(&self) -> Option<StreamExitReason> {
        self.shared.take_exit_reason()
    }

    /// Only atomics/ring occupancy are inspected; safe under K2 and never
    /// borrows the reactor-owned native socket or a WebSocket parser.
    pub fn available(&self) -> io::Result<i32> {
        if self.closed_locally {
            return Ok(0);
        }
        if !self.enabled {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "binding unconfirmed",
            ));
        }
        let len = self.shared.inbound.len();
        if len > 0 {
            return Ok(len.min(8192) as i32);
        }
        if self.shared.is_closed() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
        }
        Ok(0)
    }

    pub fn read(&mut self) -> io::Result<i32> {
        let mut byte = [0];
        match self.read_some(&mut byte)? {
            0 if self.closed_locally => Ok(0),
            0 => Ok(-1),
            _ => Ok(i32::from(byte[0])),
        }
    }

    pub fn read_bytes(&mut self, buf: &mut [u8], off: usize, len: usize) -> io::Result<()> {
        if self.closed_locally {
            return Ok(());
        }
        let end = off
            .checked_add(len)
            .filter(|end| *end <= buf.len())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "read extent outside buffer")
            })?;
        let mut cursor = off;
        while cursor < end {
            let n = self.read_some(&mut buf[cursor..end])?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
            }
            cursor += n;
        }
        Ok(())
    }

    pub fn read_some(&mut self, dest: &mut [u8]) -> io::Result<usize> {
        if dest.is_empty() || self.closed_locally {
            return Ok(0);
        }
        if !self.enabled {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "binding unconfirmed",
            ));
        }
        if self.session_authority {
            if self.health.loss().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "game transport unavailable",
                ));
            }
            let read = self.shared.take_inbound(dest);
            if read > 0 {
                let _ = self
                    .shared
                    .control
                    .resume_read(self.binding.binding, &self.bus);
                return Ok(read);
            }
        }
        let deadline = self
            .deadline
            .unwrap_or_else(|| Instant::now() + READ_TIMEOUT);
        let token = self.token.as_ref();
        let stronger = self.stronger_terminal.as_ref();
        let shared = &self.shared;
        let outcome = shared.control.park.wait(
            deadline,
            Instant::now,
            || self.health.loss().is_some(),
            || stronger.is_some_and(|state| state.load(Ordering::Acquire)),
            || token.is_some_and(CancelToken::is_cancelled),
            || {
                if !shared.inbound.is_empty() {
                    let read = shared.take_inbound(dest);
                    return Some(ReadResult::Bytes(read));
                }
                shared.is_closed().then_some(ReadResult::End)
            },
            false,
            None,
        );
        match outcome {
            ParkOutcome::Ready(ReadResult::Bytes(read)) => {
                let _ = shared.control.resume_read(self.binding.binding, &self.bus);
                Ok(read)
            }
            ParkOutcome::Ready(ReadResult::End) => Ok(0),
            ParkOutcome::Unavailable => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "game transport unavailable",
            )),
            ParkOutcome::SelectedIntent | ParkOutcome::Revoked => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "login attempt revoked",
            )),
            ParkOutcome::Deadline => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                if self.session_authority {
                    "read deadline passed"
                } else {
                    // Keep the fork client's established login diagnostic;
                    // hosted transport does not change the public error.
                    "login handshake deadline passed"
                },
            )),
        }
    }

    /// Append exactly one already-encoded semantic record. During login a
    /// full ring sleeps under K2 with its generation-scoped writer marker;
    /// a session pump returns WouldBlock so the client retains its bytes.
    pub fn write(&mut self, record: &[u8], len: usize) -> io::Result<()> {
        if self.closed_locally {
            return Ok(());
        }
        if !self.enabled || (!self.session_authority && self.deadline.is_none()) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "binding not authorized",
            ));
        }
        let record = &record[..len.min(record.len())];
        if record.is_empty() {
            return Ok(());
        }
        if self.session_authority {
            if self.health.loss().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "game transport unavailable",
                ));
            }
            return session_append_result(self.shared.append_record(
                self.binding,
                record,
                &self.bus,
            ));
        }
        let deadline = self.deadline.expect("uncommitted session cannot write");
        let token = self.token.as_ref();
        let stronger = self.stronger_terminal.as_ref();
        let binding = WriterBinding {
            connection: self.binding.connection,
            binding: self.binding.binding,
        };
        let mut refusal = None;
        let outcome = self.shared.control.park.wait(
            deadline,
            Instant::now,
            || self.health.loss().is_some(),
            || stronger.is_some_and(|state| state.load(Ordering::Acquire)),
            || token.is_some_and(CancelToken::is_cancelled),
            || match self.shared.publish_record(self.binding, record) {
                Ok(()) => Some(()),
                Err(AppendRefusal::Backpressured) => None,
                Err(reason) => {
                    refusal = Some(reason);
                    Some(())
                }
            },
            false,
            Some(binding),
        );
        match outcome {
            ParkOutcome::Ready(()) => {
                if let Some(reason) = refusal {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        format!("record refused: {reason:?}"),
                    ));
                }
                self.shared.kick_record(self.binding, &self.bus)
            }
            ParkOutcome::Unavailable => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "game transport unavailable",
            )),
            ParkOutcome::SelectedIntent | ParkOutcome::Revoked => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "login attempt revoked",
            )),
            ParkOutcome::Deadline => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "login handshake deadline passed",
            )),
        }
    }

    /// Local handle retirement is immediate. Reactor/worker custody and
    /// AttemptRetire credit persist until its actual close/deregister/drop.
    pub fn close(&mut self) {
        self.close_with_reason(CloseReason::Abort);
    }

    pub fn close_with_reason(&mut self, reason: CloseReason) {
        if self.closed_locally {
            return;
        }
        self.closed_locally = true;
        let accepted_binding = self.shared.binding();
        if self.shared.is_closed()
            || (accepted_binding != 0 && accepted_binding != self.binding.binding)
        {
            return;
        }
        let _ = self.shared.control.post_close(
            CloseCommand {
                binding: self.binding,
                reason,
            },
            &self.bus,
        );
    }
}

impl Drop for ConnectionHandle {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_record_survives_failed_wake_without_client_retry() {
        assert!(
            session_append_result(AppendResult::AcceptedWakeFailed(io::Error::other(
                "injected waker failure"
            )))
            .is_ok()
        );
        assert_eq!(
            session_append_result(AppendResult::Refused(AppendRefusal::Backpressured))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
    }
}
