//! Operator-owned public endpoints, loaded once at process startup.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, LazyLock, Weak};

use client::io::client_stream::LoginCancel;
use parking_lot::{Condvar, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicWorld {
    pub number: u16,
    pub host: String,
    pub port: u16,
    pub node_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicWorlds {
    pub schema_version: u32,
    pub worlds: Vec<PublicWorld>,
}

impl Default for PublicWorlds {
    fn default() -> Self {
        Self {
            schema_version: 1,
            worlds: vec![
                PublicWorld {
                    number: 1,
                    host: "w1.rs2b2t.com".into(),
                    port: 443,
                    node_id: 10,
                },
                PublicWorld {
                    number: 2,
                    host: "w2.rs2b2t.com".into(),
                    port: 443,
                    node_id: 11,
                },
                PublicWorld {
                    number: 3,
                    host: "w3.rs2b2t.com".into(),
                    port: 443,
                    node_id: 12,
                },
            ],
        }
    }
}

impl PublicWorlds {
    /// Membership is process-wide, including automatic full-world fallback.
    /// Never infer it from just the selected endpoint of a mixed roster.
    pub(crate) fn is_rs2b2t_members_roster(&self) -> bool {
        !self.worlds.is_empty()
            && self.worlds.iter().all(|world| {
                world.port == 443
                    && world.host.split_once('.').is_some_and(|(label, domain)| {
                        domain.eq_ignore_ascii_case("rs2b2t.com")
                            && label
                                .as_bytes()
                                .split_first()
                                .is_some_and(|(prefix, number)| {
                                    prefix.eq_ignore_ascii_case(&b'w')
                                        && !number.is_empty()
                                        && number.iter().all(u8::is_ascii_digit)
                                })
                    })
            })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                let json = serde_json::to_vec_pretty(&defaults).expect("serializable worlds");
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("worlds {}: {e}", path.display()))?;
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        file.write_all(&json)
                            .map_err(|e| format!("worlds {}: {e}", path.display()))?;
                        return Ok(defaults);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::read(path)
                        .map_err(|e| format!("worlds {}: {e}", path.display()))?,
                    Err(e) => return Err(format!("worlds {}: {e}", path.display())),
                }
            }
            Err(e) => return Err(format!("worlds {}: {e}", path.display())),
        };
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("worlds {}: {e}", path.display()))?;
        config
            .validate()
            .map_err(|e| format!("worlds {}: {e}", path.display()))?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.worlds.is_empty() {
            return Err("expected schema_version 1 and a nonempty worlds list".into());
        }
        let mut numbers = HashSet::new();
        let mut hosts = HashSet::new();
        for world in &self.worlds {
            if world.number == 0
                || world.port == 0
                || world.node_id <= 0
                || world.host.is_empty()
                || !world
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                || world.host.starts_with(['.', '-'])
                || world.host.ends_with(['.', '-'])
            {
                return Err(format!("invalid world {} endpoint/node id", world.number));
            }
            if !numbers.insert(world.number) || !hosts.insert(world.host.as_str()) {
                return Err(format!(
                    "duplicate world number or host at world {}",
                    world.number
                ));
            }
        }
        Ok(())
    }

    pub fn by_number(&self, number: u16) -> Option<&PublicWorld> {
        self.worlds.iter().find(|world| world.number == number)
    }

    pub fn by_endpoint(&self, host: &str, port: u16) -> Option<&PublicWorld> {
        self.worlds
            .iter()
            .find(|world| world.host == host && world.port == port)
    }
}

/// Per-slot full-world round: rotate on 7, pause only after visiting every world.
#[derive(Debug)]
pub struct WorldRound {
    pub index: usize,
    full_in_round: usize,
    choice: Option<u16>,
    auto_default: Option<u16>,
}

/// A slot's next action after a login error. Callers can supply a pinned
/// world to `WorldRound::new` without changing this fallback policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldErrorStep {
    Stay,
    SwitchNow,
    SwitchAfterWait,
}

impl WorldRound {
    pub fn new(
        worlds: &PublicWorlds,
        choice: Option<u16>,
        auto_default: Option<u16>,
    ) -> Result<Self, String> {
        let number = choice.or(auto_default);

        let index = number.map_or(Ok(0), |n| {
            worlds
                .worlds
                .iter()
                .position(|w| w.number == n)
                .ok_or_else(|| format!("world {n} is not in the configured public worlds"))
        })?;
        Ok(Self {
            index,
            full_in_round: 0,
            choice,
            auto_default,
        })
    }

    pub fn preference(&self) -> Option<u16> {
        self.choice
    }

