use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn shared_nav_and_walk_mutexes_panic_on_opposite_order() {
    let navs = crate::SharedMutex::new(1u8);
    let arm = crate::SharedMutex::new(2u8);
    {
        let _navs = navs.lock().expect("navs");
        let _arm = arm.lock().expect("walk arm");
    }
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _arm = arm.lock().expect("walk arm");
        let _navs = navs.lock().expect("inverted navs");
    }));
    let payload = panicked.expect_err("opposite shared-mutex order must panic");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_default();
    assert!(
        message.contains("lock order inversion"),
        "panic text: {message}"
    );
}

#[test]
fn shared_mutex_same_order_does_not_panic() {
    let navs = crate::SharedMutex::new(1u8);
    let arm = crate::SharedMutex::new(2u8);
    for _ in 0..2 {
        let _navs = navs.lock().expect("navs");
        let _arm = arm.lock().expect("walk arm");
    }
}
