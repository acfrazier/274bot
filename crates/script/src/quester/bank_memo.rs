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
        self.len = receipt.counts.len().min(MAX_BANK_MEMO) as u8;
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
}