    /// Rebuild this round when the operator's account preference changed.
    /// Auto rotation does not count as a preference change: `choice` stays
    /// `None` while `index` advances through full worlds.
    pub fn reselect_if_changed(
        &mut self,
        worlds: &PublicWorlds,
        choice: Option<u16>,
    ) -> Result<bool, String> {
        if self.choice == choice {
            return Ok(false);
        }
        *self = Self::new(worlds, choice, self.auto_default)?;
        Ok(true)
    }

    /// Only response 7 rotates an auto account. A non-full error starts a
    /// fresh round; a pinned choice never changes worlds.
    pub fn on_login_error(&mut self, code: i32, world_count: usize) -> WorldErrorStep {
        if code != 7 {
            self.full_in_round = 0;
            return WorldErrorStep::Stay;
        }
        if self.choice.is_some() || world_count == 1 {
            return WorldErrorStep::Stay;
        }
        self.full_in_round += 1;
        self.index = (self.index + 1) % world_count;
        if self.full_in_round == world_count {
            self.full_in_round = 0;
            WorldErrorStep::SwitchAfterWait
        } else {
            WorldErrorStep::SwitchNow
        }
    }

    pub fn reset(&mut self) {
        self.full_in_round = 0;
    }
}

/// A wrong-key response triggers at most one forced refetch until the next
/// successful login or world switch.
pub fn refresh_after_login_error(code: i32, refreshed: &mut bool) -> bool {
    if code == 6 && !*refreshed {
        *refreshed = true;
        true
    } else {
        false
    }
}

#[derive(Default)]
struct ModulusRecord {
    cached: Option<Arc<str>>,
    fetching: Option<Arc<ModulusFetch>>,
}

struct ModulusFetch {
    result: Mutex<Option<Option<Arc<str>>>>,
    changed: Condvar,
}

impl ModulusFetch {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    fn publish(&self, value: Option<Arc<str>>) {
        *self.result.lock() = Some(value);
        self.changed.notify_all();
    }

    fn wait(&self, cancel: &LoginCancel, self_ref: &Arc<Self>) -> Option<Arc<str>> {
        let weak: Weak<Self> = Arc::downgrade(self_ref);
        cancel.register_cancel_callback(move || {
            if let Some(fetch) = weak.upgrade() {
                // Synchronize with the predicate-to-sleep boundary, so Stop
                // cannot notify before the waiter atomically unlocks.
                let guard = fetch.result.lock();
                drop(guard);
                fetch.changed.notify_all();
            }
        });
        let mut result = self.result.lock();
        while result.is_none() && !cancel.is_cancelled() {
            self.changed.wait(&mut result);
        }
        if cancel.is_cancelled() {
            None
        } else {
            result.as_ref().cloned().flatten()
        }
    }
}

static MODULI: LazyLock<Mutex<HashMap<String, ModulusRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct FetchPublication<'a> {
    host: &'a str,
    cell: Arc<ModulusFetch>,
    published: bool,
}

impl FetchPublication<'_> {
    fn finish(&mut self, value: Option<Arc<str>>) {
        self.cell.publish(value.clone());
        let mut cache = MODULI.lock();
        if let Some(record) = cache.get_mut(self.host) {
            if record
                .fetching
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.cell))
            {
                record.cached = value;
                record.fetching = None;
            }
        }
        self.published = true;
    }
}

impl Drop for FetchPublication<'_> {
    fn drop(&mut self) {
        if !self.published {
            self.finish(None);
        }
    }
}

enum ModulusDemand {
    Cached(Arc<str>),
    Join(Arc<ModulusFetch>),
    Perform(Arc<ModulusFetch>),
}

