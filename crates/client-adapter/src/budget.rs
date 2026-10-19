//! Finite, classified I/O calls for one reactor connection turn.
//! Synthetic limits never masquerade as an underlying readiness edge.

use std::io::{self, Read, Write};

pub const TURN_BYTE_LIMIT: usize = 16 * 1024;
pub const TURN_IO_ATTEMPTS: usize = 32;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StopReason {
    /// Socket really returned WouldBlock: a new readiness edge is needed.
    SocketWouldBlock,
    /// Local byte/call budget: retain continuation in the fair FIFO.
    Budget,
    /// The underlying operation returned normally and may have private work.
    Returned,
}

#[derive(Debug, Clone)]
pub struct IoBudget {
    read_bytes: usize,
    write_bytes: usize,
    attempts: usize,
    read_stop: StopReason,
    write_stop: StopReason,
}

impl Default for IoBudget {
    fn default() -> Self {
        Self {
            read_bytes: 0,
            write_bytes: 0,
            attempts: 0,
            read_stop: StopReason::Returned,
            write_stop: StopReason::Returned,
        }
    }
}

impl IoBudget {
    /// One reset at the start of a fair reactor turn, never after a control
    /// frame or during one resumable rustls/WS operation.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn read_stop(&self) -> StopReason {
        self.read_stop
    }

    pub fn write_stop(&self) -> StopReason {
        self.write_stop
    }

    /// A nested rustls operation may surface its inner wire budget as an
    /// outer `WouldBlock`. Preserve the actual stop cause before scheduling.
    pub fn classify_read(&mut self, reason: StopReason) {
        self.read_stop = reason;
    }

    pub fn classify_write(&mut self, reason: StopReason) {
        self.write_stop = reason;
    }

    pub fn read_bytes(&self) -> usize {
        self.read_bytes
    }

    pub fn write_bytes(&self) -> usize {
        self.write_bytes
    }

    pub fn attempts(&self) -> usize {
        self.attempts
    }

    fn read_extent(&mut self, requested: usize) -> io::Result<usize> {
        let permitted = requested.min(TURN_BYTE_LIMIT - self.read_bytes);
        if permitted == 0 || self.attempts == TURN_IO_ATTEMPTS {
            self.read_stop = StopReason::Budget;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.attempts += 1;
        Ok(permitted)
    }

    fn write_extent(&mut self, requested: usize) -> io::Result<usize> {
        let permitted = requested.min(TURN_BYTE_LIMIT - self.write_bytes);
        if permitted == 0 || self.attempts == TURN_IO_ATTEMPTS {
            self.write_stop = StopReason::Budget;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.attempts += 1;
        Ok(permitted)
    }
}

/// Single-owner budget adapter around the actual nonblocking socket or
/// plaintext stream. For rustls, use separate instances for encrypted wire
/// I/O and application plaintext I/O, reset together once per reactor turn.
#[derive(Debug)]
pub struct Budgeted<S> {
    inner: S,
    budget: IoBudget,
}

impl<S> Budgeted<S> {
    pub fn new(inner: S) -> Self {
        Self { inner, budget: IoBudget::default() }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    pub fn budget(&self) -> &IoBudget {
        &self.budget
    }

    pub fn reset_turn(&mut self) {
        self.budget.reset();
    }
}

impl<S: Read> Read for Budgeted<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let limit = self.budget.read_extent(buf.len())?;
        match self.inner.read(&mut buf[..limit]) {
            Ok(n) => {
                self.budget.read_bytes += n;
                self.budget.read_stop = StopReason::Returned;
                Ok(n)
            }
            Err(error) => {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.budget.read_stop = StopReason::SocketWouldBlock;
                }
                Err(error)
            }
        }
    }
}

impl<S: Write> Write for Budgeted<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let limit = self.budget.write_extent(buf.len())?;
        match self.inner.write(&buf[..limit]) {
            Ok(n) => {
                self.budget.write_bytes += n;
                self.budget.write_stop = StopReason::Returned;
                Ok(n)
            }
            Err(error) => {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.budget.write_stop = StopReason::SocketWouldBlock;
                }
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // A TCP flush has no queued application data; rustls and tungstenite
        // track their own pending output and drive explicit writes separately.
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Drip {
        bytes: usize,
        blocked: bool,
    }

    impl Read for Drip {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.blocked {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            buf[0] = 7;
            self.bytes += 1;
            Ok(1)
        }
    }

    impl Write for Drip {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn synthetic_budget_retains_local_eligibility() {
        let mut io = Budgeted::new(Drip { bytes: 0, blocked: false });
        let mut buf = [0; 4];
        for _ in 0..TURN_IO_ATTEMPTS {
            assert_eq!(io.read(&mut buf).unwrap(), 1);
        }
        assert_eq!(io.read(&mut buf).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(io.budget().read_stop(), StopReason::Budget);
        io.reset_turn();
        assert_eq!(io.read(&mut buf).unwrap(), 1);
    }

    #[test]
    fn real_block_and_byte_budget_have_different_reasons() {
        let mut io = Budgeted::new(Drip { bytes: 0, blocked: true });
        let mut byte = [0];
        assert_eq!(io.read(&mut byte).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(io.budget().read_stop(), StopReason::SocketWouldBlock);
        io.inner_mut().blocked = false;
        let packet = [3; TURN_BYTE_LIMIT + 1];
        assert_eq!(io.write(&packet).unwrap(), TURN_BYTE_LIMIT);
        assert_eq!(io.write(&packet).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(io.budget().write_stop(), StopReason::Budget);
        assert_eq!(io.budget().write_bytes(), TURN_BYTE_LIMIT);
    }
}
