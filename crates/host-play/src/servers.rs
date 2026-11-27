//! Launch-profile codecs, validation and shipped defaults.

use std::collections::HashSet;
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

/// Failures while decoding a legacy server roster.
#[derive(Debug, thiserror::Error)]
pub enum ServersError {
    #[error("servers {path}: {source}", path = .path.display())]
    Decode {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("servers {path}: {message}", path = .path.display())]
    Invalid { path: PathBuf, message: String },
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
                local(377, 377, 43595, 81, "vault-377"),
            ],
        }
    }

    pub(crate) fn decode_legacy(path: &Path, bytes: &[u8]) -> Result<Self, ServersError> {
        let raw = std::str::from_utf8(bytes).map_err(|error| ServersError::Invalid {
            path: path.to_path_buf(),
            message: format!("saved data is not valid UTF-8: {error}"),
        })?;
        let (document, duplicate) =
            api::json::value_and_duplicate_fields(raw).map_err(|source| ServersError::Decode {
                path: path.to_path_buf(),
                source,
            })?;
        if let Some(duplicate) = duplicate {
            return Err(ServersError::Invalid {
                path: path.to_path_buf(),
                message: duplicate.to_string(),
            });
        }
        let servers: Self =
            serde_json::from_value(document).map_err(|source| ServersError::Decode {
                path: path.to_path_buf(),
                source,
            })?;
        servers
            .validate()
            .map_err(|message| ServersError::Invalid {
                path: path.to_path_buf(),
                message,
            })?;
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
            .ok_or_else(|| format!("unknown server profile {name:?} in the durable server roster"))
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
            if !matches!(profile.revision, 274 | 289 | 377) || profile.worlds.is_empty() {
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

    fn serialized(servers: &Servers) -> Vec<u8> {
        serde_json::to_vec(servers).unwrap()
    }

    #[test]
    fn legacy_codec_preserves_profile_fields_without_augmenting_roster() {
        let builtins = Servers::builtins();
        let mut engine_profile = builtins.servers[1].clone();
        engine_profile.name = "local-engine".into();
        engine_profile.vault = "vault-engine".into();
        engine_profile.login_key = LoginKey::EngineDir {
            engine_dir: Some(PathBuf::from("engine/289")),
        };

        let mut inline_profile = builtins.servers[2].clone();
        inline_profile.name = "remote-engine".into();
        inline_profile.vault = "vault-remote".into();
        inline_profile.login_key = LoginKey::Inline {
            modulus: "ab".into(),
            exponent: "10001".into(),
        };
        inline_profile.worlds[0].host = "game.example".into();
        inline_profile.worlds[0].asset_host = "assets.example".into();
        inline_profile.members = Some(true);
        inline_profile.allow_plaintext_offhost = true;

        let mut served_profile = builtins.servers[0].clone();
        served_profile.name = "public".into();
        served_profile.vault = "vault-public".into();

        let servers = Servers {
            schema_version: 1,
            servers: vec![engine_profile, inline_profile, served_profile],
        };
        let bytes = serialized(&servers);

        assert_eq!(
            Servers::decode_legacy(Path::new("servers.json"), &bytes).unwrap(),
            servers
        );
    }
    #[test]
    fn legacy_codec_rejects_nested_duplicate_members_before_untagged_projection() {
        let duplicate_login_key = br#"{
            "schema_version":1,
            "servers":[{
                "name":"custom","revision":289,"transport":"tcp",
                "worlds":[{"number":1,"host":"localhost","port":43594,"node_id":1,
                    "asset_host":"localhost","asset_port":80}],
                "login_key":{"modulus":"first","modulus":"second","exponent":"10001"},
                "vault":"vault-custom","members":null,"allow_plaintext_offhost":false
            }]
        }"#;
        let duplicate_world = br#"{
            "schema_version":1,
            "servers":[{
                "name":"custom","revision":289,"transport":"tcp",
                "worlds":[{"number":1,"number":2,"host":"localhost","port":43594,
                    "node_id":1,"asset_host":"localhost","asset_port":80}],
                "login_key":"served","vault":"vault-custom","members":null,
                "allow_plaintext_offhost":false
            }]
        }"#;

        for (bytes, field) in [
            (duplicate_login_key.as_slice(), "modulus"),
            (duplicate_world.as_slice(), "number"),
        ] {
            let error = Servers::decode_legacy(Path::new("servers.json"), bytes)
                .expect_err("duplicate members must refuse the roster");
            assert!(
                error.to_string().contains(field),
                "expected the duplicate field {field:?}, got {error}"
            );
        }
    }

    #[test]
    fn legacy_codec_keeps_schema_and_profile_validation() {
        let mut invalid_rosters = Vec::new();

        let mut invalid = Servers::builtins();
        invalid.schema_version = 2;
        invalid_rosters.push(("schema version", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers.clear();
        invalid_rosters.push(("empty roster", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[0].name = "PUBLIC-289".into();
        invalid_rosters.push(("reserved alias", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[0].name = "../profile".into();
        invalid_rosters.push(("unsafe profile name", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].name = "LOCAL-274".into();
        invalid_rosters.push(("duplicate profile name", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].vault = "VAULT-PROD".into();
        invalid_rosters.push(("duplicate vault", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].vault = "../vault".into();
        invalid_rosters.push(("unsafe vault name", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[0].allow_plaintext_offhost = true;
        invalid_rosters.push(("WSS plaintext opt-in", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].login_key = LoginKey::Named("other".into());
        invalid_rosters.push(("unsupported named key", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].revision = 200;
        invalid_rosters.push(("unsupported revision", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].worlds.clear();
        invalid_rosters.push(("empty world list", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].worlds[0].port = 0;
        invalid_rosters.push(("invalid world port", invalid));

        let mut invalid = Servers::builtins();
        invalid.servers[1].worlds[0].host = "-bad.example".into();
        invalid_rosters.push(("invalid world host", invalid));

        let mut invalid = Servers::builtins();
        let duplicate_world = invalid.servers[1].worlds[0].clone();
        invalid.servers[1].worlds.push(duplicate_world);
        invalid_rosters.push(("duplicate world number", invalid));

        for (case, servers) in invalid_rosters {
            assert!(servers.validate().is_err(), "{case}");
            assert!(
                Servers::decode_legacy(Path::new("servers.json"), &serialized(&servers)).is_err(),
                "legacy decoding must retain {case} validation"
            );
        }
    }
}
