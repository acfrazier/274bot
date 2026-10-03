//! Persistent launch profiles (`~/.274bot/servers.json`).

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
            .map(|world| LaunchWorld {
                number: world.number,
                asset_host: world.host.clone(),
                asset_port: world.port,
                host: world.host,
                port: world.port,
                node_id: world.node_id,
            })
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
                    profile.worlds = worlds
                        .worlds
                        .into_iter()
                        .map(|world| LaunchWorld {
                            number: world.number,
                            asset_host: world.host.clone(),
                            asset_port: world.port,
                            host: world.host,
                            port: world.port,
                            node_id: world.node_id,
                        })
                        .collect();
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
        let servers: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("servers {}: {error}", path.display()))?;
        servers
            .validate()
            .map_err(|error| format!("servers {}: {error}", path.display()))?;
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

pub fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
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
}
