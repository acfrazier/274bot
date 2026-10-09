//! Persistent launch profiles (`~/.274bot/servers.json`).

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vault::valid_component;

use crate::public_worlds::{PublicWorld, PublicWorlds};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchTransport {
    Tcp,
    Wss,
}

impl From<LaunchTransport> for client::Transport {
    fn from(value: LaunchTransport) -> Self {
        match value {
            LaunchTransport::Tcp => Self::Tcp,
            LaunchTransport::Wss => Self::Wss,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchWorld {
    pub number: u16,
    pub host: String,
    pub port: u16,
    pub node_id: i32,
    pub asset_host: String,
    pub asset_port: u16,
}

impl LaunchWorld {
    pub fn public_world(&self) -> PublicWorld {
        PublicWorld {
            number: self.number,
            host: self.host.clone(),
            port: self.port,
            node_id: self.node_id,
        }
    }
}

impl From<PublicWorld> for LaunchWorld {
    fn from(world: PublicWorld) -> Self {
        Self {
            number: world.number,
            asset_host: world.host.clone(),
            asset_port: world.port,
            host: world.host,
            port: world.port,
            node_id: world.node_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoginKey {
    Named(String),
    Inline {
        modulus: String,
        exponent: String,
    },
    EngineDir {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        engine_dir: Option<PathBuf>,
    },
}

impl LoginKey {
    pub fn is_served(&self) -> bool {
        matches!(self, Self::Named(name) if name == "served")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProfile {
    pub name: String,
    pub revision: u16,
    pub transport: LaunchTransport,
    pub worlds: Vec<LaunchWorld>,
    pub login_key: LoginKey,
    pub vault: String,
    pub members: Option<bool>,
    #[serde(default)]
    pub allow_plaintext_offhost: bool,
}

impl LaunchProfile {
    pub fn client_transport(&self) -> client::Transport {
        self.transport.into()
    }

    pub fn public_worlds(&self) -> PublicWorlds {
        PublicWorlds {
            schema_version: 1,
            worlds: self.worlds.iter().map(LaunchWorld::public_world).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Servers {
    pub schema_version: u32,
    pub servers: Vec<LaunchProfile>,
}

impl Servers {
    pub fn builtins() -> Self {
        let rs2b2t = PublicWorlds::default()
            .worlds
            .into_iter()
            .map(LaunchWorld::from)
            .collect();
        let local = |number, revision, game_port, asset_port, vault: &str| LaunchProfile {
            name: format!("local-{revision}"),
            revision,
            transport: LaunchTransport::Tcp,
            worlds: vec![LaunchWorld {
                number,
                host: "127.0.0.1".into(),
                port: game_port,
                node_id: number as i32,
                asset_host: "127.0.0.1".into(),
                asset_port,
            }],
            login_key: LoginKey::EngineDir { engine_dir: None },
            vault: vault.into(),
            members: None,
            allow_plaintext_offhost: false,
        };
        Self {
            schema_version: 1,
            servers: vec![
                LaunchProfile {
                    name: "rs2b2t".into(),
                    revision: 289,
                    transport: LaunchTransport::Wss,
                    worlds: rs2b2t,
                    login_key: LoginKey::Named("served".into()),
                    vault: "vault-prod".into(),
                    members: None,
                    allow_plaintext_offhost: false,
                },
                local(289, 289, 44594, 1080, "vault-289"),
                local(274, 274, 43594, 80, "vault"),
            ],
        }
    }

    pub fn load(bot_dir: &Path) -> Result<Self, String> {
        Self::load_with_before_publish(bot_dir, || {})
    }

    fn load_with_before_publish(
        bot_dir: &Path,
        before_publish: impl FnOnce(),
    ) -> Result<Self, String> {
        let path = bot_dir.join("servers.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut servers = Self::builtins();
                let worlds_path = bot_dir.join("worlds.json");
                if worlds_path.is_file() {
                    let worlds_bytes = std::fs::read(&worlds_path)
                        .map_err(|error| format!("worlds {}: {error}", worlds_path.display()))?;
                    let worlds: PublicWorlds = serde_json::from_slice(&worlds_bytes)
                        .map_err(|error| format!("worlds {}: {error}", worlds_path.display()))?;
                    worlds
                        .validate()
                        .map_err(|error| format!("worlds {}: {error}", worlds_path.display()))?;
                    let profile = servers
                        .servers
                        .iter_mut()
                        .find(|profile| profile.name == "rs2b2t")
                        .expect("rs2b2t builtin");
                    profile.worlds = worlds.worlds.into_iter().map(LaunchWorld::from).collect();
                    eprintln!(
                        "host-play: imported {} into {}; servers.json is now authoritative",
                        worlds_path.display(),
                        path.display()
                    );
                }
                servers.validate()?;
                std::fs::create_dir_all(bot_dir)
                    .map_err(|error| format!("servers {}: {error}", path.display()))?;
                let json = serde_json::to_vec_pretty(&servers).expect("serializable servers");
                match create_new_server_file(&path, &json, before_publish) {
                    Ok(true) => return Ok(servers),
                    Ok(false) => std::fs::read(&path)
                        .map_err(|error| format!("servers {}: {error}", path.display()))?,
                    Err(error) => return Err(format!("servers {}: {error}", path.display())),
                }
            }
            Err(error) => return Err(format!("servers {}: {error}", path.display())),
        };
        let mut servers: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("servers {}: {error}", path.display()))?;
        servers
            .validate()
            .map_err(|error| format!("servers {}: {error}", path.display()))?;

        let needs_upgrade = if let Some(profile) = servers
            .servers
            .iter_mut()
            .find(|profile| profile.name.eq_ignore_ascii_case("rs2b2t"))
        {
            if is_previous_rs2b2t_roster(&profile.worlds) {
                let world_three = PublicWorlds::default()
                    .worlds
                    .into_iter()
                    .find(|world| world.number == 3)
                    .expect("World 3 builtin");
                profile.worlds.push(world_three.into());
                true
            } else {
                false
            }
        } else {
            false
        };
        if needs_upgrade {
            servers
                .validate()
                .map_err(|error| format!("servers {}: {error}", path.display()))?;
            let backup_path = bot_dir.join("servers.json.pre-0.2.0.1");
            let backup_created = create_new_server_file(&backup_path, &bytes, || {})
                .map_err(|error| format!("servers {}: {error}", backup_path.display()))?;
            if !backup_created {
                let backup = std::fs::read(&backup_path)
                    .map_err(|error| format!("servers {}: {error}", backup_path.display()))?;
                if backup != bytes {
                    return Err(format!(
                        "servers {}: existing upgrade backup differs from original",
                        backup_path.display()
                    ));
                }
            }
            let json = serde_json::to_vec_pretty(&servers).expect("serializable servers");
            replace_server_file_atomically(&path, &json)
                .map_err(|error| format!("servers {}: {error}", path.display()))?;
        }
        Ok(servers)
    }

    pub fn resolve(&self, name: &str) -> Result<LaunchProfile, String> {
        let canonical = if name.eq_ignore_ascii_case("public-289") {
            "rs2b2t"
        } else {
            name
        };
        self.servers
            .iter()
            .find(|profile| profile.name.eq_ignore_ascii_case(canonical))
            .cloned()
            .ok_or_else(|| format!("unknown server profile {name:?} in servers.json"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.servers.is_empty() {
            return Err("expected schema_version 1 and a nonempty servers list".into());
        }
        let mut names = HashSet::new();
        let mut vaults = HashSet::new();
        for profile in &self.servers {
            let name = profile.name.to_ascii_lowercase();
            if name == "public-289" {
                return Err("server name \"public-289\" is reserved as the rs2b2t alias".into());
            }
            if !valid_component(&profile.name) || !names.insert(name) {
                return Err(format!(
                    "invalid or duplicate server name {:?}",
                    profile.name
                ));
            }
            if !valid_component(&profile.vault)
                || !vaults.insert(profile.vault.to_ascii_lowercase())
            {
                return Err(format!(
                    "invalid or duplicate vault component {:?}",
                    profile.vault
                ));
            }
            if !matches!(profile.revision, 274 | 289) || profile.worlds.is_empty() {
                return Err(format!(
                    "profile {:?} has an invalid revision or empty roster",
                    profile.name
                ));
            }
            if profile.transport == LaunchTransport::Wss && profile.allow_plaintext_offhost {
                return Err(format!(
                    "profile {:?}: allow_plaintext_offhost is valid only for tcp",
                    profile.name
                ));
            }
            if matches!(&profile.login_key, LoginKey::Named(name) if name != "served") {
                return Err(format!(
                    "profile {:?}: login_key string must be \"served\"",
                    profile.name
                ));
            }
            let mut numbers = HashSet::new();
            for world in &profile.worlds {
                if world.number == 0
                    || world.port == 0
                    || world.asset_port == 0
                    || world.node_id <= 0
                    || !valid_host(&world.host)
                    || !valid_host(&world.asset_host)
                    || !numbers.insert(world.number)
                {
                    return Err(format!("profile {:?} has an invalid world", profile.name));
                }
            }
        }
        Ok(())
    }
}

fn is_previous_rs2b2t_roster(worlds: &[LaunchWorld]) -> bool {
    let [world_one, world_two] = worlds else {
        return false;
    };
    matches_previous_world(world_one, 1, "w1.rs2b2t.com", 10)
        && matches_previous_world(world_two, 2, "w2.rs2b2t.com", 11)
}

fn matches_previous_world(world: &LaunchWorld, number: u16, host: &str, node_id: i32) -> bool {
    world.number == number
        && world.host == host
        && world.port == 443
        && world.node_id == node_id
        && world.asset_host == host
        && world.asset_port == 443
}

fn create_new_server_file(
    path: &Path,
    contents: &[u8],
    before_publish: impl FnOnce(),
) -> std::io::Result<bool> {
    static NEXT_TEMP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().expect("servers.json has a file name");
    let (temp_path, mut temp_file) = loop {
        let sequence = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_path = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => break (temp_path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    let write_result = temp_file.write_all(contents);
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    before_publish();
    // The complete sibling is linked into place atomically; unlike rename,
    // hard_link fails if another initializer or operator already created path.
    let publish_result = std::fs::hard_link(&temp_path, path);
    let _ = std::fs::remove_file(&temp_path);
    match publish_result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        // Filesystems without hard links (FAT/exFAT, some shares) keep the
        // previous direct create_new publish; only there can a racing reader
        // still see a partial file.
        Err(_) => create_new_in_place(path, contents),
    }
}

fn replace_server_file_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    static NEXT_TEMP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    let permissions = std::fs::metadata(path)?.permissions();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().expect("servers.json has a file name");
    let (temp_path, mut temp_file) = loop {
        let sequence = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_path = parent.join(format!(
            ".{}.upgrade.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => break (temp_path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    let write_result = (|| {
        temp_file.write_all(contents)?;
        temp_file.sync_all()?;
        temp_file.set_permissions(permissions)?;
        temp_file.sync_all()
    })();
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

fn create_new_in_place(path: &Path, contents: &[u8]) -> std::io::Result<bool> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(contents).map(|()| true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn valid_host(host: &str) -> bool {
    host == "::1"
        || (!host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            && !host.starts_with(['.', '-'])
            && !host.ends_with(['.', '-']))
}

/// Suggested vault for a new profile. Legacy vaults and existing paths never
/// become implicit associations; an editor must require an explicit value.
pub fn proposed_vault(name: &str, bot_dir: &Path, existing: &Servers) -> Result<String, String> {
    let slug = name.to_ascii_lowercase();
    if !valid_component(&slug) {
        return Err("profile name cannot form a vault component".into());
    }
    let value = format!("vault-{slug}");
    let reserved = ["vault", "vault-289", "vault-prod"];
    if reserved
        .iter()
        .any(|legacy| legacy.eq_ignore_ascii_case(&value))
        || existing
            .servers
            .iter()
            .any(|profile| profile.vault.eq_ignore_ascii_case(&value))
        || bot_dir.join(&value).exists()
    {
        return Err(format!(
            "vault {value:?} already exists or is reserved; type an explicit vault to share it"
        ));
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "274bot-servers-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn concurrent_initializers_publish_complete_json_without_replacing_existing_file() {
        let root = TempRoot::new();
        let bot_dir = root.0.join(".274bot");
        let path = bot_dir.join("servers.json");
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let first_bot_dir = bot_dir.clone();
        let first = thread::spawn(move || {
            Servers::load_with_before_publish(&first_bot_dir, || {
                paused_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
            })
        });

        paused_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first initializer reached the publication boundary");
        let observed_before_publish = std::fs::read(&path);
        let concurrent_reader = Servers::load(&bot_dir);
        resume_tx.send(()).unwrap();

        let first = first.join().unwrap().expect("first initializer succeeds");
        assert!(
            matches!(
                &observed_before_publish,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            ),
            "a reader must see no target before atomic publication, got {observed_before_publish:?}"
        );
        let concurrent_reader =
            concurrent_reader.expect("concurrent initializer reads complete JSON");
        assert_eq!(first, concurrent_reader);
        assert_eq!(
            serde_json::from_slice::<Servers>(&std::fs::read(&path).unwrap()).unwrap(),
            first
        );

        let mut operator_servers = Servers::builtins();
        operator_servers.servers[0].name = "operator-entry".into();
        operator_servers.validate().unwrap();
        let mut operator_file = vec![b'\n'];
        operator_file.extend(serde_json::to_vec_pretty(&operator_servers).unwrap());
        operator_file.push(b'\n');
        std::fs::write(&path, &operator_file).unwrap();
        let loaded = Servers::load_with_before_publish(&bot_dir, || {
            panic!("an existing operator file must not enter initialization")
        })
        .unwrap();

        assert_eq!(loaded, operator_servers);
        assert_eq!(
            std::fs::read(path).unwrap(),
            operator_file,
            "initialization must not replace or rewrite an existing file"
        );
    }

    fn legacy_builtin_servers() -> Servers {
        let mut servers = Servers::builtins();
        servers
            .servers
            .iter_mut()
            .find(|profile| profile.name == "rs2b2t")
            .unwrap()
            .worlds
            .truncate(2);
        servers
    }

    #[test]
    fn fresh_home_gets_all_three_rs2b2t_worlds() {
        let root = TempRoot::new();
        let bot_dir = root.0.join(".274bot");
        let loaded = Servers::load(&bot_dir).unwrap();
        let profile = loaded.resolve("rs2b2t").unwrap();
        for (number, host, node_id) in [
            (1, "w1.rs2b2t.com", 10),
            (2, "w2.rs2b2t.com", 11),
            (3, "w3.rs2b2t.com", 12),
        ] {
            let world = profile
                .worlds
                .iter()
                .find(|world| world.number == number)
                .unwrap();
            assert_eq!(
                (
                    world.host.as_str(),
                    world.port,
                    world.node_id,
                    world.asset_host.as_str(),
                    world.asset_port
                ),
                (host, 443, node_id, host, 443)
            );
        }
        assert_eq!(loaded, Servers::builtins());
        assert!(!bot_dir.join("servers.json.pre-0.2.0.1").exists());
    }

    #[test]
    fn legacy_builtin_roster_is_extended_once_with_original_backup() {
        let root = TempRoot::new();
        let bot_dir = root.0.join(".274bot");
        std::fs::create_dir_all(&bot_dir).unwrap();
        let path = bot_dir.join("servers.json");
        let backup_path = bot_dir.join("servers.json.pre-0.2.0.1");
        let old_servers = legacy_builtin_servers();
        let other_profiles = old_servers.servers[1..].to_vec();
        let original = serde_json::to_vec_pretty(&old_servers).unwrap();
        std::fs::write(&path, &original).unwrap();

        let upgraded = Servers::load(&bot_dir).unwrap();
        let expected_worlds = Servers::builtins().resolve("rs2b2t").unwrap().worlds;
        assert_eq!(upgraded.resolve("rs2b2t").unwrap().worlds, expected_worlds);
        assert_eq!(upgraded.servers[1..], other_profiles);
        assert_eq!(std::fs::read(&backup_path).unwrap(), original);
        let upgraded_bytes = std::fs::read(&path).unwrap();
        assert_ne!(upgraded_bytes, original);

        let loaded_again = Servers::load(&bot_dir).unwrap();
        assert_eq!(loaded_again, upgraded);
        assert_eq!(std::fs::read(&path).unwrap(), upgraded_bytes);
        assert_eq!(std::fs::read(&backup_path).unwrap(), original);
    }

    #[test]
    fn customized_legacy_rosters_are_left_byte_for_byte_untouched() {
        let root = TempRoot::new();
        for case in ["reordered", "removed", "custom-host"] {
            let bot_dir = root.0.join(case);
            std::fs::create_dir_all(&bot_dir).unwrap();
            let path = bot_dir.join("servers.json");
            let backup_path = bot_dir.join("servers.json.pre-0.2.0.1");
            let mut customized = legacy_builtin_servers();
            let profile = customized
                .servers
                .iter_mut()
                .find(|profile| profile.name == "rs2b2t")
                .unwrap();
            match case {
                "reordered" => profile.worlds.swap(0, 1),
                "removed" => {
                    profile.worlds.remove(1);
                }
                "custom-host" => profile.worlds[0].host = "custom.example".into(),
                _ => unreachable!(),
            }
            customized.validate().unwrap();
            let original = serde_json::to_vec_pretty(&customized).unwrap();
            std::fs::write(&path, &original).unwrap();

            let loaded = Servers::load(&bot_dir).unwrap();
            assert_eq!(loaded, customized, "{case}");
            assert_eq!(std::fs::read(&path).unwrap(), original, "{case}");
            assert!(!backup_path.exists(), "{case}");
        }
    }

    #[test]
    fn invalid_servers_file_is_not_backed_up_or_rewritten() {
        let root = TempRoot::new();
        let bot_dir = root.0.join(".274bot");
        std::fs::create_dir_all(&bot_dir).unwrap();
        let path = bot_dir.join("servers.json");
        let mut invalid = legacy_builtin_servers();
        invalid.servers[0].worlds[0].port = 0;
        let original = serde_json::to_vec_pretty(&invalid).unwrap();
        std::fs::write(&path, &original).unwrap();

        assert!(Servers::load(&bot_dir).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!bot_dir.join("servers.json.pre-0.2.0.1").exists());
    }
}