/// Share one active key fetch per exact host, including a forced refresh.
/// The first slot runs the fetch under its original transport phase limits.
/// Waiters observe that same result or their own cancellation; joining never
/// renews the performer's queue, connect, handshake or body-read clocks.
pub fn modulus_for(
    world: &PublicWorld,
    refresh: bool,
    cancel: &LoginCancel,
    fetch: impl FnOnce(&str, u16) -> Option<String>,
) -> String {
    let demand = {
        let mut cache = MODULI.lock();
        if let Some(record) = cache.get_mut(world.host.as_str()) {
            if let Some(active) = &record.fetching {
                ModulusDemand::Join(Arc::clone(active))
            } else if !refresh {
                if let Some(value) = &record.cached {
                    ModulusDemand::Cached(Arc::clone(value))
                } else {
                    let cell = Arc::new(ModulusFetch::new());
                    record.fetching = Some(Arc::clone(&cell));
                    ModulusDemand::Perform(cell)
                }
            } else {
                let cell = Arc::new(ModulusFetch::new());
                record.fetching = Some(Arc::clone(&cell));
                ModulusDemand::Perform(cell)
            }
        } else {
            let cell = Arc::new(ModulusFetch::new());
            cache.insert(
                world.host.clone(),
                ModulusRecord {
                    cached: None,
                    fetching: Some(Arc::clone(&cell)),
                },
            );
            ModulusDemand::Perform(cell)
        }
    };
    let result = match demand {
        ModulusDemand::Cached(value) => Some(value),
        ModulusDemand::Join(cell) => cell.wait(cancel, &cell),
        ModulusDemand::Perform(cell) => {
            let mut publication = FetchPublication {
                host: &world.host,
                cell,
                published: false,
            };
            let value = fetch(&world.host, world.port)
                .filter(|digits| {
                    let mut nonzero = false;
                    digits.len() >= 250
                        && digits.bytes().all(|byte| {
                            nonzero |= byte != b'0';
                            byte.is_ascii_digit()
                        })
                        && nonzero
                })
                .map(Arc::<str>::from);
            publication.finish(value.clone());
            value
        }
    };
    result.map_or_else(|| client::PROD_LOGIN_RSAN.into(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn modulus_for_test(
        world: &PublicWorld,
        refresh: bool,
        fetch: impl FnOnce(&str, u16) -> Option<String>,
    ) -> String {
        modulus_for(world, refresh, &LoginCancel::new(), fetch)
    }

    #[test]
    fn absent_file_written_and_invalid_file_refused() {
        let dir = std::env::temp_dir().join(format!("274bot-worlds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("worlds.json");
        let _ = std::fs::remove_file(&path);
        let defaults = PublicWorlds::load(&path).unwrap();
        for (number, host, node_id) in [
            (1, "w1.rs2b2t.com", 10),
            (2, "w2.rs2b2t.com", 11),
            (3, "w3.rs2b2t.com", 12),
        ] {
            let world = defaults.by_number(number).unwrap();
            assert_eq!(
                (world.host.as_str(), world.port, world.node_id),
                (host, 443, node_id)
            );
        }
        let loaded = PublicWorlds::load(&path).unwrap();
        assert_eq!(loaded.by_number(3), defaults.by_number(3));
        std::fs::write(
            &path,
            r#"{"schema_version":1,"worlds":[{"number":1,"host":"bad/path","port":443,"node_id":10}]}"#,
        )
        .unwrap();
        assert!(PublicWorlds::load(&path)
            .unwrap_err()
            .contains(path.to_str().unwrap()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_round_rotates_and_pinned_waits() {
        let worlds = PublicWorlds::default();
        let mut auto = WorldRound::new(&worlds, None, None).unwrap();
        let mut visited = vec![worlds.worlds[auto.index].number];
        for (number, step) in [
            (2, WorldErrorStep::SwitchNow),
            (3, WorldErrorStep::SwitchNow),
            (1, WorldErrorStep::SwitchAfterWait),
        ] {
            assert_eq!(auto.on_login_error(7, worlds.worlds.len()), step);
            let current = worlds.worlds[auto.index].number;
            assert_eq!(current, number);
            visited.push(current);
        }
        assert_eq!(visited, vec![1, 2, 3, 1]);

        let mut pinned = WorldRound::new(&worlds, Some(2), None).unwrap();
        assert_eq!(
            pinned.on_login_error(7, worlds.worlds.len()),
            WorldErrorStep::Stay
        );
        assert_eq!(worlds.worlds[pinned.index].number, 2);
        assert_eq!(
            auto.on_login_error(7, worlds.worlds.len()),
            WorldErrorStep::SwitchNow
        );
        assert_eq!(worlds.worlds[auto.index].number, 2);
        assert_eq!(
            auto.on_login_error(6, worlds.worlds.len()),
            WorldErrorStep::Stay
        );
        assert_eq!(
            auto.on_login_error(7, worlds.worlds.len()),
            WorldErrorStep::SwitchNow
        );
        assert_eq!(worlds.worlds[auto.index].number, 3);
    }
    #[test]
    fn wrong_key_refetches_once_and_failure_uses_baked_key() {
        let world = PublicWorld {
            number: 13,
            host: "stub-key-world.invalid".into(),
            port: 443,
            node_id: 22,
        };
        let first = "1".repeat(260);
        let second = "2".repeat(260);
        assert_eq!(
            modulus_for_test(&world, false, |_, _| Some(first.clone())),
            first
        );
        assert_eq!(
            modulus_for_test(&world, false, |_, _| panic!("cached key must not fetch")),
            first
        );
        let mut refreshed = false;
        assert!(!refresh_after_login_error(7, &mut refreshed));
        assert!(refresh_after_login_error(6, &mut refreshed));
        assert_eq!(
            modulus_for_test(&world, true, |_, _| Some(second.clone())),
            second
        );
        assert!(!refresh_after_login_error(6, &mut refreshed));
        assert_eq!(
            modulus_for_test(&world, false, |_, _| panic!("cached key must not fetch")),
            second
        );
        assert_eq!(
            modulus_for_test(&world, true, |_, _| None),
            client::PROD_LOGIN_RSAN
        );
        assert_eq!(
            modulus_for_test(&world, true, |_, _| Some("0".repeat(260))),
            client::PROD_LOGIN_RSAN
        );
        assert_eq!(
            modulus_for_test(&world, false, |_, _| Some(first.clone())),
            first,
            "a failed fetch must not pin a baked or stale key"
        );
    }

    fn wait_for_joiners(world: &PublicWorld, minimum_refs: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let refs = MODULI
                .lock()
                .get(&world.host)
                .and_then(|record| record.fetching.as_ref())
                .map_or(0, Arc::strong_count);
            if refs >= minimum_refs {
                return;
            }
            assert!(Instant::now() < until, "key fetch joiners did not register");
            std::thread::yield_now();
        }
    }

    #[test]
    fn forced_refresh_and_twenty_waiters_share_the_performers_fetch() {
        let world = Arc::new(PublicWorld {
            number: 93,
            host: "single-flight-w93.invalid".into(),
            port: 443,
            node_id: 31,
        });
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner_world = Arc::clone(&world);
        let performer = std::thread::spawn(move || {
            modulus_for(&owner_world, false, &LoginCancel::new(), |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Some("4".repeat(260))
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut waiters = Vec::new();
        for _ in 0..20 {
            let world = Arc::clone(&world);
            waiters.push(std::thread::spawn(move || {
                modulus_for(&world, true, &LoginCancel::new(), |_, _| {
                    panic!("forced waiter must not create a second fetch")
                })
            }));
        }
        wait_for_joiners(&world, 22);
        release_tx.send(()).unwrap();
        assert_eq!(performer.join().unwrap(), "4".repeat(260));
        for waiter in waiters {
            assert_eq!(waiter.join().unwrap(), "4".repeat(260));
        }
        assert_eq!(
            modulus_for_test(&world, false, |_, _| panic!(
                "successful key remains cached"
            )),
            "4".repeat(260),
        );
    }

    #[test]
    fn stopped_waiter_cannot_take_over_a_failed_performers_result_cell() {
        let world = Arc::new(PublicWorld {
            number: 94,
            host: "single-flight-w94.invalid".into(),
            port: 443,
            node_id: 32,
        });
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner_world = Arc::clone(&world);
        let performer = std::thread::spawn(move || {
            modulus_for(&owner_world, false, &LoginCancel::new(), |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                None
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let cancelled = LoginCancel::new();
        let wait_cancel = cancelled.clone();
        let wait_world = Arc::clone(&world);
        let waiter = std::thread::spawn(move || {
            modulus_for(&wait_world, true, &wait_cancel, |_, _| {
                panic!("cancelled waiter must not perform a second fetch")
            })
        });
        wait_for_joiners(&world, 3);
        cancelled.cancel();
        assert_eq!(waiter.join().unwrap(), client::PROD_LOGIN_RSAN);
        release_tx.send(()).unwrap();
        assert_eq!(performer.join().unwrap(), client::PROD_LOGIN_RSAN);
        assert_eq!(
            modulus_for_test(&world, false, |_, _| Some("6".repeat(260))),
            "6".repeat(260),
            "a subsequent slot may retry only after the performer publishes failure",
        );
    }

    #[test]
    fn stopping_performer_does_not_transfer_or_cancel_shared_fetch() {
        let world = Arc::new(PublicWorld {
            number: 95,
            host: "single-flight-w95.invalid".into(),
            port: 443,
            node_id: 33,
        });
        let owner_cancel = LoginCancel::new();
        let owner_wait = owner_cancel.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner_world = Arc::clone(&world);
        let performer = std::thread::spawn(move || {
            modulus_for(&owner_world, false, &owner_wait, |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Some("7".repeat(260))
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let waiting_world = Arc::clone(&world);
        let waiter = std::thread::spawn(move || {
            modulus_for(&waiting_world, true, &LoginCancel::new(), |_, _| {
                panic!("stopping the performer must not transfer its fetch")
            })
        });
        wait_for_joiners(&world, 3);
        owner_cancel.cancel();
        release_tx.send(()).unwrap();
        assert_eq!(performer.join().unwrap(), "7".repeat(260));
        assert_eq!(waiter.join().unwrap(), "7".repeat(260));
    }
}
