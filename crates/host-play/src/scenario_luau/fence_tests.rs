//! T35 fence attempt shape. Failure modes: a busy first, middle or last role
//! still publishes; an attempt waits on a busy boundary; a deferred or
//! completed attempt keeps any boundary held; publication runs more than once.

use super::*;
use scenario::file_scenario::FileRole;

fn fleet(n: u64) -> Vec<Role> {
    (1..=n)
        .map(|id| {
            Role::new(
                id,
                &format!("r{id}"),
                &format!("a{id}"),
                FileRole::default(),
            )
        })
        .collect()
}

fn all_free(roles: &[Role]) -> bool {
    roles.iter().all(|role| role.try_boundary().is_some())
}

#[test]
fn ten_free_roles_publish_once_and_release_every_boundary() {
    let roles = fleet(10);
    let mut calls = 0;
    assert_eq!(
        with_boundaries(&roles, || {
            calls += 1;
            7
        }),
        Some(7)
    );
    assert_eq!(calls, 1);
    assert!(all_free(&roles));
}

#[test]
fn a_busy_first_middle_or_last_role_defers_without_waiting_or_holding() {
    for busy in [0, 4, 9] {
        let roles = fleet(10);
        let held = roles[busy].boundary();
        let started = Instant::now();
        let mut published = false;
        assert!(with_boundaries(&roles, || published = true).is_none());
        assert!(!published, "busy role {busy} must defer publication");
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a busy boundary is tried once, never waited on"
        );
        drop(held);
        assert!(all_free(&roles), "every acquired guard was released");
    }
}
