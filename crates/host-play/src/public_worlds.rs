//! Operator-owned public endpoints, loaded once at process startup.

use std::collections::HashSet;
use std::path::Path;

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
            ],
        }
    }
}

impl PublicWorlds {
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

    fn validate(&self) -> Result<(), String> {
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
static MODULI: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

/// Called on the slot thread, never in a frontend paint callback.
pub fn modulus_for(
    world: &PublicWorld,
    refresh: bool,
    fetch: impl FnOnce(&str, u16) -> Option<String>,
) -> String {
    let cache = &*MODULI;
    if !refresh {
        if let Some(value) = cache.lock().get(&world.host) {
            return value.clone();
        }
    }
    match fetch(&world.host, world.port).filter(|digits| {
        let mut nonzero = false;
        digits.len() >= 250
            && digits.bytes().all(|b| {
                nonzero |= b != b'0';
                b.is_ascii_digit()
            })
            && nonzero
    }) {
        Some(value) => {
            cache.lock().insert(world.host.clone(), value.clone());
            value
        }
        None => {
            cache.lock().remove(&world.host);
            client::PROD_LOGIN_RSAN.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_file_written_and_invalid_file_refused() {
        let dir = std::env::temp_dir().join(format!("274bot-worlds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("worlds.json");
        let _ = std::fs::remove_file(&path);
        let defaults = PublicWorlds::load(&path).unwrap();
        assert_eq!(defaults.worlds.len(), 2);
        assert_eq!(PublicWorlds::load(&path).unwrap().worlds[1].node_id, 11);
        std::fs::write(&path, r#"{"schema_version":1,"worlds":[{"number":1,"host":"bad/path","port":443,"node_id":10}]}"#).unwrap();
        assert!(PublicWorlds::load(&path)
            .unwrap_err()
            .contains(path.to_str().unwrap()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_round_rotates_and_pinned_waits() {
        let worlds = PublicWorlds::default();
        let mut auto = WorldRound::new(&worlds, None, Some(2)).unwrap();
        assert_eq!(auto.index, 1);
        assert_eq!(auto.on_login_error(7, 2), WorldErrorStep::SwitchNow);
        assert_eq!(auto.index, 0);
        assert_eq!(auto.on_login_error(7, 2), WorldErrorStep::SwitchAfterWait);
        assert_eq!(auto.index, 1);
        let mut pinned = WorldRound::new(&worlds, Some(2), None).unwrap();
        assert_eq!(pinned.on_login_error(7, 2), WorldErrorStep::Stay);
        assert_eq!(pinned.index, 1);
        assert_eq!(auto.on_login_error(7, 2), WorldErrorStep::SwitchNow);
        assert_eq!(auto.on_login_error(6, 2), WorldErrorStep::Stay);
        assert_eq!(auto.on_login_error(7, 2), WorldErrorStep::SwitchNow);
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
            modulus_for(&world, false, |_, _| Some(first.clone())),
            first
        );
        assert_eq!(
            modulus_for(&world, false, |_, _| panic!("cached key must not fetch")),
            first
        );
        let mut refreshed = false;
        assert!(!refresh_after_login_error(7, &mut refreshed));
        assert!(refresh_after_login_error(6, &mut refreshed));
        assert_eq!(
            modulus_for(&world, true, |_, _| Some(second.clone())),
            second
        );
        assert!(!refresh_after_login_error(6, &mut refreshed));
        assert_eq!(
            modulus_for(&world, false, |_, _| panic!("cached key must not fetch")),
            second
        );
        assert_eq!(
            modulus_for(&world, true, |_, _| None),
            client::PROD_LOGIN_RSAN
        );
        assert_eq!(
            modulus_for(&world, true, |_, _| Some("0".repeat(260))),
            client::PROD_LOGIN_RSAN
        );
        assert_eq!(
            modulus_for(&world, false, |_, _| Some(first.clone())),
            first,
            "a failed fetch must not pin a baked or stale key"
        );
    }
}
