//! Dense active-Path bank memo. It is refreshed only by an observed open-bank
//! receipt and never treats an unopened/empty observation as known.
use crate::native_bank::BankReceipt;

pub const MAX_BANK_MEMO: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
struct Entry {
    id: i32,
    count: i32,
}

pub struct BankMemo {
    rows: [Entry; MAX_BANK_MEMO],
    len: u8,
    known: bool,
}

impl Default for BankMemo {
    fn default() -> Self {
        Self {
            rows: [Entry::default(); MAX_BANK_MEMO],
            len: 0,
            known: false,
        }
    }
}

impl BankMemo {
    pub fn update(&mut self, receipt: &BankReceipt) {
        if !receipt.complete || receipt.counts.len() > MAX_BANK_MEMO {
            self.clear();
            return;
        }
        self.len = receipt.counts.len() as u8;
        for (slot, row) in self
            .rows
            .iter_mut()
            .zip(receipt.counts.iter())
            .take(usize::from(self.len))
        {
            *slot = Entry {
                id: row.id,
                count: row.count.max(0),
            };
        }
        self.known = true;
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.known = false;
    }

    pub fn known(&self) -> bool {
        self.known
    }

    pub fn count(&self, id: i32) -> Option<i32> {
        self.known.then(|| {
            self.rows[..usize::from(self.len)]
                .iter()
                .find(|row| row.id == id)
                .map_or(0, |row| row.count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bank::BankCount;

    #[test]
    fn unopened_is_unknown_and_observed_zero_is_known() {
        let mut memo = BankMemo::default();
        assert_eq!(memo.count(42), None);
        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 42, count: 0 }],
            complete: true,
        });
        assert_eq!(memo.count(42), Some(0));
        assert!(memo.known());
    }

    #[test]
    fn incomplete_scan_does_not_publish_partial_counts() {
        let mut memo = BankMemo::default();
        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 42, count: 9 }],
            complete: false,
        });
        assert!(!memo.known());
        assert_eq!(memo.count(42), None);

        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 42, count: 9 }],
            complete: true,
        });
        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 42, count: 10 }],
            complete: false,
        });
        assert!(!memo.known());
        assert_eq!(memo.count(42), None);
    }

    #[test]
    fn observed_rows_are_replaced_and_clear_invalidates_them() {
        let mut memo = BankMemo::default();
        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 42, count: 9 }],
            complete: true,
        });
        assert_eq!(memo.count(42), Some(9));
        memo.update(&BankReceipt {
            counts: vec![BankCount { id: 7, count: 2 }],
            complete: true,
        });
        assert_eq!(memo.count(42), Some(0));
        assert_eq!(memo.count(7), Some(2));
        memo.clear();
        assert_eq!(memo.count(7), None);
    }
}
