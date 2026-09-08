use super::*;
fn guard() -> Guard {
    Guard::new(Limits::new(&json!({})).unwrap(), false)
}
#[test]
fn checked_numeric_limits() {
    assert!(add(i64::MAX as u64, 1).is_err());
    assert!(mul(1 << 62, 4).is_err());
    assert!(sub(0, 1).is_err());
    assert_eq!(hex(b"ffffffffffffffff").unwrap(), u64::MAX);
    assert!(hex(b"10000000000000000").is_err());
}
#[test]
fn rehash_tombstones_and_seeded_churn() {
    let mut g = guard();
    let mut m = Map::new();
    for cycle in 0..20 {
        for k in 0..1000 {
            assert!(m.insert((k, cycle), (k, cycle), &mut g).unwrap());
        }
        assert_eq!(m.len(), 1000);
        for k in 0..1000 {
            assert_eq!(m.remove((k, cycle), &mut g).unwrap(), Some((k, cycle)));
        }
        assert_eq!(m.len(), 0);
        assert!(m.high_capacity <= 2048);
    }
}
#[test]
fn hard_collision_probe_bound() {
    let mut g = guard();
    let mut m = Map::new();
    // Deliberately fill every slot, independent of the OS-selected hash seed.
    m.slots = vec![
        Slot {
            state: 1,
            key: (1, 1),
            value: (0, 0)
        };
        8192
    ];
    assert_eq!(m.index((2, 2), &mut g), Err("map collision probe cap"));
}
#[test]
fn phase_end_must_check_before_reset() {
    for phase in 1..=3 {
        let mut g = guard();
        for n in 1..=phase {
            g.next(n).unwrap();
        }
        g.start -= 301.0;
        assert_eq!(g.next(phase + 1), Err("wall guard"));
        assert_eq!(g.phase, phase);
        assert_eq!(g.measurements.len(), phase as usize - 1);
    }
}
#[test]
fn inner_work_pulse_observes_expiry() {
    let mut g = guard();
    g.start -= 301.0;
    g.last -= 1.0;
    for _ in 0..1023 {
        g.pulse(0).unwrap();
    }
    assert_eq!(g.pulse(0), Err("wall guard"));
}
#[test]
fn capacity_accounts_for_transient_buffers() {
    let l = Layout::from_size_align(64, 8).unwrap();
    assert_eq!(charge_size(l), 104);
    let old = USED.load(Ordering::SeqCst);
    let p = unsafe { ALLOC_TEST.alloc(l) };
    assert!(USED.load(Ordering::SeqCst) >= old + 104);
    unsafe {
        ALLOC_TEST.dealloc(p, l);
    }
}
static ALLOC_TEST: Charged = Charged;

#[test]
fn render_and_sort_stalls_fail_closed() {
    let mut g = guard();
    g.start -= 301.0;
    g.last -= 1.0;
    assert!(crate::replay::pretty(&vec![b'x'; 65536], &mut g).is_err());
    let mut g = Guard::new(Limits::new(&json!({"wall":1})).unwrap(), false);
    let mut first = true;
    let mut items: Vec<u64> = (0..2048).rev().collect();
    let result = guarded_sort(
        &mut items,
        |a, b| {
            if first {
                first = false;
                std::thread::sleep(std::time::Duration::from_millis(1100));
            }
            a.cmp(b)
        },
        &mut g,
    );
    assert_eq!(result, Err("wall guard"));
}
