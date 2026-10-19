//! Bounded SPSC boundaries for game-output records.
//!
//! The slot publishes complete bytes to the output ring before publishing a
//! length here. The reactor observes this queue first, then consumes exactly
//! that many bytes. Partial native writes retain their remaining record size.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

pub const RECORD_SLOTS: usize = 64;

#[derive(Debug)]
pub struct RecordQueue {
    lengths: [UnsafeCell<u16>; RECORD_SLOTS],
    head: AtomicU64,
    tail: AtomicU64,
}

// SAFETY: Each cell is written only by the slot after it is free and read
// only by the reactor after a Release tail is Acquire-observed. The reactor
// releases head only after consuming its record; wrap reuse is ordered.
unsafe impl Sync for RecordQueue {}

impl Default for RecordQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordQueue {
    pub fn new() -> Self {
        Self {
            lengths: std::array::from_fn(|_| UnsafeCell::new(0)),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }

    /// Reservation must precede output-byte publication. As only the reactor
    /// consumes records, capacity cannot decrease between this check and push.
    pub fn has_capacity(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        tail.wrapping_sub(head) < RECORD_SLOTS as u64
    }

    /// Producer-only publication after the complete record's ring bytes.
    pub fn push(&self, len: u16) {
        assert!(len > 0 && self.has_capacity());
        let tail = self.tail.load(Ordering::Relaxed);
        // SAFETY: one producer owns the free cell. The record is not
        // visible until the Release store below.
        unsafe { *self.lengths[tail as usize % RECORD_SLOTS].get() = len };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
    }

    /// Consumer-only observation, retained across partial socket writes.
    pub fn front(&self) -> Option<u16> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);
        if head == tail {
            return None;
        }
        // SAFETY: the producer published this cell before the Acquire tail;
        // it cannot reuse the cell until the consumer releases head.
        Some(unsafe { *self.lengths[head as usize % RECORD_SLOTS].get() })
    }

    /// Consumer-only completed-record release after all bytes were sent.
    pub fn pop_front(&self) {
        let head = self.head.load(Ordering::Relaxed);
        assert_ne!(head, self.tail.load(Ordering::Acquire));
        self.head.store(head.wrapping_add(1), Ordering::Release);
    }

    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        tail.wrapping_sub(head) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn full_queue_does_not_overwrite_and_wrap_keeps_order() {
        let records = RecordQueue::new();
        for len in 1..=RECORD_SLOTS {
            assert!(records.has_capacity());
            records.push(len as u16);
        }
        assert!(!records.has_capacity());
        for len in 1..=RECORD_SLOTS {
            assert_eq!(records.front(), Some(len as u16));
            records.pop_front();
            records.push((len + RECORD_SLOTS) as u16);
        }
        for len in RECORD_SLOTS + 1..=RECORD_SLOTS * 2 {
            assert_eq!(records.front(), Some(len as u16));
            records.pop_front();
        }
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn threaded_publication_never_exposes_uninitialized_length() {
        let records = Arc::new(RecordQueue::new());
        let producer = Arc::clone(&records);
        let writer = std::thread::spawn(move || {
            for index in 0..32_768u16 {
                while !producer.has_capacity() {
                    std::thread::yield_now();
                }
                producer.push(index % 4096 + 1);
            }
        });
        for index in 0..32_768u16 {
            loop {
                if let Some(len) = records.front() {
                    assert_eq!(len, index % 4096 + 1);
                    records.pop_front();
                    break;
                }
                std::thread::yield_now();
            }
        }
        writer.join().unwrap();
    }
}
