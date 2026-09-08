use crate::core::{Guard, Result};
use std::cmp::Ordering;

/// Keep sort comparisons inside the same bounded inner-loop guard cadence.
/// Once expired, comparisons become equal and the completed sort is discarded.
pub fn guarded_sort<T>(
    items: &mut [T],
    mut compare: impl FnMut(&T, &T) -> Ordering,
    guard: &mut Guard,
) -> Result<()> {
    guard.check()?;
    let mut error = None;
    items.sort_unstable_by(|a, b| {
        if error.is_none() {
            error = guard.pulse(0).err();
        }
        if error.is_some() {
            Ordering::Equal
        } else {
            compare(a, b)
        }
    });
    if let Some(reason) = error {
        return Err(reason);
    }
    guard.check()
}
