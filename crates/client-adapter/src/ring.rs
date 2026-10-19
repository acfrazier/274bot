//! Fixed-size, one-producer/one-consumer game-byte ring.
//!
//! The slot exclusively owns output publication and input consumption. The
//! reactor exclusively owns output consumption and input publication. Release
//! tail/head stores transfer byte custody; their opposites use Acquire loads.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

/// The physical output allocation; at least the 5000-byte packet extent.
pub const OUT_RING_BYTES: usize = 5000;
/// The physical input allocation, enough for two maximal WS messages.
pub const IN_RING_BYTES: usize = 32 * 1024;
/// A single accepted WS binary message cannot exceed this logical size.
pub const WS_MESSAGE_LIMIT: usize = 16 * 1024;

/// A fixed ring with exactly one producer and one consumer.
///
/// Callers uphold the unique producer/consumer thread assignment for the
/// whole connection lifetime. An adopted connection changes that assignment
/// only after a checked Rebind has quiesced its former owner.
#[derive(Debug)]
pub struct Ring<const N: usize> {
    bytes: UnsafeCell<[u8; N]>,
    head: AtomicU64,
    tail: AtomicU64,
}

// SAFETY: Producer and consumer operate on disjoint live/free spans. Tail
// Release publishes initialized bytes before consumer Acquire observes them;
// head Release publishes freed bytes before producer Acquire reuses them.
// A connection has exactly one owner of each role; see the cross-thread
// byte-conservation and wrap-boundary tests below.
unsafe impl<const N: usize> Sync for Ring<N> {}

impl<const N: usize> Default for Ring<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Ring<N> {
    /// Allocate a ring; zero-sized rings are not valid transport storage.
    pub fn new() -> Self {
        assert!(N > 0 && N < u32::MAX as usize);
        Self {
            bytes: UnsafeCell::new([0; N]),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }

    /// Observe committed unread bytes from either owner.
    pub fn len(&self) -> usize {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            let len = tail.wrapping_sub(head);
            if len <= N as u64 {
                return len as usize;
            }
            // A concurrent producer/consumer can move between snapshots;
            // never report a negative or over-capacity occupancy.
            std::hint::spin_loop();
        }
    }

    /// Observe whether there are no committed unread bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Observe producer capacity after any consumer Release of the head.
    pub fn free(&self) -> usize {
        N - self.len()
    }

    /// Publish a complete semantic record. No prefix is published on failure.
    /// Only the single producer thread calls this method.
    pub fn push_all(&self, source: &[u8]) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        if source.len() > N - tail.wrapping_sub(head) as usize {
            return false;
        }
        self.push_unchecked(source, tail);
        true
    }

    /// Publish as many bytes as fit; TCP may deliver a partial socket read.
    /// Only the single producer thread calls this method.
    pub fn push_some(&self, source: &[u8]) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        let len = source.len().min(N - tail.wrapping_sub(head) as usize);
        self.push_unchecked(&source[..len], tail);
        len
    }

    fn push_unchecked(&self, source: &[u8], tail: u64) {
        let start = tail as usize % N;
        let first = source.len().min(N - start);
        // SAFETY: The producer alone writes these free spans, bounded by its
        // Acquire-observed head. Raw slice pointers avoid creating a mutable
        // reference to the *whole* array while the consumer borrows a
        // disjoint published span. Release tail publishes both copies.
        unsafe {
            let ptr = std::ptr::addr_of_mut!((*self.bytes.get())[0]);
            std::ptr::copy_nonoverlapping(source.as_ptr(), ptr.add(start), first);
            std::ptr::copy_nonoverlapping(source.as_ptr().add(first), ptr, source.len() - first);
        }
        self.tail
            .store(tail.wrapping_add(source.len() as u64), Ordering::Release);
    }

    /// Consume up to `dest.len()` published bytes. Only the consumer calls it.
    pub fn pop(&self, dest: &mut [u8]) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);
        let len = dest.len().min(tail.wrapping_sub(head) as usize);
        let start = head as usize % N;
        let first = len.min(N - start);
        // SAFETY: The consumer alone reads these published spans after an
        // Acquire tail, and the producer cannot reuse until Release head.
        // Raw pointers avoid a shared reference to the entire array while
        // the producer mutates its disjoint free span.
        unsafe {
            let ptr = std::ptr::addr_of!((*self.bytes.get())[0]);
            std::ptr::copy_nonoverlapping(ptr.add(start), dest.as_mut_ptr(), first);
            std::ptr::copy_nonoverlapping(ptr, dest.as_mut_ptr().add(first), len - first);
        }
        self.head
            .store(head.wrapping_add(len as u64), Ordering::Release);
        len
    }

    /// Call `send` on the first contiguous published span. It must return
    /// `Ok(n)` with `n <= span.len()` or an error, never retain the borrowed
    /// span. Only that `Ok(n)` commits consumption. No copy is made for TCP.
    pub fn consume_with(
        &self,
        send: impl FnOnce(&[u8]) -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);
        let available = tail.wrapping_sub(head) as usize;
        if available == 0 {
            return Ok(0);
        }
        let start = head as usize % N;
        let len = available.min(N - start);
        // SAFETY: Only this committed contiguous span is borrowed; the
        // producer's mutable free span is disjoint. Release tail preceded
        // the Acquire above and head is not released until send returns.
        let ptr = self.bytes.get().cast::<u8>();
        let sent = send(unsafe { std::slice::from_raw_parts(ptr.add(start), len) })?;
        assert!(sent <= len, "socket reported more bytes than borrowed");
        self.head
            .store(head.wrapping_add(sent as u64), Ordering::Release);
        Ok(sent)
    }

    /// Read directly into a free TCP ring span. A socket read may be partial;
    /// zero publishes nothing. The producer does not hold an owner lock.
    pub fn produce_with(
        &self,
        recv: impl FnOnce(&mut [u8]) -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        let free = N - tail.wrapping_sub(head) as usize;
        if free == 0 {
            return Ok(0);
        }
        let start = tail as usize % N;
        let len = free.min(N - start);
        // SAFETY: Only this free contiguous span is mutably borrowed; the
        // consumer's immutable committed span is disjoint. It stays private
        // until Release tail after recv returns.
        let ptr = self.bytes.get().cast::<u8>();
        let read = recv(unsafe { std::slice::from_raw_parts_mut(ptr.add(start), len) })?;
        assert!(read <= len, "socket reported more bytes than borrowed");
        self.tail
            .store(tail.wrapping_add(read as u64), Ordering::Release);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn exact_fit_and_refused_record_keep_cursors_and_bytes() {
        let ring = Ring::<5>::new();
        assert!(ring.push_all(&[1, 2, 3, 4]));
        assert!(!ring.push_all(&[5, 6]));
        let mut first = [0; 3];
        assert_eq!(ring.pop(&mut first), 3);
        assert_eq!(first, [1, 2, 3]);
        assert!(ring.push_all(&[5, 6, 7, 8]));
        let mut second = [0; 5];
        assert_eq!(ring.pop(&mut second), 5);
        assert_eq!(second, [4, 5, 6, 7, 8]);
    }

    #[test]
    fn socket_borrows_do_not_consume_on_would_block() {
        let ring = Ring::<7>::new();
        assert!(ring.push_all(b"hello"));
        assert_eq!(
            ring.consume_with(|_| Err(std::io::ErrorKind::WouldBlock.into()))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(ring.len(), 5);
        assert_eq!(
            ring.consume_with(|span| {
                assert_eq!(span, b"hello");
                Ok(3)
            })
            .unwrap(),
            3
        );
        assert_eq!(
            ring.produce_with(|span| {
                span[..2].copy_from_slice(b"xy");
                Ok(2)
            })
            .unwrap(),
            2
        );
        let mut out = [0; 4];
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(&out, b"loxy");
    }

    #[test]
    fn producer_consumer_transfer_byte_exact_across_many_wraps() {
        let ring = Arc::new(Ring::<13>::new());
        let producer = Arc::clone(&ring);
        let writer = std::thread::spawn(move || {
            for index in 0..100_000usize {
                let byte = [(index % 251) as u8];
                while !producer.push_all(&byte) {
                    std::thread::yield_now();
                }
            }
        });
        let mut byte = [0];
        for index in 0..100_000usize {
            while ring.pop(&mut byte) == 0 {
                std::thread::yield_now();
            }
            assert_eq!(byte[0], (index % 251) as u8);
        }
        writer.join().unwrap();
        assert_eq!(ring.len(), 0);
    }
}
